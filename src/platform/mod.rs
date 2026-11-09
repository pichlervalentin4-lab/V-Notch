//! Everything that differs between Windows, Linux and macOS.
//!
//! Each OS module exposes the same functions and types. The rest of the app only sees this module.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use self::win::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use self::linux::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

#[cfg(unix)]
mod unix;

/// What is playing right now, in whichever player the OS calls current.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub source_app: String,
    pub playing: bool,
    pub position: Duration,
    pub duration: Duration,
    /// When `position` was read, to keep the progress bar moving between polls.
    pub position_read_at: Instant,
    /// Encoded image bytes; `art_key` changes when the image does.
    pub art: Option<Arc<Vec<u8>>>,
    pub art_key: u64,
    pub can_seek: bool,
}

impl MediaInfo {
    pub fn current_position(&self) -> Duration {
        let mut position = self.position;
        if self.playing {
            position += self.position_read_at.elapsed();
        }
        if !self.duration.is_zero() {
            position = position.min(self.duration);
        }
        position
    }

    /// Same track, ignoring position and play state.
    pub fn same_track(&self, other: &MediaInfo) -> bool {
        self.title == other.title && self.artist == other.artist && self.source_app == other.source_app
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaCommand {
    PlayPause,
    Next,
    Previous,
    Seek(Duration),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VolumeInfo {
    pub level: f32,
    pub muted: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioSessionInfo {
    pub id: String,
    pub name: String,
    pub level: f32,
    pub muted: bool,
    pub peak: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// Apps currently using the microphone or the camera.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrivacyInfo {
    pub mic: Vec<String>,
    pub camera: Vec<String>,
    /// False where the OS gives no way to tell, so the UI can say so.
    pub camera_detectable: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum NetworkKind {
    #[default]
    Offline,
    Wifi,
    Wired,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetworkInfo {
    pub kind: NetworkKind,
    pub name: String,
    pub wifi_on: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BluetoothDevice {
    pub name: String,
    pub connected: bool,
    /// Headphones or a speaker that the app can connect and disconnect itself.
    pub audio: bool,
}

/// A notification another app showed, as Windows lists it in the notification centre.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppNotification {
    pub id: u32,
    /// The app's id (AppUserModelId) and display name.
    pub app_id: String,
    pub app_name: String,
    /// The first text line (for chats: the sender) and the rest.
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BluetoothInfo {
    pub powered: Option<bool>,
    pub devices: Vec<BluetoothDevice>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum FullscreenKind {
    #[default]
    None,
    Windowed,
    Exclusive,
}

/// An installed application the launcher can start.
#[derive(Clone, Debug, PartialEq)]
pub struct AppEntry {
    pub name: String,
    /// A path, a `.desktop` id or a Windows AppUserModelID, depending on the OS.
    pub launch: String,
    pub icon: Option<PathBuf>,
}

/// A system action offered by Spotlight, such as "Lock" or "Empty trash".
#[derive(Clone, Debug, PartialEq)]
pub struct SystemCommand {
    pub id: &'static str,
    pub label_key: &'static str,
    pub keywords: &'static str,
    pub icon: &'static str,
    /// Disruptive actions ask for confirmation first.
    pub confirm: bool,
}

pub const COMMAND_LOCK: SystemCommand = SystemCommand {
    id: "lock",
    label_key: "port.command.lock",
    keywords: "lock sperren bildschirm screen",
    icon: crate::icons::LOCK,
    confirm: false,
};
pub const COMMAND_SLEEP: SystemCommand = SystemCommand {
    id: "sleep",
    label_key: "port.command.sleep",
    keywords: "sleep suspend ruhezustand energie standby",
    icon: crate::icons::MOON,
    confirm: false,
};
pub const COMMAND_RESTART: SystemCommand = SystemCommand {
    id: "restart",
    label_key: "port.command.restart",
    keywords: "restart reboot neustart neu starten",
    icon: crate::icons::ARROW_CLOCKWISE,
    confirm: true,
};
pub const COMMAND_SHUTDOWN: SystemCommand = SystemCommand {
    id: "shutdown",
    label_key: "port.command.shutdown",
    keywords: "shutdown power off herunterfahren ausschalten",
    icon: crate::icons::POWER,
    confirm: true,
};
pub const COMMAND_LOGOUT: SystemCommand = SystemCommand {
    id: "logout",
    label_key: "port.command.logout",
    keywords: "logout sign out abmelden",
    icon: crate::icons::SIGN_OUT,
    confirm: true,
};
pub const COMMAND_TRASH: SystemCommand = SystemCommand {
    id: "trash",
    label_key: "port.command.trash",
    keywords: "trash recycle bin papierkorb leeren empty",
    icon: crate::icons::TRASH,
    confirm: true,
};
pub const COMMAND_SETTINGS: SystemCommand = SystemCommand {
    id: "settings",
    label_key: "port.command.settings",
    keywords: "settings system einstellungen preferences",
    icon: crate::icons::GEAR,
    confirm: false,
};

/// The commands offered on every OS. Each backend's `run_system_command` handles these ids.
pub fn system_commands() -> Vec<SystemCommand> {
    vec![
        COMMAND_LOCK,
        COMMAND_SLEEP,
        COMMAND_RESTART,
        COMMAND_SHUTDOWN,
        COMMAND_LOGOUT,
        COMMAND_TRASH,
        COMMAND_SETTINGS,
    ]
}

/// Opens the system's network settings, as the original's network tile and boxes do.
pub fn open_network_settings() {
    #[cfg(windows)]
    let _ = open::that_detached("ms-settings:network-status");
    #[cfg(target_os = "macos")]
    let _ = open::that_detached("x-apple.systempreferences:com.apple.Network-Settings.extension");
    #[cfg(target_os = "linux")]
    {
        if !spawn_detached("gnome-control-center", &["network"]) {
            spawn_detached("nm-connection-editor", &[]);
        }
    }
}

/// Runs a command and waits for it, returning its stdout; `None` when it could not run or failed.
pub(crate) fn capture(program: &str, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new(program);
    command.args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    hide_console(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Starts a program and does not wait for it.
pub(crate) fn spawn_detached(program: &str, args: &[&str]) -> bool {
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    hide_console(&mut command);
    command.spawn().is_ok()
}

#[cfg(windows)]
fn hide_console(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut std::process::Command) {}

pub(crate) fn hash_bytes(bytes: &[u8]) -> u64 {
    // FNV-1a: stable across runs and enough to spot a changed image.
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(position: u64, duration: u64, playing: bool) -> MediaInfo {
        MediaInfo {
            title: "t".into(),
            artist: "a".into(),
            album: String::new(),
            source_app: "s".into(),
            playing,
            position: Duration::from_secs(position),
            duration: Duration::from_secs(duration),
            position_read_at: Instant::now(),
            art: None,
            art_key: 0,
            can_seek: true,
        }
    }

    #[test]
    fn paused_position_stays_put() {
        assert_eq!(info(10, 100, false).current_position(), Duration::from_secs(10));
    }

    #[test]
    fn position_never_passes_the_end() {
        let mut media = info(100, 100, true);
        media.position_read_at = Instant::now() - Duration::from_secs(50);
        assert_eq!(media.current_position(), Duration::from_secs(100));
    }

    #[test]
    fn same_track_ignores_position() {
        assert!(info(1, 100, true).same_track(&info(50, 100, false)));
        let mut other = info(1, 100, true);
        other.title = "other".into();
        assert!(!info(1, 100, true).same_track(&other));
    }

    #[test]
    fn hash_is_stable_and_sensitive() {
        assert_eq!(hash_bytes(b"abc"), hash_bytes(b"abc"));
        assert_ne!(hash_bytes(b"abc"), hash_bytes(b"abd"));
    }

    #[test]
    fn every_system_command_has_a_label_and_keywords() {
        for command in system_commands() {
            assert!(!command.keywords.is_empty(), "{}", command.id);
            assert!(command.label_key.starts_with("port.command."), "{}", command.id);
        }
    }
}
