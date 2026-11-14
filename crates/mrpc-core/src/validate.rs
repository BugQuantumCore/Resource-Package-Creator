//! Pack validation — checks that catch the mistakes the in-game creator
//! cannot recover from, reported before export.

use crate::pack::ResourcePack;
use crate::vehicle::VehicleResource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Hint,
}

#[derive(Debug, Clone)]
pub struct ValidationItem {
    pub severity: Severity,
    /// i18n key for the message (see `assets/i18n/*.json`, `validation.*`).
    pub key: String,
    /// Concrete detail (ids, paths).
    pub detail: String,
}

/// Character set Minecraft allows in resource identifiers.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id == id.to_ascii_lowercase()
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, 'a'..='z' | '0'..='9' | '/' | '.' | '_' | '-'))
}

fn valid_resource_id(id: &str) -> bool {
    match id.split_once(':') {
        Some((ns, path)) => valid_id(ns) && valid_id(path),
        None => valid_id(id),
    }
}

/// Validate a whole pack. Checks are pure (no I/O beyond the already-loaded
/// file table) so this can run after every edit.
pub fn validate_pack(pack: &ResourcePack) -> Vec<ValidationItem> {
    let mut out = Vec::new();
    let res = &pack.resources;

    for err in &pack.load_errors {
        out.push(ValidationItem { severity: Severity::Warning, key: "validation.load_error".into(), detail: err.clone() });
    }

    for (i, v) in res.vehicles.iter().enumerate() {
        let ctx = format!("#{i} {id}", id = v.id);

        if !valid_id(&v.id) {
            out.push(ValidationItem { severity: Severity::Error, key: "validation.vehicle_id_charset".into(), detail: ctx.clone() });
        }
        if res.vehicles.iter().filter(|o| o.id == v.id).count() > 1 {
            out.push(ValidationItem { severity: Severity::Error, key: "validation.vehicle_id_duplicate".into(), detail: ctx.clone() });
        }
        if v.name.trim().is_empty() {
            out.push(ValidationItem { severity: Severity::Warning, key: "validation.vehicle_name_missing".into(), detail: ctx.clone() });
        }
        if v.color.len() != 6 || !v.color.chars().all(|c| c.is_ascii_hexdigit()) {
            out.push(ValidationItem { severity: Severity::Warning, key: "validation.vehicle_color".into(), detail: ctx.clone() });
        }
        if v.length <= 0.0 {
            out.push(ValidationItem { severity: Severity::Error, key: "validation.vehicle_length".into(), detail: ctx.clone() });
        }
        if v.width <= 0.0 || v.width > 5.0 {
            out.push(ValidationItem { severity: Severity::Warning, key: "validation.vehicle_width".into(), detail: ctx.clone() });
        }
        if v.bogie2_position <= v.bogie1_position {
            out.push(ValidationItem { severity: Severity::Error, key: "validation.vehicle_bogies".into(), detail: ctx.clone() });
        }

        if v.is_legacy_retexture() {
            out.push(ValidationItem { severity: Severity::Hint, key: "validation.vehicle_legacy_format".into(), detail: ctx.clone() });
        } else if v.models.is_empty() {
            out.push(ValidationItem { severity: Severity::Error, key: "validation.vehicle_no_models".into(), detail: ctx.clone() });
        }

        for (j, model) in v.models.iter().enumerate() {
            let mctx = format!("{ctx} model #{j}");
            if !valid_resource_id(&model.model_resource) || model.model_resource.is_empty() {
                out.push(ValidationItem { severity: Severity::Error, key: "validation.model_resource_id".into(), detail: mctx.clone() });
            } else if pack.resource(&model.model_resource, "obj").is_none() {
                out.push(ValidationItem { severity: Severity::Error, key: "validation.model_missing".into(), detail: format!("{mctx} → {}", model.model_resource) });
            }
            if !model.texture_resource.is_empty() && pack.resource(&model.texture_resource, "png").is_none() {
                out.push(ValidationItem { severity: Severity::Error, key: "validation.texture_missing".into(), detail: format!("{mctx} → {}", model.texture_resource) });
            }
            if !model.model_properties_resource.is_empty()
                && pack.resource(&model.model_properties_resource, "json").is_none()
            {
                out.push(ValidationItem { severity: Severity::Error, key: "validation.properties_missing".into(), detail: format!("{mctx} → {}", model.model_properties_resource) });
            }
            if !model.position_definitions_resource.is_empty()
                && pack.resource(&model.position_definitions_resource, "json").is_none()
            {
                out.push(ValidationItem { severity: Severity::Error, key: "validation.positions_missing".into(), detail: format!("{mctx} → {}", model.position_definitions_resource) });
            }
        }

        // Sounds
        if !v.bve_sound_base_resource.is_empty() {
            let base = &v.bve_sound_base_resource;
            let (ns, path) = match base.split_once(':') {
                Some((ns, p)) => (ns, p),
                None => ("mtr", base.as_str()),
            };
            let cfg = format!("assets/{ns}/sounds/{path}/sound.cfg");
            if !pack.files.contains_key(&cfg) {
                out.push(ValidationItem { severity: Severity::Warning, key: "validation.bve_config_missing".into(), detail: format!("{ctx} → {cfg}") });
            }
        }
        if !v.legacy_speed_sound_base_resource.is_empty() && v.legacy_speed_sound_count <= 0 {
            out.push(ValidationItem { severity: Severity::Warning, key: "validation.legacy_speed_sound_count".into(), detail: ctx.clone() });
        }

        // Scripts (JCM)
        if let Some(script_id) = &v.script_id {
            if res.vehicle_script(script_id).is_none() {
                out.push(ValidationItem { severity: Severity::Error, key: "validation.script_ref_missing".into(), detail: format!("{ctx} → {script_id}") });
            }
        }
    }

    for (i, s) in res.vehicle_scripts.iter().enumerate() {
        let ctx = format!("#{i} {id}", id = s.id);
        if !valid_id(&s.id) {
            out.push(ValidationItem { severity: Severity::Error, key: "validation.script_id_charset".into(), detail: ctx.clone() });
        }
        if s.script_locations.is_empty() {
            out.push(ValidationItem { severity: Severity::Error, key: "validation.script_no_locations".into(), detail: ctx.clone() });
        }
        let referenced = res.vehicles.iter().any(|v| v.script_id.as_deref() == Some(s.id.as_str()));
        if !referenced && !s.force_load {
            out.push(ValidationItem { severity: Severity::Warning, key: "validation.script_unreferenced".into(), detail: ctx.clone() });
        }
    }

    if res.vehicles.is_empty() && res.legacy.is_empty() && res.signs.is_empty() {
        out.push(ValidationItem { severity: Severity::Warning, key: "validation.pack_empty".into(), detail: String::new() });
    }

    out
}

