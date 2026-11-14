//! The application: layout orchestration and pack actions.

use std::sync::Arc;

use egui::{Color32, RichText};

use mrpc_core::i18n::{t, tf, Locale};
use mrpc_script::JsSandbox;
use mrpc_vvvf::{goertzel_magnitude, write_wav_mono16, Engine};

use crate::state::{AppConfig, EditorState, Selection};
use crate::tabs::{bottom_tabs, BottomTab};
use crate::viewport::{ViewportCallback, ViewportShared};

pub struct MrpcApp {
    pub state: EditorState,
    pub viewport: Arc<ViewportShared>,
    pub bottom_tab: BottomTab,
    /// Scene version — bumped whenever the model or properties change.
    scene_version: u64,
    viewport_version: u64,
    last_texture_size: Option<(u32, u32)>,
    /// Deferred open request: (path, is_zip).
    pending_open: Option<(String, bool)>,
    /// Path prompt for File → Open.
    show_open_dialog: Option<bool>,
    #[cfg(feature = "audio")]
    pub audio: Option<crate::audio::AudioOut>,
    /// VVVF preview engine (waveform/spectrum without audio hardware).
    preview_engine: Engine,
}

impl MrpcApp {
    pub fn new(cc: &eframe::CreationContext<'_>, locale: Locale, config: AppConfig) -> MrpcApp {
        // Theme: Blender-ish dark.
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(43, 43, 46);
        visuals.window_fill = Color32::from_rgb(50, 50, 53);
        visuals.extreme_bg_color = Color32::from_rgb(24, 24, 26);
        visuals.faint_bg_color = Color32::from_rgb(35, 35, 38);
        visuals.hyperlink_color = Color32::from_rgb(254, 197, 229);
        visuals.selection.stroke.color = Color32::from_rgb(254, 197, 229);
        cc.egui_ctx.set_visuals(visuals);
        install_fonts(&cc.egui_ctx);

        let target_format = cc
            .wgpu_render_state
            .as_ref()
            .map(|rs| rs.target_format)
            .unwrap_or(egui_wgpu::wgpu::TextureFormat::Bgra8Unorm);

        MrpcApp {
            state: EditorState::new(locale, config),
            viewport: ViewportShared::new(target_format),
            bottom_tab: BottomTab::Anim,
            scene_version: 0,
            viewport_version: u64::MAX,
            last_texture_size: None,
            pending_open: None,
            show_open_dialog: None,
            #[cfg(feature = "audio")]
            audio: crate::audio::AudioOut::try_start().ok(),
            preview_engine: Engine::new(mrpc_vvvf::VvvfParams::default()),
        }
    }

    // ------------------------------------------------------------------ actions

    pub fn reload_model(&mut self) {
        self.state.reload_model();
        self.scene_version += 1;
        self.state.vvvf.bve = None;
        self.state.vvvf.bve_fundamental = None;
    }

    /// Write the edited model properties back into the pack file.
    pub fn save_properties_back(&mut self) {
        let Some(vi) = self.state.selected_vehicle() else { return };
        let Some(loaded) = self.state.model.as_ref() else { return };
        let resource = self
            .state
            .pack
            .resources
            .vehicles
            .get(vi)
            .and_then(|v| v.models.first())
            .map(|m| m.model_properties_resource.clone());
        let Some(resource) = resource else { return };
        let json = serde_json::to_vec_pretty(&loaded.properties).unwrap_or_default();
        self.state.pack.set_resource(&resource, "json", json);
        self.state.dirty = true;
        self.scene_version += 1;
    }

    /// Generate one part from all OBJ part names (MTR `addPartsIfEmpty`).
    pub fn auto_parts_from_model(&mut self) {
        let names: Vec<String> = self
            .state
            .model
            .as_ref()
            .map(|m| m.obj.parts.iter().map(|p| p.name.clone()).collect())
            .unwrap_or_default();
        if let Some(loaded) = self.state.model.as_mut() {
            let mut names = names;
            loaded.properties.add_parts_if_empty(&mut names);
            self.save_properties_back();
        }
    }

