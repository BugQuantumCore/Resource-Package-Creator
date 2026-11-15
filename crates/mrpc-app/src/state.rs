//! Editor state: the open pack, selection, caches and panel states.

use std::collections::HashMap;
use std::path::PathBuf;

use mrpc_core::door_anim::DoorCycle;
use mrpc_core::i18n::{t, Locale};
use mrpc_core::model_props::ModelProperties;
use mrpc_core::mtl::Material;
use mrpc_core::obj::ObjModel;
use mrpc_core::pack::ResourcePack;
use mrpc_core::positions::PositionDefinitions;
use mrpc_core::validate::{self, ValidationItem};
use mrpc_script::TrainSimState;
use mrpc_vvvf::VvvfParams;

use crate::viewport::{Camera, CpuMesh, Instance};

/// What is selected in the outliner / edited in the properties panel.
#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    None,
    Vehicle(usize),
    /// (vehicle index, model index, tab page)
    Model { vehicle: usize, model: usize },
    /// (vehicle index, model index, part index in the model properties)
    Part { vehicle: usize, model: usize, part: usize },
    /// Script entry index.
    Script(usize),
    /// Raw asset file path.
    Asset(String),
}

/// Persisted app configuration (locale, last paths).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AppConfig {
    pub locale: Option<String>,
    pub last_open: Option<String>,
    pub last_export: Option<String>,
}

impl AppConfig {
    /// Directory holding the per-user config (and other app-written files).
    /// `APPDATA` on Windows, `XDG_CONFIG_HOME`/`~/.config` on Linux,
    /// `~/Library/Application Support` on macOS.
    pub fn dir() -> PathBuf {
        if let Some(ad) = std::env::var_os("APPDATA") {
            return PathBuf::from(ad).join("mrpc");
        }
        if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(x).join("mrpc");
        }
        if let Some(h) = std::env::var_os("HOME") {
            let p = PathBuf::from(h);
            return match std::env::consts::OS {
                "macos" => p.join("Library/Application Support/mrpc"),
                _ => p.join(".config/mrpc"),
            };
        }
        // No home at all: fall back beside the executable (portable use).
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn path() -> PathBuf {
        Self::dir().join("mrpc-config.json")
    }

    /// The pre-user-dir location (beside the exe) — read once for migration,
    /// since installs under `C:\Program Files` can't write it.
    fn legacy_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mrpc-config.json")
    }

    pub fn load() -> AppConfig {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            // Migrate from the old beside-the-exe location.
            .or_else(|| {
                std::fs::read(Self::legacy_path())
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok())
            })
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(Self::dir());
        let _ = std::fs::write(Self::path(), serde_json::to_vec_pretty(self).unwrap_or_default());
    }
}

/// Default directory for exported files (VVVF audition WAV, pack zip).
/// Writing to the process CWD breaks on installed builds: launched from a
/// `Program Files` shortcut the CWD isn't writable. `Documents` on
/// Windows/macOS, the home directory on Linux, temp as last resort.
pub fn default_output_dir() -> PathBuf {
    let docs = |home: std::ffi::OsString| {
        let p = PathBuf::from(home);
        match std::env::consts::OS {
            "linux" => p,
            _ => {
                let d = p.join("Documents");
                if d.is_dir() { d } else { p }
            }
        }
    };
    if let Some(d) = std::env::var_os("USERPROFILE") {
        return docs(d);
    }
    if let Some(h) = std::env::var_os("HOME") {
        return docs(h);
    }
    std::env::temp_dir()
}

/// Door animation preview player.
pub struct AnimPlayer {
    pub playing: bool,
    /// Position on the door timeline, milliseconds.
    pub t_ms: f64,
    pub cycle: DoorCycle,
    pub speed: f64,
}

impl AnimPlayer {
    pub fn new() -> Self {
        Self { playing: false, t_ms: 0.0, cycle: DoorCycle::default(), speed: 1.0 }
    }

    pub fn step(&mut self, dt_ms: f64) {
        if self.playing {
            self.t_ms = (self.t_ms + dt_ms * self.speed) % self.cycle.total_ms() as f64;
        }
    }

    /// Current door value (0..1) and whether the doors are opening.
    pub fn door(&self) -> (f64, bool) {
        let (value, phase) = self.cycle.sample(self.t_ms as i64);
        let opening = matches!(
            phase,
            mrpc_core::door_anim::DoorPhase::Opening
                | mrpc_core::door_anim::DoorPhase::Open
                | mrpc_core::door_anim::DoorPhase::ClosedBeforeOpen
        );
        (value, opening)
    }
}

