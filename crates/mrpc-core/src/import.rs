//! Shared plumbing for external model import (MQO / MQOZ / glTF / GLB).
//!
//! Importers convert foreign formats into the internal [`ObjModel`] plus a
//! generated MTL and re-encoded PNG textures, so everything downstream
//! (3D preview, packaging, validation) keeps working on one representation.
//!
//! UV convention: importers emit texture coordinates with `v` increasing
//! downwards from the top-left of the image (Metasequoia and glTF both use
//! this convention), matching how Minecraft / MTR sample textures when
//! `flipTextureV` is `false` — the recommended flag for imported models.

use crate::obj::{ObjModel, ObjSubMesh, Vertex};

/// A texture carried by an import, encoded as PNG.
#[derive(Debug, Clone)]
pub struct ImportedTexture {
    /// Resource file name (without path), e.g. `tex_0.png`.
    pub filename: String,
    /// PNG-encoded image bytes.
    pub png: Vec<u8>,
}

/// One imported material, serialisable to MTL.
#[derive(Debug, Clone)]
pub struct ImportedMaterial {
    pub name: String,
    /// Diffuse colour, sRGB display values 0..1.
    pub kd: [f32; 3],
    /// Opacity 0..1 (1 = opaque).
    pub d: f32,
    /// Diffuse texture filename, if any.
    pub map_kd: Option<String>,
}

/// The result of an import, ready to be written into a resource pack.
#[derive(Debug, Clone)]
pub struct ImportedModel {
    pub obj: ObjModel,
    /// Recommended `flipTextureV` for the vehicle model entry.
    pub flip_texture_v: bool,
    pub materials: Vec<ImportedMaterial>,
    pub textures: Vec<ImportedTexture>,
}

impl ImportedModel {
    /// Number of triangles across all parts.
    pub fn triangle_count(&self) -> usize {
        self.obj
            .parts
            .iter()
            .flat_map(|p| p.meshes.iter())
            .map(|m| m.indices.len() / 3)
            .sum()
    }

    /// Number of vertices across all parts.
    pub fn vertex_count(&self) -> usize {
        self.obj
            .parts
            .iter()
            .flat_map(|p| p.meshes.iter())
            .map(|m| m.vertices.len())
            .sum()
    }
}

/// Serialise an [`ObjModel`] back to Wavefront OBJ text.
///
/// Vertices are written unindexed (already expanded per corner by the
/// parsers), with per-vertex normals and texture coordinates. Face indices
/// are global 1-based `v/vt/vn` triples with a running offset across
/// sub-meshes, as required by the OBJ format.
pub fn obj_to_text(model: &ObjModel) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("# Exported by MTR Resource Package Creator\n");
    if let Some(mtl) = &model.mtllib {
        out.push_str(&format!("mtllib {mtl}\n"));
    }
    // Global running vertex index (1-based OBJ numbering).
    let mut base: usize = 0;
    for part in &model.parts {
        out.push_str(&format!("o {}\n", sanitize_name(&part.name)));
        for (mi, mesh) in part.meshes.iter().enumerate() {
            let mat = if mesh.material.is_empty() {
                format!("part{mi}")
            } else {
                sanitize_name(&mesh.material)
            };
            out.push_str(&format!("usemtl {mat}\n"));
            for v in &mesh.vertices {
                out.push_str(&format!(
                    "v {:.6} {:.6} {:.6}\n",
                    v.pos[0], v.pos[1], v.pos[2]
                ));
            }
            for v in &mesh.vertices {
                out.push_str(&format!("vt {:.6} {:.6}\n", v.uv[0], v.uv[1]));
            }
            for v in &mesh.vertices {
                out.push_str(&format!(
                    "vn {:.6} {:.6} {:.6}\n",
                    v.normal[0], v.normal[1], v.normal[2]
                ));
            }
            for tri in mesh.indices.chunks_exact(3) {
                let i = |n: u32| base + n as usize + 1;
                out.push_str(&format!(
                    "f {}/{}/{} {}/{}/{} {}/{}/{}\n",
                    i(tri[0]),
                    i(tri[0]),
                    i(tri[0]),
                    i(tri[1]),
                    i(tri[1]),
                    i(tri[1]),
                    i(tri[2]),
                    i(tri[2]),
                    i(tri[2])
                ));
            }
            base += mesh.vertices.len();
        }
    }
    out
}