    /// Load the selected script entry into the editor buffers.
    pub fn load_script_editor(&mut self) {
        let si = match self.state.selection {
            Selection::Script(si) => Some(si),
            _ => None,
        };
        let Some(si) = si else { return };
        let Some(entry) = self.state.pack.resources.vehicle_scripts.get(si) else { return };
        let js_locations = entry.js_locations();
        if let Some(first) = js_locations.first() {
            let code = self
                .state
                .pack
                .resource(first, "js")
                .map(|b| String::from_utf8_lossy(b).to_string())
                .unwrap_or_else(|| mrpc_script::JS_TEMPLATE.to_string());
            self.state.script.code = code;
        } else {
            self.state.script.code = mrpc_script::JS_TEMPLATE.to_string();
        }
        self.state.script.input_json = entry
            .input
            .as_ref()
            .and_then(|v| serde_json::to_string_pretty(v).ok())
            .unwrap_or_else(|| "{\n  \"routeName\": \"Demo Line\"\n}".into());
    }

    /// Write the script editor buffers back into the pack.
    pub fn save_script_back(&mut self) {
        let si = match self.state.selection {
            Selection::Script(si) => Some(si),
            _ => None,
        };
        let Some(si) = si else { return };
        let first_js = self
            .state
            .pack
            .resources
            .vehicle_scripts
            .get(si)
            .and_then(|e| e.js_locations().first().cloned());
        if let Some(first) = first_js {
            self.state
                .pack
                .set_resource(&first, "js", self.state.script.code.clone().into_bytes());
        }
        let input: Option<serde_json::Value> = serde_json::from_str(&self.state.script.input_json).ok();
        if let Some(entry) = self.state.pack.resources.vehicle_scripts.get_mut(si) {
            entry.input = input;
        }
        self.state.dirty = true;
    }

    /// Create a new vehicleScripts entry from a template.
    pub fn new_script_entry(&mut self, js: bool) {
        let id = format!("script_{}", self.state.pack.resources.vehicle_scripts.len() + 1);
        if js {
            let loc = format!("mtr:custom/pack/js/{id}/main.js");
            self.state
                .pack
                .set_resource(&loc, "js", mrpc_script::JS_TEMPLATE.as_bytes().to_vec());
            self.state.pack.resources.vehicle_scripts.push(mrpc_core::script::VehicleScriptEntry {
                id: id.clone(),
                script_locations: vec![serde_json::json!(loc)],
                input: Some(serde_json::json!({ "routeName": "Demo Line" })),
                ..Default::default()
            });
        } else {
            let loc = format!("mtr:custom/pack/natives/{id}");
            self.state
                .pack
                .set_resource(&loc, "cpp", mrpc_script::CPP_TEMPLATE.as_bytes().to_vec());
            self.state.pack.resources.vehicle_scripts.push(mrpc_core::script::VehicleScriptEntry {
                id: id.clone(),
                script_locations: vec![serde_json::json!(loc)],
                ..Default::default()
            });
        }
        let idx = self.state.pack.resources.vehicle_scripts.len() - 1;
        self.state.selection = Selection::Script(idx);
        self.load_script_editor();
        self.state.dirty = true;
    }