impl Default for AnimPlayer {
    fn default() -> Self {
        Self::new()
    }
}

/// VVVF audition state.
pub struct VvvfPanel {
    /// 0 = synthesiser, 1 = BVE motor data, 2 = legacy speed sounds.
    pub mode: u8,
    pub params: VvvfParams,
    pub speed_kmh: f64,
    /// −1 braking, 0 coasting, +1..+5 power notches.
    pub notch: f64,
    pub playing: bool,
    /// Ring buffer of the last rendered samples for the waveform view.
    pub wave: Vec<f32>,
    /// Spectrum bars (recomputed each frame for the view).
    pub spectrum: Vec<(f64, f32)>,
    /// Cached parsed BVE motor data of the current vehicle.
    pub bve: Option<mrpc_core::bve::BveMotorData>,
    pub bve_config: Option<mrpc_core::bve::BveConfig>,
    /// Fundamental override while auditioning BVE curves.
    pub bve_fundamental: Option<f64>,
    /// Real-sound-source playback engine (curves drive loop resampling).
    pub bve_engine: Option<mrpc_core::bve_playback::BvePlaybackEngine>,
    /// Manual WAV loop path for vehicles whose sound set has no WAV loops.
    pub bve_wav_path: String,
    pub message: Option<String>,
}

impl Default for VvvfPanel {
    fn default() -> Self {
        Self {
            mode: 0,
            params: VvvfParams {
                noise_gain: 0.25,
                speed_fraction: 0.0,
                ..Default::default()
            },
            speed_kmh: 0.0,
            notch: 3.0,
            playing: false,
            wave: Vec::new(),
            spectrum: Vec::new(),
            bve: None,
            bve_config: None,
            bve_fundamental: None,
            bve_engine: None,
            bve_wav_path: String::new(),
            message: None,
        }
    }
}

impl VvvfPanel {
    /// Fundamental override used by the BVE audition mode.
    pub fn set_bve_fundamental(&mut self, hz: f64) {
        self.bve_fundamental = Some(hz);
    }

    /// Electrical fundamental from the preview speed: metro motor ~120 km/h at
    /// 60 Hz electrical with 4 pole pairs → shaft rps × pole pairs.
    pub fn fundamental_hz(&self) -> f64 {
        if self.mode == 1 {
            if let Some(hz) = self.bve_fundamental {
                return hz;
            }
        }
        let speed_ms = self.speed_kmh / 3.6;
        let wheel_rps = speed_ms / 0.85; // ~0.85 m wheel circumference
        let gear = 6.2; // traction gear ratio
        mrpc_vvvf::fundamental_hz(wheel_rps / gear, self.params.pole_pairs)
    }
}

/// Train-script panel state.
pub struct ScriptPanel {
    /// Edit buffer for the first JS location of the selected entry.
    pub code: String,
    /// Edit buffer for SCRIPT_INPUT JSON.
    pub input_json: String,
    pub console: Vec<String>,
    pub native_path: String,
    pub native_message: Option<String>,
    /// Last native module display text.
    pub native_display: String,
}

impl Default for ScriptPanel {
    fn default() -> Self {
        Self {
            code: String::new(),
            input_json: "{\n  \"routeName\": \"Demo Line\"\n}".to_string(),
            console: Vec::new(),
            native_path: String::new(),
            native_message: None,
            native_display: String::new(),
        }
    }
}

/// A resolved drawable instance of a model part (CPU side, shared with wgpu).
#[derive(Clone)]
pub struct SceneSnapshot {
    pub meshes: Vec<CpuMesh>,
    pub instances: Vec<Instance>,
    pub grid: bool,
}

/// The whole editor state.
pub struct EditorState {
    pub pack: ResourcePack,
    pub pack_path: Option<PathBuf>,
    pub dirty: bool,
    pub selection: Selection,
    pub locale: Locale,
    pub config: AppConfig,

    pub cam: Camera,
    pub show_grid: bool,
    /// Manual door override (when the animation player is paused).
    pub door_manual: f64,

    pub anim: AnimPlayer,
    pub vvvf: VvvfPanel,
    pub script: ScriptPanel,

    /// Parsed model data of the currently selected vehicle's primary model.
    pub model: Option<LoadedModel>,
    /// Validation results (recomputed on demand).
    pub validation: Vec<ValidationItem>,
    /// Status message shown in the status bar.
    pub status: String,
    /// Export / demo messages.
    pub export_message: Option<String>,
}

