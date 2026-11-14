//! Position definitions JSON (`positionDefinitionsResource`).
//!
//! Mirrors `org.mtr.mod.resource.PositionDefinitions` / `PositionDefinition`.
//! Positions are in model units (1/16 block); missing components default to 0.

use serde::{Deserialize, Serialize};
use serde_json::Map;

/// A single placement of a part. Only present components are serialised.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct PartPosition {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub x: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub y: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub z: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// A named set of placements (optionally with a mirrored set for flipped cars).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PositionDefinition {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub positions: Vec<PartPosition>,
    #[serde(rename = "positionsFlipped", default)]
    pub positions_flipped: Vec<PartPosition>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, serde_json::Value>,
}

impl PositionDefinition {
    /// All placements, normal + flipped (flipped mirrors X, like MTR's
    /// `iteratePositions`: `-position.getX(), y, z, flipped = true`).
    pub fn all_placements(&self) -> Vec<Placement> {
        let mut out = Vec::new();
        for p in &self.positions {
            out.push(Placement { x: p.x, y: p.y, z: p.z, flipped: false });
        }
        for p in &self.positions_flipped {
            out.push(Placement { x: -p.x, y: p.y, z: p.z, flipped: true });
        }
        out
    }
}

/// A resolved placement (mirrored X when the car is flipped).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub flipped: bool,
}

/// Root of the position definitions JSON.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PositionDefinitions {
    #[serde(default)]
    pub position_definitions: Vec<PositionDefinition>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, serde_json::Value>,
}

impl PositionDefinitions {
    pub fn get(&self, name: &str) -> Option<&PositionDefinition> {
        self.position_definitions.iter().find(|d| d.name == name)
    }

    /// Resolve a part's `positionDefinitions` names; empty names resolve to a
    /// single identity placement, matching MTR's default `PositionDefinition`.
    pub fn resolve(&self, names: &[String]) -> Vec<Placement> {
        let mut out = Vec::new();
        for name in names {
            match self.get(name) {
                Some(def) => out.extend(def.all_placements()),
                None => out.push(Placement { x: 0.0, y: 0.0, z: 0.0, flipped: false }),
            }
        }
        if out.is_empty() {
            out.push(Placement { x: 0.0, y: 0.0, z: 0.0, flipped: false });
        }
        out
    }
}
