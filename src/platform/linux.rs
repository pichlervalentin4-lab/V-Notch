//! Linux backend: MPRIS over D-Bus, PulseAudio/PipeWire through `pactl`, X11 for window state.
//!
//! The app forces the X11 backend when XWayland is around, because Wayland clients cannot position
//! themselves or stay on top on their own.

use super::unix::{self, percent_to_level};
use super::*;
use raw_window_handle::HasWindowHandle;
use std::path::Path;
use std::sync::OnceLock;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
use x11rb::rust_connection::RustConnection;
use zbus::blocking::{Connection as Bus, Proxy};
use zbus::zvariant::{OwnedValue, Value};

// ---------------------------------------------------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------------------------------------------------

pub fn prepare_notch_window(_window: &impl HasWindowHandle) {}

pub fn prepare_floating_window(_window: &impl HasWindowHandle) {}

pub fn force_focus(_window: &impl HasWindowHandle) {}

fn x11() -> Option<&'static (RustConnection, usize)> {
    static CONNECTION: OnceLock<Option<(RustConnection, usize)>> = OnceLock::new();
    CONNECTION.get_or_init(|| x11rb::connect(None).ok()).as_ref()
}

pub fn cursor_position() -> Option<(f32, f32)> {
    let (connection, screen) = x11()?;
    let root = connection.setup().roots.get(*screen)?.root;
    let reply = connection.query_pointer(root).ok()?.reply().ok()?;
    Some((f32::from(reply.root_x), f32::from(reply.root_y)))
}

pub fn primary_button_down() -> bool {
    const BUTTON1_MASK: u16 = 1 << 8;
    let read = || -> Option<bool> {
        let (connection, screen) = x11()?;
        let root = connection.setup().roots.get(*screen)?.root;
        let reply = connection.query_pointer(root).ok()?.reply().ok()?;
        Some(u16::from(reply.mask) & BUTTON1_MASK != 0)
    };
    read().unwrap_or(false)
}

fn intern(connection: &RustConnection, name: &str) -> Option<u32> {
    Some(connection.intern_atom(false, name.as_bytes()).ok()?.reply().ok()?.atom)
}

fn active_window(connection: &RustConnection, root: u32) -> Option<u32> {
    let atom = intern(connection, "_NET_ACTIVE_WINDOW")?;
    let reply = connection.get_property(false, root, atom, AtomEnum::WINDOW, 0, 1).ok()?.reply().ok()?;
    reply.value32()?.next().filter(|id| *id != 0)
}

pub fn fullscreen_state() -> FullscreenKind {
    let check = || -> Option<FullscreenKind> {
        let (connection, screen) = x11()?;
        let root = connection.setup().roots.get(*screen)?.root;
        let window = active_window(connection, root)?;
        let state = intern(connection, "_NET_WM_STATE")?;
        let fullscreen = intern(connection, "_NET_WM_STATE_FULLSCREEN")?;
        let reply = connection.get_property(false, window, state, AtomEnum::ATOM, 0, 16).ok()?.reply().ok()?;
        let is_fullscreen = reply.value32()?.any(|atom| atom == fullscreen);
        Some(if is_fullscreen { FullscreenKind::Windowed } else { FullscreenKind::None })
    };
    check().unwrap_or_default()
}

pub fn foreground_app() -> Option<String> {
    let (connection, screen) = x11()?;
    let root = connection.setup().roots.get(*screen)?.root;
    let window = active_window(connection, root)?;
    let atom = intern(connection, "_NET_WM_PID")?;
    let reply = connection.get_property(false, window, atom, AtomEnum::CARDINAL, 0, 1).ok()?.reply().ok()?;
    let pid = reply.value32()?.next()?;
    std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|name| name.trim().to_string())
}

/// There is no portable idle time on Linux without an extension; the idle hide simply stays off.
pub fn idle_seconds() -> f32 {
    0.0
}

// ---------------------------------------------------------------------------------------------------------------------
// Media: MPRIS
// ---------------------------------------------------------------------------------------------------------------------

