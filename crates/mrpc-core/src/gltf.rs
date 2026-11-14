//! glTF 2.0 / GLB model importer (subset sufficient for train models).
//!
//! Supports `.gltf` (JSON with base64 or external buffers/images) and `.glb`
//! (binary container). Decodes triangle primitives with POSITION / NORMAL /
//! TEXCOORD_0 attributes and optional indices, bakes the node hierarchy's
//! TRS / matrix transforms, and converts PBR materials to MTL: `baseColorFactor`
//! → `Kd` (linear → sRGB) and `baseColorTexture` → re-encoded PNG `map_Kd`.
//!
//! Coordinate system: glTF is right-handed Y-up like OBJ/Minecraft, so
//! positions pass through unchanged. glTF texture coordinates have their
//! origin at the top-left (`v` downwards), matching Minecraft sampling with
//! `flipTextureV = false`. Skins, morph targets, animations, sparse accessors
//! and non-triangle primitive modes are ignored.

use std::collections::HashMap;

use base64::Engine as _;
use glam::{Mat3, Mat4, Quat, Vec3};
use serde::Deserialize;

use crate::import::{build_submesh, ImportedMaterial, ImportedModel, ImportedTexture};
use crate::obj::{ObjModel, ObjPart};

#[derive(Debug, thiserror::Error)]
pub enum GltfError {
    #[error("glTF: {0}")]
    Parse(String),
    #[error("glTF: invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("glTF: image decode/re-encode: {0}")]
    Image(#[from] image::ImageError),
}

/// Callback resolving an external URI (relative file name) to its bytes.
pub type ExternalResolver<'a> = &'a mut dyn FnMut(&str) -> Option<Vec<u8>>;

/// Parse `.gltf` or `.glb` bytes into an importable model.
pub fn parse_gltf(bytes: &[u8], resolve_external: ExternalResolver) -> Result<ImportedModel, GltfError> {
    if bytes.len() >= 4 && &bytes[0..4] == b"glTF" {
        parse_glb(bytes, resolve_external)
    } else {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| GltfError::Parse("file is neither UTF-8 JSON nor a GLB".into()))?;
        let doc: Document = serde_json::from_str(text)?;
        let buffers = load_buffers(&doc, None, resolve_external)?;
        build_import(doc, buffers, resolve_external)
    }
}

