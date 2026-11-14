//! Joban Client Mod (Yanyang) train-script schema — the `vehicleScripts`
//! extension of `mtr_custom_resources.json` and the per-vehicle `scriptId`.
//!
//! Reference: `com.lx862.mtrscripting.mod.resource.VehicleResourceProvider` and
//! `native/README.md` of BugQuantumCore/Joban-Client-Mod-Yanyang:
//!
//! ```json
//! "vehicleScripts": [{
//!     "id": "cst_sp1900_lcd",
//!     "prependExpressions": ["print('Hello world')"],
//!     "scriptLocations": [
//!         "mtr:my_pack/js/sp1900_lcd/main.js",
//!         "mtr:my_pack/natives/sp1900_train_num"
//!     ],
//!     "input": { "yearEra": 2004 },
//!     "dataFetchMode": "ALL",
//!     "forceLoad": true
//! }]
//! ```
//! `scriptLocations` entries ending in `.js` are JavaScript; anything else is a
//! native (C++) module — the two can be mixed in one entry.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// How much train data JCM fetches for the script each frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DataFetchMode {
    /// Do not fetch stop data (cheapest).
    #[default]
    #[serde(rename = "SKIP")]
    Skip,
    /// Fetch data mandatory for vehicle rendering.
    #[serde(rename = "MANDATORY")]
    Mandatory,
    /// Fetch everything (all stops, interchanges…).
    #[serde(rename = "ALL")]
    All,
}

/// How a `scriptLocations` entry is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptLanguageKind {
    JavaScript,
    Native,
}

impl ScriptLanguageKind {
    pub fn detect(location: &str) -> ScriptLanguageKind {
        let lower = location.to_ascii_lowercase();
        if lower.ends_with(".js") {
            ScriptLanguageKind::JavaScript
        } else {
            ScriptLanguageKind::Native
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ScriptLanguageKind::JavaScript => "JavaScript",
            ScriptLanguageKind::Native => "C++ (native)",
        }
    }
}

/// One `vehicleScripts` entry (JCM Yanyang).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VehicleScriptEntry {
    pub id: String,
    /// JavaScript expressions evaluated before the scripts load.
    #[serde(rename = "prependExpressions", default)]
    pub prepend_expressions: Vec<String>,
    /// JS files and/or native module declarations. A native entry may be a
    /// module path (no extension), a full library path (`.so`/`.dll`/`.dylib`)
    /// or a per-platform map `{"windows":…, "linux":…, "macos":…}`.
    #[serde(rename = "scriptLocations", default)]
    pub script_locations: Vec<Value>,
    /// Arbitrary JSON handed to the script as `SCRIPT_INPUT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(rename = "dataFetchMode", default, skip_serializing_if = "Option::is_none")]
    pub data_fetch_mode: Option<DataFetchMode>,
    /// Parse the entry even when no vehicle references it.
    #[serde(rename = "forceLoad", default, skip_serializing_if = "std::ops::Not::not")]
    pub force_load: bool,
    /// Unknown keys, preserved on round-trip.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl VehicleScriptEntry {
    /// Every JS file of this entry (strings, and `.js` members of platform maps).
    pub fn js_locations(&self) -> Vec<String> {
        locations_of_kind(&self.script_locations, |s| {
            s.to_ascii_lowercase().ends_with(".js")
        })
    }

    /// Every native module declaration (non-`.js` strings, non-`.js` members of
    /// platform maps, and whole maps/objects).
    pub fn native_locations(&self) -> Vec<Value> {
        let mut out = Vec::new();
        for loc in &self.script_locations {
            match loc {
                Value::String(s) => {
                    if !s.to_ascii_lowercase().ends_with(".js") {
                        out.push(loc.clone());
                    }
                }
                Value::Object(_) => {
                    // Per-platform map: count as native if any value is non-.js.
                    let any_js = loc.as_object().map(|m| m.values().any(|v| {
                        v.as_str().map(|s| s.to_ascii_lowercase().ends_with(".js")).unwrap_or(false)
                    })).unwrap_or(false);
                    if !any_js {
                        out.push(loc.clone());
                    }
                }
                _ => out.push(loc.clone()),
            }
        }
        out
    }

    pub fn language_summary(&self) -> String {
        let js = self.js_locations().len();
        let native = self.native_locations().len();
        match (js, native) {
            (0, 0) => "—".to_string(),
            (j, 0) => format!("JavaScript ×{j}"),
            (0, n) => format!("C++ ×{n}"),
            (j, n) => format!("JS ×{j} + C++ ×{n}"),
        }
    }
}

fn locations_of_kind(locations: &[Value], pred: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    for loc in locations {
        match loc {
            Value::String(s) => {
                if pred(s) {
                    out.push(s.clone());
                }
            }
            Value::Object(map) => {
                for v in map.values() {
                    if let Some(s) = v.as_str() {
                        if pred(s) {
                            out.push(s.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_roundtrip() {
        let json = r#"{
            "id": "cst_sp1900_lcd",
            "prependExpressions": ["print('Hello world')"],
            "scriptLocations": [
                "mtr:my_pack/js/sp1900_lcd/main.js",
                "mtr:my_pack/natives/sp1900_train_num"
            ],
            "input": { "yearEra": 2004 },
            "dataFetchMode": "ALL",
            "forceLoad": true
        }"#;
        let entry: VehicleScriptEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.id, "cst_sp1900_lcd");
        assert_eq!(entry.data_fetch_mode, Some(DataFetchMode::All));
        assert!(entry.force_load);
        assert_eq!(entry.js_locations(), vec!["mtr:my_pack/js/sp1900_lcd/main.js"]);
        assert_eq!(entry.native_locations().len(), 1);
        let back = serde_json::to_string(&entry).unwrap();
        assert!(back.contains("\"vehicleScripts\"") == false);
        assert!(back.contains("\"scriptLocations\""));
        assert!(back.contains("\"dataFetchMode\":\"ALL\""));
    }

    #[test]
    fn platform_map_native() {
        let entry: VehicleScriptEntry = serde_json::from_value(serde_json::json!({
            "id": "x",
            "scriptLocations": [
                {"windows": "natives/x.dll", "linux": "natives/x.so", "macos": "natives/x.dylib"}
            ]
        }))
        .unwrap();
        assert_eq!(entry.native_locations().len(), 1);
        assert_eq!(entry.js_locations().len(), 0);
    }
}