    /// Run the selected script entry against ~7 seconds of simulated data.
    pub fn run_script_test(&mut self) {
        let si = match self.state.selection {
            Selection::Script(si) => si,
            _ => {
                if self.state.pack.resources.vehicle_scripts.is_empty() {
                    self.state.script.console.push("no script entry selected".into());
                    return;
                }
                0
            }
        };
        let Some(entry) = self.state.pack.resources.vehicle_scripts.get(si).cloned() else { return };

        // Sources: prefer the editor buffer for the first JS location.
        let js_locations = entry.js_locations();
        let mut sources: Vec<(String, String)> = Vec::new();
        for (i, loc) in js_locations.iter().enumerate() {
            let code = if i == 0 {
                self.state.script.code.clone()
            } else {
                self.state
                    .pack
                    .resource(loc, "js")
                    .map(|b| String::from_utf8_lossy(b).to_string())
                    .unwrap_or_default()
            };
            sources.push((loc.clone(), code));
        }
        if sources.is_empty() {
            self.state
                .script
                .console
                .push("entry has no .js scriptLocations — use the C++ preview below".into());
            return;
        }

        let input: Option<serde_json::Value> = serde_json::from_str(&self.state.script.input_json).ok();
        let mut sandbox = match JsSandbox::new(&sources, &entry.prepend_expressions, input.as_ref()) {
            Ok(sb) => sb,
            Err(e) => {
                self.state.script.console.push(format!("load error: {e}"));
                return;
            }
        };

        self.state.script.console.clear();
        self.state.script.console.push("── test run: ~7 s @ 30 fps ──".into());
        let frames = 200;
        for frame in 0..frames {
            // A believable run: accelerate, hold, brake, dwell with open doors.
            let t = frame as f64 / 30.0;
            let (speed, notch, door) = if t < 6.0 {
                ((t * 10.0).min(60.0), 3.0, 0.0)
            } else if t < 9.0 {
                (60.0, 0.0, 0.0)
            } else if t < 10.5 {
                ((60.0 - (t - 9.0) * 40.0).max(0.0), -3.0, 0.0)
            } else {
                (0.0, 0.0, ((t - 10.5) / 3.2).clamp(0.0, 1.0))
            };
            let sim = mrpc_script::TrainSimState {
                speed_kmh: speed,
                door_value: door,
                door_opening: door > 0.0 && door < 1.0,
                notch: notch as i32,
                sim_time_ms: t * 1000.0,
                frame_delta_ms: 1000.0 / 30.0,
                ..self.state.train_sim()
            };
            sandbox.set_sim(sim);
            if let Err(e) = sandbox.render() {
                self.state.script.console.push(format!("error at frame {frame}: {e}"));
                break;
            }
        }
        let _ = sandbox.dispose();
        self.state.script.console.extend(sandbox.take_output());
        if self.state.script.console.len() > 400 {
            let overflow = self.state.script.console.len() - 400;
            self.state.script.console.drain(0..overflow);
        }
    }

    /// Load and run a native C++ module against simulated data.
    pub fn run_native_test(&mut self) {
        self.state.script.native_message = None;
        let path = std::path::PathBuf::from(&self.state.script.native_path);
        if !path.exists() {
            self.state.script.native_message = Some(format!("file not found: {}", path.display()));
            return;
        }
        let sim = self.state.train_sim();
        // Safety: the user explicitly pointed at their own compiled module.
        unsafe {
            match mrpc_script::NativePreview::load(&path, &sim) {
                Ok(mut native) => {
                    self.state.script.console.clear();
                    self.state.script.console.push(format!(
                        "── native module {} (ABI {}) ──",
                        native.module_id(),
                        native.abi()
                    ));
                    for frame in 0..90 {
                        let t = frame as f64 / 30.0;
                        let sim = mrpc_script::TrainSimState {
                            speed_kmh: (t * 8.0).min(60.0),
                            door_value: (t / 3.0).clamp(0.0, 1.0),
                            sim_time_ms: t * 1000.0,
                            ..self.state.train_sim()
                        };
                        if let Err(e) = native.render(&sim) {
                            self.state.script.native_message = Some(format!("{e}"));
                            break;
                        }
                    }
                    self.state.script.native_display = native.take_display();
                    self.state.script.console.extend(native.take_output());
                }
                Err(e) => {
                    self.state.script.native_message = Some(format!("{e}"));
                }
            }
        }
    }