/// Everything needed to preview one vehicle model.
pub struct LoadedModel {
    pub obj: ObjModel,
    pub materials: HashMap<String, Material>,
    pub properties: ModelProperties,
    pub positions: PositionDefinitions,
    /// Vehicle texture RGBA8 (from textureResource or generated).
    pub texture: Option<(Vec<u8>, u32, u32)>,
    /// Source ids for reference.
    #[allow(dead_code)]
    pub model_resource: String,
    #[allow(dead_code)]
    pub texture_resource: String,
}

impl EditorState {
    pub fn new(locale: Locale, config: AppConfig) -> EditorState {
        EditorState {
            pack: ResourcePack::new("New pack"),
            pack_path: None,
            dirty: false,
            selection: Selection::None,
            locale,
            config,
            cam: Camera::default(),
            show_grid: true,
            door_manual: 0.0,
            anim: AnimPlayer::new(),
            vvvf: VvvfPanel::default(),
            script: ScriptPanel::default(),
            model: None,
            validation: Vec::new(),
            status: t(locale, "status.ready"),
            export_message: None,
        }
    }

    /// Selected vehicle index, if any.
    pub fn selected_vehicle(&self) -> Option<usize> {
        match self.selection {
            Selection::Vehicle(i)
            | Selection::Model { vehicle: i, .. }
            | Selection::Part { vehicle: i, .. } => Some(i),
            _ => None,
        }
    }

    /// Load the model of the selected vehicle's primary model entry.
    ///
    /// Supports every model format MTR itself loads (see `OptimizedModel`
    /// / `MqoModelConverter` in the mod): `.obj`, `.bbmodel`, `.mqo` and the
    /// gzipped `.mqoz` — many published packs (including the major Chinese
    /// train packs) use Metasequoia models, which previously fell into the
    /// OBJ branch and silently left an empty viewport.
    pub fn reload_model(&mut self) {
        self.model = None;
        let Some(vi) = self.selected_vehicle() else { return };
        let Some(vehicle) = self.pack.resources.vehicles.get(vi) else { return };
        let Some(model) = vehicle.models.first() else {
            self.status = t(self.locale, "model.no_model");
            return;
        };

        let model_resource = model.model_resource.clone();
        let Some(obj_bytes) = self.pack.resource(&model_resource, "obj") else {
            self.status = format!("{}: {}", t(self.locale, "error.io"), model_resource);
            return;
        };
        let lower = model_resource.to_ascii_lowercase();

        // (model, materials carried over from the importer — MQO has its
        // own; Blockbench names materials after internal texture names,
        // which MTR binds to the model entry's `textureResource`).
        let (obj, imported_materials): (ObjModel, HashMap<String, Material>) =
            if lower.ends_with(".bbmodel") {
            match mrpc_core::bbmodel::parse_bbmodel(&String::from_utf8_lossy(obj_bytes)) {
                Ok(m) => {
                    // Without this map every bbmodel part rendered flat
                    // gray: `build_scene` only marks a submesh `textured`
                    // when its material has a `map_kd`, and the bbmodel
                    // parser names submeshes after Blockbench texture names
                    // that never appear in any `.mtl`.
                    let mut mats = HashMap::new();
                    for part in &m.parts {
                        for sub in &part.meshes {
                            if !sub.material.is_empty() {
                                mats.insert(
                                    sub.material.clone(),
                                    Material {
                                        map_kd: Some(model.texture_resource.clone()),
                                        ..Default::default()
                                    },
                                );
                            }
                        }
                    }
                    (m, mats)
                }
                Err(e) => {
                    self.status = format!("{}: {e}", t(self.locale, "error.parse"));
                    return;
                }
            }
        } else if lower.ends_with(".mqoz") {
            match mrpc_core::mqo::parse_mqoz(obj_bytes) {
                Ok(im) => {
                    let materials = mqo_materials(&im);
                    (im.obj, materials)
                }
                Err(e) => {
                    self.status = format!("{}: {e}", t(self.locale, "error.parse"));
                    return;
                }
            }
        } else if lower.ends_with(".mqo") {
            match mrpc_core::mqo::parse_mqo(&String::from_utf8_lossy(obj_bytes)) {
                Ok(im) => {
                    let materials = mqo_materials(&im);
                    (im.obj, materials)
                }
                Err(e) => {
                    self.status = format!("{}: {e}", t(self.locale, "error.parse"));
                    return;
                }
            }
        } else {
            match ObjModel::parse(&String::from_utf8_lossy(obj_bytes)) {
                Ok(m) => (m, HashMap::new()),
                Err(e) => {
                    self.status = format!("{}: {e}", t(self.locale, "error.parse"));
                    return;
                }
            }
        };

        // Materials: the importer's (MQO) or the sibling `.mtl` of the OBJ.
        let mut materials = imported_materials;
        if let Some(mtllib) = &obj.mtllib {
            let mtl_path = sibling_resource(&model_resource, mtllib, "mtl");
            if let Some(bytes) = self.pack.resource(&mtl_path, "mtl") {
                let parsed = mrpc_core::mtl::parse_mtl(&String::from_utf8_lossy(bytes));
                if !parsed.is_empty() {
                    materials = parsed;
                }
            }
        }

        // Properties + positions.
        let mut properties: ModelProperties = self
            .pack
            .resource(&model.model_properties_resource, "json")
            .and_then(|b| serde_json::from_slice(b).ok())
            .unwrap_or_default();
        let positions: PositionDefinitions = self
            .pack
            .resource(&model.position_definitions_resource, "json")
            .and_then(|b| serde_json::from_slice(b).ok())
            .unwrap_or_default();

        // MTR parity (`ModelProperties.addPartsIfEmpty`): a properties file
        // with no parts renders the WHOLE model as one part at the origin —
        // model entries without a `modelPropertiesResource` (bogies, simple
        // cars, …) are still visible instead of producing an empty viewport.
        let mut part_names: Vec<String> = obj.parts.iter().map(|p| p.name.clone()).collect();
        properties.add_parts_if_empty(&mut part_names);

        // Texture.
        let texture_resource = model.texture_resource.clone();
        let texture = self
            .pack
            .resource(&texture_resource, "png")
            .and_then(|b| load_rgba8(b));

        self.model = Some(LoadedModel {
            obj,
            materials,
            properties,
            positions,
            texture,
            model_resource,
            texture_resource,
        });
        self.status = t(self.locale, "status.ready");
    }