const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const MPRIS_PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const MPRIS_ROOT: &str = "org.mpris.MediaPlayer2";

pub struct MediaBackend {
    bus: Option<Bus>,
    current: Option<String>,
    last_art: Option<(String, u64, Arc<Vec<u8>>)>,
}

impl MediaBackend {
    pub fn new() -> Self {
        Self { bus: Bus::session().ok(), current: None, last_art: None }
    }

    fn players(&self) -> Vec<String> {
        let Some(bus) = self.bus.as_ref() else { return Vec::new() };
        let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(bus) else { return Vec::new() };
        let Ok(names) = proxy.list_names() else { return Vec::new() };
        names
            .into_iter()
            .map(|name| name.to_string())
            .filter(|name| name.starts_with("org.mpris.MediaPlayer2."))
            .collect()
    }

    fn player(&self, name: &str) -> Option<Proxy<'_>> {
        Proxy::new(self.bus.as_ref()?, name.to_string(), MPRIS_PATH, MPRIS_PLAYER).ok()
    }

    /// The playing player if there is one, else the one followed so far, else the first.
    fn choose(&mut self) -> Option<String> {
        let players = self.players();
        let playing = players.iter().find(|name| {
            self.player(name)
                .and_then(|proxy| proxy.get_property::<String>("PlaybackStatus").ok())
                .is_some_and(|status| status == "Playing")
        });
        let chosen = playing
            .or_else(|| players.iter().find(|name| Some(*name) == self.current.as_ref()))
            .or_else(|| players.first())
            .cloned();
        self.current.clone_from(&chosen);
        chosen
    }

    pub fn poll(&mut self) -> Option<MediaInfo> {
        let name = self.choose()?;
        let proxy = self.player(&name)?;
        let metadata: std::collections::HashMap<String, OwnedValue> = proxy.get_property("Metadata").ok()?;
        let title = meta_string(&metadata, "xesam:title")?;
        if title.is_empty() {
            return None;
        }
        let playing = proxy.get_property::<String>("PlaybackStatus").is_ok_and(|status| status == "Playing");
        let position = proxy.get_property::<i64>("Position").map_or(0, |value| value.max(0));
        let duration = meta_i64(&metadata, "mpris:length").unwrap_or(0).max(0);
        let artist = meta_string_list(&metadata, "xesam:artist").unwrap_or_default();
        let album = meta_string(&metadata, "xesam:album").unwrap_or_default();
        let art_url = meta_string(&metadata, "mpris:artUrl").unwrap_or_default();

        let identity = Proxy::new(self.bus.as_ref()?, name.clone(), MPRIS_PATH, MPRIS_ROOT)
            .ok()
            .and_then(|root| root.get_property::<String>("Identity").ok())
            .unwrap_or_else(|| name.trim_start_matches("org.mpris.MediaPlayer2.").to_string());

        let (art, art_key) = self.art_for(&art_url);
        Some(MediaInfo {
            title,
            artist,
            album,
            source_app: identity,
            playing,
            position: Duration::from_micros(position as u64),
            duration: Duration::from_micros(duration as u64),
            position_read_at: Instant::now(),
            art,
            art_key,
            can_seek: proxy.get_property::<bool>("CanSeek").unwrap_or(false),
        })
    }

    fn art_for(&mut self, url: &str) -> (Option<Arc<Vec<u8>>>, u64) {
        if url.is_empty() {
            return (None, 0);
        }
        if let Some((known, key, bytes)) = &self.last_art
            && known == url
        {
            return (Some(bytes.clone()), *key);
        }
        let bytes = if let Some(path) = url.strip_prefix("file://") {
            std::fs::read(percent_decode(path)).ok()
        } else if url.starts_with("http") {
            crate::services::http::get_bytes(url, Duration::from_secs(5)).ok()
        } else {
            None
        };
        match bytes {
            Some(bytes) => {
                let key = hash_bytes(&bytes);
                let bytes = Arc::new(bytes);
                self.last_art = Some((url.to_string(), key, bytes.clone()));
                (Some(bytes), key)
            }
            None => (None, 0),
        }
    }

    pub fn send(&mut self, command: MediaCommand) {
        let Some(name) = self.current.clone().or_else(|| self.choose()) else { return };
        let Some(proxy) = self.player(&name) else { return };
        let _: zbus::Result<()> = match command {
            MediaCommand::PlayPause => proxy.call("PlayPause", &()),
            MediaCommand::Next => proxy.call("Next", &()),
            MediaCommand::Previous => proxy.call("Previous", &()),
            MediaCommand::Seek(target) => {
                let current = proxy.get_property::<i64>("Position").unwrap_or(0);
                proxy.call("Seek", &(target.as_micros() as i64 - current))
            }
        };
    }
}

