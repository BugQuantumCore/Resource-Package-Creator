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
