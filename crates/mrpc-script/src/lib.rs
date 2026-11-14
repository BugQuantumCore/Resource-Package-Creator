//! Train-script runtime for the MTR Resource Package Creator.
//!
//! Two engines, mirroring the JCM (Yanyang) hybrid model:
//!
//! * **JavaScript** — [`JsSandbox`] embeds QuickJS and exposes a JCM-compatible
//!   subset of the vehicle script API (`create`/`render`/`dispose` lifecycle,
//!   `include`, `console`, `Timing`, `SCRIPT_INPUT`, and a `VehicleWrapper`
//!   built from simulated train data).
//! * **C++** — [`NativePreview`] `dlopen`s a module compiled against
//!   `cpp/mrpc_script.hpp` (`MRPC_REGISTER_VEHICLE_SCRIPT`) and drives its
//!   create/render/dispose lifecycle with the same snapshot data.
//!
//! Both are *preview* runtimes: scripts intended for gameplay must ultimately
//! run inside Minecraft with JCM installed; the creator exists to write, test
//! and package them.

use std::cell::RefCell;
use std::ffi::{c_char, c_double, c_void, CStr};
use std::os::raw::{c_int as RawCInt};
use std::path::Path;
use std::rc::Rc;

use rquickjs::prelude::Opt;
use rquickjs::{Context, Ctx, Function, Object, Runtime, Value};

// ---------------------------------------------------------------------------
// Shared simulated train state
// ---------------------------------------------------------------------------

/// Simulated train data fed to both script engines each frame.
#[derive(Debug, Clone)]
pub struct TrainSimState {
    pub speed_kmh: f64,
    pub door_value: f64,
    pub door_opening: bool,
    /// Traction notch (positive = power, negative = brake, 0 = coast).
    pub notch: i32,
    pub car_count: i32,
    /// `"not"` | `"left"` | `"right"` | `"all"`.
    pub door_direction: String,
    pub reversed: bool,
    pub on_route: bool,
    pub total_dwell_ms: i64,
    pub elapsed_dwell_ms: i64,
    pub route_stops: Vec<String>,
    pub next_stop_index: usize,
    pub sim_time_ms: f64,
    pub frame_delta_ms: f64,
}

impl Default for TrainSimState {
    fn default() -> Self {
        Self {
            speed_kmh: 0.0,
            door_value: 0.0,
            door_opening: false,
            notch: 0,
            car_count: 8,
            door_direction: "all".to_string(),
            reversed: false,
            on_route: true,
            total_dwell_ms: 20_000,
            elapsed_dwell_ms: 0,
            route_stops: vec![
                "Central".to_string(),
                "Admiralty".to_string(),
                "Mong Kok".to_string(),
                "Kowloon Tong".to_string(),
            ],
            next_stop_index: 1,
            sim_time_ms: 0.0,
            frame_delta_ms: 16.0,
        }
    }
}

impl TrainSimState {
    fn length_of(&self, _car: usize) -> f64 {
        20.0
    }
    fn width_of(&self, _car: usize) -> f64 {
        2.5
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("JavaScript error: {0}")]
    Js(String),
    #[error("native module error: {0}")]
    Native(String),
}

// ---------------------------------------------------------------------------
// JavaScript sandbox (QuickJS)
// ---------------------------------------------------------------------------

type Shared<T> = Rc<RefCell<T>>;

const STRINGIFY_HELPER: &str = r#"
globalThis.__mrpc_stringify = function(v) {
    try {
        if (typeof v === "string") return v;
        if (v === null) return "null";
        if (v === undefined) return "undefined";
        if (typeof v === "object") return JSON.stringify(v);
        return String(v);
    } catch (e) { return String(v); }
};
globalThis.print = function(v) { __mrpc_log(__mrpc_stringify(v)); };
globalThis.console = {
    log:   function(v) { __mrpc_log("[log] "   + __mrpc_stringify(v)); },
    info:  function(v) { __mrpc_log("[info] "  + __mrpc_stringify(v)); },
    warn:  function(v) { __mrpc_log("[warn] "  + __mrpc_stringify(v)); },
    error: function(v) { __mrpc_log("[error] " + __mrpc_stringify(v)); }
};
"#;

/// A QuickJS sandbox running one vehicle script entry.
pub struct JsSandbox {
    ctx: Context,
    logs: Shared<Vec<String>>,
    sim: Shared<TrainSimState>,
    created: bool,
    disposed: bool,
}

