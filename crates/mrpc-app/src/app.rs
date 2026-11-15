//! The application: layout orchestration and pack actions.

use std::path::PathBuf;
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
    /// File → 预览添加 flow state: pick a .zip or folder with the system
    /// file dialog, preview the pack, then add it to the current pack.
    preview_add: Option<PreviewAdd>,
    /// Deferred model-import request (path to .mqo/.mqoz/.gltf/.glb).
    pending_import: Option<String>,
    /// Path prompt for File → Import model.
    show_import_dialog: bool,
    /// Path prompt for the manual BVE motor WAV loop.
    show_bve_wav_dialog: bool,
    pending_bve_wav: Option<String>,
    /// Live audio output, initialised on a background thread (`feature = "audio"`).
    #[cfg(feature = "audio")]
    pub audio: crate::audio::AudioInit,
    /// VVVF preview engine (waveform/spectrum without audio hardware).
    preview_engine: Engine,
    /// UI passes seen so far; used for first-frame startup diagnostics
    /// (the second pass only happens after the first frame was painted and
    /// presented, which is when eframe makes the window visible).
    frame_count: u64,
}

// ------------------------------------------------------------------ preview-add

/// The File → 预览添加… flow: pick a `.zip` archive or an unpacked folder
/// with the system file dialog (Explorer / Finder / XDG portal), preview the
/// pack's contents, then add them to the pack being edited — or open the
/// picked pack as the current one.
struct PreviewAdd {
    stage: PreviewAddStage,
}

enum PreviewAddStage {
    /// Waiting for the user to pick a source. `error` is set when a previous
    /// pick could not be loaded as a resource pack.
    Pick { error: Option<String> },
    /// A pack was loaded; its contents are shown and can be added / opened.
    Preview { source: Box<PreviewedPack> },
    /// The flow finished; show the outcome summary until dismissed.
    Done { message: String },
}

/// A pack loaded from a picked path, awaiting the add / open decision.
struct PreviewedPack {
    path: PathBuf,
    is_zip: bool,
    pack: mrpc_core::pack::ResourcePack,
    /// One-shot display data for the preview window (see [`PreviewDisplay`]).
    display: PreviewDisplay,
}

/// Locale-independent data shown by the preview window, computed **once**
/// when the pack is loaded instead of per frame — egui repaints at UI rate,
/// and re-cloning the vehicle list / script ids and re-summing every file's
/// size on each frame stutters with large packs (hundreds of vehicles,
/// thousands of files).
struct PreviewDisplay {
    /// `id` or `id — name` per vehicle.
    vehicle_rows: Vec<String>,
    script_ids: Vec<String>,
    /// Sum of every file's byte length.
    total_bytes: u64,
    /// Whether the pack holds anything at all (enables the 添加 button).
    has_content: bool,
}

fn preview_display(pack: &mrpc_core::pack::ResourcePack) -> PreviewDisplay {
    let r = &pack.resources;
    PreviewDisplay {
        vehicle_rows: r
            .vehicles
            .iter()
            .map(|v| {
                if v.name.is_empty() {
                    v.id.clone()
                } else {
                    format!("{} — {}", v.id, v.name)
                }
            })
            .collect(),
        script_ids: r.vehicle_scripts.iter().map(|s| s.id.clone()).collect(),
        total_bytes: pack.files.values().map(|b| b.len() as u64).sum(),
        has_content: !(r.vehicles.is_empty()
            && r.vehicle_scripts.is_empty()
            && r.signs.is_empty()
            && r.rails.is_empty()
            && r.objects.is_empty()
            && r.lifts.is_empty()
            && r.extra.is_empty()
            && r.legacy.is_empty()
            && pack.files.is_empty()),
    }
}

