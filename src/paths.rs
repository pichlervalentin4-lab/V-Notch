//! Where V-Notch keeps its files on each platform.

use directories::ProjectDirs;
use std::path::PathBuf;
use std::sync::OnceLock;

static OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Moves all app files elsewhere, for demo and test runs.
pub fn set_override(path: PathBuf) {
    let _ = OVERRIDE.set(path);
}

fn overridden(sub: &str) -> Option<PathBuf> {
    let root = OVERRIDE.get().cloned().or_else(|| std::env::var_os("VNOTCH_HOME").map(PathBuf::from))?;
    let dir = root.join(sub);
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

fn dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "V-Notch")
}

fn fallback() -> PathBuf {
    std::env::temp_dir().join("v-notch")
}

/// Settings live here.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = overridden("config") {
        return dir;
    }
    let dir = dirs().map_or_else(fallback, |d| d.config_dir().to_path_buf());
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Clipboard history, shelf, chat history and the log live here.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = overridden("data") {
        return dir;
    }
    let dir = dirs().map_or_else(fallback, |d| d.data_local_dir().to_path_buf());
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Downloaded artwork and lyrics; safe to delete at any time.
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = overridden("cache") {
        return dir;
    }
    let dir = dirs().map_or_else(fallback, |d| d.cache_dir().to_path_buf());
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .unwrap_or_else(fallback)
}
