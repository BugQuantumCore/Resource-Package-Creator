//! Wavefront OBJ parser (subset used by MTR train models).
//!
//! Parts are delimited by `o` and `g` statements — matching how MTR's
//! `OptimizedModel.ObjModel` splits a file into named parts (the names the
//! model-properties `names` array refers to). Faces are triangulated with a
//! fan; missing normals are computed per-face; `vt` indices may be absent.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// A sub-mesh sharing one material inside a part.
#[derive(Debug, Clone)]
pub struct ObjSubMesh {
    pub material: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// A named part (`o`/`g` group) of the OBJ file.
#[derive(Debug, Clone)]
pub struct ObjPart {
    pub name: String,
    pub meshes: Vec<ObjSubMesh>,
}

/// Parsed OBJ file.
#[derive(Debug, Clone, Default)]
pub struct ObjModel {
    pub parts: Vec<ObjPart>,
    pub mtllib: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ObjError {
    #[error("invalid face token: {0}")]
    BadFaceToken(String),
    #[error("face references missing vertex index {0}")]
    MissingVertex(usize),
}

fn parse_index(tok: &str, len: usize) -> Option<usize> {
    if tok.is_empty() {
        return None;
    }
    match tok.parse::<isize>() {
        Ok(i) if i > 0 => Some((i - 1) as usize),
        Ok(i) if i < 0 => Some((len as isize + i).max(0) as usize),
        _ => None,
    }
}

impl ObjModel {
    pub fn parse(text: &str) -> Result<ObjModel, ObjError> {
        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut uvs: Vec<[f32; 2]> = Vec::new();
        let mut normals: Vec<[f32; 3]> = Vec::new();

        let mut parts: Vec<ObjPart> = Vec::new();
        let mut current_part: usize = usize::MAX;
        let mut current_material = String::new();
        let mut mtllib = None;

        // (part idx, material) → (vertices, indices)
        let mut buckets: HashMap<(usize, String), (Vec<Vertex>, Vec<u32>)> = HashMap::new();

        'lines: for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut tokens = line.split_whitespace();
            let keyword = tokens.next().unwrap_or("");
            match keyword {
                "v" => {
                    let mut it = [0f32; 3];
                    for (i, tok) in tokens.take(3).enumerate() {
                        it[i] = parse_f32(tok);
                    }
                    positions.push(it);
                }
                "vt" => {
                    let mut it = [0f32; 2];
                    for (i, tok) in tokens.take(2).enumerate() {
                        it[i] = parse_f32(tok);
                    }
                    uvs.push(it);
                }
                "vn" => {
                    let mut it = [0f32; 3];
                    for (i, tok) in tokens.take(3).enumerate() {
                        it[i] = parse_f32(tok);
                    }
                    // Normalise; zero-length stays zero.
                    let l = (it[0] * it[0] + it[1] * it[1] + it[2] * it[2]).sqrt();
                    if l > 1e-12 {
                        for c in &mut it {
                            *c /= l;
                        }
                    }
                    normals.push(it);
                }
                "mtllib" => {
                    mtllib = tokens.next().map(|s| s.trim_matches('"').to_string());
                }
                "usemtl" => {
                    current_material = tokens.next().unwrap_or("").trim_matches('"').to_string();
                }
                "o" | "g" => {
                    let name = line[keyword.len()..].trim().trim_matches('"').to_string();
                    // MTR models: both `o` and `g` introduce parts; repeated
                    // names merge into the first part with that name.
                    if let Some(idx) = parts.iter().position(|p| p.name == name) {
                        current_part = idx;
                    } else {
                        parts.push(ObjPart { name, meshes: Vec::new() });
                        current_part = parts.len() - 1;
                    }
                }
                "f" => {
                    if current_part == usize::MAX {
                        parts.push(ObjPart { name: String::new(), meshes: Vec::new() });
                        current_part = 0;
                    }
                    let mut face: Vec<Vertex> = Vec::new();
                    for tok in tokens {
                        let comps: Vec<&str> = tok.split('/').collect();
                        let vi = parse_index(comps[0], positions.len())
                            .ok_or_else(|| ObjError::BadFaceToken(tok.to_string()))?;
                        let pos = *positions.get(vi).ok_or(ObjError::MissingVertex(vi))?;
                        let uv = comps
                            .get(1)
                            .and_then(|s| parse_index(s, uvs.len()))
                            .map(|i| *uvs.get(i).unwrap_or(&[0.0, 0.0]))
                            .unwrap_or([0.0, 0.0]);
                        let normal = comps
                            .get(2)
                            .and_then(|s| parse_index(s, normals.len()))
                            .map(|i| *normals.get(i).unwrap_or(&[0.0, 0.0, 1.0]))
                            .unwrap_or([0.0, 0.0, 0.0]);
                        face.push(Vertex { pos, normal, uv });
                    }
                    if face.len() < 3 {
                        continue 'lines;
                    }
                    // Fill missing normals with the face normal.
                    if face.iter().any(|v| v.normal == [0.0, 0.0, 0.0]) {
                        let n = face_normal(&face);
                        for v in &mut face {
                            if v.normal == [0.0, 0.0, 0.0] {
                                v.normal = n;
                            }
                        }
                    }
                    let bucket = buckets
                        .entry((current_part, current_material.clone()))
                        .or_insert_with(|| (Vec::new(), Vec::new()));
                    // Fan triangulation.
                    for i in 1..face.len() - 1 {
                        for v in [0, i, i + 1] {
                            bucket.0.push(face[v]);
                            bucket.1.push((bucket.0.len() - 1) as u32);
                        }
                    }
                }
                _ => {}
            }
        }