// ---------------------------------------------------------------------------
// glTF JSON document structures
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Document {
    #[serde(default)]
    meshes: Vec<Mesh>,
    #[serde(default)]
    materials: Vec<MaterialDef>,
    #[serde(default)]
    accessors: Vec<Accessor>,
    #[serde(default)]
    buffer_views: Vec<BufferView>,
    #[serde(default)]
    buffers: Vec<BufferDef>,
    #[serde(default)]
    nodes: Vec<Node>,
    #[serde(default)]
    scenes: Vec<Scene>,
    #[serde(default)]
    scene: Option<usize>,
    #[serde(default)]
    images: Vec<ImageDef>,
    #[serde(default)]
    textures: Vec<TextureDef>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Mesh {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    primitives: Vec<Primitive>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Primitive {
    #[serde(default)]
    attributes: HashMap<String, usize>,
    #[serde(default)]
    indices: Option<usize>,
    #[serde(default)]
    material: Option<usize>,
    #[serde(default = "default_mode")]
    mode: u32,
}

fn default_mode() -> u32 {
    4
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaterialDef {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    pbr_metallic_roughness: PbrMetallicRoughness,
    #[serde(default)]
    alpha_mode: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PbrMetallicRoughness {
    #[serde(default = "default_base_color")]
    base_color_factor: [f32; 4],
    #[serde(default)]
    base_color_texture: Option<TextureInfo>,
}

fn default_base_color() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextureInfo {
    index: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Node {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    mesh: Option<usize>,
    #[serde(default)]
    children: Vec<usize>,
    #[serde(default)]
    matrix: Option<[f32; 16]>,
    #[serde(default)]
    translation: Option<[f32; 3]>,
    #[serde(default)]
    rotation: Option<[f32; 4]>,
    #[serde(default)]
    scale: Option<[f32; 3]>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scene {
    #[serde(default)]
    nodes: Vec<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageDef {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    mime_type: Option<String>,
    #[serde(default)]
    buffer_view: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextureDef {
    #[serde(default)]
    source: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BufferDef {
    #[serde(default)]
    uri: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BufferView {
    buffer: usize,
    #[serde(default)]
    byte_offset: usize,
    #[serde(default)]
    byte_length: usize,
    #[serde(default)]
    byte_stride: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Accessor {
    #[serde(default)]
    buffer_view: Option<usize>,
    #[serde(default)]
    byte_offset: usize,
    #[serde(default)]
    count: usize,
    component_type: u32,
    #[serde(rename = "type")]
    type_: String,
}

// ---------------------------------------------------------------------------
// Binary loading
// ---------------------------------------------------------------------------

fn parse_glb(bytes: &[u8], resolve_external: ExternalResolver) -> Result<ImportedModel, GltfError> {
    if bytes.len() < 12 {
        return Err(GltfError::Parse("GLB too short".into()));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version != 2 {
        return Err(GltfError::Parse(format!("unsupported GLB version {version}")));
    }
    let mut json_bytes: Option<&[u8]> = None;
    let mut bin: Option<Vec<u8>> = None;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let ctype = &bytes[pos + 4..pos + 8];
        let start = pos + 8;
        let end = start.checked_add(len).ok_or_else(|| GltfError::Parse("bad chunk length".into()))?;
        if end > bytes.len() {
            return Err(GltfError::Parse("chunk overruns file".into()));
        }
        if ctype == b"JSON" {
            json_bytes = Some(&bytes[start..end]);
        } else if ctype == b"BIN\0" {
            bin = Some(bytes[start..end].to_vec());
        }
        pos = end + ((4 - (len & 3)) & 3); // chunks are 4-byte aligned
    }
    let json_bytes = json_bytes.ok_or_else(|| GltfError::Parse("GLB has no JSON chunk".into()))?;
    let doc: Document = serde_json::from_slice(json_bytes)?;
    let buffers = load_buffers(&doc, bin, resolve_external)?;
    build_import(doc, buffers, resolve_external)
}

fn load_buffers(
    doc: &Document,
    glb_bin: Option<Vec<u8>>,
    resolve_external: ExternalResolver,
) -> Result<Vec<Vec<u8>>, GltfError> {
    let mut out = Vec::with_capacity(doc.buffers.len());
    for (i, b) in doc.buffers.iter().enumerate() {
        let data = match (&b.uri, &glb_bin) {
            (Some(uri), _) if uri.starts_with("data:") => {
                let payload = uri
                    .split_once(',')
                    .map(|(_, d)| d)
                    .ok_or_else(|| GltfError::Parse("malformed data URI".into()))?;
                base64::engine::general_purpose::STANDARD
                    .decode(payload.trim())
                    .map_err(|e| GltfError::Parse(format!("data URI decode: {e}")))?
            }
            (Some(uri), _) => {
                let fname = uri.rsplit(['/']).next().unwrap_or(uri);
                resolve_external(fname)
                    .ok_or_else(|| GltfError::Parse(format!("external buffer not found: {uri}")))?
            }
            (None, Some(bin)) => bin.clone(),
            (None, None) => {
                if i == 0 {
                    return Err(GltfError::Parse("buffer 0 has neither URI nor GLB BIN chunk".into()));
                }
                Vec::new()
            }
        };
        out.push(data);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Accessor decoding
// ---------------------------------------------------------------------------

fn component_size(component_type: u32) -> Option<usize> {
    match component_type {
        5120 | 5121 => Some(1), // (u)int8
        5122 | 5123 => Some(2), // (u)int16
        5125 => Some(4),        // uint32
        5126 => Some(4),        // float32
        _ => None,
    }
}

fn type_components(type_: &str) -> Option<usize> {
    match type_ {
        "SCALAR" => Some(1),
        "VEC2" => Some(2),
        "VEC3" => Some(3),
        "VEC4" => Some(4),
        "MAT4" => Some(16),
        _ => None,
    }
}

/// Decode one accessor into flat f32 values (row-major per element).
fn read_accessor_f32(doc: &Document, buffers: &[Vec<u8>], acc: &Accessor) -> Result<Vec<f32>, GltfError> {
    let Some(view_idx) = acc.buffer_view else {
        return Ok(Vec::new()); // sparse / computed accessor: treated as absent
    };
    let comps = type_components(&acc.type_)
        .ok_or_else(|| GltfError::Parse(format!("unsupported accessor type {}", acc.type_)))?;
    let csize = component_size(acc.component_type)
        .ok_or_else(|| GltfError::Parse(format!("unsupported component type {}", acc.component_type)))?;
    let view = doc
        .buffer_views
        .get(view_idx)
        .ok_or_else(|| GltfError::Parse("accessor references missing bufferView".to_string()))?;
    let buffer = buffers
        .get(view.buffer)
        .ok_or_else(|| GltfError::Parse("bufferView references missing buffer".to_string()))?;
    let elem_size = comps * csize;
    let stride = view.byte_stride.unwrap_or(elem_size).max(elem_size);
    let mut out = Vec::with_capacity(acc.count * comps);
    for i in 0..acc.count {
        let off = view.byte_offset + acc.byte_offset + i * stride;
        let end = off + elem_size;
        if end > buffer.len() {
            return Err(GltfError::Parse("accessor data out of bounds".into()));
        }
        for c in 0..comps {
            let b = &buffer[off + c * csize..off + (c + 1) * csize];
            let v = match acc.component_type {
                5120 => b[0] as i8 as f32,
                5121 => b[0] as f32,
                5122 => i16::from_le_bytes([b[0], b[1]]) as f32,
                5123 => u16::from_le_bytes([b[0], b[1]]) as f32,
                5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                5126 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                _ => unreachable!(),
            };
            out.push(v);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Import assembly
// ---------------------------------------------------------------------------

fn build_import(
    doc: Document,
    buffers: Vec<Vec<u8>>,
    resolve_external: ExternalResolver,
) -> Result<ImportedModel, GltfError> {
    // ---- textures ----
    let mut textures: Vec<ImportedTexture> = Vec::new();
    // glTF image index → index into `textures`.
    let mut image_map: HashMap<usize, usize> = HashMap::new();
    let encode_png = |bytes: &[u8]| -> Result<Vec<u8>, GltfError> {
        let img = image::load_from_memory(bytes)?;
        let rgba = img.to_rgba8();
        let mut png = Vec::new();
        rgba.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
        Ok(png)
    };
    for (i, img) in doc.images.iter().enumerate() {
        let png: Option<Vec<u8>> = if let Some(view_idx) = img.buffer_view {
            let view = doc.buffer_views.get(view_idx);
            let bytes = view.and_then(|v| {
                buffers.get(v.buffer).and_then(|b| {
                    let end = v.byte_offset.checked_add(v.byte_length)?;
                    b.get(v.byte_offset..end)
                })
            });
            match bytes {
                Some(b) => Some(encode_png(b)?),
                None => None,
            }
        } else if let Some(uri) = &img.uri {
            if uri.starts_with("data:") {
                match uri.split_once(',').map(|(_, d)| d) {
                    Some(payload) => match base64::engine::general_purpose::STANDARD
                        .decode(payload.trim())
                    {
                        Ok(b) => Some(encode_png(&b)?),
                        Err(_) => None,
                    },
                    None => None,
                }
            } else {
                let fname = uri.rsplit(['/']).next().unwrap_or(uri);
                match resolve_external(fname) {
                    Some(b) => Some(encode_png(&b)?),
                    None => None,
                }
            }
        } else {
            None
        };
        if let Some(png) = png {
            let name = img
                .name
                .clone()
                .unwrap_or_else(|| format!("tex_{i}"))
                .rsplit(['/']).next().unwrap_or("tex").to_string();
            let filename = if name.to_ascii_lowercase().ends_with(".png") {
                name
            } else {
                format!("tex_{i}.png")
            };
            image_map.insert(i, textures.len());
            textures.push(ImportedTexture { filename, png });
        }
    }

    // ---- materials ----
    let materials: Vec<ImportedMaterial> = doc
        .materials
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let f = m.pbr_metallic_roughness.base_color_factor;
            let map = m
                .pbr_metallic_roughness
                .base_color_texture
                .as_ref()
                .and_then(|t| doc.textures.get(t.index))
                .and_then(|t| t.source)
                .and_then(|s| image_map.get(&s))
                .map(|&ti| textures[ti].filename.clone());
            ImportedMaterial {
                name: m.name.clone().unwrap_or_else(|| format!("mat{i}")),
                kd: [linear_to_srgb(f[0]), linear_to_srgb(f[1]), linear_to_srgb(f[2])],
                d: if m.alpha_mode.as_deref() == Some("BLEND") {
                    f[3].clamp(0.0, 1.0)
                } else {
                    1.0
                },
                map_kd: map,
            }
        })
        .collect();

    // ---- scene traversal with transforms ----
    let scene_idx = doc.scene.unwrap_or(0);
    let root_nodes: Vec<usize> = doc
        .scenes
        .get(scene_idx)
        .map(|s| s.nodes.clone())
        .unwrap_or_else(|| (0..doc.nodes.len()).collect());

    let mut parts: Vec<ObjPart> = Vec::new();
    let mut queue: Vec<(usize, Mat4)> = root_nodes
        .into_iter()
        .filter(|&n| n < doc.nodes.len())
        .map(|n| (n, Mat4::IDENTITY))
        .collect();
    while let Some((ni, parent)) = queue.pop() {
        let node = &doc.nodes[ni];
        let local = node_matrix(node);
        let world = parent * local;
        for &c in &node.children {
            if c < doc.nodes.len() {
                queue.push((c, world));
            }
        }
        if let Some(mi) = node.mesh {
            let node_name = node.name.as_deref();
            if let Some(part) = build_part(&doc, &buffers, mi, node_name, &world, &materials)? {
                parts.push(part);
            }
        }
    }

    // Merge parts with duplicate names (keep first).
    let mut merged: Vec<ObjPart> = Vec::new();
    for p in parts {
        if let Some(existing) = merged.iter_mut().find(|e| e.name == p.name) {
            existing.meshes.extend(p.meshes);
        } else {
            merged.push(p);
        }
    }

    Ok(ImportedModel {
        obj: ObjModel { parts: merged, mtllib: Some("model.mtl".into()) },
        flip_texture_v: false,
        materials,
        textures,
    })
}

fn node_matrix(node: &Node) -> Mat4 {
    if let Some(m) = node.matrix {
        // glTF matrices are column-major.
        return Mat4::from_cols_array(&m);
    }
    let t = node.translation.map(Vec3::from).unwrap_or(Vec3::ZERO);
    let s = node.scale.map(Vec3::from).unwrap_or(Vec3::ONE);
    let q = match node.rotation {
        Some([x, y, z, w]) => Quat::from_xyzw(x, y, z, w),
        None => Quat::IDENTITY,
    };
    Mat4::from_scale_rotation_translation(s, q, t)
}

fn build_part(
    doc: &Document,
    buffers: &[Vec<u8>],
    mesh_idx: usize,
    node_name: Option<&str>,
    world: &Mat4,
    materials: &[ImportedMaterial],
) -> Result<Option<ObjPart>, GltfError> {
    let mesh = match doc.meshes.get(mesh_idx) {
        Some(m) => m,
        None => return Ok(None),
    };
    let name = mesh
        .name
        .clone()
        .or_else(|| node_name.map(|s| s.to_string()))
        .unwrap_or_else(|| format!("mesh{}", mesh_idx));
    let mut part = ObjPart { name, meshes: Vec::new() };

    let normal_mat = Mat3::from_mat4(*world);
    let normal_mat = normal_mat
        .try_inverse()
        .map(|inv| inv.transpose())
        .unwrap_or(normal_mat);

    for prim in &mesh.primitives {
        if prim.mode != 4 {
            continue; // triangles only
        }
        let Some(&pos_acc) = prim.attributes.get("POSITION") else { continue };
        let positions = read_accessor_f32(
            doc,
            buffers,
            doc.accessors.get(pos_acc).ok_or_else(|| GltfError::Parse("missing POSITION accessor".to_string()))?,
        )?;
        let normals = prim
            .attributes
            .get("NORMAL")
            .and_then(|&a| doc.accessors.get(a))
            .map(|a| read_accessor_f32(doc, buffers, a))
            .transpose()?;
        let uvs = prim
            .attributes
            .get("TEXCOORD_0")
            .and_then(|&a| doc.accessors.get(a))
            .map(|a| read_accessor_f32(doc, buffers, a))
            .transpose()?;

        let vcount = positions.len() / 3;
        let indices: Vec<usize> = match prim.indices {
            Some(ia) => {
                let a = doc
                    .accessors
                    .get(ia)
                    .ok_or_else(|| GltfError::Parse("missing indices accessor".to_string()))?;
                read_accessor_f32(doc, buffers, a)?
                    .into_iter()
                    .map(|f| f as usize)
                    .collect()
            }
            None => (0..vcount).collect(),
        };

        let mut corners: Vec<([f32; 3], [f32; 2])> = Vec::with_capacity(indices.len());
        for &vi in &indices {
            if vi >= vcount {
                return Err(GltfError::Parse("index out of range".into()));
            }
            let raw = [
                positions[vi * 3],
                positions[vi * 3 + 1],
                positions[vi * 3 + 2],
            ];
            let p = *world * glam::Vec4::new(raw[0], raw[1], raw[2], 1.0);
            let pos = [p.x, p.y, p.z];
            let uv = match &uvs {
                Some(u) if vi * 2 + 1 < u.len() => [u[vi * 2], u[vi * 2 + 1]],
                _ => [0.0, 0.0],
            };
            corners.push((pos, uv));
        }

        // Smooth normals from the NORMAL attribute, if present.
        let corner_normals: Option<Vec<[f32; 3]>> = normals.as_ref().map(|ns| {
            let mut out = Vec::with_capacity(indices.len());
            for &vi in &indices {
                let raw = if vi * 3 + 2 < ns.len() {
                    [ns[vi * 3], ns[vi * 3 + 1], ns[vi * 3 + 2]]
                } else {
                    [0.0, 0.0, 0.0]
                };
                let tn = normal_mat * glam::Vec3::new(raw[0], raw[1], raw[2]);
                let l = tn.length();
                if l > 1e-9 {
                    out.push([tn.x / l, tn.y / l, tn.z / l]);
                } else {
                    out.push([0.0, 0.0, 0.0]);
                }
            }
            out
        });

        let material = prim
            .material
            .and_then(|mi| materials.get(mi))
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "default".to_string());

        let mut submesh = build_submesh(material, &corners);
        if let Some(ns) = corner_normals {
            for (v, n) in submesh.vertices.iter_mut().zip(ns) {
                if n != [0.0, 0.0, 0.0] {
                    v.normal = n;
                }
            }
        }
        if !submesh.indices.is_empty() {
            part.meshes.push(submesh);
        }
    }

    if part.meshes.is_empty() {
        Ok(None)
    } else {
        Ok(Some(part))
    }
}

/// Linear-sRGB → sRGB transfer (approximate curve, adequate for MTL Kd).
fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal .gltf: one triangle, baseColorFactor, embedded data URI buffer.
    fn sample_gltf() -> Vec<u8> {
        // 3 positions × vec3 f32 + 3 uvs × vec2 f32 + 3 indices × u16
        let pos: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let uv: Vec<f32> = vec![0.0, 1.0, 1.0, 1.0, 0.0, 0.0];
        let idx: Vec<u16> = vec![0, 1, 2];
        let mut bin = Vec::new();
        for v in pos {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        for v in uv {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        for v in idx {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bin);
        let json = format!(
            r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"mesh": 0, "name": "tri", "translation": [0.0, 0.0, 5.0]}}],
  "meshes": [{{"name": "tri", "primitives": [{{"attributes": {{"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}}, "indices": 3, "material": 0}}]}}],
  "materials": [{{"name": "mymat", "pbrMetallicRoughness": {{"baseColorFactor": [1.0, 0.0, 0.0, 1.0]}}}}],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
    {{"bufferView": 0, "byteOffset": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
    {{"bufferView": 0, "byteOffset": 36, "componentType": 5126, "count": 3, "type": "VEC2"}},
    {{"bufferView": 0, "byteOffset": 60, "componentType": 5123, "count": 3, "type": "SCALAR"}}
  ],
  "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": {len}}}],
  "buffers": [{{"uri": "data:application/octet-stream;base64,{b64}"}}]
}}"#,
            len = bin.len(),
            b64 = b64
        );
        json.into_bytes()
    }

    #[test]
    fn parses_gltf_with_data_uri() {
        let mut none = |_uri: &str| -> Option<Vec<u8>> { None };
        let imported = parse_gltf(&sample_gltf(), &mut none).unwrap();
        assert_eq!(imported.obj.parts.len(), 1);
        let part = &imported.obj.parts[0];
        assert_eq!(part.name, "tri");
        assert_eq!(part.meshes.len(), 1);
        assert_eq!(part.meshes[0].material, "mymat");
        assert_eq!(part.meshes[0].vertices.len(), 3);
        // Node translation applied (z +5).
        assert_eq!(part.meshes[0].vertices[0].pos, [0.0, 0.0, 5.0]);
        // Normals decoded from the accessor (all zero here → face normal used).
        assert!(part.meshes[0].vertices.iter().all(|v| v.normal != [0.0, 0.0, 0.0]));
        // UV passthrough (v downwards).
        assert_eq!(part.meshes[0].vertices[0].uv, [0.0, 1.0]);
        // Material colour: linear 1,0,0 → sRGB 1,0,0.
        assert!((imported.materials[0].kd[0] - 1.0).abs() < 1e-6);
        assert_eq!(imported.materials[0].kd[1], 0.0);
    }

    #[test]
    fn glb_container_roundtrip() {
        // Build a minimal GLB with a BIN chunk (no buffer URI).
        let pos: Vec<f32> = vec![0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0, 0.0];
        let mut bin = Vec::new();
        for v in pos {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        let doc = r#"{"asset":{"version":"2.0"},"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"bufferViews":[{"buffer":0,"byteLength":36}],"buffers":[{}]}"#;
        let json = doc.as_bytes().to_vec();
        let mut glb = Vec::new();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        let json_padded_len = (json.len() + 3) & !3;
        let bin_padded_len = (bin.len() + 3) & !3;
        let total = 12 + 8 + json_padded_len + 8 + bin_padded_len;
        glb.extend_from_slice(&(total as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend(std::iter::repeat_n(0x20u8, json_padded_len - json.len()));
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&bin);
        glb.extend(std::iter::repeat_n(0u8, bin_padded_len - bin.len()));

        let mut none = |_uri: &str| -> Option<Vec<u8>> { None };
        let imported = parse_gltf(&glb, &mut none).unwrap();
        assert_eq!(imported.obj.parts.len(), 1);
        assert_eq!(imported.obj.parts[0].meshes[0].vertices.len(), 3);
        assert_eq!(imported.obj.parts[0].meshes[0].vertices[1].pos, [2.0, 0.0, 0.0]);
    }

    #[test]
    fn external_image_becomes_png_texture() {
        // 1×1 red PNG.
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let pos: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let mut bin = Vec::new();
        for v in pos {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bin);
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"material":0}}]}}],"materials":[{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}}}}}}],"images":[{{"uri":"sign.png"}}],"textures":[{{"source":0}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}}],"bufferViews":[{{"buffer":0,"byteLength":36}}],"buffers":[{{"uri":"data:application/octet-stream;base64,{b64}"}}]}}"#
        );
        let mut resolver = |uri: &str| -> Option<Vec<u8>> {
            if uri == "sign.png" {
                Some(png.clone())
            } else {
                None
            }
        };
        let imported = parse_gltf(json.as_bytes(), &mut resolver).unwrap();
        assert_eq!(imported.textures.len(), 1);
        assert_eq!(imported.textures[0].filename, "tex_0.png");
        assert_eq!(imported.materials[0].map_kd.as_deref(), Some("tex_0.png"));
        assert!(imported.textures[0].png.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn linear_to_srgb_curve() {
        assert!((linear_to_srgb(0.5) - 0.7354).abs() < 0.001);
        assert_eq!(linear_to_srgb(0.0), 0.0);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-6);
    }
}
