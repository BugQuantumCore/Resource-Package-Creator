//! Blockbench `.bbmodel` parser → the same mesh representation as the OBJ
//! loader, so previews and exporters treat both identically (MTR 4 loads
//! `.bbmodel` natively as a model resource).

use crate::obj::{ObjModel, ObjPart, ObjSubMesh, Vertex};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct BbModel {
    /// Model name (informational; parts carry their own names).
    #[serde(default)]
    #[allow(dead_code)]
    name: String,
    /// Elements are parsed one-by-one (as raw JSON) so that a single
    /// unsupported element — e.g. a Blockbench `mesh` (which has no `from`/
    /// `to` box and differently-shaped `faces`) — is skipped instead of
    /// failing the WHOLE model file. MTR renders cube elements; meshes are
    /// not supported there either.
    #[serde(default)]
    elements: Vec<serde_json::Value>,
    #[serde(default)]
    textures: Vec<BbTexture>,
}

#[derive(Debug, Deserialize)]
struct BbElement {
    #[serde(default)]
    name: String,
    #[serde(default)]
    from: [f64; 3],
    #[serde(default)]
    to: [f64; 3],
    #[serde(default)]
    origin: Option<[f64; 3]>,
    #[serde(default)]
    rotation: Option<[f64; 3]>,
    #[serde(default)]
    faces: HashMap<String, BbFace>,
}

#[derive(Debug, Deserialize)]
struct BbFace {
    #[serde(default)]
    uv: Option<[f64; 4]>,
    /// Index into the model's `textures` array; -1 = no texture.
    texture: i64,
}

#[derive(Debug, Deserialize)]
struct BbTexture {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    name: String,
}

/// Face normals for the six box faces, keyed like Blockbench (`north`…`up`).
const FACE_NORMALS: [(&str, [f32; 3]); 6] = [
    ("north", [0.0, 0.0, -1.0]),
    ("south", [0.0, 0.0, 1.0]),
    ("west", [-1.0, 0.0, 0.0]),
    ("east", [1.0, 0.0, 0.0]),
    ("down", [0.0, -1.0, 0.0]),
    ("up", [0.0, 1.0, 0.0]),
];