    /// Parse the current vehicle's BVE sound set.
    pub fn load_bve_data(&mut self) -> Option<mrpc_core::bve::BveMotorData> {
        let vi = self.state.selected_vehicle()?;
        let vehicle = self.state.pack.resources.vehicles.get(vi)?;
        let base = &vehicle.bve_sound_base_resource;
        if base.is_empty() {
            return None;
        }
        let (ns, path) = match base.split_once(':') {
            Some((ns, p)) => (ns, p),
            None => ("mtr", base.as_str()),
        };
        let read = |name: &str| -> Option<String> {
            self.state
                .pack
                .files
                .get(&format!("assets/{ns}/sounds/{path}/{name}"))
                .map(|b| String::from_utf8_lossy(b).to_string())
        };
        let cfg_text = read("sound.cfg")?;
        let cfg = mrpc_core::bve::BveConfig::parse(&cfg_text);
        let data = if cfg.motor_noise_data_type == 4 {
            read("train.dat")
                .map(|t| mrpc_core::bve::BveMotorData::V4(mrpc_core::bve::BveMotorData4::parse(&t)))
        } else {
            let powervol = read("powervol.csv")?;
            let powerfreq = read("powerfreq.csv")?;
            let brakevol = read("brakevol.csv")?;
            let brakefreq = read("brakefreq.csv")?;
            Some(mrpc_core::bve::BveMotorData::V5(mrpc_core::bve::BveMotorData5 {
                power_volume: mrpc_core::bve::FloatSplines::parse(&powervol),
                power_frequency: mrpc_core::bve::FloatSplines::parse(&powerfreq),
                brake_volume: mrpc_core::bve::FloatSplines::parse(&brakevol),
                brake_frequency: mrpc_core::bve::FloatSplines::parse(&brakefreq),
            }))
        };
        self.state.vvvf.bve_config = Some(cfg);
        data
    }

    /// Render a 5-second audition WAV.
    pub fn render_vvvf_wav(&mut self) {
        let path = std::path::PathBuf::from("vvvf-audition.wav");
        let sr = 48_000u32;
        let params = self.state.vvvf.params;
        let mut engine = Engine::new(params);
        let mut samples: Vec<f32> = Vec::with_capacity((sr * 5) as usize);
        let target = self.state.vvvf.speed_kmh.max(1.0);
        let notch = self.state.vvvf.notch;
        let power = if notch == 0.0 {
            0.0
        } else if notch > 0.0 {
            (notch / 5.0).min(1.0)
        } else {
            (notch / 4.0).max(-1.0)
        };
        let blocks = 50;
        let block = (sr as usize * 5) / blocks;
        for b in 0..blocks {
            let t = b as f64 / blocks as f64 * 5.0;
            let speed = if t < 2.0 { target * t / 2.0 } else { target };
            let effort = if t < 3.5 { power } else { 0.0 };
            let fundamental = if let Some(hz) = self.state.vvvf.bve_fundamental {
                hz * (speed / target).clamp(0.0, 1.0)
            } else {
                let speed_ms = speed / 3.6;
                let wheel_rps = speed_ms / 0.85;
                mrpc_vvvf::fundamental_hz(wheel_rps / 6.2, params.pole_pairs)
            };
            let mut buf = vec![0f32; block];
            engine.render(fundamental, effort, &mut buf);
            samples.extend_from_slice(&buf);
        }
        match write_wav_mono16(&path, sr, &samples) {
            Ok(()) => {
                self.state.vvvf.message = Some(tf(
                    self.state.locale,
                    "vvvf.wav_written",
                    &[("path", &path.display().to_string())],
                ));
            }
            Err(e) => {
                self.state.vvvf.message =
                    Some(format!("{}: {e}", t(self.state.locale, "error.io")));
            }
        }
    }

    #[cfg(feature = "audio")]
    pub fn set_audio_playing(&mut self, playing: bool) {
        if let Some(audio) = &mut self.audio {
            audio.set_playing(playing);
        }
    }

    #[cfg(not(feature = "audio"))]
    pub fn set_audio_playing(&mut self, _playing: bool) {}

    // ------------------------------------------------------------------ update