fn meta_string(metadata: &std::collections::HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    match &**metadata.get(key)? {
        Value::Str(text) => Some(text.to_string()),
        _ => None,
    }
}

fn meta_string_list(metadata: &std::collections::HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    match &**metadata.get(key)? {
        Value::Str(text) => Some(text.to_string()),
        Value::Array(items) => {
            let names: Vec<String> = items
                .iter()
                .filter_map(|item| if let Value::Str(text) = item { Some(text.to_string()) } else { None })
                .collect();
            Some(names.join(", "))
        }
        _ => None,
    }
}

fn meta_i64(metadata: &std::collections::HashMap<String, OwnedValue>, key: &str) -> Option<i64> {
    match &**metadata.get(key)? {
        Value::I64(value) => Some(*value),
        Value::U64(value) => i64::try_from(*value).ok(),
        Value::I32(value) => Some(i64::from(*value)),
        Value::U32(value) => Some(i64::from(*value)),
        _ => None,
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(hex) = text.get(index + 1..index + 3)
            && let Ok(value) = u8::from_str_radix(hex, 16)
        {
            out.push(value);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------------------------------------------------
// Audio: pactl works on both PulseAudio and PipeWire
// ---------------------------------------------------------------------------------------------------------------------

pub struct AudioBackend;

fn pactl(args: &[&str]) -> Option<String> {
    capture("pactl", args)
}

fn parse_volume(output: &str) -> Option<f32> {
    let percent = output.split_whitespace().find(|word| word.ends_with('%'))?;
    percent_to_level(percent)
}

fn parse_mute(output: &str) -> bool {
    output.to_lowercase().contains("yes") || output.to_lowercase().contains("ja")
}

fn pactl_json(kind: &str) -> Option<Vec<serde_json::Value>> {
    let output = pactl(&["--format=json", "list", kind])?;
    serde_json::from_str(&output).ok()
}

fn json_level(item: &serde_json::Value) -> f32 {
    item["volume"]
        .as_object()
        .and_then(|channels| channels.values().next())
        .and_then(|channel| channel["value_percent"].as_str())
        .and_then(percent_to_level)
        .unwrap_or(1.0)
}

impl AudioBackend {
    pub fn new() -> Self {
        Self
    }

    fn sink_state(sink_or_source: &str, default: &str) -> Option<VolumeInfo> {
        let volume = pactl(&[&format!("get-{sink_or_source}-volume"), default])?;
        let mute = pactl(&[&format!("get-{sink_or_source}-mute"), default])?;
        Some(VolumeInfo { level: parse_volume(&volume)?, muted: parse_mute(&mute) })
    }

    pub fn master(&self) -> Option<VolumeInfo> {
        Self::sink_state("sink", "@DEFAULT_SINK@")
    }

    pub fn set_master(&self, level: f32) {
        let value = format!("{}%", (level.clamp(0.0, 1.0) * 100.0).round() as u32);
        pactl(&["set-sink-volume", "@DEFAULT_SINK@", &value]);
    }

    pub fn set_master_mute(&self, muted: bool) {
        pactl(&["set-sink-mute", "@DEFAULT_SINK@", if muted { "1" } else { "0" }]);
    }

    /// `pactl` has no level meter; the visualizer animates from the play state instead.
    pub fn master_peak(&self) -> f32 {
        0.0
    }

    pub fn mic(&self) -> Option<VolumeInfo> {
        Self::sink_state("source", "@DEFAULT_SOURCE@")
    }

    pub fn set_mic_muted(&self, muted: bool) {
        pactl(&["set-source-mute", "@DEFAULT_SOURCE@", if muted { "1" } else { "0" }]);
    }

    pub fn sessions(&self) -> Vec<AudioSessionInfo> {
        let Some(items) = pactl_json("sink-inputs") else { return Vec::new() };
        items
            .iter()
            .filter_map(|item| {
                let id = item["index"].as_u64()?.to_string();
                let properties = &item["properties"];
                let name = properties["application.name"]
                    .as_str()
                    .or_else(|| properties["media.name"].as_str())
                    .unwrap_or("Audio")
                    .to_string();
                Some(AudioSessionInfo {
                    id,
                    name,
                    level: json_level(item),
                    muted: item["mute"].as_bool().unwrap_or(false),
                    peak: 0.0,
                })
            })
            .collect()
    }

    pub fn set_session_volume(&self, id: &str, level: f32) {
        let value = format!("{}%", (level.clamp(0.0, 1.0) * 100.0).round() as u32);
        pactl(&["set-sink-input-volume", id, &value]);
    }

    pub fn set_session_mute(&self, id: &str, muted: bool) {
        pactl(&["set-sink-input-mute", id, if muted { "1" } else { "0" }]);
    }

    fn devices(kind: &str, default_key: &str) -> Vec<AudioDeviceInfo> {
        let default = pactl(&[default_key]).unwrap_or_default().trim().to_string();
        let Some(items) = pactl_json(kind) else { return Vec::new() };
        items
            .iter()
            .filter_map(|item| {
                let id = item["name"].as_str()?.to_string();
                // Monitor sources mirror an output; they are not microphones.
                if kind == "sources" && item["monitor_source"].as_str().is_some_and(|m| !m.is_empty()) && id.ends_with(".monitor") {
                    return None;
                }
                let name = item["description"].as_str().unwrap_or(&id).to_string();
                Some(AudioDeviceInfo { is_default: id == default, id, name })
            })
            .collect()
    }

    pub fn output_devices(&self) -> Vec<AudioDeviceInfo> {
        Self::devices("sinks", "get-default-sink")
    }

    pub fn input_devices(&self) -> Vec<AudioDeviceInfo> {
        Self::devices("sources", "get-default-source")
    }

    pub fn set_default_device(&self, id: &str) {
        if self.input_devices().iter().any(|device| device.id == id) {
            pactl(&["set-default-source", id]);
        } else {
            pactl(&["set-default-sink", id]);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Privacy
// ---------------------------------------------------------------------------------------------------------------------

/// Processes holding `/dev/video*` open.
fn camera_users() -> Vec<String> {
    let mut users = Vec::new();
    let Ok(processes) = std::fs::read_dir("/proc") else { return users };
    for process in processes.flatten() {
        let name = process.file_name();
        let Some(pid) = name.to_str().filter(|n| n.chars().all(|c| c.is_ascii_digit())) else { continue };
        let Ok(descriptors) = std::fs::read_dir(format!("/proc/{pid}/fd")) else { continue };
        let holds_camera = descriptors
            .flatten()
            .filter_map(|fd| std::fs::read_link(fd.path()).ok())
            .any(|target| target.to_string_lossy().starts_with("/dev/video"));
        if holds_camera && let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
            users.push(comm.trim().to_string());
        }
    }
    users.sort();
    users.dedup();
    users
}

pub fn privacy() -> PrivacyInfo {
    let mic = pactl_json("source-outputs")
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item["properties"]["application.name"].as_str().map(str::to_string))
        .collect();
    PrivacyInfo { mic, camera: camera_users(), camera_detectable: true }
}

// ---------------------------------------------------------------------------------------------------------------------
// Network and Bluetooth
// ---------------------------------------------------------------------------------------------------------------------

pub fn network() -> NetworkInfo {
    let wifi_on = capture("nmcli", &["radio", "wifi"]).map(|text| text.trim() == "enabled");
    let mut info = NetworkInfo { kind: NetworkKind::Offline, name: String::new(), wifi_on };
    let Some(output) = capture("nmcli", &["-t", "-f", "TYPE,STATE,CONNECTION", "device"]) else {
        // Without NetworkManager a default route still tells whether we are online.
        if std::fs::read_to_string("/proc/net/route").is_ok_and(|routes| routes.lines().skip(1).any(|l| l.split('\t').nth(1) == Some("00000000"))) {
            info.kind = NetworkKind::Wired;
        }
        return info;
    };
    for line in output.lines() {
        let mut parts = line.splitn(3, ':');
        let (Some(kind), Some(state), Some(name)) = (parts.next(), parts.next(), parts.next()) else { continue };
        if state != "connected" {
            continue;
        }
        match kind {
            "wifi" => return NetworkInfo { kind: NetworkKind::Wifi, name: name.to_string(), wifi_on },
            "ethernet" => info.kind = NetworkKind::Wired,
            _ => {}
        }
    }
    info
}

pub fn set_wifi(on: bool) {
    capture("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
}

pub fn set_bluetooth(on: bool) {
    capture("bluetoothctl", &["power", if on { "on" } else { "off" }]);
}

pub fn bluetooth() -> BluetoothInfo {
    let Some(show) = capture("bluetoothctl", &["show"]) else { return BluetoothInfo::default() };
    let powered = show.contains("Powered: yes");
    let mut devices = Vec::new();
    if powered && let Some(list) = capture("bluetoothctl", &["devices", "Connected"]) {
        for line in list.lines() {
            // "Device AA:BB:CC:DD:EE:FF Some Name"
            if let Some(name) = line.splitn(3, ' ').nth(2) {
                devices.push(BluetoothDevice { name: name.to_string(), connected: true });
            }
        }
    }
    BluetoothInfo { powered: Some(powered), devices }
}

// ---------------------------------------------------------------------------------------------------------------------
// Apps and system commands
// ---------------------------------------------------------------------------------------------------------------------

/// Reads the fields that matter from a `.desktop` file; hidden entries give `None`.
pub fn parse_desktop_entry(text: &str) -> Option<(String, String)> {
    let mut in_entry = false;
    let (mut name, mut icon) = (None, None);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "Name" => name = Some(value.trim().to_string()),
            "Icon" => icon = Some(value.trim().to_string()),
            "NoDisplay" | "Hidden" if value.trim() == "true" => return None,
            "Type" if value.trim() != "Application" => return None,
            _ => {}
        }
    }
    Some((name?, icon.unwrap_or_default()))
}

pub fn installed_apps() -> Vec<AppEntry> {
    let home = crate::paths::home_dir();
    let mut dirs = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
        home.join(".local/share/applications"),
        home.join(".local/share/flatpak/exports/share/applications"),
    ];
    if let Ok(extra) = std::env::var("XDG_DATA_DIRS") {
        dirs.extend(extra.split(':').map(|dir| Path::new(dir).join("applications")));
    }
    let mut apps = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "desktop") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Some((name, _icon)) = parse_desktop_entry(&text) else { continue };
            let id = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
            apps.push(AppEntry { name, launch: id, icon: None });
        }
    }
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    apps
}

