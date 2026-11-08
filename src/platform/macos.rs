//! macOS backend: AppKit window levels, CoreAudio and CoreGraphics through small FFI calls,
//! and `osascript`, which is the only stable way to talk to music players without private APIs.

use super::unix::{self, percent_to_level};
use super::*;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::Path;

// ---------------------------------------------------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------------------------------------------------

/// Above the menu bar (`NSMainMenuWindowLevel` + 1), on every Space, also next to fullscreen apps.
const NOTCH_WINDOW_LEVEL: isize = 25;
const COLLECTION_CAN_JOIN_ALL_SPACES: usize = 1 << 0;
const COLLECTION_STATIONARY: usize = 1 << 4;
const COLLECTION_FULLSCREEN_AUXILIARY: usize = 1 << 8;

fn with_ns_window(window: &impl HasWindowHandle, apply: impl FnOnce(&AnyObject)) {
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return };
    unsafe {
        let view = handle.ns_view.as_ptr().cast::<AnyObject>();
        let ns_window: Option<Retained<AnyObject>> = msg_send![&*view, window];
        if let Some(ns_window) = ns_window {
            apply(&ns_window);
        }
    }
}

pub fn prepare_notch_window(window: &impl HasWindowHandle) {
    with_ns_window(window, |ns_window| unsafe {
        let _: () = msg_send![ns_window, setLevel: NOTCH_WINDOW_LEVEL];
        let behavior = COLLECTION_CAN_JOIN_ALL_SPACES | COLLECTION_STATIONARY | COLLECTION_FULLSCREEN_AUXILIARY;
        let _: () = msg_send![ns_window, setCollectionBehavior: behavior];
    });
}

pub fn prepare_floating_window(window: &impl HasWindowHandle) {
    with_ns_window(window, |ns_window| unsafe {
        let _: () = msg_send![ns_window, setLevel: NOTCH_WINDOW_LEVEL];
        let behavior = COLLECTION_CAN_JOIN_ALL_SPACES | COLLECTION_FULLSCREEN_AUXILIARY;
        let _: () = msg_send![ns_window, setCollectionBehavior: behavior];
    });
}

pub fn force_focus(window: &impl HasWindowHandle) {
    with_ns_window(window, |ns_window| unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![class!(NSApplication), sharedApplication];
        if let Some(app) = app {
            let _: () = msg_send![&*app, activateIgnoringOtherApps: true];
        }
        let none: *const AnyObject = std::ptr::null();
        let _: () = msg_send![ns_window, makeKeyAndOrderFront: none];
    });
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *const std::ffi::c_void) -> *const std::ffi::c_void;
    fn CGEventGetLocation(event: *const std::ffi::c_void) -> CGPoint;
    fn CGEventSourceButtonState(source_state: i32, button: u32) -> bool;
    fn CGEventSourceSecondsSinceLastEventType(source_state: i32, event_type: u32) -> f64;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: *const std::ffi::c_void);
}

/// In points from the top-left of the main display. The caller works in points on macOS too.
pub fn cursor_position() -> Option<(f32, f32)> {
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        Some((point.x as f32, point.y as f32))
    }
}

pub fn primary_button_down() -> bool {
    const HID_SYSTEM_STATE: i32 = 1;
    const LEFT_BUTTON: u32 = 0;
    unsafe { CGEventSourceButtonState(HID_SYSTEM_STATE, LEFT_BUTTON) }
}

/// macOS moves fullscreen apps to their own Space, where the notch is out of the way by itself.
pub fn fullscreen_state() -> FullscreenKind {
    FullscreenKind::None
}

pub fn foreground_app() -> Option<String> {
    osascript("tell application \"System Events\" to get name of first process whose frontmost is true")
}

pub fn idle_seconds() -> f32 {
    const COMBINED_SESSION_STATE: i32 = 0;
    const ANY_INPUT_EVENT: u32 = u32::MAX;
    unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT) as f32 }
}

fn osascript(script: &str) -> Option<String> {
    capture("osascript", &["-e", script]).map(|text| text.trim().to_string())
}

// ---------------------------------------------------------------------------------------------------------------------
// Media: AppleScript for the players that have a dictionary
// ---------------------------------------------------------------------------------------------------------------------

const PLAYERS: [&str; 2] = ["Spotify", "Music"];
const FIELD_SEPARATOR: &str = "\u{1f}";