/// Parse a `.bbmodel` into the shared mesh representation.
///
/// Each Blockbench element becomes one named part (MTR semantics: named parts
/// are what model-properties `names` reference). Texture references resolve to
/// the Blockbench texture *name* (e.g. `body.png`), matching MTR's
/// `BlockbenchModel` which treats them like OBJ `map_Kd` entries.
pub fn parse_bbmodel(text: &str) -> Result<ObjModel, String> {
    let model: BbModel =
        serde_json::from_str(text).map_err(|e| format!("invalid bbmodel JSON: {e}"))?;
    let textures: HashMap<i64, String> = model
        .textures
        .iter()
        .map(|t| (t.id, t.name.clone()))
        .collect();

    let mut parts: Vec<ObjPart> = Vec::new();
    for element_value in &model.elements {
        // Tolerant per-element parse: cube elements keep working even when
        // the file also contains meshes or other unknown element shapes.
        let Some(element) = serde_json::from_value::<BbElement>(element_value.clone()).ok() else {
            continue;
        };
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();

        // Box corners (before rotation), Y-up like Blockbench.
        let [x0, y0, z0] = element.from;
        let [x1, y1, z1] = element.to;
        let (x0, x1) = (x0 as f32, x1 as f32);
        let (y0, y1) = (y0 as f32, y1 as f32);
        let (z0, z1) = (z0 as f32, z1 as f32);
        let corners: [[f32; 3]; 8] = [
            [x0, y0, z0],
            [x1, y0, z0],
            [x1, y1, z0],
            [x0, y1, z0],
            [x0, y0, z1],
            [x1, y0, z1],
            [x1, y1, z1],
            [x0, y1, z1],
        ];
        let corners: Vec<[f32; 3]> = match (element.rotation, element.origin) {
            (Some(rot), Some(origin)) if rot.iter().any(|r| *r != 0.0) => corners
                .iter()
                .map(|c| rotate_xyz(*c, origin, rot))
                .collect(),
            _ => corners.to_vec(),
        };

        let texture_name = |face: &BbFace| -> String {
            if face.texture < 0 {
                String::new()
            } else {
                textures.get(&face.texture).cloned().unwrap_or_default()
            }
        };

        // Per-face quad → two triangles; UV from the bbmodel (already 0..1).
        for (key, normal) in FACE_NORMALS {
            let Some(face) = element.faces.get(key) else { continue };
            let uv = face.uv.unwrap_or([0.0, 0.0, 1.0, 1.0]);
            let (u0, v0, u1, v1) =
                (uv[0] as f32, uv[1] as f32, uv[2] as f32, uv[3] as f32);
            let quad: [usize; 4] = match key {
                "north" => [4, 5, 6, 7], // z = z1? north is -z in Blockbench
                "south" => [0, 1, 2, 3],
                "west" => [0, 3, 7, 4],
                "east" => [1, 2, 6, 5],
                "down" => [0, 1, 5, 4],
                _ => [3, 2, 6, 7],
            };
            let _ = texture_name(face); // material decided below per part
            let uvs = [[u0, v1], [u1, v1], [u1, v0], [u0, v0]];
            let base = vertices.len() as u32;
            for (i, ci) in quad.iter().enumerate() {
                vertices.push(Vertex {
                    pos: corners[*ci],
                    normal,
                    uv: uvs[i],
                });
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }

        if vertices.is_empty() {
            continue;
        }
        // One material per element: the first textured face's texture name.
        let material = element
            .faces
            .values()
            .find(|f| f.texture >= 0)
            .and_then(|f| textures.get(&f.texture).cloned())
            .unwrap_or_default();
        parts.push(ObjPart {
            name: element.name.clone(),
            meshes: vec![ObjSubMesh { material, vertices, indices }],
        });
    }

    Ok(ObjModel { parts, mtllib: None })
}

fn rotate_xyz(p: [f32; 3], origin: [f64; 3], rot: [f64; 3]) -> [f32; 3] {
    // Rotate around the element origin, X then Y then Z (degrees).
    let mut v = [
        p[0] - origin[0] as f32,
        p[1] - origin[1] as f32,
        p[2] - origin[2] as f32,
    ];
    let (rx, ry, rz) = (
        (rot[0] as f32).to_radians(),
        (rot[1] as f32).to_radians(),
        (rot[2] as f32).to_radians(),
    );
    // X
    let (s, c) = (rx.sin(), rx.cos());
    v = [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c];
    // Y
    let (s, c) = (ry.sin(), ry.cos());
    v = [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c];
    // Z
    let (s, c) = (rz.sin(), rz.cos());
    v = [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]];
    [v[0] + origin[0] as f32, v[1] + origin[1] as f32, v[2] + origin[2] as f32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_box() {
        let bb = r#"{
            "meta": {"format_version": "4.5"},
            "name": "demo",
            "textures": [{"id": 0, "name": "body.png"}],
            "elements": [{
                "name": "body",
                "from": [0, 0, 0],
                "to": [16, 16, 64],
                "origin": [8, 8, 32],
                "faces": {
                    "north": {"uv": [0, 0, 1, 1], "texture": 0},
                    "south": {"uv": [0, 0, 1, 1], "texture": 0},
                    "up": {"uv": [0, 0, 1, 1], "texture": 0},
                    "down": {"uv": [0, 0, 1, 1], "texture": 0}
                }
            }]
        }"#;
        let m = parse_bbmodel(bb).unwrap();
        assert_eq!(m.parts.len(), 1);
        assert_eq!(m.parts[0].name, "body");
        assert_eq!(m.parts[0].meshes[0].material, "body.png");
        assert_eq!(m.parts[0].meshes[0].vertices.len(), 16); // 4 faces × 4 verts
        assert_eq!(m.parts[0].meshes[0].indices.len(), 24); // 4 faces × 6 indices
    }
}
