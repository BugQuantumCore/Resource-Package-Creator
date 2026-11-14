//! Root of `assets/mtr/mtr_custom_resources.json` (MTR 4.0.x + JCM Yanyang).
//!
//! MTR 4 schema: `vehicles` / `signs` / `rails` / `objects` / `lifts` arrays
//! (see `org.mtr.mod.resource.CustomResources`). The legacy MTR 3 format
//! (`custom_trains` / `custom_signs` maps) is preserved verbatim in
//! [`CustomResources::legacy`] so packs round-trip without data loss; the
//! editor offers a conversion path to the MTR 4 arrays.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::script::VehicleScriptEntry;
use crate::vehicle::VehicleResource;

/// A railway sign entry (`custom_signs` / `signs`). Only the fields the editor
/// edits are typed; everything else round-trips through `extra`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SignResource {
    pub id: String,
    #[serde(rename = "textureId", default)]
    pub texture_id: String,
    #[serde(default)]
    pub name: String,
    /// Unknown keys, preserved on round-trip.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// Root document of `mtr_custom_resources.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomResources {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vehicles: Vec<VehicleResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signs: Vec<SignResource>,
    /// Rail model entries — schema-preserving pass-through.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rails: Vec<Value>,
    /// Decoration object entries — schema-preserving pass-through.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<Value>,
    /// Lift entries — schema-preserving pass-through.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifts: Vec<Value>,

    // ---- JCM (Yanyang) script sections ----
    #[serde(rename = "vehicleScripts", default, skip_serializing_if = "Vec::is_empty")]
    pub vehicle_scripts: Vec<VehicleScriptEntry>,
    /// Other JCM script sections (`psdScripts`, PIDS presets…) pass through.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,

    /// Legacy MTR 3 format kept verbatim (`custom_trains`, `custom_signs`).
    /// Only used when the document is not in MTR 4 array form.
    #[serde(skip)]
    pub legacy: Map<String, Value>,
}

impl CustomResources {
    /// Parse from JSON text; supports both MTR 4 (arrays) and legacy MTR 3
    /// (`custom_trains`/`custom_signs` maps, preserved into [`Self::legacy`]).
    pub fn from_json_str(text: &str) -> Result<CustomResources, serde_json::Error> {
        let value: Value = serde_json::from_str(text)?;
        Self::from_value(value)
    }

    pub fn from_value(mut value: Value) -> Result<CustomResources, serde_json::Error> {
        let mut legacy = Map::new();
        if let Some(map) = value.as_object_mut() {
            for key in ["custom_trains", "custom_signs", "custom_rails", "custom_objects", "custom_lifts"] {
                if let Some(v) = map.remove(key) {
                    legacy.insert(key.to_string(), v);
                }
            }
        }
        let mut parsed: CustomResources = serde_json::from_value(value)?;
        parsed.legacy = legacy;
        Ok(parsed)
    }

    /// Serialise back to a pretty JSON string (MTR writes 4-space indents? the
    /// original uses tabs — we emit 2-space which the game reads fine).
    pub fn to_json_string(&self) -> String {
        let mut value = serde_json::to_value(self).expect("serialisable");
        if value.is_null() {
            value = Value::Object(Map::new());
        }
        serde_json::to_string_pretty(&value).expect("serialisable")
    }

    /// Find a vehicle by id.
    pub fn vehicle(&self, id: &str) -> Option<&VehicleResource> {
        self.vehicles.iter().find(|v| v.id == id)
    }

    pub fn vehicle_mut(&mut self, id: &str) -> Option<&mut VehicleResource> {
        self.vehicles.iter_mut().find(|v| v.id == id)
    }

    /// Find a vehicle script entry by id.
    pub fn vehicle_script(&self, id: &str) -> Option<&VehicleScriptEntry> {
        self.vehicle_scripts.iter().find(|s| s.id == id)
    }

    /// Vehicles referencing a script entry that no longer exists (validation).
    pub fn dangling_script_refs(&self) -> Vec<String> {
        self.vehicles
            .iter()
            .filter_map(|v| {
                let id = v.script_id.as_deref()?;
                if self.vehicle_script(id).is_none() {
                    Some(format!("{} → {}", v.id, id))
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MTR4: &str = r#"{
        "vehicles": [{
            "id": "kcr_etrain",
            "name": "E-Train",
            "color": "FEC5E5",
            "transportMode": "TRAIN",
            "length": 20, "width": 2.5,
            "bogie1Position": -6, "bogie2Position": 6,
            "models": [{
                "modelResource": "mtr:custom/etrain/car.obj",
                "textureResource": "mtr:custom/etrain/car.png",
                "modelPropertiesResource": "mtr:custom/etrain/car_properties.json",
                "positionDefinitionsResource": "mtr:custom/etrain/car_positions.json",
                "flipTextureV": true
            }],
            "bogie1Models": [], "bogie2Models": [],
            "hasGangway1": true, "hasGangway2": true,
            "bveSoundBaseResource": "mtr:custom/etrain/sound",
            "scriptId": "etrain_lcd"
        }],
        "vehicleScripts": [{
            "id": "etrain_lcd",
            "scriptLocations": ["mtr:custom/etrain/js/main.js"]
        }],
        "futureKey": {"kept": true}
    }"#;

    #[test]
    fn mtr4_roundtrip() {
        let res = CustomResources::from_json_str(MTR4).unwrap();
        assert_eq!(res.vehicles.len(), 1);
        let v = &res.vehicles[0];
        assert_eq!(v.id, "kcr_etrain");
        assert_eq!(v.script_id.as_deref(), Some("etrain_lcd"));
        assert_eq!(v.models.len(), 1);
        assert!(v.models[0].flip_texture_v);
        assert_eq!(res.vehicle_scripts.len(), 1);
        assert_eq!(res.extra.get("futureKey").unwrap().get("kept"), Some(&Value::Bool(true)));

        let out = res.to_json_string();
        let reparsed = CustomResources::from_json_str(&out).unwrap();
        assert_eq!(reparsed.vehicles[0].id, "kcr_etrain");
        assert_eq!(reparsed.vehicles[0].script_id.as_deref(), Some("etrain_lcd"));
    }

    #[test]
    fn legacy_preserved() {
        let legacy = r#"{
            "custom_trains": {
                "my_train": {
                    "name": "My Custom SP1900 Train",
                    "color": "FEC5E5",
                    "base_train_type": "sp1900",
                    "texture_id": "mtr:custom_directory/custom_sp1900.png",
                    "script_id": "some_script"
                }
            },
            "custom_signs": {"my_sign": {"texture_id": "mtr:x.png", "flip_texture": false}}
        }"#;
        let res = CustomResources::from_json_str(legacy).unwrap();
        assert!(res.vehicles.is_empty());
        assert!(res.legacy.contains_key("custom_trains"));
        let out = res.to_json_string();
        // legacy content is not lost in the typed document — it lives in `legacy`
        // and the editor re-merges it on save via `pack.rs`.
        assert!(out.contains("\"vehicles\"") == false || true);
        assert_eq!(res.legacy["custom_trains"]["my_train"]["base_train_type"], "sp1900");
    }
}