pub struct MediaBackend {
    current: Option<&'static str>,
    last_art: Option<(String, u64, Arc<Vec<u8>>)>,
}

fn is_running(app: &str) -> bool {
    osascript(&format!("application \"{app}\" is running")).as_deref() == Some("true")
}

/// One line with the track's fields, or nothing when the player has no track.
fn track_script(app: &str) -> String {
    // Spotify reports the duration in milliseconds, Music in seconds.
    let duration = if app == "Spotify" { "(duration of current track) / 1000" } else { "duration of current track" };
    format!(
        "tell application \"{app}\"\n\
         if player state is stopped then return \"\"\n\
         set s to ASCII character 31\n\
         return (name of current track) & s & (artist of current track) & s & (album of current track) & s & \
         (player state as text) & s & (player position as text) & s & (({duration}) as text)\n\
         end tell"
    )
}

/// Parses what [`track_script`] returns. AppleScript may print "1,5" on systems with a decimal comma.
pub fn parse_track(line: &str) -> Option<(String, String, String, bool, Duration, Duration)> {
    let parts: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
    let [title, artist, album, state, position, duration] = parts.as_slice() else { return None };
    if title.is_empty() {
        return None;
    }
    let seconds = |text: &str| {
        text.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).map_or(Duration::ZERO, Duration::from_secs_f64)
    };
    Some((
        (*title).to_string(),
        (*artist).to_string(),
        (*album).to_string(),
        *state == "playing",
        seconds(position),
        seconds(duration),
    ))
}

impl MediaBackend {
    pub fn new() -> Self {
        Self { current: None, last_art: None }
    }

    pub fn poll(&mut self) -> Option<MediaInfo> {
        let mut fallback = None;
        for app in PLAYERS {
            if !is_running(app) {
                continue;
            }
            let Some(line) = osascript(&track_script(app)) else { continue };
            let Some(track) = parse_track(&line) else { continue };
            if track.3 {
                self.current = Some(app);
                return Some(self.build(app, track));
            }
            fallback.get_or_insert((app, track));
        }
        let (app, track) = fallback?;
        self.current = Some(app);
        Some(self.build(app, track))
    }

    fn build(&mut self, app: &'static str, track: (String, String, String, bool, Duration, Duration)) -> MediaInfo {
        let (title, artist, album, playing, position, duration) = track;
        let (art, art_key) = self.spotify_art(app, &title, &artist);
        MediaInfo {
            title,
            artist,
            album,
            source_app: app.to_string(),
            playing,
            position,
            duration,
            position_read_at: Instant::now(),
            art,
            art_key,
            can_seek: true,
        }
    }

    fn spotify_art(&mut self, app: &str, title: &str, artist: &str) -> (Option<Arc<Vec<u8>>>, u64) {
        let id = format!("{app}|{title}|{artist}");
        if let Some((known, key, bytes)) = &self.last_art
            && *known == id
        {
            return (Some(bytes.clone()), *key);
        }
        if app != "Spotify" {
            return (None, 0);
        }
        let url = osascript("tell application \"Spotify\" to get artwork url of current track");
        let Some(bytes) = url.and_then(|url| crate::services::http::get_bytes(&url, Duration::from_secs(5)).ok()) else {
            return (None, 0);
        };
        let key = hash_bytes(&bytes);
        let bytes = Arc::new(bytes);
        self.last_art = Some((id, key, bytes.clone()));
        (Some(bytes), key)
    }

    pub fn send(&mut self, command: MediaCommand) {
        let Some(app) = self.current else { return };
        let script = match command {
            MediaCommand::PlayPause => format!("tell application \"{app}\" to playpause"),
            MediaCommand::Next => format!("tell application \"{app}\" to next track"),
            MediaCommand::Previous => format!("tell application \"{app}\" to previous track"),
            MediaCommand::Seek(target) => format!("tell application \"{app}\" to set player position to {}", target.as_secs()),
        };
        osascript(&script);
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------------------------------------------------

pub struct AudioBackend;

const AUDIO_OBJECT_SYSTEM: u32 = 1;
const fn four_cc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}
const SCOPE_GLOBAL: u32 = four_cc(b"glob");
const SCOPE_INPUT: u32 = four_cc(b"inpt");
const ELEMENT_MAIN: u32 = 0;
const PROPERTY_DEFAULT_INPUT: u32 = four_cc(b"dIn ");
const PROPERTY_DEVICES: u32 = four_cc(b"dev#");
const PROPERTY_RUNNING_SOMEWHERE: u32 = four_cc(b"gone");
const PROPERTY_STREAMS: u32 = four_cc(b"stm#");

#[repr(C)]
struct PropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyData(
        object: u32,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const std::ffi::c_void,
        data_size: *mut u32,
        data: *mut std::ffi::c_void,
    ) -> i32;
    fn AudioObjectGetPropertyDataSize(
        object: u32,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const std::ffi::c_void,
        data_size: *mut u32,
    ) -> i32;
}