    /// Build the drawable scene for the current model & door state.
    pub fn build_scene(&self) -> SceneSnapshot {
        let mut meshes: Vec<CpuMesh> = Vec::new();
        let mut instances: Vec<Instance> = Vec::new();
        let Some(loaded) = &self.model else {
            return SceneSnapshot { meshes, instances, grid: self.show_grid };
        };

        let (door_value, opening) = if self.anim.playing {
            self.anim.door()
        } else {
            (self.door_manual, true)
        };

        let model_y_offset = loaded.properties.model_y_offset;

        // Every properties part × OBJ part name × placement.
        for (part_idx, part) in loaded.properties.parts.iter().enumerate() {
            let placements = loaded.positions.resolve(&part.position_definitions);
            for name in &part.names {
                let Some(obj_part) = loaded.obj.parts.iter().find(|p| &p.name == name) else {
                    continue;
                };
                for placement in &placements {
                    // Door animation offset (model units).
                    let dx = part.door_animation_type.door_animation_x(
                        part.door_x_multiplier,
                        placement.flipped,
                        door_value,
                    );
                    let dz = part.door_animation_type.door_animation_z(
                        part.door_z_multiplier,
                        placement.flipped,
                        door_value,
                        opening,
                    );

                    for sub in &obj_part.meshes {
                        let material = loaded
                            .materials
                            .get(&sub.material)
                            .cloned()
                            .unwrap_or_default();
                        let tint = if sub.material.is_empty() || material.map_kd.is_none() {
                            // Unmapped material: use Kd (default black → light gray).
                            let kd = material.kd;
                            if kd == [0.0, 0.0, 0.0] { [0.8, 0.8, 0.8] } else { kd }
                        } else {
                            [1.0, 1.0, 1.0]
                        };
                        let selected = matches!(
                            self.selection,
                            Selection::Part { part, .. } if part == part_idx
                        );
                        let emissive =
                            part.part_type == mrpc_core::model_props::PartType::Display
                                || part.render_stage == mrpc_core::model_props::RenderStage::Light
                                || part.render_stage
                                    == mrpc_core::model_props::RenderStage::AlwaysOnLight;

                        let mesh_idx = meshes.len();
                        let mut centroid = [0f32; 3];
                        if !sub.vertices.is_empty() {
                            for v in &sub.vertices {
                                centroid[0] += v.pos[0];
                                centroid[1] += v.pos[1];
                                centroid[2] += v.pos[2];
                            }
                            for c in &mut centroid {
                                *c /= sub.vertices.len() as f32;
                            }
                        }
                        meshes.push(CpuMesh {
                            vertices: sub.vertices.clone(),
                            indices: sub.indices.clone(),
                            tint,
                            alpha: material.d,
                            textured: material.map_kd.is_some(),
                            emissive,
                            selected,
                            centroid,
                        });
                        instances.push(Instance::new(
                            mesh_idx,
                            placement.x,
                            placement.y + model_y_offset * 16.0,
                            placement.z,
                            dx,
                            dz,
                        ));
                    }
                }
            }
        }

        SceneSnapshot { meshes, instances, grid: self.show_grid }
    }