        let mut model = ObjModel { parts, mtllib };
        let keys: Vec<(usize, String)> = buckets.keys().cloned().collect();
        for (part_idx, material) in keys {
            let (vertices, indices) = buckets.remove(&(part_idx, material.clone())).unwrap();
            model.parts[part_idx].meshes.push(ObjSubMesh { material, vertices, indices });
        }
        Ok(model)
    }

    /// Apply `flipTextureV` (MTR texture convention) in place.
    pub fn flipped_texture_v(mut self) -> ObjModel {
        for part in &mut self.parts {
            for mesh in &mut part.meshes {
                for v in &mut mesh.vertices {
                    v.uv[1] = 1.0 - v.uv[1];
                }
            }
        }
        self
    }

    /// Bounding box `[min, max]` over all parts.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        let mut any = false;
        for part in &self.parts {
            for mesh in &part.meshes {
                for v in &mesh.vertices {
                    any = true;
                    for c in 0..3 {
                        min[c] = min[c].min(v.pos[c]);
                        max[c] = max[c].max(v.pos[c]);
                    }
                }
            }
        }
        if any { Some((min, max)) } else { None }
    }
}

fn parse_f32(tok: &str) -> f32 {
    tok.trim().parse::<f32>().unwrap_or(0.0)
}

fn face_normal(face: &[Vertex]) -> [f32; 3] {
    let a = face[0].pos;
    let b = face[1].pos;
    let c = face[2].pos;
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let mut n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l > 1e-12 {
        for c in &mut n {
            *c /= l;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "
mtllib car.mtl
o body
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
vt 0 0
vt 1 0
vt 1 1
vn 0 0 1
usemtl body_mat
f 1/1/1 2/2/1 3/3/1 4/1/1
o door_left_1
v 0.2 0 0.1
v 0.4 0 0.1
v 0.4 0.8 0.1
f -3// -2// -1//
";

    #[test]
    fn parses_parts_and_materials() {
        let m = ObjModel::parse(SAMPLE).unwrap();
        assert_eq!(m.mtllib.as_deref(), Some("car.mtl"));
        assert_eq!(m.parts.len(), 2);
        assert_eq!(m.parts[0].name, "body");
        assert_eq!(m.parts[0].meshes[0].material, "body_mat");
        // Quad → 2 triangles → 6 vertices (unindexed fan).
        assert_eq!(m.parts[0].meshes[0].vertices.len(), 6);
        assert_eq!(m.parts[1].name, "door_left_1");
        assert_eq!(m.parts[1].meshes[0].vertices.len(), 3);
        // Negative indices resolved, missing normal filled from face.
        assert!(m.parts[1].meshes[0].vertices.iter().all(|v| v.normal != [0.0, 0.0, 0.0]));
    }

    #[test]
    fn flip_texture_v() {
        let m = ObjModel::parse(SAMPLE).unwrap().flipped_texture_v();
        let v = &m.parts[0].meshes[0].vertices[0];
        assert_eq!(v.uv[1], 1.0);
    }
}