/// Serialise imported materials to MTL text.
pub fn mtl_to_text(materials: &[ImportedMaterial]) -> String {
    let mut out = String::from("# Exported by MTR Resource Package Creator\n");
    for m in materials {
        // `kd` is stored LINEAR (see the parsers); MTL Kd is sRGB, so
        // convert back or a round-trip darkens the color twice.
        let kd = crate::linear3_to_srgb(m.kd);
        out.push_str(&format!(
            "newmtl {}\nKd {:.4} {:.4} {:.4}\n",
            sanitize_name(&m.name),
            kd[0].clamp(0.0, 1.0),
            kd[1].clamp(0.0, 1.0),
            kd[2].clamp(0.0, 1.0)
        ));
        if (m.d - 1.0).abs() > 1e-3 {
            out.push_str(&format!("d {:.4}\n", m.d.clamp(0.0, 1.0)));
        }
        if let Some(map) = &m.map_kd {
            out.push_str(&format!("map_Kd {}\n", sanitize_name(map)));
        }
        out.push('\n');
    }
    out
}

/// Replace characters that are unsafe in OBJ/MTL names / file names.
fn sanitize_name(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| match c {
            ' ' | '\t' | '\n' | '\r' | '"' | '#' | '\\' => '_',
            _ => c,
        })
        .collect();
    if s.is_empty() {
        s.push_str("unnamed");
    }
    s
}

/// Compute a face normal for three positions (normalised).
pub(crate) fn tri_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let mut n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l > 1e-12 {
        for x in &mut n {
            *x /= l;
        }
    }
    n
}

/// Build an expanded sub-mesh from corner data (fan triangulation).
///
/// Polygons become triangle fans — corner `0`, `i`, `i+1` — mirroring how
/// the OBJ parser triangulates `f` lines, so exports and re-parses agree.
pub(crate) fn build_submesh(
    material: String,
    corners: &[([f32; 3], [f32; 2])],
) -> ObjSubMesh {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    if corners.len() >= 3 {
        let n = tri_normal(corners[0].0, corners[1].0, corners[2].0);
        // Fan: (0, i, i+1) for i in 1..len-1 — corners duplicated per
        // triangle, matching how the OBJ parser expands `f` lines.
        for i in 1..corners.len() - 1 {
            for &j in &[0usize, i, i + 1] {
                let (pos, uv) = corners[j];
                vertices.push(Vertex { pos, normal: n, uv });
                indices.push((vertices.len() - 1) as u32);
            }
        }
    }
    ObjSubMesh { material, vertices, indices }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obj::ObjModel;

    #[test]
    fn roundtrip_obj_text() {
        let text = "
o body
v 0 0 0
v 1 0 0
v 0 1 0
vt 0 0
vt 1 0
vt 0 1
f 1/1 2/2 3/3
";
        let model = ObjModel::parse(text).unwrap();
        let exported = obj_to_text(&model);
        let reparsed = ObjModel::parse(&exported).unwrap();
        assert_eq!(reparsed.parts.len(), 1);
        assert_eq!(reparsed.parts[0].meshes[0].vertices.len(), 3);
        let p0 = reparsed.parts[0].meshes[0].vertices[0].pos;
        assert_eq!(p0, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn roundtrip_multi_submesh_offsets() {
        // Two sub-meshes in one part: global face indices must not restart.
        let text = "
o body
usemtl a
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
usemtl b
v 0 2 0
v 1 2 0
v 0 3 0
f 4 5 6
";
        let model = ObjModel::parse(text).unwrap();
        assert_eq!(model.parts[0].meshes.len(), 2);
        let exported = obj_to_text(&model);
        let re = ObjModel::parse(&exported).unwrap();
        assert_eq!(re.parts[0].meshes.len(), 2);
        // Sub-mesh order after re-parse follows the parser's bucket map —
        // locate by material name.
        let mb = re.parts[0].meshes.iter().find(|m| m.material == "b").unwrap();
        assert_eq!(mb.vertices.len(), 3);
        assert_eq!(mb.vertices[0].pos, [0.0, 2.0, 0.0]);
        let ma = re.parts[0].meshes.iter().find(|m| m.material == "a").unwrap();
        assert_eq!(ma.vertices.len(), 3);
        assert_eq!(ma.vertices[0].pos, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn mtl_text_has_entries() {
        let mtl = mtl_to_text(&[
            ImportedMaterial {
                name: "red".into(),
                kd: [1.0, 0.1, 0.1],
                d: 1.0,
                map_kd: None,
            },
            ImportedMaterial {
                name: "glass".into(),
                kd: [0.8, 0.9, 1.0],
                d: 0.5,
                map_kd: Some("tex_0.png".into()),
            },
        ]);
        assert!(mtl.contains("newmtl red"));
        assert!(mtl.contains("newmtl glass"));
        assert!(mtl.contains("d 0.5000"));
        assert!(mtl.contains("map_Kd tex_0.png"));
    }
}
