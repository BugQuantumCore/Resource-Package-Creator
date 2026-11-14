//! Vehicle entries of `mtr_custom_resources.json` (`vehicles` array, MTR 4.0.x).
//!
//! Field names mirror `org.mtr.mod.resource.VehicleResource` (and its generated
//! schema class) exactly — camelCase keys, all fields written on export so the
//! game-side reader never sees missing keys.

use serde::{Deserialize, Serialize};
use serde_json::Map;

/// Base transport modes understood by MTR / TSC (`org.mtr.core.data.TransportMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransportMode {
    #[default]
    #[serde(rename = "TRAIN")]
    Train,
    #[serde(rename = "BOAT")]
    Boat,
    #[serde(rename = "CABLE_CAR")]
    CableCar,
    #[serde(rename = "AIRPLANE")]
    Airplane,
    /// Tolerant fallback for unknown values; serialises as `UNKNOWN`.
    #[serde(other, rename = "UNKNOWN")]
    Unknown,
}

/// One car model variant of a vehicle (`VehicleModel` in MTR source).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VehicleModel {
    /// Model file, e.g. `mtr:custom/my_train/car.obj`, `.bbmodel`, `.mqo`, `.mqoz`.
    #[serde(rename = "modelResource")]
    pub model_resource: String,
    /// Texture applied to material `default.png` / empty material, e.g. `mtr:custom/my_train/car.png`.
    #[serde(rename = "textureResource", default)]
    pub texture_resource: String,
    /// Model properties JSON (`parts`, gangway/barrier…).
    #[serde(rename = "modelPropertiesResource", default)]
    pub model_properties_resource: String,
    /// Position definitions JSON (named position sets for replicated parts).
    #[serde(rename = "positionDefinitionsResource", default)]
    pub position_definitions_resource: String,
    #[serde(rename = "flipTextureV", default)]
    pub flip_texture_v: bool,
    /// Unknown keys, preserved on round-trip.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, serde_json::Value>,
}

impl Default for VehicleModel {
    fn default() -> Self {
        Self {
            model_resource: String::new(),
            texture_resource: String::new(),
            model_properties_resource: String::new(),
            position_definitions_resource: String::new(),
            flip_texture_v: false,
            extra: Map::new(),
        }
    }
}

/// A vehicle entry of the `vehicles` array.
///
/// In MTR 4 a "vehicle" describes a single car shape; the game assembles trains
/// out of these. JCM Yanyang adds `scriptId` and `hideDisplayParts` on the same
/// entry (see `com.lx862.mtrscripting.mod.resource.VehicleResourceProvider`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VehicleResource {
    pub id: String,
    /// Display name (MTR built-ins use translation keys; custom packs use plain text).
    #[serde(default)]
    pub name: String,
    /// Hex colour without `#`, e.g. `FEC5E5`.
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub transport_mode: TransportMode,
    /// Car length in blocks.
    #[serde(default = "default_length")]
    pub length: f64,
    /// Car width in blocks.
    #[serde(default = "default_width")]
    pub width: f64,
    #[serde(rename = "bogie1Position", default = "default_bogie1")]
    pub bogie1_position: f64,
    #[serde(rename = "bogie2Position", default = "default_bogie2")]
    pub bogie2_position: f64,
    #[serde(rename = "couplingPadding1", default)]
    pub coupling_padding1: f64,
    #[serde(rename = "couplingPadding2", default)]
    pub coupling_padding2: f64,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "wikipediaArticle", default)]
    pub wikipedia_article: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub models: Vec<VehicleModel>,
    #[serde(rename = "bogie1Models", default)]
    pub bogie1_models: Vec<VehicleModel>,
    #[serde(rename = "bogie2Models", default)]
    pub bogie2_models: Vec<VehicleModel>,
    #[serde(rename = "hasGangway1", default)]
    pub has_gangway1: bool,
    #[serde(rename = "hasGangway2", default)]
    pub has_gangway2: bool,
    #[serde(rename = "hasBarrier1", default)]
    pub has_barrier1: bool,
    #[serde(rename = "hasBarrier2", default)]
    pub has_barrier2: bool,
    #[serde(rename = "legacyRiderOffset", default)]
    pub legacy_rider_offset: f64,
    /// BVE sound set base, e.g. `mtr:custom/my_train/sound` → `sounds/…/sound.cfg`.
    #[serde(rename = "bveSoundBaseResource", default)]
    pub bve_sound_base_resource: String,
    #[serde(rename = "legacySpeedSoundBaseResource", default)]
    pub legacy_speed_sound_base_resource: String,
    #[serde(rename = "legacySpeedSoundCount", default)]
    pub legacy_speed_sound_count: i64,
    #[serde(rename = "legacyUseAccelerationSoundsWhenCoasting", default)]
    pub legacy_use_acceleration_sounds_when_coasting: bool,
    #[serde(rename = "legacyConstantPlaybackSpeed", default)]
    pub legacy_constant_playback_speed: bool,
    #[serde(rename = "legacyDoorSoundBaseResource", default)]
    pub legacy_door_sound_base_resource: String,
    #[serde(rename = "legacyDoorCloseSoundTime", default)]
    pub legacy_door_close_sound_time: f64,

    // ---- JCM (Yanyang) extensions on the vehicle entry ----
    /// References a `vehicleScripts` entry id.
    #[serde(rename = "scriptId", default, skip_serializing_if = "Option::is_none")]
    pub script_id: Option<String>,
    /// Let scripts draw their own displays instead of the built-in display cube.
    #[serde(rename = "hideDisplayParts", default, skip_serializing_if = "std::ops::Not::not")]
    pub hide_display_parts: bool,

    /// Unknown keys, preserved on round-trip.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, serde_json::Value>,
}