/// Count items by severity.
pub fn summarize(items: &[ValidationItem]) -> (usize, usize, usize) {
    let errors = items.iter().filter(|i| i.severity == Severity::Error).count();
    let warnings = items.iter().filter(|i| i.severity == Severity::Warning).count();
    let hints = items.iter().filter(|i| i.severity == Severity::Hint).count();
    (errors, warnings, hints)
}

/// Validate a single vehicle (used by the properties panel live).
pub fn validate_vehicle(v: &VehicleResource) -> Vec<ValidationItem> {
    let mut single = ResourcePack::new("");
    single.resources.vehicles.push(v.clone());
    validate_pack(&single)
        .into_iter()
        .filter(|i| !matches!(i.key.as_str(), "validation.pack_empty"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_missing_model_and_bad_id() {
        let mut pack = ResourcePack::new("t");
        pack.resources.vehicles.push(VehicleResource {
            id: "BadID".to_string(), // uppercase
            name: "X".into(),
            models: vec![crate::vehicle::VehicleModel {
                model_resource: "mtr:custom/missing.obj".into(),
                ..Default::default()
            }],
            bogie1_position: 6.0,
            bogie2_position: -6.0, // reversed
            ..Default::default()
        });
        let items = validate_pack(&pack);
        let keys: Vec<&str> = items.iter().map(|i| i.key.as_str()).collect();
        assert!(keys.contains(&"validation.vehicle_id_charset"));
        assert!(keys.contains(&"validation.model_missing"));
        assert!(keys.contains(&"validation.vehicle_bogies"));
    }

    #[test]
    fn empty_pack_warns() {
        let pack = ResourcePack::new("t");
        let items = validate_pack(&pack);
        assert!(items.iter().any(|i| i.key == "validation.pack_empty"));
    }
}
