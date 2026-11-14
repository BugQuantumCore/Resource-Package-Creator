//! Metasequoia MQO / MQOZ model importer.
//!
//! Parses the text-based `.mqo` format (and its gzipped `.mqoz` variant):
//! `Material` chunk with `col`/`tex` attributes and `Object` chunks with
//! `vertex` / `BVertex` and `face` (`V`, `M`, `UV`) data. Unknown chunks and
//! attributes are skipped tolerantly. The tokeniser keeps line breaks as
//! `Eol` tokens so vertex/face lines can be grouped reliably.
//!
//! Coordinate conversion: Metasequoia is left-handed (x right, y up, z into
//! the screen); OBJ and Minecraft are right-handed, so `z` is negated and
//! the face winding order reversed, preserving which way faces point.
//! UVs keep the Metasequoia top-left origin (`v` downwards), matching
//! Minecraft texture sampling with `flipTextureV = false`.
//!
//! Textures referenced by `tex("...")` are not read here — the caller
//! resolves them next to the source file (see the app import flow).

use std::io::Read;

use crate::import::{build_submesh, ImportedMaterial, ImportedModel};
use crate::obj::{ObjModel, ObjPart};

/// One mesh corner: position + UV.
type Corner = ([f32; 3], [f32; 2]);

#[derive(Debug, thiserror::Error)]
pub enum MqoError {
    #[error("invalid MQO data: {0}")]
    Parse(String),
    #[error("MQOZ is not valid gzip data: {0}")]
    Gzip(#[from] std::io::Error),
}

/// Parse a plain-text `.mqo` document into an importable model.
pub fn parse_mqo(text: &str) -> Result<ImportedModel, MqoError> {
    let toks = tokenize(text)?;
    let mut p = Parser { toks: &toks, pos: 0 };
    let mut materials: Vec<ImportedMaterial> = Vec::new();
    let mut parts: Vec<ObjPart> = Vec::new();

    while !p.at_end() {
        p.skip_eols();
        if p.at_end() {
            break;
        }
        match p.peek_ident() {
            Some("Material") => {
                p.pos += 1;
                let _count = p.next_number();
                p.expect_lbrace()?;
                while !p.at_end() && !p.match_rbrace() {
                    p.skip_eols();
                    if p.match_rbrace() {
                        break;
                    }
                    if p.stray_brace() {
                        continue;
                    }
                    let Some(name) = p.next_string() else {
                        p.pos += 1; // tolerate stray tokens
                        continue;
                    };
                    materials.push(parse_material_attrs(&mut p, name));
                }
            }
            Some("Object") => {
                p.pos += 1;
                let name = p.next_string().unwrap_or_else(|| format!("object{}", parts.len()));
                p.expect_lbrace()?;
                parts.push(parse_object(&mut p, &materials, name)?);
            }
            _ => {
                if !p.skip_chunk() {
                    p.pos += 1;
                }
            }
        }
    }

    // Drop parts without geometry (cameras, empties).
    parts.retain(|p| p.meshes.iter().any(|m| !m.indices.is_empty()));

    Ok(ImportedModel {
        obj: ObjModel { parts, mtllib: Some("model.mtl".into()) },
        flip_texture_v: false,
        materials,
        textures: Vec::new(),
    })
}

/// Parse a gzipped `.mqoz` document.
pub fn parse_mqoz(bytes: &[u8]) -> Result<ImportedModel, MqoError> {
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut text = String::new();
    decoder.read_to_string(&mut text).map_err(MqoError::Gzip)?;
    if text.is_empty() {
        return Err(MqoError::Parse("empty after decompression".into()));
    }
    parse_mqo(&text)
}

fn parse_material_attrs(p: &mut Parser, name: String) -> ImportedMaterial {
    let mut mat = ImportedMaterial { name, kd: [0.8, 0.8, 0.8], d: 1.0, map_kd: None };
    // Attributes run until the next quoted string, the closing brace, or the
    // end of the file.
    loop {
        match p.toks.get(p.pos) {
            None | Some(Tok::Str(_)) | Some(Tok::RBrace) => break,
            Some(Tok::Eol) | Some(Tok::LParen) | Some(Tok::RParen) | Some(Tok::LBrace) => {
                p.pos += 1;
            }
            Some(Tok::Num(_)) => {
                p.pos += 1; // stray number between attributes
            }
            Some(Tok::Ident(key)) => {
                p.pos += 1;
                let args = p.take_paren_args();
                match key.as_str() {
                    "col" => {
                        // col(r g b a) — floats 0..1.
                        let nums = args.numbers();
                        if nums.len() >= 3 {
                            for c in 0..3 {
                                mat.kd[c] = nums[c].clamp(0.0, 1.0);
                            }
                        }
                        if nums.len() >= 4 {
                            mat.d = nums[3].clamp(0.0, 1.0);
                        }
                    }
                    "tex" => {
                        if let Some(Tok::Str(s)) = args.tokens.first() {
                            mat.map_kd = Some(s.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    mat
}

struct ObjectMeshAcc {
    vertices: Vec<([f32; 3], [f32; 2])>,
    // (material index or None, corner indices, corner uvs)
    faces: Vec<(Option<usize>, Vec<usize>, Vec<[f32; 2]>)>,
}

fn parse_object(p: &mut Parser, materials: &[ImportedMaterial], name: String) -> Result<ObjPart, MqoError> {
    let mut acc = ObjectMeshAcc { vertices: Vec::new(), faces: Vec::new() };
    while !p.at_end() && !p.match_rbrace() {
        p.skip_eols();
        if p.at_end() || p.match_rbrace() {
            break;
        }
        if p.stray_brace() {
            continue;
        }
        let Some(Tok::Ident(key)) = p.toks.get(p.pos).cloned() else {
            p.pos += 1;
            continue;
        };
        p.pos += 1;
        match key.as_str() {
            "vertex" | "BVertex" => {
                let _count = p.next_number();
                p.expect_lbrace()?;
                while !p.at_end() && !p.match_rbrace() {
                    // One vertex per line: x y z [u v w ...]
                    let nums = p.next_line_numbers();
                    if nums.len() >= 3 {
                        let uv = if nums.len() >= 5 {
                            [nums[3], nums[4]]
                        } else {
                            [0.0, 0.0]
                        };
                        acc.vertices.push(([nums[0], nums[1], nums[2]], uv));
                    }
                }
            }
            "face" => {
                let _count = p.next_number();
                p.expect_lbrace()?;
                while !p.at_end() && !p.match_rbrace() {
                    if p.stray_brace() {
                        continue;
                    }
                    parse_face_line(p, &mut acc)?;
                }
            }
            _ => {
                if !p.skip_chunk() {
                    let _ = p.next_number_or_string();
                }
            }
        }
    }

    // Convert to ObjPart: one sub-mesh per material bucket.
    let mut buckets: Vec<(Option<usize>, Vec<Corner>)> = Vec::new();
    for (mat_idx, idxs, uvs) in acc.faces {
        if idxs.len() < 3 {
            continue;
        }
        let bucket = match buckets.iter_mut().find(|(m, _)| *m == mat_idx) {
            Some(b) => b,
            None => {
                buckets.push((mat_idx, Vec::new()));
                buckets.last_mut().unwrap()
            }
        };
        // Left-handed → right-handed: negate z, reverse winding.
        for (n, &vi) in idxs.iter().enumerate() {
            let (pos, vuv) = acc.vertices.get(vi).copied().unwrap_or(([0.0; 3], [0.0; 2]));
            let uv = uvs.get(n).copied().unwrap_or(vuv);
            bucket.1.push(([pos[0], pos[1], -pos[2]], uv));
        }
    }

    let meshes = buckets
        .into_iter()
        .map(|(mat_idx, corners)| {
            let material = mat_idx
                .and_then(|i| materials.get(i))
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "default".to_string());
            build_submesh(material, &corners)
        })
        .collect();
    Ok(ObjPart { name, meshes })
}

fn parse_face_line(p: &mut Parser, acc: &mut ObjectMeshAcc) -> Result<(), MqoError> {
    // <corner count> V(..) [M(n)] [UV(..)] [COL(..)] ...
    let Some(Tok::Num(count)) = p.toks.get(p.pos).cloned() else {
        p.pos += 1; // tolerate stray tokens (incl. line breaks)
        return Ok(());
    };
    p.pos += 1;
    let mut idxs: Vec<usize> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut mat_idx: Option<usize> = None;
    while let Some(tok) = p.toks.get(p.pos).cloned() {
        match tok {
            // Braces, line breaks and leading numbers/strings end the line.
            Tok::LBrace | Tok::RBrace | Tok::Eol | Tok::Str(_) | Tok::Num(_) => break,
            Tok::LParen | Tok::RParen => {
                p.pos += 1;
            }
            Tok::Ident(key) => {
                p.pos += 1;
                let args = p.take_paren_args();
                match key.as_str() {
                    "V" => {
                        for t in &args.tokens {
                            if let Tok::Num(n) = t {
                                idxs.push((*n).max(0.0) as usize);
                            }
                        }
                    }
                    "M" => {
                        if let Some(Tok::Num(n)) = args.tokens.first() {
                            mat_idx = Some((*n).max(0.0) as usize);
                        }
                    }
                    "UV" => {
                        let nums = args.numbers();
                        for pair in nums.chunks_exact(2) {
                            uvs.push([pair[0], pair[1]]);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // Trust V(...) over the declared count.
    let _ = count;
    // Reverse winding for the axis flip.
    idxs.reverse();
    uvs.reverse();
    acc.faces.push((mat_idx, idxs, uvs));
    Ok(())
}

// ---------------------------------------------------------------------------
// Tokeniser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Num(f32),
    Str(String),
    LParen,
    RParen,
    LBrace,
    RBrace,
    /// Line break — kept so vertex/face lines can be grouped.
    Eol,
}

fn tokenize(text: &str) -> Result<Vec<Tok>, MqoError> {
    let mut toks = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some(&(i, c)) = chars.peek() {
        match c {
            '\n' => {
                toks.push(Tok::Eol);
                chars.next();
            }
            ' ' | '\t' | '\r' => {
                chars.next();
            }
            '(' => {
                toks.push(Tok::LParen);
                chars.next();
            }
            ')' => {
                toks.push(Tok::RParen);
                chars.next();
            }
            '{' => {
                toks.push(Tok::LBrace);
                chars.next();
            }
            '}' => {
                toks.push(Tok::RBrace);
                chars.next();
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                for (_, c) in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                    s.push(c);
                }
                toks.push(Tok::Str(s));
            }
            _ => {
                let start = i;
                let mut end = i + c.len_utf8();
                while let Some(&(j, c2)) = chars.peek() {
                    if c2.is_whitespace() || matches!(c2, '(' | ')' | '{' | '}' | '"') {
                        break;
                    }
                    end = j + c2.len_utf8();
                    chars.next();
                }
                let word = &text[start..end];
                if let Ok(n) = word.parse::<f32>() {
                    toks.push(Tok::Num(n));
                } else {
                    toks.push(Tok::Ident(word.to_string()));
                }
            }
        }
    }
    if toks.is_empty() {
        return Err(MqoError::Parse("no tokens".into()));
    }
    Ok(toks)
}

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn peek_ident(&self) -> Option<&str> {
        match self.toks.get(self.pos) {
            Some(Tok::Ident(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    fn expect_lbrace(&mut self) -> Result<(), MqoError> {
        self.skip_eols();
        match self.toks.get(self.pos) {
            Some(Tok::LBrace) => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(MqoError::Parse(format!("expected '{{' at token {}", self.pos))),
        }
    }

    /// If the next token is `}`, consume it and return true.
    fn match_rbrace(&mut self) -> bool {
        if let Some(Tok::RBrace) = self.toks.get(self.pos) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Consume consecutive line-break tokens.
    fn skip_eols(&mut self) {
        while let Some(Tok::Eol) = self.toks.get(self.pos) {
            self.pos += 1;
        }
    }

    /// A stray `{` at statement level: skip the whole balanced chunk.
    fn stray_brace(&mut self) -> bool {
        if let Some(Tok::LBrace) = self.toks.get(self.pos) {
            self.skip_chunk();
            true
        } else {
            false
        }
    }

    fn next_number(&mut self) -> f32 {
        match self.toks.get(self.pos) {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                *n
            }
            _ => 0.0,
        }
    }

    /// Numbers of one line (terminated by a line break or a non-number).
    fn next_line_numbers(&mut self) -> Vec<f32> {
        let mut out = Vec::new();
        while let Some(Tok::Num(n)) = self.toks.get(self.pos) {
            out.push(*n);
            self.pos += 1;
        }
        // Consume the terminating line break so the caller makes progress.
        if let Some(Tok::Eol) = self.toks.get(self.pos) {
            self.pos += 1;
        }
        out
    }

    fn next_string(&mut self) -> Option<String> {
        match self.toks.get(self.pos) {
            Some(Tok::Str(s)) => {
                self.pos += 1;
                Some(s.clone())
            }
            _ => None,
        }
    }

    fn next_number_or_string(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// Consume `( ... )` and return the tokens inside.
    fn take_paren_args(&mut self) -> ArgList {
        let mut inner: Vec<Tok> = Vec::new();
        if let Some(Tok::LParen) = self.toks.get(self.pos) {
            self.pos += 1;
            let mut depth = 1;
            while let Some(t) = self.toks.get(self.pos).cloned() {
                self.pos += 1;
                match t {
                    Tok::LParen => {
                        depth += 1;
                        inner.push(t);
                    }
                    Tok::RParen => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        inner.push(t);
                    }
                    _ => inner.push(t),
                }
            }
        }
        ArgList { tokens: inner }
    }

    /// Skip a `{ ... }` chunk if one starts here; returns whether consumed.
    fn skip_chunk(&mut self) -> bool {
        if let Some(Tok::LBrace) = self.toks.get(self.pos) {
            self.pos += 1;
            let mut depth = 1;
            while depth > 0 {
                match self.toks.get(self.pos) {
                    Some(Tok::LBrace) => {
                        depth += 1;
                        self.pos += 1;
                    }
                    Some(Tok::RBrace) => {
                        depth -= 1;
                        self.pos += 1;
                    }
                    None => break,
                    _ => {
                        self.pos += 1;
                    }
                }
            }
            true
        } else {
            false
        }
    }
}

struct ArgList {
    tokens: Vec<Tok>,
}

impl ArgList {
    fn numbers(&self) -> Vec<f32> {
        self.tokens
            .iter()
            .filter_map(|t| match t {
                Tok::Num(n) => Some(*n),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
Material 2 {
        "red" col(1.000 0.200 0.200 1.000) dif(0.800) amb(0.600) spc(0.000) power(5.00)
        "glass" col(0.700 0.800 0.900 0.500) dif(0.800) tex("panel.png")
}
Object "body" {
        visible 15
        shading 1
        facet 59.500
        vertex 4 {
                0.0000 0.0000 1.0000
                1.0000 0.0000 1.0000
                1.0000 1.0000 1.0000
                0.0000 1.0000 1.0000
        }
        face 2 {
                4 V(0 1 2 3) M(0)
                4 V(0 1 2) M(1) UV(0.0 0.0 1.0 0.0 1.0 1.0)
        }
}
Object "door" {
        vertex 3 {
                0.0000 0.0000 0.5000
                0.4000 0.0000 0.5000
                0.4000 0.8000 0.5000
        }
        face 1 {
                3 V(0 1 2)
        }
}
"#;

    #[test]
    fn parses_materials_objects_faces() {
        let imported = parse_mqo(SAMPLE).unwrap();
        assert_eq!(imported.materials.len(), 2);
        assert_eq!(imported.materials[0].name, "red");
        assert_eq!(imported.materials[0].kd, [1.0, 0.2, 0.2]);
        assert!(imported.materials[0].map_kd.is_none());
        assert_eq!(imported.materials[1].name, "glass");
        assert_eq!(imported.materials[1].d, 0.5);
        assert_eq!(imported.materials[1].map_kd.as_deref(), Some("panel.png"));

        assert_eq!(imported.obj.parts.len(), 2);
        let body = &imported.obj.parts[0];
        assert_eq!(body.name, "body");
        // Two material buckets (red quad → 2 triangles, glass triangle).
        assert_eq!(body.meshes.len(), 2);
        let red = body.meshes.iter().find(|m| m.material == "red").unwrap();
        let glass = body.meshes.iter().find(|m| m.material == "glass").unwrap();
        assert_eq!(red.vertices.len(), 6);
        // z flip + reversed winding: V(0 1 2 3) → corners [3, 2, 1, 0].
        assert_eq!(red.vertices[0].pos, [0.0, 1.0, -1.0]);
        // Fan triangulation: triangle 2 = corners (3, 1, 0).
        assert_eq!(red.vertices[5].pos, [0.0, 0.0, -1.0]);
        // normals present
        assert!(red.vertices.iter().all(|v| v.normal != [0.0, 0.0, 0.0]));
        // UV from the face UV() attribute (reversed with the corners)
        assert_eq!(glass.vertices.len(), 3);
        assert_eq!(glass.vertices[0].uv, [1.0, 1.0]);

        let door = &imported.obj.parts[1];
        assert_eq!(door.meshes.len(), 1);
        assert_eq!(door.meshes[0].material, "default");
        assert_eq!(door.meshes[0].vertices.len(), 3);
        assert_eq!(door.meshes[0].vertices[0].pos, [0.4, 0.8, -0.5]);
    }

    #[test]
    fn mqoz_roundtrip() {
        use flate2::write::GzEncoder;
        use std::io::Write;
        let mut enc = GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(SAMPLE.as_bytes()).unwrap();
        let z = enc.finish().unwrap();
        let imported = parse_mqoz(&z).unwrap();
        assert_eq!(imported.obj.parts.len(), 2);
    }

    #[test]
    fn export_roundtrip_through_obj() {
        let imported = parse_mqo(SAMPLE).unwrap();
        let text = crate::import::obj_to_text(&imported.obj);
        let re = crate::obj::ObjModel::parse(&text).unwrap();
        assert_eq!(re.parts.len(), 2);
        assert_eq!(re.parts[0].meshes.len(), 2);
        // The quad re-parses as a fan: 2 triangles → 6 vertices.
        let mred = re.parts[0].meshes.iter().find(|m| m.material == "red").unwrap();
        assert_eq!(mred.vertices.len(), 6);
        assert_eq!(re.parts[1].meshes[0].vertices.len(), 3);
        assert!(imported.obj.bounds().is_some());
    }

    #[test]
    fn skips_unknown_chunks() {
        let text = r#"
Scene {
        pos 0.0000 0.0000 0.0000
}
Object "a" {
        depth 0
        fog 0
        vertex 3 {
                0 0 0
                1 0 0
                0 1 0
        }
        face 1 {
                3 V(0 1 2) COL(255 255 255 255)
        }
}
"#;
        let imported = parse_mqo(text).unwrap();
        assert_eq!(imported.obj.parts.len(), 1);
        assert_eq!(imported.obj.parts[0].meshes[0].vertices.len(), 3);
    }

    #[test]
    fn bvertex_uvs_used() {
        let text = r#"
Object "a" {
        BVertex 3 {
                0 0 0 0.1 0.2 0
                1 0 0 0.8 0.9 0
                0 1 0 0.5 0.6 0
        }
        face 1 {
                3 V(0 1 2)
        }
}
"#;
        let imported = parse_mqo(text).unwrap();
        assert_eq!(imported.obj.parts.len(), 1);
        let mesh = &imported.obj.parts[0].meshes[0];
        assert_eq!(mesh.vertices.len(), 3);
        // Reversed winding: corner 0 is vertex 2 → its BVertex UV.
        assert_eq!(mesh.vertices[0].uv, [0.5, 0.6]);
        assert_eq!(mesh.vertices[2].uv, [0.1, 0.2]);
    }
}