fn default_color() -> String {
    "888888".to_string()
}
fn default_length() -> f64 {
    20.0
}
fn default_width() -> f64 {
    2.5
}
fn default_bogie1() -> f64 {
    -6.0
}
fn default_bogie2() -> f64 {
    6.0
}

impl VehicleResource {
    /// Colour as `0xRRGGBB` (defaults to gray on parse failure).
    pub fn color_rgb(&self) -> u32 {
        u32::from_str_radix(self.color.trim_start_matches('#'), 16).unwrap_or(0x888888)
    }

    /// The primary car model (first entry of `models`), if any.
    pub fn primary_model(&self) -> Option<&VehicleModel> {
        self.models.first()
    }

    /// Is this a legacy MTR 3 style "retexture" entry (no models, only extras)?
    /// Such entries keep `base_train_type` / `texture_id` inside `extra`.
    pub fn is_legacy_retexture(&self) -> bool {
        self.models.is_empty()
            && (self.extra.contains_key("base_train_type") || self.extra.contains_key("texture_id"))
    }
}

impl Default for VehicleResource {
    fn default() -> Self {
        Self {
            id: "custom_train".to_string(),
            name: "Custom Train".to_string(),
            color: default_color(),
            transport_mode: TransportMode::Train,
            length: default_length(),
            width: default_width(),
            bogie1_position: default_bogie1(),
            bogie2_position: default_bogie2(),
            coupling_padding1: 0.0,
            coupling_padding2: 0.0,
            description: String::new(),
            wikipedia_article: String::new(),
            tags: Vec::new(),
            models: vec![VehicleModel::default()],
            bogie1_models: Vec::new(),
            bogie2_models: Vec::new(),
            has_gangway1: false,
            has_gangway2: false,
            has_barrier1: false,
            has_barrier2: false,
            legacy_rider_offset: 0.0,
            bve_sound_base_resource: String::new(),
            legacy_speed_sound_base_resource: String::new(),
            legacy_speed_sound_count: 0,
            legacy_use_acceleration_sounds_when_coasting: false,
            legacy_constant_playback_speed: false,
            legacy_door_sound_base_resource: String::new(),
            legacy_door_close_sound_time: 0.0,
            script_id: None,
            hide_display_parts: false,
            extra: Map::new(),
        }
    }
}