impl JsSandbox {
    /// Create a sandbox and evaluate the entry's scripts.
    ///
    /// * `sources` — `(resource id, code)` pairs for every JS file of the entry.
    /// * `prepend` — `prependExpressions` strings.
    /// * `input` — the entry's `input` JSON (exposed as `SCRIPT_INPUT`).
    pub fn new(
        sources: &[(String, String)],
        prepend: &[String],
        input: Option<&serde_json::Value>,
    ) -> Result<JsSandbox, ScriptError> {
        let rt = Runtime::new().map_err(|e| ScriptError::Js(e.to_string()))?;
        let ctx = Context::full(&rt).map_err(|e| ScriptError::Js(e.to_string()))?;
        let logs: Shared<Vec<String>> = Rc::new(RefCell::new(Vec::new()));
        let sim: Shared<TrainSimState> = Rc::new(RefCell::new(TrainSimState::default()));
        let source_map: Shared<Vec<(String, String)>> = Rc::new(RefCell::new(sources.to_vec()));

        ctx.with(|ctx| -> Result<(), rquickjs::Error> {
            let globals = ctx.globals();

            // Stringify helper + console/print, implemented in JS on top of the
            // native __mrpc_log(String) sink.
            let logs_for_sink = logs.clone();
            let log_sink = Function::new(ctx.clone(), move |s: String| {
                logs_for_sink.borrow_mut().push(s);
            })?;
            globals.set("__mrpc_log", log_sink)?;
            ctx.eval::<Value, _>(STRINGIFY_HELPER)?;

            // Timing ------------------------------------------------------------
            let timing = Object::new(ctx.clone())?;
            let sim_millis = sim.clone();
            timing.set(
                "millis",
                Function::new(ctx.clone(), move || sim_millis.borrow().sim_time_ms)?,
            )?;
            let sim_delta = sim.clone();
            timing.set(
                "delta",
                Function::new(ctx.clone(), move || sim_delta.borrow().frame_delta_ms)?,
            )?;
            globals.set("Timing", timing)?;

            // include() ---------------------------------------------------------
            let sources_for_include = source_map.clone();
            let include_fn = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, id: String| -> Result<(), rquickjs::Error> {
                    let code = sources_for_include
                        .borrow()
                        .iter()
                        .find(|(rid, _)| rid == &id)
                        .map(|(_, code)| code.clone());
                    match code {
                        Some(code) => {
                            ctx.eval::<Value, _>(code.as_str())?;
                            Ok(())
                        }
                        None => {
                            ctx.eval::<Value, _>(format!("throw new Error(\"include(): unknown resource {id}\");").as_str())?;
                            Ok(())
                        }
                    }
                },
            )?;
            globals.set("include", include_fn)?;

            // Resources.id() passthrough (the JCM id object stringifies to itself).
            let resources = Object::new(ctx.clone())?;
            resources.set("id", Function::new(ctx.clone(), |s: String| s)?)?;
            globals.set("Resources", resources)?;

            // TextUtil.cycleString("A|B", ms?) ---------------------------------
            let textutil = Object::new(ctx.clone())?;
            let sim_cycle = sim.clone();
            textutil.set(
                "cycleString",
                Function::new(ctx.clone(), move |s: String, duration: Opt<f64>| {
                    let period = duration.0.unwrap_or(2000.0).max(1.0);
                    let t = sim_cycle.borrow().sim_time_ms;
                    let parts: Vec<&str> = s.split('|').collect();
                    if parts.is_empty() {
                        return s;
                    }
                    let idx = ((t / period) as usize) % parts.len();
                    parts[idx].to_string()
                })?,
            )?;
            globals.set("TextUtil", textutil)?;

            // SCRIPT_INPUT --------------------------------------------------------
            if let Some(input) = input {
                let text = serde_json::to_string(input).unwrap_or_else(|_| "null".into());
                ctx.eval::<Value, _>(format!("globalThis.SCRIPT_INPUT = ({text});").as_str())?;
            }

            // Prepend expressions, then the scripts themselves.
            for expr in prepend {
                ctx.eval::<Value, _>(expr.as_str())?;
            }
            for (_, code) in sources {
                ctx.eval::<Value, _>(code.as_str())?;
            }
            Ok(())
        })
        .map_err(|e| ScriptError::Js(e.to_string()))?;