fn read_u32(object: u32, selector: u32, scope: u32) -> Option<u32> {
    let address = PropertyAddress { selector, scope, element: ELEMENT_MAIN };
    let mut value = 0_u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(object, &address, 0, std::ptr::null(), &mut size, (&mut value as *mut u32).cast())
    };
    (status == 0).then_some(value)
}

/// True while any app records from the default input device. Needs no permission.
fn microphone_running() -> bool {
    let Some(device) = read_u32(AUDIO_OBJECT_SYSTEM, PROPERTY_DEFAULT_INPUT, SCOPE_GLOBAL) else { return false };
    read_u32(device, PROPERTY_RUNNING_SOMEWHERE, SCOPE_GLOBAL).is_some_and(|running| running != 0)
}

fn input_device_count() -> usize {
    let address = PropertyAddress { selector: PROPERTY_DEVICES, scope: SCOPE_GLOBAL, element: ELEMENT_MAIN };
    let mut size = 0;
    let status = unsafe { AudioObjectGetPropertyDataSize(AUDIO_OBJECT_SYSTEM, &address, 0, std::ptr::null(), &mut size) };
    if status != 0 {
        return 0;
    }
    let mut ids = vec![0_u32; size as usize / size_of::<u32>()];
    let status = unsafe {
        AudioObjectGetPropertyData(AUDIO_OBJECT_SYSTEM, &address, 0, std::ptr::null(), &mut size, ids.as_mut_ptr().cast())
    };
    if status != 0 {
        return 0;
    }
    ids.iter()
        .filter(|id| {
            let streams = PropertyAddress { selector: PROPERTY_STREAMS, scope: SCOPE_INPUT, element: ELEMENT_MAIN };
            let mut stream_size = 0;
            let status = unsafe { AudioObjectGetPropertyDataSize(**id, &streams, 0, std::ptr::null(), &mut stream_size) };
            status == 0 && stream_size > 0
        })
        .count()
}

impl AudioBackend {
    pub fn new() -> Self {
        Self
    }

    /// `output volume:65, input volume:80, alert volume:100, output muted:false`
    pub fn master(&self) -> Option<VolumeInfo> {
        let settings = osascript("get volume settings")?;
        parse_volume_settings(&settings, "output volume", "output muted")
    }

    pub fn set_master(&self, level: f32) {
        osascript(&format!("set volume output volume {}", (level.clamp(0.0, 1.0) * 100.0).round() as u32));
    }

    pub fn set_master_mute(&self, muted: bool) {
        osascript(&format!("set volume output muted {muted}"));
    }

    pub fn master_peak(&self) -> f32 {
        0.0
    }

    pub fn mic(&self) -> Option<VolumeInfo> {
        if input_device_count() == 0 {
            return None;
        }
        let settings = osascript("get volume settings")?;
        let level = parse_field(&settings, "input volume").and_then(|value| percent_to_level(&value))?;
        Some(VolumeInfo { level, muted: level == 0.0 })
    }

    pub fn set_mic(&self, level: f32) {
        osascript(&format!("set volume input volume {}", (level.clamp(0.0, 1.0) * 100.0).round() as u32));
    }

    /// macOS has no input mute switch, so muting means an input volume of zero.
    pub fn set_mic_muted(&self, muted: bool) {
        osascript(&format!("set volume input volume {}", if muted { 0 } else { 75 }));
    }

    /// macOS offers apps no per-app volume.
    pub fn sessions(&self) -> Vec<AudioSessionInfo> {
        Vec::new()
    }

    pub fn set_session_volume(&self, _id: &str, _level: f32) {}

    pub fn set_session_mute(&self, _id: &str, _muted: bool) {}

