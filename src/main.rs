//! V-Notch: a desktop status and media companion for Windows, Linux and macOS.
//! A Rust port of https://github.com/DavidLeitnerHTL/V-Notch (Apache-2.0).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod anim;
mod app;
mod i18n;
mod icons;
mod notch;
mod paths;
mod platform;
mod services;
mod settings;
mod settings_ui;
mod spotlight;
mod theme;
mod translate;
mod vlog;

use app::Options;
use settings::SharedSettings;
use std::path::PathBuf;

fn parse_args() -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--demo" => options.demo = true,
            "--view" => options.view = Some(value("--view")?),
            "--spotlight" => options.spotlight = true,
            "--spotlight-ai" => options.spotlight_ai = true,
            "--query" => options.query = Some(value("--query")?),
            "--settings" => options.settings_section = Some(value("--settings")?),
            "--island" => options.island = true,
            "--toast" => options.toast = Some(value("--toast")?),
            "--language" => options.language = Some(value("--language")?),
            "--screenshot" => options.screenshot = Some(PathBuf::from(value("--screenshot")?)),
            "--exit-after" => {
                let seconds = value("--exit-after")?;
                options.exit_after = Some(seconds.parse().map_err(|_| "--exit-after needs seconds".to_string())?);
            }
            "--version" => {
                println!("v-notch {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(options)
}

/// One running copy is enough. The lock disappears with the process, even after a crash.
fn acquire_single_instance() -> Option<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths::data_dir().join("v-notch.lock"))
        .ok()?;
    file.try_lock().ok()?;
    Some(file)
}

fn main() {
    let options = match parse_args() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    // A demo run must never touch the real settings, clipboard history or shelf.
    if options.demo {
        let home = std::env::temp_dir().join(format!("v-notch-demo-{}", std::process::id()));
        paths::set_override(home);
    }

    // Wayland clients cannot place themselves or stay on top, so XWayland is preferred where it exists.
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_some() {
        // SAFETY: no other thread is running yet.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
    }

    std::panic::set_hook(Box::new(|info| {
        vlog!("PANIC", "{info}");
        eprintln!("{info}");
    }));

    let _lock = if options.demo {
        None
    } else {
        match acquire_single_instance() {
            Some(lock) => Some(lock),
            None => {
                eprintln!("{}", i18n::t("error.alreadyRunning"));
                return;
            }
        }
    };

    let mut loaded = if options.demo { settings::Settings::default() } else { settings::load() };
    if options.island {
        loaded.enable_dynamic_island_mode = true;
    }
    let shared = SharedSettings::new(loaded);
    vlog!("APP", "starting {}", env!("CARGO_PKG_VERSION"));
    if let Err(error) = app::run(shared, options) {
        vlog!("APP", "window failed: {error}");
        eprintln!("{error}");
        std::process::exit(1);
    }
}