pub fn launch_app(app: &AppEntry) -> bool {
    spawn_detached("gtk-launch", &[&app.launch]) || spawn_detached("gio", &["launch", &app.launch])
}

pub fn run_system_command(id: &str) -> bool {
    match id {
        "lock" => spawn_detached("loginctl", &["lock-session"]),
        "sleep" => spawn_detached("systemctl", &["suspend"]),
        "restart" => spawn_detached("systemctl", &["reboot"]),
        "shutdown" => spawn_detached("systemctl", &["poweroff"]),
        "logout" => spawn_detached("loginctl", &["terminate-user", &std::env::var("USER").unwrap_or_default()]),
        "trash" => spawn_detached("gio", &["trash", "--empty"]),
        "settings" => spawn_detached("gnome-control-center", &[]) || spawn_detached("systemsettings", &[]),
        _ => false,
    }
}

pub fn file_icon(_path: &Path) -> Option<(Vec<u8>, u32)> {
    None
}

// ---------------------------------------------------------------------------------------------------------------------
// Files, autostart, secrets, input
// ---------------------------------------------------------------------------------------------------------------------

pub fn reveal_path(path: &Path) {
    let folder = path.parent().unwrap_or(path);
    let _ = open::that_detached(folder);
}

fn autostart_file() -> PathBuf {
    crate::paths::home_dir().join(".config/autostart/v-notch.desktop")
}