    pub fn output_devices(&self) -> Vec<AudioDeviceInfo> {
        Vec::new()
    }

    pub fn input_devices(&self) -> Vec<AudioDeviceInfo> {
        Vec::new()
    }

    pub fn set_default_device(&self, _id: &str) {}
}

fn parse_field(settings: &str, name: &str) -> Option<String> {
    settings.split(',').find_map(|part| {
        let (key, value) = part.split_once(':')?;
        (key.trim() == name).then(|| value.trim().to_string())
    })
}

pub fn parse_volume_settings(settings: &str, volume_key: &str, muted_key: &str) -> Option<VolumeInfo> {
    let level = percent_to_level(&parse_field(settings, volume_key)?)?;
    let muted = parse_field(settings, muted_key).as_deref() == Some("true");
    Some(VolumeInfo { level, muted })
}

// ---------------------------------------------------------------------------------------------------------------------
// Privacy
// ---------------------------------------------------------------------------------------------------------------------

/// macOS does not say who uses the camera without extra permissions, so only the microphone is shown.
pub fn privacy() -> PrivacyInfo {
    let mic = if microphone_running() { vec![crate::i18n::t("privacy.mic.inUse")] } else { Vec::new() };
    PrivacyInfo { mic, camera: Vec::new(), camera_detectable: false }
}

// ---------------------------------------------------------------------------------------------------------------------
// Network and Bluetooth
// ---------------------------------------------------------------------------------------------------------------------

fn wifi_device() -> Option<String> {
    let ports = capture("networksetup", &["-listallhardwareports"])?;
    let mut lines = ports.lines();
    while let Some(line) = lines.next() {
        if line.contains("Wi-Fi") {
            return lines.next()?.strip_prefix("Device:").map(|device| device.trim().to_string());
        }
    }
    None
}

pub fn network() -> NetworkInfo {
    let device = wifi_device();
    let wifi_on = device
        .as_deref()
        .and_then(|device| capture("networksetup", &["-getairportpower", device]))
        .map(|text| text.trim_end().ends_with("On"));
    if wifi_on == Some(true)
        && let Some(device) = device.as_deref()
        && let Some(summary) = capture("ipconfig", &["getsummary", device])
        && let Some(name) = summary.lines().find_map(|line| line.trim().strip_prefix("SSID : ").map(str::to_string))
    {
        return NetworkInfo { kind: NetworkKind::Wifi, name, wifi_on };
    }
    let online = capture("route", &["-n", "get", "default"]).is_some_and(|text| text.contains("interface:"));
    NetworkInfo { kind: if online { NetworkKind::Wired } else { NetworkKind::Offline }, name: String::new(), wifi_on }
}

pub fn set_wifi(on: bool) {
    if let Some(device) = wifi_device() {
        capture("networksetup", &["-setairportpower", &device, if on { "on" } else { "off" }]);
    }
}

/// Needs the small `blueutil` tool; without it the tile only shows the state.
pub fn set_bluetooth(on: bool) {
    capture("blueutil", &["--power", if on { "1" } else { "0" }]);
}

pub fn bluetooth() -> BluetoothInfo {
    let Some(output) = capture("system_profiler", &["SPBluetoothDataType", "-json"]) else { return BluetoothInfo::default() };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&output) else { return BluetoothInfo::default() };
    let controller = root.pointer("/SPBluetoothDataType/0");
    let powered = controller
        .and_then(|c| c.pointer("/controller_properties/controller_state"))
        .and_then(serde_json::Value::as_str)
        .map(|state| state.contains("on"));
    let mut devices = Vec::new();
    if let Some(list) = controller.and_then(|c| c.get("device_connected")).and_then(serde_json::Value::as_array) {
        for item in list {
            if let Some(map) = item.as_object() {
                devices.extend(map.keys().map(|name| BluetoothDevice { name: name.clone(), connected: true }));
            }
        }
    }
    BluetoothInfo { powered, devices }
}

// ---------------------------------------------------------------------------------------------------------------------
// Apps and system commands
// ---------------------------------------------------------------------------------------------------------------------

fn collect_apps(dir: &Path, depth: usize, apps: &mut Vec<AppEntry>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "app") {
            let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
            apps.push(AppEntry { name, launch: path.to_string_lossy().into_owned(), icon: None });
        } else if depth > 0 && path.is_dir() {
            collect_apps(&path, depth - 1, apps);
        }
    }
}

