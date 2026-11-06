//! A small append-only log file. Never log secrets, clipboard contents or chat text here.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_LOG_BYTES: u64 = 1024 * 1024;

static ENABLED: AtomicBool = AtomicBool::new(true);
static FILE: Mutex<Option<File>> = Mutex::new(None);

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn path() -> std::path::PathBuf {
    crate::paths::data_dir().join("v-notch.log")
}

pub fn write(tag: &str, message: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let line = format!(
        "{} [{}] {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        tag,
        message
    );
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }
    let Ok(mut guard) = FILE.lock() else { return };
    if guard.is_none() {
        let path = path();
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
            let _ = std::fs::remove_file(&path);
        }
        *guard = OpenOptions::new().create(true).append(true).open(path).ok();
    }
    if let Some(file) = guard.as_mut() {
        let _ = file.write_all(line.as_bytes());
    }
}

#[macro_export]
macro_rules! vlog {
    ($tag:expr, $($arg:tt)*) => {
        $crate::vlog::write($tag, &format!($($arg)*))
    };
}