pub fn set_autostart(enabled: bool) -> bool {
    let path = autostart_file();
    if !enabled {
        return match std::fs::remove_file(&path) {
            Ok(()) => true,
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        };
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    let entry = format!("[Desktop Entry]\nType=Application\nName=V-Notch\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\n", exe.display());
    std::fs::write(path, entry).is_ok()
}

pub fn is_autostart_enabled() -> bool {
    autostart_file().exists()
}

/// The desktop's keyring through `secret-tool` when installed, otherwise a private file.
pub fn secret_get(name: &str) -> Option<String> {
    capture("secret-tool", &["lookup", "application", "v-notch", "name", name])
        .map(|value| value.trim_end_matches('\n').to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| unix::file_secret_get(name))
}

pub fn secret_set(name: &str, value: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};
    if value.is_empty() {
        let _ = Command::new("secret-tool").args(["clear", "application", "v-notch", "name", name]).output();
        return unix::file_secret_set(name, "");
    }
    let stored = Command::new("secret-tool")
        .args(["store", "--label=V-Notch", "application", "v-notch", "name", name])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
        .and_then(|mut child| {
            child.stdin.take()?.write_all(value.as_bytes()).ok()?;
            child.wait().ok()
        })
        .is_some_and(|status| status.success());
    stored || unix::file_secret_set(name, value)
}

