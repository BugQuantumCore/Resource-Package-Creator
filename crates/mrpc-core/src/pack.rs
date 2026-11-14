//! Resource-pack container: load from a `.zip` or an unpacked directory,
//! edit, and export as a zip that MTR (and JCM) can consume.
//!
//! Layout of a valid pack (mirrors the official `examples/MTR Custom Resources.zip`):
//! ```text
//! pack.mcmeta
//! pack.png                       (128×128, optional — generated if missing)
//! assets/mtr/mtr_custom_resources.json
//! assets/mtr/<custom paths…>     (models, textures, properties, sounds, scripts)
//! ```

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::format::CustomResources;

pub const CUSTOM_RESOURCES_PATH: &str = "assets/mtr/mtr_custom_resources.json";

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("invalid mtr_custom_resources.json: {0}")]
    BadResources(#[from] serde_json::Error),
    #[error("path is not a resource pack (missing {CUSTOM_RESOURCES_PATH})")]
    NotAPack,
}

/// `pack.mcmeta`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackMcMeta {
    pub pack: PackMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackMeta {
    #[serde(rename = "pack_format", default = "default_pack_format")]
    pub pack_format: u32,
    #[serde(default)]
    pub description: String,
}

fn default_pack_format() -> u32 {
    9
}

/// An open resource pack.
#[derive(Debug, Clone)]
pub struct ResourcePack {
    pub mcmeta: PackMcMeta,
    /// `assets/mtr/mtr_custom_resources.json`, parsed.
    pub resources: CustomResources,
    /// All files, path → bytes (forward slashes, no leading slash).
    pub files: BTreeMap<String, Vec<u8>>,
    /// Parse/consistency diagnostics produced while loading.
    pub load_errors: Vec<String>,
}

impl ResourcePack {
    /// A minimal, valid empty pack.
    pub fn new(description: &str) -> ResourcePack {
        ResourcePack {
            mcmeta: PackMcMeta {
                pack: PackMeta { pack_format: default_pack_format(), description: description.to_string() },
            },
            resources: CustomResources::default(),
            files: BTreeMap::new(),
            load_errors: Vec::new(),
        }
    }

    /// Load from a `.zip` archive.
    pub fn load_zip(path: &Path) -> Result<ResourcePack, PackError> {
        let file = std::fs::File::open(path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        let mut files = BTreeMap::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            if entry.is_dir() {
                continue;
            }
            let raw_name = entry.name().map_err(|e| PackError::Zip(e))?;
            let name = raw_name.trim_start_matches("./").replace('\\', "/");
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            files.insert(name, bytes);
        }
        Self::from_files(files)
    }

    /// Load from an unpacked pack directory.
    pub fn load_dir(path: &Path) -> Result<ResourcePack, PackError> {
        let mut files = BTreeMap::new();
        collect_files(path, path, &mut files)?;
        Self::from_files(files)
    }

    fn from_files(mut files: BTreeMap<String, Vec<u8>>) -> Result<ResourcePack, PackError> {
        let mut load_errors = Vec::new();

        let mcmeta = match files.remove("pack.mcmeta") {
            Some(bytes) => serde_json::from_slice::<PackMcMeta>(&bytes).unwrap_or_else(|e| {
                load_errors.push(format!("pack.mcmeta: {e}"));
                PackMcMeta {
                    pack: PackMeta { pack_format: default_pack_format(), description: String::new() },
                }
            }),
            None => {
                load_errors.push("pack.mcmeta missing; default created".to_string());
                PackMcMeta {
                    pack: PackMeta { pack_format: default_pack_format(), description: String::new() },
                }
            }
        };

        let resources = match files.remove(CUSTOM_RESOURCES_PATH) {
            Some(bytes) => CustomResources::from_json_str(&String::from_utf8_lossy(&bytes))
                .map_err(PackError::BadResources)?,
            None => {
                // An empty pack may legitimately have no custom resources yet.
                CustomResources::default()
            }
        };

        Ok(ResourcePack { mcmeta, resources, files, load_errors })
    }

    /// Write the pack (including edited `mtr_custom_resources.json` and
    /// re-merged legacy sections) to a zip file.
    pub fn export_zip(&self, path: &Path) -> Result<(), PackError> {
        let mut entries: BTreeMap<String, Vec<u8>> = self.files.clone();
        entries.insert(
            "pack.mcmeta".to_string(),
            serde_json::to_vec_pretty(&self.mcmeta)?,
        );
        if !entries.contains_key("pack.png") {
            entries.insert("pack.png".to_string(), default_pack_png());
        }
        entries.insert(
            CUSTOM_RESOURCES_PATH.to_string(),
            self.resources_json().into_bytes(),
        );
        // pack.png may have been replaced by the editor — keep whatever bytes exist.

        let file = std::fs::File::create(path)?;
        let mut writer = zip::ZipWriter::new(file);
        let options: zip::write::SimpleFileOptions =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in &entries {
            writer.start_file(name.as_str(), options)?;
            writer.write_all(bytes)?;
        }
        writer.finish()?;
        Ok(())
    }