/// What the preview-add window asked the app to do after a frame.
enum PreviewAddAction {
    PickZip,
    PickFolder,
    BackToPick,
    Add,
    Replace,
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
            preview_add: None,
            pending_import: None,
            show_import_dialog: false,
            show_bve_wav_dialog: false,
            pending_bve_wav: None,
            // Audio opens on a background thread: a hung/slow audio stack
            // (e.g. WASAPI in an RDP session) must not block the first frame.
            #[cfg(feature = "audio")]
            audio: crate::audio::AudioInit::spawn(),
            preview_engine: Engine::new(mrpc_vvvf::VvvfParams::default()),
            frame_count: 0,
        }
    }

    // ------------------------------------------------------------------ actions

    pub fn reload_model(&mut self) {
        self.state.reload_model();
        self.scene_version += 1;
        self.state.vvvf.bve = None;
        self.state.vvvf.bve_fundamental = None;
        self.state.vvvf.bve_engine = None;
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

    /// Load the BVE motor sound loops of the current vehicle into a playback
    /// engine. Channels come from the `sound.cfg` `[SoundList]` section; a
    /// vehicle without WAV loops keeps `None` (manual load remains possible).
    pub fn load_bve_sounds(&mut self) -> Option<mrpc_core::bve_playback::BvePlaybackEngine> {
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
        let cfg = self.state.vvvf.bve_config.clone()?;
        let sound_count = self
            .state
            .vvvf
            .bve
            .as_ref()
            .map(|b| b.sound_count())
            .unwrap_or(0);
        let sr = 48_000.0;
        let mut loops: Vec<mrpc_core::bve_playback::WavSample> = Vec::new();
        for i in 0..sound_count {
            let Some(file) = cfg.sound_file(i) else {
                loops.push(mrpc_core::bve_playback::WavSample::default());
                continue;
            };
            let bytes = self
                .state
                .pack
                .files
                .get(&format!("assets/{ns}/sounds/{path}/{file}"))
                .cloned();
            let wav = bytes
                .as_deref()
                .and_then(|b| mrpc_core::bve_playback::read_wav(b).ok());
            match wav {
                Some(w) => loops.push(w),
                None => loops.push(mrpc_core::bve_playback::WavSample::default()),
            }
        }
        if loops.iter().all(|l| l.data.is_empty()) {
            return None;
        }
        Some(mrpc_core::bve_playback::BvePlaybackEngine::new(&loops, sr))
    }

    /// Replace the BVE playback engine (also on the audio thread).
    pub fn set_bve_engine(&mut self, engine: mrpc_core::bve_playback::BvePlaybackEngine) {
        #[cfg(feature = "audio")]
        self.audio.set_bve_engine(engine.clone_shared());
        self.state.vvvf.bve_engine = Some(engine);
    }

    /// Load a manual WAV loop and use it for every motor channel.
    pub fn load_manual_bve_wav(&mut self, path: &str) {
        match std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|b| mrpc_core::bve_playback::read_wav(&b).map_err(|e| e.to_string()))
        {
            Ok(wav) => {
                let channels = self
                    .state
                    .vvvf
                    .bve
                    .as_ref()
                    .map(|b| b.sound_count().max(1))
                    .unwrap_or(1);
                // Build an engine with `channels` copies of the loop.
                let engine = mrpc_core::bve_playback::BvePlaybackEngine::new(
                    &vec![wav; channels.max(1)],
                    48_000.0,
                );
                self.set_bve_engine(engine);
                self.state.vvvf.message = Some(tf(
                    self.state.locale,
                    "vvvf.bve_loaded",
                    &[("n", &channels.to_string())],
                ));
            }
            Err(e) => {
                self.state.vvvf.message =
                    Some(format!("{}: {e}", t(self.state.locale, "import.failed")));
            }
        }
    }

    /// Import an external model file into the pack, attached to the selected
    /// vehicle (or a freshly created one).
    fn import_model_file(&mut self, path: &str) {
        let locale = self.state.locale;
        let p = std::path::PathBuf::from(path);
        let bytes = match std::fs::read(&p) {
            Ok(b) => b,
            Err(e) => {
                self.state.status = tf(locale, "import.failed", &[("msg", &e.to_string())]);
                return;
            }
        };
        let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
        let mut resolver = move |uri: &str| -> Option<Vec<u8>> {
            let candidate = dir.join(uri);
            std::fs::read(&candidate).ok().or_else(|| {
                // Case-insensitive fallback (Windows-authored packs).
                let fname = uri.to_ascii_lowercase();
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for e in entries.flatten() {
                        if e.file_name().to_string_lossy().to_ascii_lowercase() == fname {
                            return std::fs::read(e.path()).ok();
                        }
                    }
                }
                None
            })
        };
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let imported = match ext.as_str() {
            "mqo" => std::str::from_utf8(&bytes)
                .map_err(|e| e.to_string())
                .and_then(|t| mrpc_core::mqo::parse_mqo(t).map_err(|e| e.to_string())),
            "mqoz" => mrpc_core::mqo::parse_mqoz(&bytes).map_err(|e| e.to_string()),
            "gltf" | "glb" => {
                mrpc_core::gltf::parse_gltf(&bytes, &mut resolver).map_err(|e| e.to_string())
            }
            other => Err(format!("unsupported extension: .{other}")),
        };
        let imported = match imported {
            Ok(i) => i,
            Err(msg) => {
                self.state.status = tf(locale, "import.failed", &[("msg", &msg)]);
                return;
            }
        };

        // Resource ids under a fresh custom namespace.
        let n = self.state.pack.resources.vehicles.len() + 1;
        let ns = format!("mtr:custom/imported_{n}");
        let obj_id = format!("{ns}/model");
        let mtl_id = format!("{ns}/model.mtl");
        let prop_id = format!("{ns}/model_properties.json");
        let pos_id = format!("{ns}/model_positions.json");
        let mut texture_resource = String::new();
        if let Some(tex) = imported.textures.first() {
            texture_resource = format!("{ns}/{}", tex.filename);
        }

        // OBJ + MTL + textures.
        let obj_text = mrpc_core::import::obj_to_text(&imported.obj);
        self.state.pack.set_resource(&obj_id, "obj", obj_text.into_bytes());
        let mtl_text = mrpc_core::import::mtl_to_text(&imported.materials);
        self.state
            .pack
            .set_resource(&mtl_id, "mtl", mtl_text.into_bytes());
        for tex in &imported.textures {
            let id = format!("{ns}/{}", tex.filename);
            self.state.pack.set_resource(&id, "png", tex.png.clone());
        }
        // Minimal properties / position definitions so validation passes.
        let props = self.build_model_properties(&imported);
        self.state
            .pack
            .set_resource(&prop_id, "json", serde_json::to_vec(&props).unwrap_or_default());
        self.state
            .pack
            .set_resource(&pos_id, "json", b"{}".to_vec());

        // Attach to the selected vehicle, or create one.
        let model = mrpc_core::vehicle::VehicleModel {
            model_resource: obj_id.clone(),
            texture_resource: texture_resource.clone(),
            model_properties_resource: prop_id,
            position_definitions_resource: pos_id,
            flip_texture_v: imported.flip_texture_v,
            ..Default::default()
        };
        let target = match self.state.selected_vehicle() {
            Some(vi) => vi,
            None => {
                let mut v = mrpc_core::vehicle::VehicleResource::default();
                v.id = format!("custom_imported_{n}");
                v.name = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("Imported {n}"));
                self.state.pack.resources.vehicles.push(v);
                self.state.pack.resources.vehicles.len() - 1
            }
        };
        if let Some(vehicle) = self.state.pack.resources.vehicles.get_mut(target) {
            vehicle.models.push(model);
        }

        self.state.selection = Selection::Vehicle(target);
        self.state.dirty = true;
        self.reload_model();
        self.state.revalidate();
        self.state.status = tf(
            locale,
            "import.success",
            &[
                ("parts", &imported.obj.parts.len().to_string()),
                ("verts", &imported.vertex_count().to_string()),
                ("tex", &imported.textures.len().to_string()),
            ],
        );
    }

    /// Minimal model-properties JSON covering the imported part names.
    fn build_model_properties(
        &self,
        imported: &mrpc_core::import::ImportedModel,
    ) -> serde_json::Value {
        let mut arr = Vec::new();
        for part in &imported.obj.parts {
            arr.push(serde_json::json!({
                "name": part.name,
                "type": "part",
            }));
        }
        serde_json::json!({ "parts": arr })
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
        let mut synth = Engine::new(params);
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
        // In BVE mode with a loaded sound source, render the real playback.
        let use_bve = self.state.vvvf.mode == 1 && self.state.vvvf.bve_engine.is_some();
        let bve_data = self.state.vvvf.bve.clone();
        let mut bve_engine = self.state.vvvf.bve_engine.clone();
        let blocks = 50;
        let block = (sr as usize * 5) / blocks;
        for b in 0..blocks {
            let t = b as f64 / blocks as f64 * 5.0;
            let speed = if t < 2.0 { target * t / 2.0 } else { target };
            let effort = if t < 3.5 { power } else { 0.0 };
            let mut buf = vec![0f32; block];
            if use_bve {
                if let (Some(engine), Some(data)) = (&mut bve_engine, &bve_data) {
                    engine.render_curves(data, (speed / 3.6) as f32, effort as f32, &mut buf);
                }
            } else {
                let fundamental = if let Some(hz) = self.state.vvvf.bve_fundamental {
                    hz * (speed / target).clamp(0.0, 1.0)
                } else {
                    let speed_ms = speed / 3.6;
                    let wheel_rps = speed_ms / 0.85;
                    mrpc_vvvf::fundamental_hz(wheel_rps / 6.2, params.pole_pairs)
                };
                synth.render(fundamental, effort, &mut buf);
            }
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
        self.audio.set_playing(playing);
    }

    #[cfg(not(feature = "audio"))]
    pub fn set_audio_playing(&mut self, _playing: bool) {}

    // ------------------------------------------------------------------ update

    /// Take the previewed pack out of the preview-add flow (only valid while
    /// the flow is in the `Preview` stage; the stage is reset to `Pick`).
    fn take_previewed_pack(&mut self) -> Option<PreviewedPack> {
        let flow = self.preview_add.as_mut()?;
        if !matches!(flow.stage, PreviewAddStage::Preview { .. }) {
            return None;
        }
        let stage = std::mem::replace(&mut flow.stage, PreviewAddStage::Pick { error: None });
        match stage {
            PreviewAddStage::Preview { source } => Some(*source),
            _ => unreachable!("the stage was checked to be Preview above"),
        }
    }

    /// Load the pack at a picked path into the preview stage.
    fn load_previewed(&mut self, path: PathBuf, is_zip: bool) {
        let locale = self.state.locale;
        log::info!(
            "preview-add: loading {} ({})",
            path.display(),
            if is_zip { "zip" } else { "directory" }
        );
        let result = if is_zip {
            mrpc_core::pack::ResourcePack::load_zip(&path)
        } else {
            mrpc_core::pack::ResourcePack::load_dir(&path)
        };
        match result {
            Ok(pack) => {
                log::info!(
                    "preview-add: loaded \"{}\" — {} vehicle(s), {} script(s), {} file(s)",
                    pack.mcmeta.pack.description,
                    pack.resources.vehicles.len(),
                    pack.resources.vehicle_scripts.len(),
                    pack.files.len()
                );
                // Remember where the last pack came from so the next picker
                // starts in the same place.
                self.state.config.last_open = Some(path.display().to_string());
                self.state.config.save();
                // Compute the preview window's display data once, here — not
                // on every repaint.
                let display = preview_display(&pack);
                let stage = PreviewAddStage::Preview {
                    source: Box::new(PreviewedPack { path, is_zip, pack, display }),
                };
                if let Some(flow) = self.preview_add.as_mut() {
                    flow.stage = stage;
                } else {
                    self.preview_add = Some(PreviewAdd { stage });
                }
            }
            Err(e) => {
                let error = tf(locale, "preview.not_pack", &[("msg", &e.to_string())]);
                log::warn!("preview-add: load failed: {e}");
                if let Some(flow) = self.preview_add.as_mut() {
                    flow.stage = PreviewAddStage::Pick { error: Some(error) };
                }
            }
        }
    }

    /// Merge the previewed pack into the one being edited (the 添加 action).
    fn apply_preview_merge(&mut self) {
        let locale = self.state.locale;
        let Some(source) = self.take_previewed_pack() else { return };
        let first_new_vehicle = self.state.pack.resources.vehicles.len();
        let stats = self.state.pack.merge_from(source.pack);
        log::info!(
            "preview-add: merged +{} vehicle(s), +{} script(s), +{} sign(s), +{} other, +{} file(s), {} kept, {} renamed",
            stats.vehicles_added,
            stats.scripts_added,
            stats.signs_added,
            stats.other_added,
            stats.files_added,
            stats.files_kept,
            stats.ids_renamed
        );
        // A merge that added nothing (e.g. adding the same pack twice) must
        // not mark the pack dirty or needlessly rebuild the model and
        // re-run validation on unchanged data.
        if stats.added_anything() {
            self.state.dirty = true;
            if stats.vehicles_added > 0 {
                // Select the first vehicle that came in with the merge.
                self.state.selection = Selection::Vehicle(first_new_vehicle);
            }
            self.reload_model();
            self.state.revalidate();
        }

        let mut message = if stats.added_anything() {
            tf(
                locale,
                "preview.merged",
                &[
                    ("v", &stats.vehicles_added.to_string()),
                    ("s", &stats.scripts_added.to_string()),
                    ("g", &stats.signs_added.to_string()),
                    ("f", &stats.files_added.to_string()),
                ],
            )
        } else {
            t(locale, "preview.nothing")
        };
        if stats.other_added > 0 {
            message.push(' ');
            message.push_str(&tf(locale, "preview.others_added", &[("n", &stats.other_added.to_string())]));
        }
        if stats.files_kept > 0 {
            message.push(' ');
            message.push_str(&tf(locale, "preview.files_kept", &[("n", &stats.files_kept.to_string())]));
        }
        if stats.ids_renamed > 0 {
            message.push(' ');
            message.push_str(&tf(locale, "preview.renamed", &[("n", &stats.ids_renamed.to_string())]));
        }
        self.state.status = message.clone();
        if let Some(flow) = self.preview_add.as_mut() {
            flow.stage = PreviewAddStage::Done { message };
        }
    }

    /// Open the previewed pack as the current one (the old 打开 behaviour,
    /// kept as the secondary action of the preview window).
    fn apply_preview_replace(&mut self) {
        let locale = self.state.locale;
        let Some(source) = self.take_previewed_pack() else { return };
        log::info!("preview-add: opening as the current pack: {}", source.path.display());
        self.state.pack = source.pack;
        self.state.pack_path = Some(source.path.clone());
        self.state.config.last_open = Some(source.path.display().to_string());
        self.state.config.save();
        self.state.selection = Selection::None;
        if !self.state.pack.resources.vehicles.is_empty() {
            self.state.selection = Selection::Vehicle(0);
        }
        self.reload_model();
        self.state.revalidate();
        let path_text = source.path.display().to_string();
        self.state.status = path_text.clone();
        let message = tf(locale, "preview.opened", &[("path", &path_text)]);
        if let Some(flow) = self.preview_add.as_mut() {
            flow.stage = PreviewAddStage::Done { message };
        }
    }

    fn process_pending_import(&mut self) {
        let Some(path) = self.pending_import.clone() else { return };
        self.pending_import = None;
        self.import_model_file(&path);
    }

    fn process_pending_bve_wav(&mut self) {
        let Some(path) = self.pending_bve_wav.clone() else { return };
        self.pending_bve_wav = None;
        self.load_manual_bve_wav(&path);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.frame_count == 0 {
            crate::logging::set_stage(crate::logging::Stage::FirstUi);
        }
        self.process_pending_import();
        self.process_pending_bve_wav();
        let dt = ui.input(|i| i.unstable_dt).min(0.1);
        self.state.anim.step(dt as f64 * 1000.0);

        self.menu_bar(ui);
        self.preview_add_window(ui.ctx(), frame);
        self.import_dialog(ui.ctx());
        self.bve_wav_dialog(ui.ctx());

        // VVVF preview synthesis (waveform + spectrum).
        self.update_vvvf_preview();

        crate::panels::outliner_panel(self, ui);
        crate::panels::properties_panel(self, ui);
        bottom_tabs(self, ui);
        self.viewport_center(ui);
        self.status_bar(ui);

        // Push the scene to the render thread each frame.
        self.sync_viewport();

        self.frame_count += 1;
        if self.frame_count == 1 {
            // The first UI pass is followed by the first paint+present; make
            // sure a second pass is scheduled right away, both to react to the
            // just-created scene and so the "startup complete" marker (below)
            // proves the first present went through.
            ui.ctx().request_repaint();
        } else if self.frame_count == 2 {
            crate::logging::set_stage(crate::logging::Stage::FirstFrame);
            crate::logging::mark_startup_complete();
        }
    }

    fn update_vvvf_preview(&mut self) {
        if self.state.vvvf.mode == 1 && self.state.vvvf.bve.is_none() {
            let bve = self.load_bve_data();
            self.state.vvvf.bve = bve;
        }
        if self.state.vvvf.mode == 1
            && self.state.vvvf.bve.is_some()
            && self.state.vvvf.bve_engine.is_none()
        {
            // Try the pack's own motor WAV loops first.
            if let Some(engine) = self.load_bve_sounds() {
                self.set_bve_engine(engine);
            }
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

        // Waveform: BVE real-source playback when available, else the synth.
        let use_bve = vvvf.mode == 1 && vvvf.bve_engine.is_some();
        let mut wave = vec![0f32; 1024];
        if use_bve {
            let speed_ms = (vvvf.speed_kmh / 3.6) as f32;
            let data = vvvf.bve.clone();
            if let Some(engine) = &mut vvvf.bve_engine {
                if let Some(data) = &data {
                    engine.render_curves(data, speed_ms, power as f32, &mut wave);
                }
            }
        } else {
            self.preview_engine = Engine::new(params);
            self.preview_engine.render(fundamental, power, &mut wave);
        }
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
        {
            if use_bve {
                let speed_ms = (vvvf.speed_kmh / 3.6) as f32;
                let n = vvvf.bve_engine.as_ref().map(|e| e.channel_count()).unwrap_or(0);
                let (rates, gains) = match vvvf.bve.as_ref() {
                    Some(d) => mrpc_core::bve_playback::BvePlaybackEngine::curve_drive(
                        d, n, speed_ms, power as f32,
                    ),
                    None => (vec![0f32; n], vec![0f32; n]),
                };
                self.audio.update(crate::audio::AudioDrive::Bve { rates, gains });
            } else {
                self.audio.update(crate::audio::AudioDrive::Synth {
                    params,
                    fundamental_hz: fundamental,
                    power,
                });
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let locale = self.state.locale;
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
                    if ui.button(t(locale, "app.preview_add")).clicked() {
                        // 预览添加: system file dialog → preview → add/open.
                        self.preview_add =
                            Some(PreviewAdd { stage: PreviewAddStage::Pick { error: None } });
                    }
                    if ui.button(t(locale, "app.open_demo")).clicked() {
                        open_demo = true;
                    }
                    ui.separator();
                    if ui.button(t(locale, "menu.import")).clicked() {
                        self.show_import_dialog = true;
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

    /// File → 预览添加… window: pick a `.zip` / folder with the system file
    /// dialog, preview the pack, then add it to the current pack (or open it
    /// as the current one).
    fn preview_add_window(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let locale = self.state.locale;
        let Some(flow) = self.preview_add.as_mut() else { return };
        let mut close = false;
        let mut action: Option<PreviewAddAction> = None;

        match &mut flow.stage {
            PreviewAddStage::Pick { error } => {
                egui::Window::new(t(locale, "preview.title"))
                    .collapsible(false)
                    .resizable(false)
                    .min_width(500.0)
                    .show(ctx, |ui| {
                        ui.label(RichText::new(t(locale, "preview.hint")).weak());
                        if let Some(err) = error {
                            ui.add_space(6.0);
                            ui.label(RichText::new(err.as_str()).color(Color32::from_rgb(235, 120, 120)));
                        }
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let zip = ui.add_sized(
                                [228.0, 36.0],
                                egui::Button::new(t(locale, "preview.pick_zip")),
                            );
                            let dir = ui.add_sized(
                                [228.0, 36.0],
                                egui::Button::new(t(locale, "preview.pick_folder")),
                            );
                            if zip.clicked() {
                                action = Some(PreviewAddAction::PickZip);
                            }
                            if dir.clicked() {
                                action = Some(PreviewAddAction::PickFolder);
                            }
                        });
                        ui.add_space(8.0);
                        if ui.button(t(locale, "common.cancel")).clicked() {
                            close = true;
                        }
                    });
            }
            PreviewAddStage::Preview { source } => {
                // The heavy display data (vehicle rows, script ids, total
                // size) was computed once when the pack was loaded — see
                // `preview_display`; only cheap locale-dependent labels are
                // built per frame.
                let pack = &source.pack;
                let display = &source.display;
                let path_text = source.path.display().to_string();
                let kind = t(
                    locale,
                    if source.is_zip { "preview.source_zip" } else { "preview.source_dir" },
                );
                let description = pack.mcmeta.pack.description.clone();
                let pack_format = pack.mcmeta.pack.pack_format.to_string();
                let counts = tf(
                    locale,
                    "preview.counts",
                    &[
                        ("v", &pack.resources.vehicles.len().to_string()),
                        ("s", &pack.resources.vehicle_scripts.len().to_string()),
                        ("g", &pack.resources.signs.len().to_string()),
                    ],
                );
                let files_line = tf(
                    locale,
                    "preview.files",
                    &[
                        ("f", &pack.files.len().to_string()),
                        ("size", &format_size(display.total_bytes)),
                    ],
                );
                let vehicle_rows = display.vehicle_rows.as_slice();
                let script_ids = display.script_ids.as_slice();
                let load_errors = pack.load_errors.as_slice();
                let has_content = display.has_content;

                egui::Window::new(t(locale, "preview.title"))
                    .collapsible(false)
                    .resizable(false)
                    .min_width(560.0)
                    .show(ctx, |ui| {
                        ui.label(RichText::new(format!("{}: {}", t(locale, "preview.path"), path_text)).small());
                        ui.label(RichText::new(kind).small().weak());
                        ui.add_space(6.0);
                        ui.label(format!(
                            "{}: {}",
                            t(locale, "preview.description"),
                            if description.is_empty() { t(locale, "common.none") } else { description }
                        ));
                        ui.label(format!("{}: {}", t(locale, "preview.pack_format"), pack_format));
                        ui.label(counts.as_str());
                        ui.label(files_line.as_str());

                        if !vehicle_rows.is_empty() {
                            ui.add_space(6.0);
                            ui.label(RichText::new(t(locale, "preview.vehicles")).strong());
                            egui::ScrollArea::vertical()
                                .max_height(150.0)
                                .show(ui, |ui| {
                                    for row in vehicle_rows {
                                        ui.label(row);
                                    }
                                });
                        }
                        if !script_ids.is_empty() {
                            ui.add_space(4.0);
                            let shown = script_ids.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
                            let more = if script_ids.len() > 6 {
                                format!(" (+{})", script_ids.len() - 6)
                            } else {
                                String::new()
                            };
                            ui.label(
                                RichText::new(format!(
                                    "{}: {}{}",
                                    t(locale, "preview.scripts"),
                                    shown,
                                    more
                                ))
                                .weak(),
                            );
                        }
                        if !load_errors.is_empty() {
                            ui.add_space(4.0);
                            ui.label(RichText::new(t(locale, "preview.load_notes")).strong());
                            for err in load_errors {
                                ui.label(
                                    RichText::new(err.as_str())
                                        .small()
                                        .color(Color32::from_rgb(235, 170, 120)),
                                );
                            }
                        }
                        if !has_content {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(t(locale, "preview.empty"))
                                    .color(Color32::from_rgb(235, 180, 90)),
                            );
                        }

                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let add = ui.add_enabled(
                                has_content,
                                egui::Button::new(t(locale, "preview.add")),
                            );
                            if add.clicked() {
                                action = Some(PreviewAddAction::Add);
                            }
                            if ui.button(t(locale, "preview.replace")).clicked() {
                                action = Some(PreviewAddAction::Replace);
                            }
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.button(t(locale, "preview.repick")).clicked() {
                                action = Some(PreviewAddAction::BackToPick);
                            }
                            if ui.button(t(locale, "common.cancel")).clicked() {
                                close = true;
                            }
                        });
                    });
            }
            PreviewAddStage::Done { message } => {
                let message = message.clone();
                egui::Window::new(t(locale, "preview.title"))
                    .collapsible(false)
                    .resizable(false)
                    .min_width(420.0)
                    .show(ctx, |ui| {
                        ui.label(&message);
                        ui.add_space(8.0);
                        if ui.button(t(locale, "common.ok")).clicked() {
                            close = true;
                        }
                    });
            }
        }

        // Handle the requested action after the borrows of the stage are done.
        if let Some(a) = action {
            match a {
                PreviewAddAction::PickZip => {
                    if let Some(path) = pick_pack_path(locale, &*frame, &self.state.config, true) {
                        self.load_previewed(path, true);
                    }
                }
                PreviewAddAction::PickFolder => {
                    if let Some(path) = pick_pack_path(locale, &*frame, &self.state.config, false) {
                        self.load_previewed(path, false);
                    }
                }
                PreviewAddAction::Add => self.apply_preview_merge(),
                PreviewAddAction::Replace => self.apply_preview_replace(),
                PreviewAddAction::BackToPick => {
                    self.preview_add = Some(PreviewAdd {
                        stage: PreviewAddStage::Pick { error: None },
                    });
                }
            }
        }
        if close {
            self.preview_add = None;
        }
    }

    /// File → Import model path prompt.
    fn import_dialog(&mut self, ctx: &egui::Context) {
        let locale = self.state.locale;
        if !self.show_import_dialog {
            return;
        }
        let mut path = if self.state.vvvf.bve_wav_path.is_empty() {
            "model.mqo".to_string()
        } else {
            self.state.vvvf.bve_wav_path.clone()
        };
        // Prefer a dedicated import-path memory over the wav field default.
        if let Some(last) = &self.state.config.last_open {
            let dir = std::path::Path::new(last)
                .parent()
                .map(|d| d.to_string_lossy().to_string())
                .unwrap_or_default();
            if !dir.is_empty() {
                path = format!("{dir}/model.mqo");
            }
        }
        let mut close = false;
        let mut apply = false;
        egui::Window::new(t(locale, "import.title"))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(t(locale, "import.formats"));
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
            self.pending_import = Some(path);
        }
        if close {
            self.show_import_dialog = false;
        }
    }

    /// Manual BVE motor WAV loop path prompt.
    fn bve_wav_dialog(&mut self, ctx: &egui::Context) {
        let locale = self.state.locale;
        if !self.show_bve_wav_dialog {
            return;
        }
        let mut path = if self.state.vvvf.bve_wav_path.is_empty() {
            "motor_loop.wav".to_string()
        } else {
            self.state.vvvf.bve_wav_path.clone()
        };
        let mut close = false;
        let mut apply = false;
        egui::Window::new(t(locale, "vvvf.bve_load"))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(t(locale, "vvvf.bve_hint"));
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
            self.state.vvvf.bve_wav_path = path.clone();
            self.pending_bve_wav = Some(path);
        }
        if close {
            self.show_bve_wav_dialog = false;
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

/// Open the system file dialog (Explorer / Finder / XDG portal) to pick a
/// resource pack source: a `.zip` archive (`zip = true`) or an unpacked pack
/// folder. Returns `None` when the user cancelled.
///
/// Blocking call — used from the UI pass, which is the main thread, as the
/// native dialogs require. The dialog is parented to the app window so it
/// always appears on top of it.
fn pick_pack_path(
    locale: Locale,
    frame: &eframe::Frame,
    config: &AppConfig,
    zip: bool,
) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new();
    // Start where the last pack was picked from, when that still exists.
    if let Some(last) = config.last_open.as_deref().map(PathBuf::from) {
        let start = if last.is_dir() {
            Some(last)
        } else {
            last.parent().filter(|p| p.is_dir()).map(|p| p.to_path_buf())
        };
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
    }
    if zip {
        dialog
            .set_title(t(locale, "preview.pick_title_zip"))
            .add_filter(t(locale, "preview.zip_filter"), &["zip"])
            .set_parent(frame)
            .pick_file()
    } else {
        dialog
            .set_title(t(locale, "preview.pick_title_folder"))
            .set_parent(frame)
            .pick_folder()
    }
}

/// Human-readable byte size for the pack preview (B / KB / MB).
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

impl eframe::App for MrpcApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        MrpcApp::ui(self, ui, frame);
    }
}
