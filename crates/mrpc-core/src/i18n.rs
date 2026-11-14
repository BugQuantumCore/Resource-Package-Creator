//! Embedded UI localisation for the seven supported languages:
//! zh-CN, zh-HK, zh-TW, lzh (Classical Chinese), en-US, en-GB, fr-FR.
//!
//! Locale files are JSON objects (`key → string`) living in `assets/i18n/`.
//! They are embedded at compile time and also reloadable from disk for pack
//! authors who want to tweak wording. Lookup falls back to en-US, then to the
//! key itself. `t!`-style interpolation uses `{name}` placeholders.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Supported UI languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locale {
    ZhCn,
    ZhHk,
    ZhTw,
    Lzh,
    EnUs,
    EnGb,
    FrFr,
}

impl Locale {
    /// BCP-47 / Minecraft style tag.
    pub const fn tag(self) -> &'static str {
        match self {
            Locale::ZhCn => "zh-CN",
            Locale::ZhHk => "zh-HK",
            Locale::ZhTw => "zh-TW",
            Locale::Lzh => "lzh",
            Locale::EnUs => "en-US",
            Locale::EnGb => "en-GB",
            Locale::FrFr => "fr-FR",
        }
    }

    /// Endonym shown in the language menu.
    pub const fn endonym(self) -> &'static str {
        match self {
            Locale::ZhCn => "简体中文（中国大陆）",
            Locale::ZhHk => "繁體中文（香港特別行政區）",
            Locale::ZhTw => "繁體中文（台灣省）",
            Locale::Lzh => "文言（華夏）",
            Locale::EnUs => "English (United States)",
            Locale::EnGb => "English (United Kingdom)",
            Locale::FrFr => "Français (France)",
        }
    }

    pub const ALL: [Locale; 7] = [
        Locale::ZhCn,
        Locale::ZhHk,
        Locale::ZhTw,
        Locale::Lzh,
        Locale::EnUs,
        Locale::EnGb,
        Locale::FrFr,
    ];

    pub fn from_tag(tag: &str) -> Option<Locale> {
        let norm = tag.replace('_', "-").to_ascii_lowercase();
        Locale::ALL.into_iter().find(|l| l.tag().to_ascii_lowercase() == norm)
    }

    fn raw(self) -> &'static str {
        match self {
            Locale::ZhCn => include_str!("../../../assets/i18n/zh-CN.json"),
            Locale::ZhHk => include_str!("../../../assets/i18n/zh-HK.json"),
            Locale::ZhTw => include_str!("../../../assets/i18n/zh-TW.json"),
            Locale::Lzh => include_str!("../../../assets/i18n/lzh.json"),
            Locale::EnUs => include_str!("../../../assets/i18n/en-US.json"),
            Locale::EnGb => include_str!("../../../assets/i18n/en-GB.json"),
            Locale::FrFr => include_str!("../../../assets/i18n/fr-FR.json"),
        }
    }

    fn table(self) -> &'static HashMap<String, String> {
        static TABLES: OnceLock<HashMap<Locale, HashMap<String, String>>> = OnceLock::new();
        let all = TABLES.get_or_init(|| {
            Locale::ALL
                .iter()
                .map(|&l| {
                    let map: HashMap<String, String> = serde_json::from_str(l.raw())
                        .unwrap_or_default();
                    (l, map)
                })
                .collect()
        });
        all.get(&self)
            .unwrap_or(all.get(&Locale::EnUs).expect("en-US table initialised"))
    }
}

static FALLBACK: OnceLock<HashMap<String, String>> = OnceLock::new();

fn fallback() -> &'static HashMap<String, String> {
    FALLBACK.get_or_init(|| serde_json::from_str(Locale::EnUs.raw()).unwrap_or_default())
}

/// Translate `key` in `locale`, interpolating `{placeholders}` from `args`.
pub fn t(locale: Locale, key: &str) -> String {
    let s = locale
        .table()
        .get(key)
        .or_else(|| fallback().get(key))
        .map(|s| s.as_str())
        .unwrap_or(key);
    s.to_string()
}

/// Like [`t`] with `{name}` interpolation: pass `&[("name", value)]`.
pub fn tf(locale: Locale, key: &str, args: &[(&str, &str)]) -> String {
    let mut s = t(locale, key);
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// All keys defined in en-US (the reference locale).
pub fn known_keys() -> Vec<String> {
    let mut v: Vec<String> = fallback().keys().cloned().collect();
    v.sort();
    v
}

/// Report missing keys per locale (used by tests and the diagnostics panel).
pub fn missing_keys(locale: Locale) -> Vec<String> {
    let ref_ = fallback();
    locale
        .table()
        .keys()
        .filter(|k| !ref_.contains_key(*k))
        .cloned()
        .chain(ref_.keys().filter(|k| !locale.table().contains_key(*k)).cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_locales_parse_and_cover_reference_keys() {
        for locale in Locale::ALL {
            let map: HashMap<String, String> =
                serde_json::from_str(locale.raw()).unwrap_or_else(|e| panic!("{}: {}", locale.tag(), e));
            assert!(!map.is_empty(), "{} is empty", locale.tag());
            let missing: Vec<String> = fallback()
                .keys()
                .filter(|k| !map.contains_key(*k))
                .cloned()
                .collect();
            assert!(
                missing.is_empty(),
                "{} missing keys: {:?}",
                locale.tag(),
                missing.iter().take(5).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn interpolation() {
        assert!(tf(Locale::EnUs, "status.selected_vehicle", &[("id", "K8")]).contains("K8"));
    }
}