    /// Serialise `mtr_custom_resources.json`, merging the legacy MTR 3 map back in.
    pub fn resources_json(&self) -> String {
        let mut value = serde_json::to_value(&self.resources).expect("serialisable");
        if let (Some(map), Some(obj)) = (value.as_object_mut(), (!self.resources.legacy.is_empty()).then_some(())) {
            let _ = obj;
            for (k, v) in &self.resources.legacy {
                map.insert(k.clone(), v.clone());
            }
        }
        serde_json::to_string_pretty(&value).expect("serialisable")
    }

    /// Resolve an MTR resource id (`namespace:path`) to file bytes.
    /// A path with no namespace defaults to `mtr:`; an extension-less path
    /// gets `.png` for textures / `.json` for properties, like
    /// `CustomResourceTools.formatIdentifierWithDefault`.
    pub fn resource(&self, id: &str, default_ext: &str) -> Option<&[u8]> {
        let (namespace, path) = match id.split_once(':') {
            Some((ns, p)) => (ns, p),
            None => ("mtr", id),
        };
        let mut candidates = vec![format!("assets/{namespace}/{path}")];
        if !path.contains('.') {
            candidates.push(format!("assets/{namespace}/{path}.{default_ext}"));
        }
        for c in candidates {
            if let Some(bytes) = self.files.get(&c) {
                return Some(bytes);
            }
        }
        None
    }

    /// Insert or replace a file by MTR resource id.
    pub fn set_resource(&mut self, id: &str, default_ext: &str, bytes: Vec<u8>) {
        let (namespace, path) = match id.split_once(':') {
            Some((ns, p)) => (ns, p),
            None => ("mtr", id),
        };
        let file = if !path.contains('.') {
            format!("assets/{namespace}/{path}.{default_ext}")
        } else {
            format!("assets/{namespace}/{path}")
        };
        self.files.insert(file, bytes);
    }

    /// Paths of all files under `assets/` (for the asset browser).
    pub fn asset_paths(&self) -> Vec<&str> {
        self.files.keys().map(|s| s.as_str()).collect()
    }
}

fn collect_files(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) -> Result<(), PackError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let key = rel.to_string_lossy().replace('\\', "/");
            let mut bytes = Vec::new();
            std::fs::File::open(&path)?.read_to_end(&mut bytes)?;
            out.insert(key, bytes);
        }
    }
    Ok(())
}

/// A generated 128×128 pack icon (dark rail + MTR-ish colours) so exported
/// packs always have a `pack.png`.
fn default_pack_png() -> Vec<u8> {
    use image::{ImageBuffer, Rgb};
    const W: u32 = 128;
    const H: u32 = 128;
    let bg: Rgb<u8> = Rgb([30, 34, 44]);
    let rail: Rgb<u8> = Rgb([168, 176, 192]);
    let accent: Rgb<u8> = Rgb([254, 197, 229]);
    let mut img = ImageBuffer::from_fn(W, H, |x, _| {
        // vertical gradient background
        let t = x as f32 / W as f32;
        let c = (bg.0[0] as f32 * (1.0 - t * 0.25)) as u8;
        Rgb([c, (c as f32 * 1.05) as u8, (c as f32 * 1.15) as u8])
    });
    // two rails converging into the distance
    for y in 0..H {
        let spread = 8.0 + (y as f32 / H as f32) * 40.0;
        for (dx, color) in [(-spread, rail), (spread, rail)] {
            let x = (W as f32 / 2.0 + dx) as i64;
            for w in -2..=2 {
                let px = x + w;
                if (0..W as i64).contains(&px) {
                    img.put_pixel(px as u32, y, color);
                }
            }
        }
        if y % 8 == 0 {
            // sleepers
            let spread = 8.0 + (y as f32 / H as f32) * 40.0;
            let x0 = (W as f32 / 2.0 - spread - 6.0) as u32;
            let x1 = (W as f32 / 2.0 + spread + 6.0) as u32;
            for x in x0.min(W - 1)..x1.min(W) {
                img.put_pixel(x, y, accent);
            }
        }
    }
    let mut out = Vec::new();
    let enc = image::codecs::png::PngEncoder::new(&mut out);
    image::ImageEncoder::write_image(
        enc,
        img.as_raw(),
        W,
        H,
        image::ExtendedColorType::Rgb8,
    )
    .expect("encode png");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_zip() {
        let mut pack = ResourcePack::new("Test pack");
        pack.resources.vehicles.push(crate::vehicle::VehicleResource {
            id: "demo_train".into(),
            name: "Demo Train".into(),
            ..Default::default()
        });
        pack.set_resource("mtr:custom/demo/train.obj", "obj", b"o body\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec());

        let dir = std::env::temp_dir().join("mrpc_test_pack");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("pack.zip");
        pack.export_zip(&zip_path).unwrap();

        let loaded = ResourcePack::load_zip(&zip_path).unwrap();
        assert_eq!(loaded.resources.vehicles.len(), 1);
        assert_eq!(loaded.resources.vehicles[0].id, "demo_train");
        assert!(loaded.resource("mtr:custom/demo/train.obj", "obj").is_some());
        assert!(loaded.files.contains_key("pack.png"));
        assert!(loaded.mcmeta.pack.pack_format >= 9);
    }

    #[test]
    fn default_png_is_valid() {
        let bytes = default_pack_png();
        assert!(bytes.len() > 100);
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!((img.width(), img.height()), (128, 128));
    }
}
