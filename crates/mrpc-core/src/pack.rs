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

use std::collections::{BTreeMap, HashSet};
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

/// Outcome of merging another pack into this one ([`ResourcePack::merge_from`]).
///
/// "Preview & add" (预览添加): the contents of another pack are merged into the
/// pack being edited. Resources with colliding ids are renamed (with a `_2`,
/// `_3`… suffix) instead of being dropped, identical entries are de-duplicated,
/// and on file-path collisions the currently edited version wins.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeStats {
    pub vehicles_added: usize,
    pub scripts_added: usize,
    pub signs_added: usize,
    /// Rail / object / lift / other pass-through entries (incl. unknown JCM
    /// sections) added.
    pub other_added: usize,
    pub files_added: usize,
    /// Files whose path already existed with different content; the current
    /// pack's version was kept.
    pub files_kept: usize,
    /// Vehicle / script / sign ids renamed to avoid collisions.
    pub ids_renamed: usize,
}

impl MergeStats {
    /// Whether anything at all was added.
    pub fn added_anything(&self) -> bool {
        self.vehicles_added > 0
            || self.scripts_added > 0
            || self.signs_added > 0
            || self.other_added > 0
            || self.files_added > 0
    }
}

/// `base`, or `base_2`, `base_3`… — the first candidate not in `taken`.
fn unique_id(base: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let base = if base.is_empty() { "imported" } else { base };
    if !taken(base) {
        return base.to_string();
    }
    for n in 2u32.. {
        let candidate = format!("{base}_{n}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    unreachable!("an unused candidate is found within u32 iterations");
}

impl ResourcePack {
    /// Merge all resources of `other` into `self` (File → 预览添加).
    ///
    /// Policy:
    /// - `vehicles` / `vehicleScripts` / `signs`: appended. An entry that
    ///   already exists **byte-identical** is skipped (so adding the same pack
    ///   twice is a no-op); an entry whose id collides with a different entry
    ///   is renamed (`id_2`, `id_3`…). Renamed script ids are re-linked from
    ///   the vehicles added in the same merge.
    /// - `rails` / `objects` / `lifts` / unknown extra sections: appended
    ///   (identical values de-duplicated); an extra section that already
    ///   exists keeps the current pack's version.
    /// - `files`: inserted; on a path collision with **different** content the
    ///   currently edited version wins (`files_kept`).
    /// - `mcmeta` (description, pack_format) and legacy MTR-3 sections of the
    ///   current pack are preserved; incoming legacy sections are only copied
    ///   across when the current pack has none.
    pub fn merge_from(&mut self, other: ResourcePack) -> MergeStats {
        let mut stats = MergeStats::default();
        let ResourcePack { mut resources, files, mcmeta: _, load_errors: _ } = other;

        // ---- files (current pack wins on conflict) ----
        for (path, bytes) in files {
            match self.files.get(&path) {
                Some(existing) if *existing != bytes => stats.files_kept += 1,
                Some(_) => {} // identical — nothing to do
                None => {
                    self.files.insert(path, bytes);
                    stats.files_added += 1;
                }
            }
        }

        // ---- vehicle scripts (rename on collision, remember the mapping) ----
        // Existing scripts are serialised once up front so that recognising
        // an identical incoming script costs a single serialisation + hash
        // lookup, instead of re-serialising every existing script for every
        // incoming one (O(n×m) → O(n+m); serde_json's Map is a BTreeMap, so
        // `to_string` is a deterministic, faithful key for Value equality).
        let mut known_scripts: HashSet<String> = self
            .resources
            .vehicle_scripts
            .iter()
            .filter_map(|s| serde_json::to_string(s).ok())
            .collect();
        let mut script_renames: Vec<(String, String)> = Vec::new();
        for mut script in std::mem::take(&mut resources.vehicle_scripts) {
            if let Some(serialized) = serde_json::to_string(&script).ok() {
                if known_scripts.contains(serialized.as_str()) {
                    continue; // already present, identical — reuse it
                }
                known_scripts.insert(serialized);
            }
            if self.resources.vehicle_script(&script.id).is_some() {
                let old = script.id.clone();
                let new = unique_id(&old, &|id| self.resources.vehicle_script(id).is_some());
                script.id = new.clone();
                script_renames.push((old, new));
                stats.ids_renamed += 1;
            }
            self.resources.vehicle_scripts.push(script);
            stats.scripts_added += 1;
        }

        // ---- vehicles (rename on collision; re-link renamed scripts) ----
        // Same one-serialisation-per-entry dedup as for scripts above.
        let mut known_vehicles: HashSet<String> = self
            .resources
            .vehicles
            .iter()
            .filter_map(|v| serde_json::to_string(v).ok())
            .collect();
        let first_new_vehicle = self.resources.vehicles.len();
        for mut vehicle in std::mem::take(&mut resources.vehicles) {
            if let Some(serialized) = serde_json::to_string(&vehicle).ok() {
                if known_vehicles.contains(serialized.as_str()) {
                    continue;
                }
                known_vehicles.insert(serialized);
            }
            if self.resources.vehicle(&vehicle.id).is_some() {
                vehicle.id = unique_id(&vehicle.id, &|id| self.resources.vehicle(id).is_some());
                stats.ids_renamed += 1;
            }
            self.resources.vehicles.push(vehicle);
            stats.vehicles_added += 1;
        }
        if !script_renames.is_empty() {
            for vehicle in self.resources.vehicles.iter_mut().skip(first_new_vehicle) {
                if let Some(script_id) = &vehicle.script_id {
                    if let Some((_, renamed)) =
                        script_renames.iter().find(|(old, _)| old == script_id)
                    {
                        vehicle.script_id = Some(renamed.clone());
                    }
                }
            }
        }

        // ---- signs ----
        let mut known_signs: HashSet<String> = self
            .resources
            .signs
            .iter()
            .filter_map(|s| serde_json::to_string(s).ok())
            .collect();
        for mut sign in std::mem::take(&mut resources.signs) {
            if let Some(serialized) = serde_json::to_string(&sign).ok() {
                if known_signs.contains(serialized.as_str()) {
                    continue;
                }
                known_signs.insert(serialized);
            }
            if self.resources.signs.iter().any(|s| s.id == sign.id) {
                sign.id =
                    unique_id(&sign.id, &|id| self.resources.signs.iter().any(|s| s.id == id));
                stats.ids_renamed += 1;
            }
            self.resources.signs.push(sign);
            stats.signs_added += 1;
        }

        // ---- rails / objects / lifts / unknown extra sections ----
        append_dedup(
            &mut self.resources.rails,
            std::mem::take(&mut resources.rails),
            &mut stats.other_added,
        );
        append_dedup(
            &mut self.resources.objects,
            std::mem::take(&mut resources.objects),
            &mut stats.other_added,
        );
        append_dedup(
            &mut self.resources.lifts,
            std::mem::take(&mut resources.lifts),
            &mut stats.other_added,
        );
        for (key, value) in resources.extra {
            if !self.resources.extra.contains_key(&key) {
                self.resources.extra.insert(key, value);
                stats.other_added += 1;
            }
        }
        // Legacy MTR-3 sections: only copy across when the current pack has none.
        if self.resources.legacy.is_empty() {
            stats.other_added += resources.legacy.len();
            self.resources.legacy = resources.legacy;
        }

        stats
    }

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

/// Append `incoming` pass-through values to `target`, skipping values that are
/// already present (identity by serialised equality).
///
/// The existing entries are serialised once up front, so deduplicating a
/// large incoming section costs O(n + m) serialisations instead of the
/// O(n × m) of re-serialising every existing entry for every incoming one
/// (rails / objects / lifts sections routinely hold thousands of entries).
fn append_dedup(
    target: &mut Vec<serde_json::Value>,
    incoming: Vec<serde_json::Value>,
    added: &mut usize,
) {
    let mut known: HashSet<String> = target
        .iter()
        .filter_map(|v| serde_json::to_string(v).ok())
        .collect();
    for value in incoming {
        match serde_json::to_string(&value).ok() {
            Some(serialized) => {
                if known.insert(serialized) {
                    target.push(value);
                    *added += 1;
                }
            }
            // A Value that fails to serialise (impossible for values parsed
            // from JSON) is simply appended, matching the old comparison.
            None => {
                target.push(value);
                *added += 1;
            }
        }
    }
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

    fn vehicle(id: &str, name: &str, script: Option<&str>) -> crate::vehicle::VehicleResource {
        crate::vehicle::VehicleResource {
            id: id.into(),
            name: name.into(),
            script_id: script.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn merge_disjoint_packs() {
        let mut a = ResourcePack::new("A");
        a.resources.vehicles.push(vehicle("train_a", "Train A", Some("script_a")));
        a.resources.vehicle_scripts.push(crate::script::VehicleScriptEntry {
            id: "script_a".into(),
            ..Default::default()
        });
        a.set_resource("mtr:custom/a/train.obj", "obj", b"o a\n".to_vec());

        let mut b = ResourcePack::new("B");
        b.resources.vehicles.push(vehicle("train_b", "Train B", Some("script_b")));
        b.resources.vehicle_scripts.push(crate::script::VehicleScriptEntry {
            id: "script_b".into(),
            ..Default::default()
        });
        b.set_resource("mtr:custom/b/train.obj", "obj", b"o b\n".to_vec());

        let stats = a.merge_from(b);
        assert_eq!(stats.vehicles_added, 1);
        assert_eq!(stats.scripts_added, 1);
        assert_eq!(stats.files_added, 1);
        assert_eq!(stats.ids_renamed, 0);
        assert_eq!(stats.files_kept, 0);
        assert_eq!(a.resources.vehicles.len(), 2);
        // The added vehicle keeps pointing at its own (added) script.
        assert_eq!(a.resources.vehicles[1].script_id.as_deref(), Some("script_b"));
        assert!(a.files.contains_key("assets/mtr/custom/b/train.obj"));
        assert_eq!(a.mcmeta.pack.description, "A");
    }

    #[test]
    fn merge_renames_collisions_and_relinks_scripts() {
        let mut a = ResourcePack::new("A");
        a.resources.vehicles.push(vehicle("k8", "Current K8", Some("lcd")));
        a.resources.vehicle_scripts.push(crate::script::VehicleScriptEntry {
            id: "lcd".into(),
            ..Default::default()
        });
        a.set_resource("mtr:custom/shared.txt", "txt", b"current".to_vec());

        let mut b = ResourcePack::new("B");
        b.resources.vehicles.push(vehicle("k8", "Other K8", Some("lcd")));
        b.resources.vehicle_scripts.push(crate::script::VehicleScriptEntry {
            id: "lcd".into(),
            prepend_expressions: vec!["extra".into()],
            ..Default::default()
        });
        b.set_resource("mtr:custom/shared.txt", "txt", b"incoming".to_vec());

        let stats = a.merge_from(b);
        assert_eq!(stats.vehicles_added, 1);
        assert_eq!(stats.scripts_added, 1);
        assert_eq!(stats.ids_renamed, 2); // vehicle id + script id
        assert_eq!(stats.files_kept, 1);
        assert_eq!(stats.files_added, 0);

        let ids: Vec<&str> = a.resources.vehicles.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, vec!["k8", "k8_2"]);
        let script_ids: Vec<&str> =
            a.resources.vehicle_scripts.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(script_ids, vec!["lcd", "lcd_2"]);
        // The renamed vehicle is re-linked to the renamed script.
        assert_eq!(a.resources.vehicles[1].script_id.as_deref(), Some("lcd_2"));
        // Current file content wins.
        assert_eq!(a.files.get("assets/mtr/custom/shared.txt").unwrap(), b"current");
    }

    #[test]
    fn merge_same_pack_twice_is_noop() {
        let mut a = ResourcePack::new("A");
        a.set_resource("mtr:custom/a/train.obj", "obj", b"o a\n".to_vec());

        let mut b = ResourcePack::new("B");
        b.resources.vehicles.push(vehicle("train_b", "Train B", None));
        b.set_resource("mtr:custom/b/train.obj", "obj", b"o b\n".to_vec());

        let first = a.merge_from(b.clone());
        assert!(first.added_anything());
        let second = a.merge_from(b);
        assert!(!second.added_anything());
        assert_eq!(second.files_kept, 0); // identical files are not conflicts
        assert_eq!(a.resources.vehicles.len(), 1);
    }

    #[test]
    fn merge_signs_and_passthrough_sections() {
        let mut a = ResourcePack::new("A");
        a.resources.signs.push(crate::format::SignResource {
            id: "sign_a".into(),
            ..Default::default()
        });
        a.resources.rails.push(serde_json::json!({"id": "rail_a"}));

        let mut b = ResourcePack::new("B");
        b.resources.signs.push(crate::format::SignResource {
            id: "sign_a".into(),
            name: "different".into(),
            ..Default::default()
        });
        b.resources.rails.push(serde_json::json!({"id": "rail_a"}));
        b.resources.rails.push(serde_json::json!({"id": "rail_b"}));

        let stats = a.merge_from(b);
        assert_eq!(stats.signs_added, 1);
        assert_eq!(stats.other_added, 1); // rail_b only; identical rail_a is deduped
        assert_eq!(stats.ids_renamed, 1); // sign_a → sign_a_2
        let sign_ids: Vec<&str> = a.resources.signs.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(sign_ids, vec!["sign_a", "sign_a_2"]);
        assert_eq!(a.resources.rails.len(), 2);
    }

    #[test]
    fn unique_id_appends_suffixes() {
        // Free id passes through unchanged.
        assert_eq!(unique_id("y", &|_| false), "y");
        assert_eq!(unique_id("", &|_| false), "imported");
        // "x" and "x_2" taken → first free candidate is "x_3".
        assert_eq!(unique_id("x", &|id| matches!(id, "x" | "x_2")), "x_3");
        // "x", "x_2" and "x_3" taken → "x_4".
        assert_eq!(unique_id("x", &|id| matches!(id, "x" | "x_2" | "x_3")), "x_4");
    }
}