pub fn clipboard_change_counter() -> Option<u64> {
    None
}

/// Needs `xdotool`, which most desktop setups have or can install.
pub fn copy_selection() {
    spawn_detached("xdotool", &["key", "--clearmodifiers", "ctrl+c"]);
}

/// Dragging out needs a toolkit drag source; on Linux the shelf offers "open folder" instead.
pub fn start_file_drag(_window: &impl HasWindowHandle, _paths: Vec<PathBuf>) -> bool {
    false
}

pub fn is_legion() -> bool {
    false
}

pub fn legion_request(_request: &str) -> Option<String> {
    None
}

pub fn legion_setup() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entries_are_read() {
        let entry = "[Desktop Entry]\nType=Application\nName=Firefox\nIcon=firefox\n[Desktop Action new]\nName=New Window\n";
        assert_eq!(parse_desktop_entry(entry), Some(("Firefox".into(), "firefox".into())));
    }

    #[test]
    fn hidden_and_non_app_entries_are_skipped() {
        assert_eq!(parse_desktop_entry("[Desktop Entry]\nName=X\nNoDisplay=true\n"), None);
        assert_eq!(parse_desktop_entry("[Desktop Entry]\nName=X\nType=Link\n"), None);
        assert_eq!(parse_desktop_entry("[Other]\nName=X\n"), None);
    }

    #[test]
    fn pactl_output_is_read() {
        let volume = "Volume: front-left: 42598 /  65% / -11.23 dB,   front-right: 42598 /  65% / -11.23 dB";
        assert_eq!(parse_volume(volume), Some(0.65));
        assert!(parse_mute("Mute: yes"));
        assert!(!parse_mute("Mute: no"));
    }

    #[test]
    fn file_urls_are_decoded() {
        assert_eq!(percent_decode("/home/a%20b/c%C3%A4.png"), "/home/a b/cä.png");
        assert_eq!(percent_decode("/no/escape%"), "/no/escape%");
    }
}
