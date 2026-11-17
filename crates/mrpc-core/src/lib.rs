//! mrpc-core — data model for Minecraft Transit Railway 4.0.x custom resource packs.
//!
//! This crate implements the `mtr_custom_resources.json` schema (vehicles / signs /
//! rails / objects / lifts, plus the Joban Client Mod Yanyang `vehicleScripts`
//! extension) together with the auxiliary formats referenced by vehicle entries:
//! OBJ + MTL models, Blockbench `.bbmodel` models, model properties JSON,
//! position definitions JSON, and BVE motor-sound configuration files.
//!
//! All field names are kept byte-compatible with the upstream MTR 4.0.x
//! serializer (camelCase JSON keys); unknown keys are preserved on round-trip.

pub mod bbmodel;
pub mod bve;
pub mod bve_playback;
pub mod door_anim;
pub mod demo;
pub mod format;
pub mod gltf;
pub mod i18n;
pub mod import;
pub mod model_props;
pub mod mqo;
pub mod mtl;
pub mod obj;
pub mod pack;
pub mod positions;
pub mod script;
pub mod validate;
pub mod vehicle;

pub use door_anim::{DoorAnimationType, DoorCycle};
pub use format::CustomResources;
pub use model_props::{
    DisplayOption, DisplayType, ModelProperties, ModelPropertiesPart, PartCondition, PartType,
    RenderStage,
};
pub use positions::{PartPosition, PositionDefinition, PositionDefinitions};
pub use script::{ScriptLanguageKind, VehicleScriptEntry};
pub use vehicle::{TransportMode, VehicleModel, VehicleResource};

/// Convert an sRGB-encoded channel (0..1) to linear light.
///
/// MTL `Kd` and MQO `col` values are authored as display (sRGB) colors, but
/// the viewport writes to an sRGB render target — feeding them straight into
/// the linear pipeline double-brightens them (a dark-gray `Kd 0.3 0.3 0.3`
/// lit as 0.3 linear displays as ~0.58 sRGB, i.e. washed out). glTF
/// `baseColorFactor` is already linear and is NOT passed through this.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Convert an sRGB RGB triple to linear.
pub fn srgb3_to_linear(c: [f32; 3]) -> [f32; 3] {
    [srgb_to_linear(c[0]), srgb_to_linear(c[1]), srgb_to_linear(c[2])]
}

/// Inverse of [`srgb_to_linear`] — used when serialising linear material
/// colors back into sRGB-encoded files (e.g. imported → MTL).
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Convert a linear RGB triple to sRGB.
pub fn linear3_to_srgb(c: [f32; 3]) -> [f32; 3] {
    [linear_to_srgb(c[0]), linear_to_srgb(c[1]), linear_to_srgb(c[2])]
}