    fn process_pending_open(&mut self) {
        let Some((path, is_zip)) = self.pending_open.clone() else { return };
        self.pending_open = None;
        let p = std::path::PathBuf::from(&path);
        let result = if is_zip {
            mrpc_core::pack::ResourcePack::load_zip(&p)
        } else {
            mrpc_core::pack::ResourcePack::load_dir(&p)
        };
        match result {
            Ok(pack) => {
                self.state.pack = pack;
                self.state.pack_path = Some(p.clone());
                self.state.config.last_open = Some(path.clone());
                self.state.config.save();
                self.state.selection = Selection::None;
                if !self.state.pack.resources.vehicles.is_empty() {
                    self.state.selection = Selection::Vehicle(0);
                }
                self.reload_model();
                self.state.revalidate();
                self.state.status = path;
            }
            Err(e) => {
                self.state.status = format!("{}: {e}", t(self.state.locale, "error.io"));
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.process_pending_open();
        let dt = ui.input(|i| i.unstable_dt).min(0.1);
        self.state.anim.step(dt as f64 * 1000.0);

        self.menu_bar(ui);
        self.open_dialog(ui.ctx());

        // VVVF preview synthesis (waveform + spectrum).
        self.update_vvvf_preview();

        crate::panels::outliner_panel(self, ui);
        crate::panels::properties_panel(self, ui);
        bottom_tabs(self, ui);
        self.viewport_center(ui);
        self.status_bar(ui);

        // Push the scene to the render thread each frame.
        self.sync_viewport();
    }

    fn update_vvvf_preview(&mut self) {
        if self.state.vvvf.mode == 1 && self.state.vvvf.bve.is_none() {
            let bve = self.load_bve_data();
            self.state.vvvf.bve = bve;
        }
        let vvvf = &mut self.state.vvvf;
        let fundamental = vvvf.fundamental_hz();
        let notch = vvvf.notch;
        let power = if notch == 0.0 {
            0.0
        } else if notch > 0.0 {
            (notch / 5.0).min(1.0)
        } else {
            (notch / 4.0).max(-1.0)
        };
        let mut params = vvvf.params;
        params.speed_fraction = (vvvf.speed_kmh / 130.0).clamp(0.0, 1.0);
        self.preview_engine = Engine::new(params);
        let mut wave = vec![0f32; 1024];
        self.preview_engine.render(fundamental, power, &mut wave);
        vvvf.wave = wave;

        // Spectrum: log-spaced Goertzel from 30 Hz to 4 kHz.
        let sr = params.sample_rate;
        vvvf.spectrum.clear();
        for i in 0..48 {
            let freq = 30.0 * (4000.0f64 / 30.0).powf(i as f64 / 47.0);
            let mag = goertzel_magnitude(&vvvf.wave, sr, freq);
            vvvf.spectrum.push((freq, mag));
        }

        #[cfg(feature = "audio")]
        if let Some(audio) = &mut self.audio {
            audio.update(params, fundamental, power);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let locale = self.state.locale;
        let mut open_zip = false;
        let mut open_dir = false;
        let mut open_demo = false;
        let mut quit = false;

        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button(t(locale, "menu.file"), |ui| {
                    if ui.button(t(locale, "app.new_pack")).clicked() {
                        self.state.pack = mrpc_core::pack::ResourcePack::new("New pack");
                        self.state.pack_path = None;
                        self.state.selection = Selection::None;
                        self.reload_model();
                    }
                    if ui.button(t(locale, "app.open_zip")).clicked() {
                        open_zip = true;
                    }
                    if ui.button(t(locale, "app.open_dir")).clicked() {
                        open_dir = true;
                    }
                    if ui.button(t(locale, "app.open_demo")).clicked() {
                        open_demo = true;
                    }
                    ui.separator();
                    if ui.button(t(locale, "app.export")).clicked() {
                        self.bottom_tab = BottomTab::Export;
                        self.state.revalidate();
                    }
                    ui.separator();
                    if ui.button(t(locale, "app.quit")).clicked() {
                        quit = true;
                    }
                });
                ui.menu_button(t(locale, "menu.edit"), |ui| {
                    if ui.button(t(locale, "edit.add_vehicle")).clicked() {
                        let mut v = mrpc_core::vehicle::VehicleResource::default();
                        v.id = format!("custom_train_{}", self.state.pack.resources.vehicles.len() + 1);
                        self.state.pack.resources.vehicles.push(v);
                        self.state.dirty = true;
                    }
                });
                ui.menu_button(t(locale, "menu.view"), |ui| {
                    ui.checkbox(&mut self.state.show_grid, t(locale, "view.grid"));
                    if ui.button(t(locale, "view.reset_camera")).clicked() {
                        self.state.cam = Default::default();
                    }
                });
                ui.menu_button(t(locale, "menu.language"), |ui| {
                    for l in Locale::ALL {
                        if ui.button(l.endonym()).clicked() {
                            self.state.locale = l;
                            self.state.config.locale = Some(l.tag().to_string());
                            self.state.config.save();
                        }
                    }
                });
                ui.menu_button(t(locale, "menu.help"), |ui| {
                    ui.label(tf(locale, "about.version", &[("v", env!("CARGO_PKG_VERSION"))]));
                    ui.label(RichText::new(t(locale, "about.description")).weak().small());
                    ui.separator();
                    ui.label(RichText::new(t(locale, "about.license_note")).weak().small());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.state.dirty {
                        ui.label(
                            RichText::new(t(locale, "app.unsaved"))
                                .small()
                                .color(Color32::from_rgb(230, 180, 70)),
                        );
                    }
                    ui.heading(t(locale, "app.title"));
                });
            });
        });

