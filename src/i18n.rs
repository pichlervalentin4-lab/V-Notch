//! UI strings. The locale files come from the original project; `extra_*.json` adds what the port needs.

use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

pub const LANGUAGES: &[(&str, &str)] = &[
    ("de", "Deutsch"),
    ("en", "English"),
    ("ar", "العربية"),
    ("es", "Español"),
    ("fr", "Français"),
    ("hi", "हिन्दी"),
    ("id", "Bahasa Indonesia"),
    ("it", "Italiano"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("nl", "Nederlands"),
    ("pl", "Polski"),
    ("pt", "Português"),
    ("ru", "Русский"),
    ("tr", "Türkçe"),
    ("vi", "Tiếng Việt"),
    ("zh", "中文"),
];

fn raw_locale(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "ar" => include_str!("../assets/locales/ar.json"),
        "de" => include_str!("../assets/locales/de.json"),
        "en" => include_str!("../assets/locales/en.json"),
        "es" => include_str!("../assets/locales/es.json"),
        "fr" => include_str!("../assets/locales/fr.json"),
        "hi" => include_str!("../assets/locales/hi.json"),
        "id" => include_str!("../assets/locales/id.json"),
        "it" => include_str!("../assets/locales/it.json"),
        "ja" => include_str!("../assets/locales/ja.json"),
        "ko" => include_str!("../assets/locales/ko.json"),
        "nl" => include_str!("../assets/locales/nl.json"),
        "pl" => include_str!("../assets/locales/pl.json"),
        "pt" => include_str!("../assets/locales/pt.json"),
        "ru" => include_str!("../assets/locales/ru.json"),
        "tr" => include_str!("../assets/locales/tr.json"),
        "vi" => include_str!("../assets/locales/vi.json"),
        "zh" => include_str!("../assets/locales/zh.json"),
        _ => return None,
    })
}

const EXTRA_EN: &str = include_str!("../assets/locales/extra_en.json");
const EXTRA_DE: &str = include_str!("../assets/locales/extra_de.json");

type Table = HashMap<String, String>;

fn parse_into(table: &mut Table, json: &str) {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(json) {
        flatten(table, "", &map);
    }
}

fn flatten(table: &mut Table, prefix: &str, map: &Map<String, Value>) {
    for (key, value) in map {
        let full = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match value {
            Value::String(text) => {
                table.insert(full, text.clone());
            }
            Value::Object(child) => flatten(table, &full, child),
            _ => {}
        }
    }
}

fn build(lang: &str) -> Table {
    let mut table = Table::new();
    parse_into(&mut table, raw_locale("en").unwrap_or("{}"));
    parse_into(&mut table, EXTRA_EN);
    if lang != "en" {
        if let Some(raw) = raw_locale(lang) {
            parse_into(&mut table, raw);
        }
        if lang == "de" {
            parse_into(&mut table, EXTRA_DE);
        }
    }
    table
}

fn current() -> &'static RwLock<Arc<Table>> {
    static CURRENT: OnceLock<RwLock<Arc<Table>>> = OnceLock::new();
    CURRENT.get_or_init(|| RwLock::new(Arc::new(build("de"))))
}

pub fn set_language(lang: &str) {
    let table = Arc::new(build(lang));
    if let Ok(mut guard) = current().write() {
        *guard = table;
    }
}

/// The string for `key`, or the key itself so a missing string stays visible.
pub fn t(key: &str) -> String {
    current()
        .read()
        .ok()
        .and_then(|table| table.get(key).cloned())
        .unwrap_or_else(|| key.to_string())
}

/// Like [`t`], filling `{0}`, `{1}` … with `args`.
pub fn tf(key: &str, args: &[&str]) -> String {
    let mut text = t(key);
    for (index, arg) in args.iter().enumerate() {
        text = text.replace(&format!("{{{index}}}"), arg);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_parses_and_has_keys() {
        let english = build("en");
        assert!(english.len() > 1000);
        for (lang, _) in LANGUAGES {
            let raw = raw_locale(lang).expect("locale listed but not embedded");
            let mut only = Table::new();
            parse_into(&mut only, raw);
            assert!(only.len() > 500, "{lang} has only {} keys", only.len());
        }
    }

    #[test]
    fn german_overrides_english() {
        assert_ne!(build("de").get("tray.exit"), build("en").get("tray.exit"));
    }

    #[test]
    fn extra_files_define_the_same_keys() {
        let (mut en, mut de) = (Table::new(), Table::new());
        parse_into(&mut en, EXTRA_EN);
        parse_into(&mut de, EXTRA_DE);
        let missing: Vec<_> = en.keys().filter(|k| !de.contains_key(*k)).collect();
        assert!(missing.is_empty(), "missing in extra_de: {missing:?}");
        let surplus: Vec<_> = de.keys().filter(|k| !en.contains_key(*k)).collect();
        assert!(surplus.is_empty(), "missing in extra_en: {surplus:?}");
    }

    #[test]
    fn missing_key_falls_back_to_key() {
        assert_eq!(t("this.key.does.not.exist"), "this.key.does.not.exist");
    }

    #[test]
    fn placeholders_are_filled() {
        let table = build("en");
        let text = table.get("clipboard.deleteSelected").unwrap().replace("{0}", "3");
        assert_eq!(text, "Delete 3 selected items");
    }
}
