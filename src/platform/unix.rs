//! Shared by the Linux and macOS backends.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn secrets_path() -> PathBuf {
    crate::paths::config_dir().join("secrets.json")
}

/// Fallback when no OS secret store is reachable: a file only this user may read.
pub fn file_secret_get(name: &str) -> Option<String> {
    let text = std::fs::read_to_string(secrets_path()).ok()?;
    let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&text).ok()?;
    map.get(name)?.as_str().map(str::to_string)
}

pub fn file_secret_set(name: &str, value: &str) -> bool {
    let path = secrets_path();
    let mut map: serde_json::Map<String, serde_json::Value> =
        std::fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
    if value.is_empty() {
        map.remove(name);
    } else {
        map.insert(name.to_string(), value.into());
    }
    let Ok(text) = serde_json::to_string(&map) else { return false };
    // Created empty and locked down first, so the secret is never readable by others.
    if std::fs::write(&path, "").is_err() {
        return false;
    }
    if std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).is_err() {
        return false;
    }
    std::fs::write(&path, text).is_ok()
}

/// Clamps a percentage text such as "65%" to 0..1.
pub fn percent_to_level(text: &str) -> Option<f32> {
    let value: f32 = text.trim().trim_end_matches('%').trim().parse().ok()?;
    Some((value / 100.0).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_parse() {
        assert_eq!(percent_to_level("65%"), Some(0.65));
        assert_eq!(percent_to_level(" 150 %"), Some(1.0));
        assert_eq!(percent_to_level("x"), None);
    }
}