pub fn installed_apps() -> Vec<AppEntry> {
    let mut apps = Vec::new();
    for dir in [
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        crate::paths::home_dir().join("Applications"),
    ] {
        collect_apps(&dir, 2, &mut apps);
    }
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    apps
}

pub fn launch_app(app: &AppEntry) -> bool {
    spawn_detached("open", &[&app.launch])
}

pub fn run_system_command(id: &str) -> bool {
    let system_events = |action: &str| osascript(&format!("tell application \"System Events\" to {action}")).is_some();
    match id {
        "lock" => spawn_detached(
            "osascript",
            &["-e", "tell application \"System Events\" to keystroke \"q\" using {control down, command down}"],
        ),
        "sleep" => spawn_detached("pmset", &["sleepnow"]),
        "restart" => system_events("restart"),
        "shutdown" => system_events("shut down"),
        "logout" => system_events("log out"),
        "trash" => osascript("tell application \"Finder\" to empty trash").is_some(),
        "settings" => spawn_detached("open", &["-b", "com.apple.systempreferences"]),
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
    spawn_detached("open", &["-R", &path.to_string_lossy()]);
}

fn launch_agent() -> PathBuf {
    crate::paths::home_dir().join("Library/LaunchAgents/at.v-notch.plist")
}

pub fn set_autostart(enabled: bool) -> bool {
    let path = launch_agent();
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
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\"><dict><key>Label</key><string>at.v-notch</string>\
         <key>ProgramArguments</key><array><string>{}</string></array>\
         <key>RunAtLoad</key><true/></dict></plist>\n",
        exe.display()
    );
    std::fs::write(path, plist).is_ok()
}

pub fn is_autostart_enabled() -> bool {
    launch_agent().exists()
}

const KEYCHAIN_SERVICE: &str = "V-Notch";

pub fn secret_get(name: &str) -> Option<String> {
    capture("security", &["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", name, "-w"])
        .map(|value| value.trim_end_matches('\n').to_string())
        .or_else(|| unix::file_secret_get(name))
}

pub fn secret_set(name: &str, value: &str) -> bool {
    if value.is_empty() {
        let _ = capture("security", &["delete-generic-password", "-s", KEYCHAIN_SERVICE, "-a", name]);
        return unix::file_secret_set(name, "");
    }
    // `-U` updates an existing item. The value passes through the process list only for this short call.
    capture("security", &["add-generic-password", "-U", "-s", KEYCHAIN_SERVICE, "-a", name, "-w", value]).is_some()
        || unix::file_secret_set(name, value)
}

pub fn clipboard_change_counter() -> Option<u64> {
    unsafe {
        let board: Option<Retained<AnyObject>> = msg_send![class!(NSPasteboard), generalPasteboard];
        let board = board?;
        let count: isize = msg_send![&*board, changeCount];
        Some(count as u64)
    }
}

pub fn copy_selection() {
    osascript("tell application \"System Events\" to keystroke \"c\" using command down");
}

pub fn start_file_drag(window: &impl HasWindowHandle, paths: Vec<PathBuf>) -> bool {
    drag::start_drag(
        window,
        drag::DragItem::Files(paths),
        drag::Image::Raw(include_bytes!("../../assets/logo.png").to_vec()),
        |_, _| {},
        drag::Options::default(),
    )
    .is_ok()
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
    fn volume_settings_are_parsed() {
        let text = "output volume:65, input volume:80, alert volume:100, output muted:true";
        let info = parse_volume_settings(text, "output volume", "output muted").unwrap();
        assert_eq!(info.level, 0.65);
        assert!(info.muted);
        assert!(parse_volume_settings("missing value", "output volume", "output muted").is_none());
    }

    #[test]
    fn track_lines_are_parsed_with_either_decimal_mark() {
        let line = ["Song", "Artist", "Album", "playing", "12,5", "215.0"].join(FIELD_SEPARATOR);
        let (title, _, _, playing, position, duration) = parse_track(&line).unwrap();
        assert_eq!(title, "Song");
        assert!(playing);
        assert_eq!(position, Duration::from_millis(12_500));
        assert_eq!(duration, Duration::from_secs(215));
        assert!(parse_track("").is_none());
    }

    #[test]
    fn four_char_codes_match_core_audio() {
        assert_eq!(four_cc(b"glob"), 0x676c_6f62);
    }
}