        if open_zip || open_dir {
            self.show_open_dialog = Some(open_zip);
        }
        if open_demo {
            self.state.pack = mrpc_core::demo::create_demo_pack();
            self.state.pack_path = None;
            self.state.selection = Selection::Vehicle(0);
            self.state.dirty = false;
            self.reload_model();
            self.state.revalidate();
            let n = self.state.pack.resources.vehicles.len();
            let f = self.state.pack.files.len();
            self.state.status =
                tf(locale, "demo.generated", &[("n", &n.to_string()), ("f", &f.to_string())]);
        }
        if quit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        let locale = self.state.locale;
        let Some(is_zip) = self.show_open_dialog else { return };
        let last = self
            .state
            .config
            .last_open
            .clone()
            .unwrap_or_else(|| if is_zip { "pack.zip".into() } else { "./pack-dir".into() });
        let mut path = last;
        let mut close = false;
        let mut apply = false;
        egui::Window::new(t(locale, "dialog.open_title"))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut path).desired_width(420.0));
                ui.horizontal(|ui| {
                    if ui.button(t(locale, "common.ok")).clicked() {
                        apply = true;
                        close = true;
                    }
                    if ui.button(t(locale, "common.cancel")).clicked() {
                        close = true;
                    }
                });
            });
        if apply {
            self.pending_open = Some((path, is_zip));
        }
        if close {
            self.show_open_dialog = None;
        }
    }

    fn viewport_center(&mut self, ui: &mut egui::Ui) {
        let locale = self.state.locale;
        egui::CentralPanel::default().show(ui, |ui| {
            let available = ui.available_rect_before_wrap();
            let response = ui.allocate_rect(available, egui::Sense::click_and_drag());
            if response.dragged_by(egui::PointerButton::Primary) {
                let d = response.drag_delta();
                self.state.cam.orbit(d.x, d.y);
            }
            if response.dragged_by(egui::PointerButton::Secondary) {
                let d = response.drag_delta();
                self.state.cam.pan(d.x, d.y);
            }
            if response.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    self.state.cam.zoom(scroll);
                }
            }

            *self.viewport.aspect.lock().unwrap() =
                response.rect.width() / response.rect.height().max(1.0);
            ViewportCallback::paint(response.rect, self.viewport.clone(), ui.painter());

            // Overlay: hint + manual door readout when the animation is paused.
            let painter = ui.painter_at(response.rect);
            painter.text(
                response.rect.left_top() + egui::vec2(10.0, 8.0),
                egui::Align2::LEFT_TOP,
                t(locale, "viewport.hint"),
                egui::FontId::proportional(11.0),
                Color32::from_rgb(160, 160, 168),
            );
            if !self.state.anim.playing {
                let label = format!(
                    "{}: {:.0}%",
                    t(locale, "viewport.door_slider"),
                    self.state.door_manual * 100.0
                );
                painter.text(
                    response.rect.left_bottom() + egui::vec2(10.0, -8.0),
                    egui::Align2::LEFT_BOTTOM,
                    label,
                    egui::FontId::proportional(11.0),
                    Color32::from_rgb(200, 160, 200),
                );
            }
            if self.state.model.is_none() {
                painter.text(
                    response.rect.center(),
                    egui::Align2::CENTER_CENTER,
                    t(locale, "viewport.no_model"),
                    egui::FontId::proportional(16.0),
                    Color32::from_rgb(140, 140, 148),
                );
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let locale = self.state.locale;
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(vi) = self.state.selected_vehicle() {
                    let id = self
                        .state
                        .pack
                        .resources
                        .vehicles
                        .get(vi)
                        .map(|v| v.id.clone())
                        .unwrap_or_default();
                    ui.label(tf(locale, "status.selected_vehicle", &[("id", &id)]));
                } else {
                    ui.label(t(locale, "status.ready"));
                }
                ui.separator();
                ui.weak(&self.state.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(format!("wgpu · Rust + C++ · v{}", env!("CARGO_PKG_VERSION")));
                });
            });
        });
    }

    fn sync_viewport(&mut self) {
        *self.viewport.camera.lock().unwrap() = self.state.cam;
        let scene = self.state.build_scene();
        let texture = self.state.model.as_ref().and_then(|m| m.texture.clone());
        let flip = self
            .state
            .pack
            .resources
            .vehicles
            .get(self.state.selected_vehicle().unwrap_or(0))
            .and_then(|v| v.models.first())
            .map(|model| model.flip_texture_v)
            .unwrap_or(false);

        let texture_size = texture.as_ref().map(|t| (t.1, t.2));
        let version_changed = self.scene_version != self.viewport_version;
        let texture_changed = texture_size != self.last_texture_size;
        if version_changed || texture_changed {
            *self.viewport.texture.lock().unwrap() = texture;
            *self.viewport.flip_texture_v.lock().unwrap() = flip;
            self.viewport.invalidate();
            self.last_texture_size = texture_size;
            self.viewport_version = self.scene_version;
        }
        *self.viewport.scene.lock().unwrap() = crate::viewport::SceneCpu {
            meshes: scene.meshes,
            instances: scene.instances,
            grid: scene.grid,
        };
    }
}