        Ok(JsSandbox { ctx, logs, sim, created: false, disposed: false })
    }

    /// Update the simulated train state (speed, doors, notch…).
    pub fn set_sim(&mut self, state: TrainSimState) {
        *self.sim.borrow_mut() = state;
    }

    /// Build the `vehicle` object from the current simulation state.
    fn vehicle_object<'js>(
        ctx: &Ctx<'js>,
        sim: &TrainSimState,
    ) -> Result<Object<'js>, rquickjs::Error> {
        let vehicle = Object::new(ctx.clone())?;
        let snap = sim.clone();

        macro_rules! method0 {
            ($js_name:expr, $body:expr) => {{
                let snap_here = snap.clone();
                vehicle.set(
                    $js_name,
                    Function::new(ctx.clone(), move || {
                        let f = $body;
                        f(&snap_here)
                    })?,
                )?;
            }};
        }

        method0!("getSpeedKmh", |s: &TrainSimState| s.speed_kmh);
        method0!("getSpeedMs", |s: &TrainSimState| s.speed_kmh / 3.6);
        method0!("getDoorValue", |s: &TrainSimState| s.door_value);
        method0!("isDoorOpening", |s: &TrainSimState| s.door_opening);
        method0!("getNotchLevel", |s: &TrainSimState| s.notch as f64);
        method0!("getNotchPosition", |s: &TrainSimState| s.notch as f64);
        method0!("getCarCount", |s: &TrainSimState| s.car_count as f64);
        method0!("getTransportType", |_: &TrainSimState| "train".to_string());
        method0!("getTransportMode", |_: &TrainSimState| "TRAIN".to_string());
        method0!("getDirectionOfDoorOpening", |s: &TrainSimState| s.door_direction.clone());
        method0!("isReversed", |s: &TrainSimState| s.reversed);
        method0!("isOnRoute", |s: &TrainSimState| s.on_route);
        method0!("isClientPlayerRiding", |_: &TrainSimState| false);
        method0!("getRailProgress", |s: &TrainSimState| s.elapsed_dwell_ms.max(0) as f64);
        method0!("getTotalDwellTime", |s: &TrainSimState| s.total_dwell_ms as f64);
        method0!("getElapsedDwellTime", |s: &TrainSimState| s.elapsed_dwell_ms as f64);
        method0!("getId", |_: &TrainSimState| 1.0f64);

        // getLength / getWidth take a car index.
        let snap_for_len = sim.clone();
        vehicle.set(
            "getLength",
            Function::new(ctx.clone(), move |car: f64| snap_for_len.length_of(car as usize))?,
        )?;
        let snap_for_width = sim.clone();
        vehicle.set(
            "getWidth",
            Function::new(ctx.clone(), move |car: f64| snap_for_width.width_of(car as usize))?,
        )?;

        // Stops: objects with a `name` property and `getName()` / `destination`.
        let make_stops =
            |ctx: &Ctx<'js>, slice: &[String], eta: bool| -> Result<Value<'js>, rquickjs::Error> {
                let arr = rquickjs::Array::new(ctx.clone())?;
                for (i, name) in slice.iter().enumerate() {
                    let stop = Object::new(ctx.clone())?;
                    stop.set("name", name.clone())?;
                    stop.set("destination", name.clone())?;
                    let name_for_fn = name.clone();
                    stop.set(
                        "getName",
                        Function::new(ctx.clone(), move || name_for_fn.clone())?,
                    )?;
                    let eta_ms = if eta { (i as f64) * 120_000.0 } else { -1.0 };
                    stop.set(
                        "getArrivalTime",
                        Function::new(ctx.clone(), move || eta_ms)?,
                    )?;
                    arr.set(i, stop)?;
                }
                Ok(arr.into_value())
            };
        let next_idx = sim.next_stop_index.min(sim.route_stops.len().saturating_sub(1));
        let remaining: Vec<String> = sim.route_stops[next_idx.min(sim.route_stops.len())..].to_vec();
        let all_stops = make_stops(ctx, &sim.route_stops, false)?;
        vehicle.set(
            "getStops",
            Function::new(ctx.clone(), move || all_stops.clone())?,
        )?;
        let this_route = make_stops(ctx, &sim.route_stops, false)?;
        vehicle.set(
            "getThisRouteStops",
            Function::new(ctx.clone(), move || this_route.clone())?,
        )?;
        let next_route = make_stops(ctx, &remaining, true)?;
        vehicle.set(
            "getNextRouteStops",
            Function::new(ctx.clone(), move || next_route.clone())?,
        )?;
        Ok(vehicle)
    }

    /// Call the script's global lifecycle functions (if declared).
    fn call_lifecycle(&mut self, name: &str) -> Result<(), ScriptError> {
        let logs = self.logs.clone();
        let sim = self.sim.borrow().clone();
        let result = self.ctx.with(|ctx| -> Result<(), rquickjs::Error> {
            let globals = ctx.globals();
            let callback: Option<Function> = globals.get(name).ok();
            let Some(callback) = callback else {
                return Ok(()); // lifecycle functions are optional
            };

            // The `ctx` argument of the JCM signature: a small object.
            let ctx_obj = Object::new(ctx.clone())?;
            ctx_obj.set("simTime", sim.sim_time_ms)?;

            // Persistent `state` object lives on globalThis.
            let state: Object = match globals.get::<_, Object>("__mrpc_state") {
                Ok(o) => o,
                Err(_) => {
                    let o = Object::new(ctx.clone())?;
                    globals.set("__mrpc_state", o.clone())?;
                    o
                }
            };

            let vehicle = Self::vehicle_object(&ctx, &sim)?;
            callback.call::<_, ()>((ctx_obj, state, vehicle))
        });
        if let Err(e) = result {
            logs.borrow_mut().push(format!("[error] {name}: {e}"));
            return Err(ScriptError::Js(e.to_string()));
        }
        Ok(())
    }

    /// Run `create` once.
    pub fn create(&mut self) -> Result<(), ScriptError> {
        if !self.created {
            self.created = true;
            self.call_lifecycle("create")?;
        }
        Ok(())
    }

    /// Run `render` with the current simulation state.
    pub fn render(&mut self) -> Result<(), ScriptError> {
        if !self.created {
            self.create()?;
        }
        if !self.disposed {
            self.call_lifecycle("render")?;
        }
        Ok(())
    }

    /// Run `dispose` once.
    pub fn dispose(&mut self) -> Result<(), ScriptError> {
        if self.created && !self.disposed {
            self.disposed = true;
            self.call_lifecycle("dispose")?;
        }
        Ok(())
    }

    /// Take console output accumulated so far.
    pub fn take_output(&mut self) -> Vec<String> {
        let mut logs = self.logs.borrow_mut();
        std::mem::take(&mut *logs)
    }

    /// Does the script declare a given global function?
    pub fn has_function(&self, name: &str) -> bool {
        self.ctx.with(|ctx| {
            let globals = ctx.globals();
            let v: Result<Function, _> = globals.get(name);
            v.is_ok()
        })
    }

    /// Evaluate a maintenance expression in the sandbox (used by the editor's
    /// "run selection" feature). Returns the value as a string.
    pub fn eval_expression(&mut self, code: &str) -> Result<String, ScriptError> {
        self.ctx
            .with(|ctx| -> Result<String, rquickjs::Error> {
                let v: Value = ctx.eval(code)?;
                let helper: Function = ctx.globals().get("__mrpc_stringify")?;
                let s: String = helper.call((v,))?;
                Ok(s)
            })
            .map_err(|e| ScriptError::Js(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Native (C++) module preview host
// ---------------------------------------------------------------------------

/// `mrpc::VehicleSnapshot` — must stay layout-compatible with
/// `crates/mrpc-script/cpp/mrpc_script.hpp`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NativeVehicleSnapshot {
    pub speed_kmh: c_double,
    pub speed_ms: c_double,
    pub door_value: c_double,
    pub door_opening: RawCInt,
    pub notch: RawCInt,
    pub car_count: RawCInt,
    /// 0 = not, 1 = left, 2 = right, 3 = all.
    pub door_direction: i8,
    pub reversed: RawCInt,
    pub on_route: RawCInt,
    pub total_dwell_ms: i64,
    pub elapsed_dwell_ms: i64,
}

impl From<&TrainSimState> for NativeVehicleSnapshot {
    fn from(s: &TrainSimState) -> Self {
        let dir = match s.door_direction.as_str() {
            "left" => 1,
            "right" => 2,
            "all" => 3,
            _ => 0,
        };
        Self {
            speed_kmh: s.speed_kmh,
            speed_ms: s.speed_kmh / 3.6,
            door_value: s.door_value,
            door_opening: s.door_opening as RawCInt,
            notch: s.notch,
            car_count: s.car_count,
            door_direction: dir,
            reversed: s.reversed as RawCInt,
            on_route: s.on_route as RawCInt,
            total_dwell_ms: s.total_dwell_ms,
            elapsed_dwell_ms: s.elapsed_dwell_ms,
        }
    }
}

/// Host services passed to the native module (layout of `mrpc::Host` is a
/// vtable-free C struct here; the C++ side receives it as an abstract class
/// implemented over these two pointers).
#[repr(C)]
struct NativeHost {
    log: unsafe extern "C" fn(user: *mut c_void, msg: *const c_char),
    display: unsafe extern "C" fn(user: *mut c_void, text: *const c_char),
    user: *mut c_void,
}

struct HostState {
    logs: Vec<String>,
    display: String,
}

unsafe extern "C" fn host_log(user: *mut c_void, msg: *const c_char) {
    if msg.is_null() {
        return;
    }
    let state = &mut *(user as *mut HostState);
    if let Ok(s) = CStr::from_ptr(msg).to_str() {
        state.logs.push(s.to_string());
    }
}

unsafe extern "C" fn host_display(user: *mut c_void, text: *const c_char) {
    if text.is_null() {
        return;
    }
    let state = &mut *(user as *mut HostState);
    if let Ok(s) = CStr::from_ptr(text).to_str() {
        state.display = s.to_string();
    }
}

/// A loaded native (C++) vehicle script module.
///
/// The module must be compiled against `cpp/mrpc_script.hpp` and export the
/// `mrpc_module_*` C ABI (abi version 1).
pub struct NativePreview {
    lib: libloading::Library,
    instance: *mut c_void,
    host_state: *mut HostState,
    abi: i32,
    id: String,
}

impl NativePreview {
    /// Load and create a module instance.
    ///
    /// # Safety
    /// The library at `path` must be a valid `mrpc_module_*` module; loading
    /// arbitrary shared libraries is inherently unsafe.
    pub unsafe fn load(path: &Path, sim: &TrainSimState) -> Result<NativePreview, ScriptError> {
        let lib = libloading::Library::new(path)
            .map_err(|e| ScriptError::Native(format!("dlopen {}: {e}", path.display())))?;

        let abi_fn: libloading::Symbol<unsafe extern "C" fn() -> RawCInt> =
            lib.get(b"mrpc_module_abi\0").map_err(|e| ScriptError::Native(e.to_string()))?;
        let abi = abi_fn();
        if abi != 1 {
            return Err(ScriptError::Native(format!("unsupported module ABI {abi} (expected 1)")));
        }

        let id_fn: libloading::Symbol<unsafe extern "C" fn() -> *const c_char> =
            lib.get(b"mrpc_module_id\0").map_err(|e| ScriptError::Native(e.to_string()))?;
        let id_ptr = id_fn();
        let id = if id_ptr.is_null() {
            String::new()
        } else {
            CStr::from_ptr(id_ptr).to_string_lossy().into_owned()
        };

        let host_state = Box::into_raw(Box::new(HostState { logs: Vec::new(), display: String::new() }));
        let host = NativeHost { log: host_log, display: host_display, user: host_state as *mut c_void };

        let create_fn: libloading::Symbol<
            unsafe extern "C" fn(*const NativeVehicleSnapshot, *const NativeHost) -> *mut c_void,
        > = lib.get(b"mrpc_module_create\0").map_err(|e| ScriptError::Native(e.to_string()))?;
        let snapshot = NativeVehicleSnapshot::from(sim);
        let instance = create_fn(&snapshot, &host);
        if instance.is_null() {
            let _ = Box::from_raw(host_state);
            return Err(ScriptError::Native("mrpc_module_create returned null".into()));
        }

        Ok(NativePreview { lib, instance, host_state, abi, id })
    }

    pub fn module_id(&self) -> &str {
        &self.id
    }

    pub fn abi(&self) -> i32 {
        self.abi
    }

    /// Feed one frame of simulated data.
    ///
    /// # Safety
    /// The instance must not have been disposed.
    pub unsafe fn render(&mut self, sim: &TrainSimState) -> Result<(), ScriptError> {
        let render_fn: libloading::Symbol<
            unsafe extern "C" fn(*mut c_void, *const NativeVehicleSnapshot),
        > = self
            .lib
            .get(b"mrpc_module_render\0")
            .map_err(|e| ScriptError::Native(e.to_string()))?;
        let snapshot = NativeVehicleSnapshot::from(sim);
        render_fn(self.instance, &snapshot);
        Ok(())
    }

    /// Console output since the last take.
    pub fn take_output(&mut self) -> Vec<String> {
        if self.host_state.is_null() {
            return Vec::new();
        }
        unsafe { std::mem::take(&mut (*self.host_state).logs) }
    }

    /// Last `host().display(text)` string, if any.
    pub fn take_display(&mut self) -> String {
        if self.host_state.is_null() {
            return String::new();
        }
        unsafe { std::mem::take(&mut (*self.host_state).display) }
    }
}

impl Drop for NativePreview {
    fn drop(&mut self) {
        unsafe {
            if !self.instance.is_null() {
                if let Ok(dispose_fn) = self.lib.get::<libloading::Symbol<
                    unsafe extern "C" fn(*mut c_void, *const NativeVehicleSnapshot),
                >>(b"mrpc_module_dispose\0")
                {
                    let snapshot = NativeVehicleSnapshot::from(&TrainSimState::default());
                    dispose_fn(self.instance, &snapshot);
                }
            }
            if !self.host_state.is_null() {
                drop(Box::from_raw(self.host_state));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

/// A starter JavaScript vehicle script (JCM-compatible lifecycle).
pub const JS_TEMPLATE: &str = r#"// Vehicle script for Joban Client Mod (MTR Scripting).
// Lifecycle: create(ctx, state, vehicle) -> render(...) -> dispose(...)
// The `vehicle` wrapper exposes getSpeedKmh(), getDoorValue(), isDoorOpening(),
// getNotchLevel(), getCarCount(), getStops(), getDirectionOfDoorOpening(), ...

// SCRIPT_INPUT comes from the entry's "input" JSON in mtr_custom_resources.json.
const ROUTE_NAME = SCRIPT_INPUT && SCRIPT_INPUT.routeName ? SCRIPT_INPUT.routeName : "Demo Line";

function create(ctx, state, vehicle) {
    state.frame = 0;
    state.lastDoor = 0;
    print("[" + ROUTE_NAME + "] script created, cars: " + vehicle.getCarCount());
}

function render(ctx, state, vehicle) {
    state.frame++;
    const door = vehicle.getDoorValue();
    if (door !== state.lastDoor && door > 0) {
        print("doors opening: " + Math.round(door * 100) + "%");
    }
    state.lastDoor = door;
    if (state.frame % 60 === 0) {
        print("speed " + vehicle.getSpeedKmh().toFixed(1) + " km/h, notch " + vehicle.getNotchLevel());
    }
}

function dispose(ctx, state, vehicle) {
    print("script disposed after " + state.frame + " frames");
}
"#;

/// A starter C++ vehicle script for the preview SDK (`cpp/mrpc_script.hpp`).
pub const CPP_TEMPLATE: &str = r#"// Native vehicle script for the MTR Resource Package Creator preview SDK.
// Compile:   g++ -std=c++17 -O2 -shared -fPIC my_lcd.cpp -o my_lcd.so
// (Windows: cl /std:c++17 /LD my_lcd.cpp /Fe:my_lcd.dll)
// Then reference the library from a vehicleScripts entry's scriptLocations.
#include "mrpc_script.hpp"
#include <string>

struct State {
    int frames = 0;
    double lastDoor = 0;
};

struct MyLcd : mrpc::VehicleScript<State> {
    static constexpr auto ID = "my_pack:my_lcd";
    using State = ::State;

    void create(const mrpc::VehicleSnapshot& s, State& st) override {
        st.frames = 0;
        host().log("native script created, cars: " + std::to_string(s.car_count));
    }

    void render(const mrpc::VehicleSnapshot& s, State& st) override {
        st.frames++; if (s.door_value != st.lastDoor && s.door_value > 0) {
            host().log("doors opening");
        }
        st.lastDoor = s.door_value;
        if (st.frames % 60 == 0) {
            host().display("Speed: " + std::to_string((int)s.speed_kmh) + " km/h  Door: "
                           + std::to_string((int)(s.door_value * 100)) + "%");
        }
    }

    void dispose(const mrpc::VehicleSnapshot& s, State& st) override {
        host().log("native script disposed after " + std::to_string(st.frames) + " frames");
    }
};

MRPC_REGISTER_VEHICLE_SCRIPT(MyLcd)
"#;

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r#"
        print("hello " + (1 + 1));
        const input = typeof SCRIPT_INPUT !== "undefined" ? SCRIPT_INPUT : {};
        function create(ctx, state, vehicle) { state.n = 0; print("created, input=" + JSON.stringify(input)); }
        function render(ctx, state, vehicle) { state.n++; state.speed = vehicle.getSpeedKmh(); }
        function dispose(ctx, state, vehicle) { print("frames=" + state.n + " speed=" + state.speed); }
    "#;

    #[test]
    fn lifecycle_and_console() {
        let mut sb = JsSandbox::new(&[("mtr:pack/js/main.js".into(), SCRIPT.into())], &[], None).unwrap();
        sb.create().unwrap();
        let sim = TrainSimState { speed_kmh: 42.5, ..Default::default() };
        sb.set_sim(sim);
        sb.render().unwrap();
        sb.render().unwrap();
        sb.dispose().unwrap();
        let out = sb.take_output();
        assert!(out.iter().any(|l| l.contains("hello 2")));
        assert!(out.iter().any(|l| l.contains("frames=2")));
        assert!(out.iter().any(|l| l.contains("speed=42.5")));
    }

    #[test]
    fn script_input_visible() {
        let input = serde_json::json!({ "yearEra": 2004 });
        let mut sb = JsSandbox::new(
            &[("x.js".into(), "function render(c,s,v){ print('era=' + SCRIPT_INPUT.yearEra); }".into())],
            &[],
            Some(&input),
        )
        .unwrap();
        sb.render().unwrap();
        assert!(sb.take_output().iter().any(|l| l.contains("era=2004")));
    }

    #[test]
    fn include_resolves_sources() {
        let sources = vec![
            ("mtr:pack/js/lib.js".to_string(), "globalThis.libValue = 7;".to_string()),
            ("mtr:pack/js/main.js".to_string(), "include(Resources.id('mtr:pack/js/lib.js')); function render(c,s,v){ print('lib=' + libValue); }".to_string()),
        ];
        let mut sb = JsSandbox::new(&sources, &[], None).unwrap();
        sb.render().unwrap();
        assert!(sb.take_output().iter().any(|l| l.contains("lib=7")));
    }

    #[test]
    fn timing_and_cycle() {
        let code = r#"
            function render(ctx, state, vehicle) {
                print("t=" + Math.floor(Timing.millis()) + " stop=" + TextUtil.cycleString("Alpha|Beta"));
            }
        "#;
        let mut sb = JsSandbox::new(&[("x.js".into(), code.into())], &[], None).unwrap();
        sb.set_sim(TrainSimState { sim_time_ms: 3000.0, ..Default::default() });
        sb.render().unwrap();
        let out = sb.take_output().join(" ");
        assert!(out.contains("t=3000"));
        assert!(out.contains("stop=Alpha") || out.contains("stop=Beta"));
    }

    #[test]
    fn vehicle_wrapper_api() {
        let code = r#"
            function render(ctx, state, vehicle) {
                print("dir=" + vehicle.getDirectionOfDoorOpening()
                    + " cars=" + vehicle.getCarCount()
                    + " stops=" + vehicle.getStops().length
                    + " next=" + vehicle.getThisRouteStops()[0].getName());
            }
        "#;
        let mut sb = JsSandbox::new(&[("x.js".into(), code.into())], &[], None).unwrap();
        sb.render().unwrap();
        let out = sb.take_output().join(" ");
        assert!(out.contains("dir=all"));
        assert!(out.contains("cars=8"));
        assert!(out.contains("stops=4"));
        assert!(out.contains("next=Central"));
    }

    #[test]
    fn optional_lifecycle_missing_is_fine() {
        let mut sb = JsSandbox::new(&[("x.js".into(), "globalThis.just_var = 1;".into())], &[], None).unwrap();
        sb.create().unwrap();
        sb.render().unwrap();
        sb.dispose().unwrap();
    }

    #[test]
    fn js_error_reported() {
        let mut sb = JsSandbox::new(&[("x.js".into(), "function render(){ nope(); }".into())], &[], None).unwrap();
        assert!(sb.render().is_err());
    }
}
