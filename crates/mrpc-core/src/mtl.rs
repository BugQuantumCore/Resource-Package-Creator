//! Minimal MTL (Wavefront material library) parser.

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Material {
    pub kd: [f32; 3],
    pub ka: [f32; 3],
    pub ks: [f32; 3],
    pub ns: f32,
    pub d: f32,
    /// Diffuse texture filename referenced by `map_Kd` (relative to the OBJ).
    pub map_kd: Option<String>,
}

impl Material {
    pub const WHITE: Material = Material {
        kd: [1.0, 1.0, 1.0],
        ka: [0.1, 0.1, 0.1],
        ks: [0.2, 0.2, 0.2],
        ns: 32.0,
        d: 1.0,
        map_kd: None,
    };
}

pub fn parse_mtl(text: &str) -> HashMap<String, Material> {
    let mut out: HashMap<String, Material> = HashMap::new();
    let mut current: Option<(String, Material)> = None;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let keyword = tokens.next().unwrap_or("");
        match keyword.to_ascii_lowercase().as_str() {
            "newmtl" => {
                if let Some((name, mat)) = current.take() {
                    out.insert(name, mat);
                }
                // `get`, not indexing: a bare `newmtl` (truncated/hand-edited
                // file) is 6 bytes and `[7..]` would panic on pack load.
                let name = line.get(7..).unwrap_or_default().trim().trim_matches('"').to_string();
                current = Some((name, Material::default()));
            }
            "kd" | "map_kd" => {
                if let Some((_, mat)) = &mut current {
                    if keyword.eq_ignore_ascii_case("kd") {
                        let v: Vec<f32> = tokens.take(3).map(|t| t.parse().unwrap_or(0.0)).collect();
                        mat.kd = [v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0), v.get(2).copied().unwrap_or(0.0)];
                    } else {
                        mat.map_kd = tokens.next().map(|s| s.trim_matches('"').to_string());
                    }
                }
            }
            "ka" => {
                if let Some((_, mat)) = &mut current {
                    let v: Vec<f32> = tokens.take(3).map(|t| t.parse().unwrap_or(0.0)).collect();
                    mat.ka = [v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0), v.get(2).copied().unwrap_or(0.0)];
                }
            }
            "ks" => {
                if let Some((_, mat)) = &mut current {
                    let v: Vec<f32> = tokens.take(3).map(|t| t.parse().unwrap_or(0.0)).collect();
                    mat.ks = [v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0), v.get(2).copied().unwrap_or(0.0)];
                }
            }
            "ns" => {
                if let Some((_, mat)) = &mut current {
                    mat.ns = tokens.next().and_then(|t| t.parse().ok()).unwrap_or(32.0);
                }
            }
            "d" | "tr" => {
                if let Some((_, mat)) = &mut current {
                    mat.d = tokens.next().and_then(|t| t.parse().ok()).unwrap_or(1.0);
                }
            }
            _ => {}
        }
    }
    if let Some((name, mat)) = current.take() {
        out.insert(name, mat);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_mtl() {
        let mtl = "
# comment
newmtl body_mat
Kd 0.8 0.8 0.85
Ka 0.1 0.1 0.1
Ks 0.3 0.3 0.3
Ns 48
map_Kd body.png

newmtl glass
Kd 0.5 0.6 0.7
d 0.6
";
        let m = parse_mtl(mtl);
        assert_eq!(m.len(), 2);
        let body = &m["body_mat"];
        assert_eq!(body.kd, [0.8, 0.8, 0.85]);
        assert_eq!(body.map_kd.as_deref(), Some("body.png"));
        let glass = &m["glass"];
        assert_eq!(glass.d, 0.6);
        assert_eq!(glass.map_kd, None);
    }
}