/// Register a CJK-capable system font so all seven UI languages render.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Some((name, data)) = find_cjk_font() {
        fonts
            .font_data
            .insert(name.clone(), std::sync::Arc::new(egui::FontData::from_owned(data)));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.push(name.clone());
            }
        }
    }
    ctx.set_fonts(fonts);
}

fn find_cjk_font() -> Option<(String, Vec<u8>)> {
    const CANDIDATES: &[(&str, &[&str])] = &[
        (
            "noto_sans_sc",
            &[
                "/usr/share/fonts/truetype/chinese/NotoSansSC-Regular.ttf",
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            ],
        ),
        (
            "msyh",
            &[
                "C:\\Windows\\Fonts\\msyh.ttc",
                "C:\\Windows\\Fonts\\simhei.ttf",
            ],
        ),
        ("pingfang", &["/System/Library/Fonts/PingFang.ttc"]),
        ("hiragino", &["/System/Library/Fonts/Hiragino Sans GB.ttc"]),
    ];
    for (name, paths) in CANDIDATES {
        for path in *paths {
            if let Ok(data) = std::fs::read(path) {
                return Some((name.to_string(), data));
            }
        }
    }
    // Last resort: $HOME/.fonts and /Library/Fonts.
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        "/Library/Fonts/Arial Unicode.ttf".to_string(),
        format!("{home}/.fonts/NotoSansSC-Regular.ttf"),
    ];
    for path in candidates {
        if let Ok(data) = std::fs::read(&path) {
            let name = path.rsplit('/').next().unwrap_or("cjk").to_string();
            return Some((name, data));
        }
    }
    None
}

impl eframe::App for MrpcApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        MrpcApp::ui(self, ui, frame);
    }
}