    /// Train simulation state shared by the VVVF and script previews.
    pub fn train_sim(&self) -> TrainSimState {
        let (door_value, opening) = if self.anim.playing {
            self.anim.door()
        } else {
            (self.door_manual, false)
        };
        let (_, phase) = self.anim.cycle.sample(self.anim.t_ms as i64);
        TrainSimState {
            speed_kmh: self.vvvf.speed_kmh,
            door_value,
            door_opening: opening
                || matches!(phase, mrpc_core::door_anim::DoorPhase::Opening),
            notch: self.vvvf.notch as i32,
            elapsed_dwell_ms: self.anim.t_ms as i64,
            sim_time_ms: self.anim.t_ms,
            frame_delta_ms: 16.0,
            ..Default::default()
        }
    }

    /// Re-run validation.
    pub fn revalidate(&mut self) {
        self.validation = validate::validate_pack(&self.pack);
    }

    /// Fit the orbit camera to the loaded model so it is actually visible.
    ///
    /// Real MTR cars are ~20 blocks = 320 model units long, while the default
    /// viewing distance (34 units, a grid of ±14) only frames the middle
    /// sliver of the car — a freshly opened pack looked broken. Mirrors what
    /// a user would expect from any 3D editor's "frame selected" action.
    pub fn fit_camera(&mut self) {
        let scene = self.build_scene();
        let mut min = [f64::MAX, f64::MAX, f64::MAX];
        let mut max = [f64::MIN, f64::MIN, f64::MIN];
        let mut any = false;
        for inst in &scene.instances {
            let Some(mesh) = scene.meshes.get(inst.mesh) else { continue };
            for v in &mesh.vertices {
                any = true;
                let p = [
                    v.pos[0] as f64 + inst.x + inst.dx,
                    v.pos[1] as f64 + inst.y,
                    v.pos[2] as f64 + inst.z + inst.dz,
                ];
                for a in 0..3 {
                    min[a] = min[a].min(p[a]);
                    max[a] = max[a].max(p[a]);
                }
            }
        }
        if !any {
            return;
        }
        let center = [
            (min[0] + max[0]) * 0.5,
            (min[1] + max[1]) * 0.5,
            (min[2] + max[2]) * 0.5,
        ];
        let extent = (max[0] - min[0])
            .max(max[1] - min[1])
            .max(max[2] - min[2])
            .max(1.0);
        self.cam.target = glam::Vec3::new(
            center[0] as f32,
            center[1] as f32,
            center[2] as f32,
        );
        // ~76° of the view taken by the object at 50° fov leaves comfortable
        // margin around it; real cars land at a Blender-like default distance.
        self.cam.dist = (extent * 1.35).clamp(6.0, 200.0) as f32;
    }
}

/// Compute the sibling resource id (`mtllib` next to the OBJ, per MTR rules).
fn sibling_resource(model_resource: &str, file: &str, ext: &str) -> String {
    let (ns, path) = match model_resource.split_once(':') {
        Some((ns, p)) => (ns, p),
        None => ("mtr", model_resource),
    };
    let dir = match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    };
    let file = file.trim_matches('"');
    let file = if file.contains('.') { file.to_string() } else { format!("{file}.{ext}") };
    if dir.is_empty() {
        format!("{ns}:{file}")
    } else {
        format!("{ns}:{dir}/{file}")
    }
}

fn load_rgba8(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some((rgba.into_raw(), w, h))
}

/// Convert the MQO importer's material list into the OBJ/MTL material map the
/// scene builder expects (MQO models carry their materials inside the file).
fn mqo_materials(imported: &mrpc_core::import::ImportedModel) -> HashMap<String, Material> {
    imported
        .materials
        .iter()
        .map(|m| {
            (
                m.name.clone(),
                Material {
                    kd: m.kd,
                    ka: [0.0, 0.0, 0.0],
                    ks: [0.0, 0.0, 0.0],
                    ns: 0.0,
                    d: m.d,
                    map_kd: m.map_kd.clone(),
                },
            )
        })
        .collect()
}
