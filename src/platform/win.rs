//! Windows backend: WinRT media sessions, Core Audio, the consent store and Win32 windows.

use super::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::HashSet;
use std::ffi::c_void;
use std::path::Path;
use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice as WinBluetoothDevice, BluetoothLEDevice};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Devices::Radios::{Radio, RadioKind, RadioState};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::Win32::Media::Audio::*;
use windows::Win32::NetworkManagement::IpHelper::{
    GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GAA_FLAG_SKIP_UNICAST, GetAdaptersAddresses,
    IP_ADAPTER_ADDRESSES_LH,
};
use windows::Win32::Networking::WinSock::AF_UNSPEC;
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Com::*;
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHEmptyRecycleBinW, SHERB_NOCONFIRMATION,
    SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW,
    SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{GUID, HRESULT, HSTRING, Interface, PCWSTR, PWSTR};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn hwnd_of(window: &impl HasWindowHandle) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut c_void)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------------------------------------------------

/// Keeps the notch out of Alt+Tab and stops clicks on it from taking focus away from the active app.
pub fn prepare_notch_window(window: &impl HasWindowHandle) {
    let Some(hwnd) = hwnd_of(window) else { return };
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let wanted = (style | (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) as isize) & !(WS_EX_APPWINDOW.0 as isize);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted);
    }
    remove_frame(hwnd);
}

/// A floating window like Spotlight stays out of the taskbar and Alt+Tab but may take keyboard focus.
/// Found by its title, because egui gives no handle for extra viewports. Returns whether it was found.
pub fn prepare_floating_window_by_title(title: &str) -> bool {
    let title = wide(title);
    let Ok(hwnd) = (unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) }) else { return false };
    if hwnd.is_invalid() {
        return false;
    }
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (style | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize));
    }
    remove_frame(hwnd);
    true
}

/// Windows 11 draws a thin border and rounds the corners of every window that keeps caption styles,
/// which winit does even without decorations. On a transparent window that border outlines the whole
/// window, far larger than what is drawn in it.
fn remove_frame(hwnd: HWND) {
    use windows::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute,
    };
    unsafe {
        // Both fail harmlessly on Windows 10, which draws neither.
        let none = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, (&raw const none).cast(), std::mem::size_of_val(&none) as u32);
        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, (&raw const corner).cast(), std::mem::size_of_val(&corner) as u32);
    }
}

/// Windows ignores focus requests from background apps unless it is told the user asked for this.
pub fn force_focus(window: &impl HasWindowHandle) {
    let Some(hwnd) = hwnd_of(window) else { return };
    unsafe {
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_F24, dwFlags: KEYBD_EVENT_FLAGS(0), ..Default::default() } },
        };
        let up = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_F24, dwFlags: KEYEVENTF_KEYUP, ..Default::default() } },
        };
        SendInput(&[input, up], size_of::<INPUT>() as i32);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
}

pub fn cursor_position() -> Option<(f32, f32)> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).ok()? };
    Some((point.x as f32, point.y as f32))
}

pub fn primary_button_down() -> bool {
    unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON.0)) as u16 & 0x8000 != 0 }
}

pub fn fullscreen_state() -> FullscreenKind {
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState()
            && (state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE || state == QUNS_BUSY)
        {
            return FullscreenKind::Exclusive;
        }
        let window = GetForegroundWindow();
        if window.is_invalid() {
            return FullscreenKind::None;
        }
        let mut class = [0u16; 64];
        let length = GetClassNameW(window, &mut class);
        let class = String::from_utf16_lossy(&class[..length.max(0) as usize]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
            return FullscreenKind::None;
        }
        let mut rect = RECT::default();
        if GetWindowRect(window, &mut rect).is_err() {
            return FullscreenKind::None;
        }
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return FullscreenKind::None;
        }
        let area = info.rcMonitor;
        if rect.left <= area.left && rect.top <= area.top && rect.right >= area.right && rect.bottom >= area.bottom {
            FullscreenKind::Windowed
        } else {
            FullscreenKind::None
        }
    }
}

/// The executable name of the active window, without extension.
pub fn foreground_app() -> Option<String> {
    unsafe {
        let window = GetForegroundWindow();
        if window.is_invalid() {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        process_name(pid)
    }
}

/// Seconds since the user last touched mouse or keyboard.
pub fn idle_seconds() -> f32 {
    unsafe {
        let mut info = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if GetLastInputInfo(&mut info).as_bool() {
            let now = windows::Win32::System::SystemInformation::GetTickCount();
            return now.wrapping_sub(info.dwTime) as f32 / 1000.0;
        }
    }
    0.0
}

fn process_name(pid: u32) -> Option<String> {
    let path = process_path(pid)?;
    Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned())
}

fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(process, PROCESS_NAME_FORMAT(0), PWSTR(buffer.as_mut_ptr()), &mut size);
        let _ = CloseHandle(process);
        result.ok()?;
        Some(String::from_utf16_lossy(&buffer[..size as usize]))
    }
}

/// (lower-case exe file name, process id) of every process. Cheap: no process is opened.
fn process_list() -> Vec<(String, u32)> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    let mut list = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return list };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let length = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            list.push((String::from_utf16_lossy(&entry.szExeFile[..length]).to_lowercase(), entry.th32ProcessID));
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    list
}

/// Whether the app behind a consent-store key still runs. A crashed app leaves "in use" behind
/// for good, so the registry alone would show the microphone as busy forever.
/// `path_of` reads a process's image path; it is only called where the exe name could match.
fn consent_owner_running(key: &str, packaged: bool, processes: &[(String, u32)], path_of: impl Fn(u32) -> Option<String>) -> bool {
    let key = key.to_lowercase();
    if packaged {
        // "Vendor.App_publisherid" runs from "...\WindowsApps\Vendor.App_1.2.3.0_x64__publisherid\..."
        // or, for system apps, from "...\SystemApps\Vendor.App_publisherid\...". The exe name says
        // nothing about the package, so every process has to be asked.
        let (name, publisher) = key.rsplit_once('_').unwrap_or((key.as_str(), ""));
        let folder = format!("\\{name}_");
        processes.iter().any(|&(_, pid)| {
            path_of(pid).is_some_and(|path| {
                let path = path.to_lowercase();
                path.contains(&folder) && path.contains(publisher)
            })
        })
    } else {
        // "C:#Program Files#App#app.exe" is the full path with '#' for '\'.
        let path = key.replace('#', "\\");
        let exe = path.rsplit('\\').next().unwrap_or(&path);
        processes
            .iter()
            .filter(|(name, _)| name == exe)
            .any(|&(_, pid)| path_of(pid).is_some_and(|running| running.to_lowercase() == path))
    }
}

/// Verdicts per (consent key, start time). A process that is gone never comes back with the same
/// start time, so "not running" is final; "running" is checked again now and then to notice a crash.
static CONSENT_VERDICTS: std::sync::Mutex<Vec<(String, u64, bool, std::time::Instant)>> = std::sync::Mutex::new(Vec::new());
const RUNNING_RECHECK: std::time::Duration = std::time::Duration::from_secs(10);

// ---------------------------------------------------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------------------------------------------------

pub struct MediaBackend {
    manager: Option<SessionManager>,
    last_art: Option<(String, u64, Arc<Vec<u8>>)>,
}

impl MediaBackend {
    pub fn new() -> Self {
        Self { manager: None, last_art: None }
    }

    fn session(&mut self) -> Option<Session> {
        if self.manager.is_none() {
            self.manager = SessionManager::RequestAsync().ok().and_then(|op| op.join().ok());
        }
        self.manager.as_ref()?.GetCurrentSession().ok()
    }

    pub fn poll(&mut self) -> Option<MediaInfo> {
        let session = self.session()?;
        let properties = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
        let title = properties.Title().ok()?.to_string();
        if title.is_empty() {
            return None;
        }
        let artist = properties.Artist().map(|s| s.to_string()).unwrap_or_default();
        let album = properties.AlbumTitle().map(|s| s.to_string()).unwrap_or_default();
        let source = session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default();
        let info = session.GetPlaybackInfo().ok()?;
        let playing = info.PlaybackStatus().ok() == Some(PlaybackStatus::Playing);
        let timeline = session.GetTimelineProperties().ok()?;
        let to_duration = |ticks: i64| Duration::from_nanos((ticks.max(0) as u64).saturating_mul(100));
        let duration = to_duration(timeline.EndTime().map(|t| t.Duration).unwrap_or(0));
        let mut position = to_duration(timeline.Position().map(|t| t.Duration).unwrap_or(0));
        if playing && let Ok(updated) = timeline.LastUpdatedTime() {
            // The position is only as fresh as the player's last report.

            let now = unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() };
            let now = ((u64::from(now.dwHighDateTime)) << 32 | u64::from(now.dwLowDateTime)) as i64;
            position += to_duration(now - updated.UniversalTime);
        }
        if !duration.is_zero() {
            position = position.min(duration);
        }

        let track_id = format!("{source}|{title}|{artist}");
        let (art, art_key) = match &self.last_art {
            Some((id, key, bytes)) if *id == track_id => (Some(bytes.clone()), *key),
            _ => match read_thumbnail(&properties) {
                Some(bytes) => {
                    let key = hash_bytes(&bytes);
                    let bytes = Arc::new(bytes);
                    self.last_art = Some((track_id, key, bytes.clone()));
                    (Some(bytes), key)
                }
                None => (None, 0),
            },
        };

        Some(MediaInfo {
            title,
            artist,
            album,
            source_app: friendly_app_name(&source),
            playing,
            position,
            duration,
            position_read_at: Instant::now(),
            art,
            art_key,
            can_seek: info.Controls().and_then(|c| c.IsPlaybackPositionEnabled()).unwrap_or(false),
        })
    }

    pub fn send(&mut self, command: MediaCommand) {
        let Some(session) = self.session() else { return };
        let result = match command {
            MediaCommand::PlayPause => session.TryTogglePlayPauseAsync(),
            MediaCommand::Next => session.TrySkipNextAsync(),
            MediaCommand::Previous => session.TrySkipPreviousAsync(),
            MediaCommand::Seek(position) => session.TryChangePlaybackPositionAsync((position.as_nanos() / 100) as i64),
        };
        if let Ok(operation) = result {
            let _ = operation.join();
        }
    }
}

fn read_thumbnail(properties: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties) -> Option<Vec<u8>> {
    let reference = properties.Thumbnail().ok()?;
    let stream = reference.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 || size > 32 * 1024 * 1024 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    reader.LoadAsync(size).ok()?.join().ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    Some(bytes)
}

fn friendly_app_name(id: &str) -> String {
    let lower = id.to_lowercase();
    for (needle, name) in [
        ("spotify", "Spotify"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("zunemusic", "Media Player"),
        ("vlc", "VLC"),
        ("brave", "Brave"),
        ("opera", "Opera"),
    ] {
        if lower.contains(needle) {
            return name.to_string();
        }
    }
    id.split(['!', '_']).next().unwrap_or(id).trim_end_matches(".exe").to_string()
}

// ---------------------------------------------------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------------------------------------------------

/// `IPolicyConfig` is undocumented but stable since Vista; it is the only way to switch the default device.
#[windows::core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: windows::core::IUnknown {
    fn get_mix_format(&self) -> HRESULT;
    fn get_device_format(&self) -> HRESULT;
    fn reset_device_format(&self) -> HRESULT;
    fn set_device_format(&self) -> HRESULT;
    fn get_processing_period(&self) -> HRESULT;
    fn set_processing_period(&self) -> HRESULT;
    fn get_share_mode(&self) -> HRESULT;
    fn set_share_mode(&self) -> HRESULT;
    fn get_property_value(&self) -> HRESULT;
    fn set_property_value(&self) -> HRESULT;
    fn set_default_endpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
}

const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// The default output's volume and meter controls.
#[derive(Clone)]
struct RenderControls {
    volume: IAudioEndpointVolume,
    meter: IAudioMeterInformation,
    found: std::time::Instant,
}

/// How long the output controls are reused. Looking them up is a call into the audio service,
/// too slow to repeat on every poll; a switch of the default device shows up after this at most.
const RENDER_KEEP: std::time::Duration = std::time::Duration::from_secs(3);

pub struct AudioBackend {
    enumerator: Option<IMMDeviceEnumerator>,
    render: std::cell::RefCell<Option<RenderControls>>,
}

impl AudioBackend {
    /// Call on the thread that keeps using it: this joins that thread to COM.
    pub fn new() -> Self {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            Self { enumerator: CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok(), render: std::cell::RefCell::new(None) }
        }
    }

    fn endpoint(&self, flow: EDataFlow) -> Option<IMMDevice> {
        unsafe { self.enumerator.as_ref()?.GetDefaultAudioEndpoint(flow, eConsole).ok() }
    }

    fn endpoint_volume(&self, flow: EDataFlow) -> Option<IAudioEndpointVolume> {
        if flow == eRender {
            return self.render_controls().map(|controls| controls.volume);
        }
        unsafe { self.endpoint(flow)?.Activate(CLSCTX_ALL, None).ok() }
    }

    fn render_controls(&self) -> Option<RenderControls> {
        let mut cached = self.render.borrow_mut();
        if let Some(controls) = cached.as_ref()
            && controls.found.elapsed() < RENDER_KEEP
        {
            return Some(controls.clone());
        }
        let device = self.endpoint(eRender)?;
        let controls = unsafe {
            RenderControls {
                volume: device.Activate(CLSCTX_ALL, None).ok()?,
                meter: device.Activate(CLSCTX_ALL, None).ok()?,
                found: std::time::Instant::now(),
            }
        };
        *cached = Some(controls.clone());
        Some(controls)
    }

    /// Looks the output up again on the next call, after the default device was changed here.
    fn forget_render(&self) {
        self.render.borrow_mut().take();
    }

    pub fn master(&self) -> Option<VolumeInfo> {
        let volume = self.endpoint_volume(eRender)?;
        unsafe {
            Some(VolumeInfo {
                level: volume.GetMasterVolumeLevelScalar().ok()?,
                muted: volume.GetMute().ok()?.as_bool(),
            })
        }
    }

    pub fn set_master(&self, level: f32) {
        if let Some(volume) = self.endpoint_volume(eRender) {
            unsafe {
                let _ = volume.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
            }
        }
    }

    pub fn set_master_mute(&self, muted: bool) {
        if let Some(volume) = self.endpoint_volume(eRender) {
            unsafe {
                let _ = volume.SetMute(muted, std::ptr::null());
            }
        }
    }

    pub fn master_peak(&self) -> f32 {
        let Some(controls) = self.render_controls() else { return 0.0 };
        unsafe { controls.meter.GetPeakValue().unwrap_or(0.0) }
    }

    pub fn mic(&self) -> Option<VolumeInfo> {
        let volume = self.endpoint_volume(eCapture)?;
        unsafe {
            Some(VolumeInfo {
                level: volume.GetMasterVolumeLevelScalar().ok()?,
                muted: volume.GetMute().ok()?.as_bool(),
            })
        }
    }

    pub fn set_mic_muted(&self, muted: bool) {
        if let Some(volume) = self.endpoint_volume(eCapture) {
            unsafe {
                let _ = volume.SetMute(muted, std::ptr::null());
            }
        }
    }

    pub fn sessions(&self) -> Vec<AudioSessionInfo> {
        self.sessions_inner().unwrap_or_default()
    }

    fn session_enumerator(&self) -> Option<IAudioSessionEnumerator> {
        let device = self.endpoint(eRender)?;
        unsafe {
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).ok()?;
            manager.GetSessionEnumerator().ok()
        }
    }

    fn sessions_inner(&self) -> Option<Vec<AudioSessionInfo>> {
        let sessions = self.session_enumerator()?;
        let mut merged: Vec<AudioSessionInfo> = Vec::new();
        unsafe {
            for index in 0..sessions.GetCount().ok()? {
                let Ok(control) = sessions.GetSession(index) else { continue };
                if control.GetState().ok() != Some(AudioSessionStateActive) {
                    continue;
                }
                let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
                if control2.IsSystemSoundsSession() == S_OK {
                    continue;
                }
                let pid = control2.GetProcessId().unwrap_or(0);
                let Some(name) = process_name(pid) else { continue };
                let Ok(volume) = control.cast::<ISimpleAudioVolume>() else { continue };
                let peak = control
                    .cast::<IAudioMeterInformation>()
                    .and_then(|meter| meter.GetPeakValue())
                    .unwrap_or(0.0);
                let level = volume.GetMasterVolume().unwrap_or(1.0);
                let muted = volume.GetMute().map(|b| b.as_bool()).unwrap_or(false);
                // Browsers open several sessions per process: show them as one row.
                if let Some(existing) = merged.iter_mut().find(|s| s.id == name) {
                    existing.peak = existing.peak.max(peak);
                    continue;
                }
                merged.push(AudioSessionInfo { id: name.clone(), name, level, muted, peak });
            }
        }
        merged.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Some(merged)
    }

    fn for_session(&self, id: &str, apply: impl Fn(&ISimpleAudioVolume)) {
        let Some(sessions) = self.session_enumerator() else { return };
        unsafe {
            let Ok(count) = sessions.GetCount() else { return };
            for index in 0..count {
                let Ok(control) = sessions.GetSession(index) else { continue };
                let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
                let pid = control2.GetProcessId().unwrap_or(0);
                if process_name(pid).as_deref() == Some(id)
                    && let Ok(volume) = control.cast::<ISimpleAudioVolume>()
                {
                    apply(&volume);
                }
            }
        }
    }

    pub fn set_session_volume(&self, id: &str, level: f32) {
        self.for_session(id, |volume| unsafe {
            let _ = volume.SetMasterVolume(level.clamp(0.0, 1.0), std::ptr::null());
        });
    }

    pub fn set_session_mute(&self, id: &str, muted: bool) {
        self.for_session(id, |volume| unsafe {
            let _ = volume.SetMute(muted, std::ptr::null());
        });
    }

    pub fn output_devices(&self) -> Vec<AudioDeviceInfo> {
        self.devices(eRender)
    }

    pub fn input_devices(&self) -> Vec<AudioDeviceInfo> {
        self.devices(eCapture)
    }

    fn devices(&self, flow: EDataFlow) -> Vec<AudioDeviceInfo> {
        let Some(enumerator) = self.enumerator.as_ref() else { return Vec::new() };
        let default_id = self
            .endpoint(flow)
            .and_then(|device| unsafe { device.GetId().ok().map(|id| pwstr_take(id)) })
            .unwrap_or_default();
        let mut result = Vec::new();
        unsafe {
            let Ok(devices) = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) else { return result };
            let Ok(count) = devices.GetCount() else { return result };
            for index in 0..count {
                let Ok(device) = devices.Item(index) else { continue };
                let Ok(id) = device.GetId() else { continue };
                let id = pwstr_take(id);
                let name = device
                    .OpenPropertyStore(STGM_READ)
                    .and_then(|store| store.GetValue(&PKEY_Device_FriendlyName))
                    .map(|value| value.to_string())
                    .unwrap_or_else(|_| "Audio".into());
                result.push(AudioDeviceInfo { is_default: id == default_id, id, name });
            }
        }
        result
    }

    pub fn set_default_device(&self, id: &str) {
        unsafe {
            let Ok(policy) = CoCreateInstance::<_, IPolicyConfig>(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) else {
                return;
            };
            let id = wide(id);
            for role in [eConsole, eMultimedia, eCommunications] {
                let _ = policy.set_default_endpoint(PCWSTR(id.as_ptr()), role);
            }
        }
        self.forget_render();
    }
}

fn pwstr_take(text: PWSTR) -> String {
    unsafe {
        let value = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.0 as *const c_void));
        value
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Privacy
// ---------------------------------------------------------------------------------------------------------------------

const CONSENT_STORE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\";

struct RegKey(HKEY);

impl RegKey {
    fn open(parent: HKEY, path: &str, access: REG_SAM_FLAGS) -> Option<Self> {
        let path = wide(path);
        let mut key = HKEY::default();
        let status = unsafe { RegOpenKeyExW(parent, PCWSTR(path.as_ptr()), None, access, &mut key) };
        (status == ERROR_SUCCESS).then_some(Self(key))
    }

    fn subkeys(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut index = 0;
        loop {
            let mut buffer = [0u16; 512];
            let mut length = buffer.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(self.0, index, Some(PWSTR(buffer.as_mut_ptr())), &mut length, None, None, None, None)
            };
            if status != ERROR_SUCCESS {
                return names;
            }
            names.push(String::from_utf16_lossy(&buffer[..length as usize]));
            index += 1;
        }
    }

    fn read_qword(&self, name: &str) -> Option<u64> {
        let name = wide(name);
        let mut value = 0u64;
        let mut size = size_of::<u64>() as u32;
        let status = unsafe {
            RegQueryValueExW(self.0, PCWSTR(name.as_ptr()), None, None, Some((&mut value as *mut u64).cast()), Some(&mut size))
        };
        (status == ERROR_SUCCESS).then_some(value)
    }

    fn read_string(&self, name: &str) -> Option<String> {
        let name = wide(name);
        let mut buffer = [0u16; 1024];
        let mut size = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegQueryValueExW(self.0, PCWSTR(name.as_ptr()), None, None, Some(buffer.as_mut_ptr().cast()), Some(&mut size))
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let chars = (size as usize / 2).min(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..chars]).trim_end_matches('\0').to_string())
    }
}

impl Drop for RegKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// An app counts as using a capability when its last use has a start but no stop time yet
/// and the app is still running.
fn apps_using(capability: &str) -> Vec<String> {
    let root = format!("{CONSENT_STORE}{capability}");
    let Some(store) = RegKey::open(HKEY_CURRENT_USER, &root, KEY_READ) else { return Vec::new() };
    // Keys the registry marks as in use: (key name, packaged app, start time).
    let mut candidates: Vec<(String, bool, u64)> = Vec::new();
    let mut visit = |key_path: String, name: &str, packaged: bool| {
        if let Some(key) = RegKey::open(HKEY_CURRENT_USER, &key_path, KEY_READ)
            && key.read_qword("LastUsedTimeStop") == Some(0)
            && let Some(start) = key.read_qword("LastUsedTimeStart").filter(|&start| start != 0)
        {
            candidates.push((name.to_string(), packaged, start));
        }
    };
    for name in store.subkeys() {
        if name == "NonPackaged" {
            let parent = format!("{root}\\NonPackaged");
            if let Some(non_packaged) = RegKey::open(HKEY_CURRENT_USER, &parent, KEY_READ) {
                for app in non_packaged.subkeys() {
                    visit(format!("{parent}\\{app}"), &app, false);
                }
            }
        } else {
            visit(format!("{root}\\{name}"), &name, true);
        }
    }
    let Ok(mut verdicts) = CONSENT_VERDICTS.lock() else { return Vec::new() };
    // Forget verdicts for keys of this capability that are no longer marked as in use.
    let prefix = format!("{capability}/");
    verdicts.retain(|(id, start, ..)| {
        !id.starts_with(&prefix) || candidates.iter().any(|(name, _, s)| *id == format!("{prefix}{name}") && s == start)
    });
    let mut processes: Option<Vec<(String, u32)>> = None;
    let mut apps = Vec::new();
    for (name, packaged, start) in &candidates {
        let id = format!("{prefix}{name}");
        let known = verdicts.iter().position(|(i, s, ..)| *i == id && s == start);
        let running = match known.map(|index| verdicts[index].clone()) {
            Some((_, _, false, _)) => false,
            Some((_, _, true, checked)) if checked.elapsed() < RUNNING_RECHECK => true,
            _ => {
                let list = processes.get_or_insert_with(process_list);
                let running = consent_owner_running(name, *packaged, list, process_path);
                if let Some(index) = known {
                    verdicts.remove(index);
                }
                verdicts.push((id, *start, running, std::time::Instant::now()));
                running
            }
        };
        if running {
            apps.push(consent_key_name(name));
        }
    }
    apps.sort();
    apps.dedup();
    apps
}

/// "C:#Program Files#App#app.exe" and "Vendor.App_hash" both become a short readable name.
fn consent_key_name(key: &str) -> String {
    let last = key.rsplit('#').next().unwrap_or(key);
    let last = last.split('_').next().unwrap_or(last);
    last.trim_end_matches(".exe").rsplit('.').next().unwrap_or(last).to_string()
}

pub fn privacy() -> PrivacyInfo {
    PrivacyInfo { mic: apps_using("microphone"), camera: apps_using("webcam"), camera_detectable: true }
}

// ---------------------------------------------------------------------------------------------------------------------
// Network and Bluetooth
// ---------------------------------------------------------------------------------------------------------------------

const IF_TYPE_ETHERNET: u32 = 6;
const IF_TYPE_WIFI: u32 = 71;
const IF_OPER_UP: i32 = 1;

fn adapters_up() -> (bool, bool) {
    let mut wired = false;
    let mut wifi = false;
    unsafe {
        let flags = GAA_FLAG_SKIP_UNICAST | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let mut size = 32 * 1024_u32;
        let mut buffer = vec![0u64; size as usize / 8];
        let first = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        if GetAdaptersAddresses(u32::from(AF_UNSPEC.0), flags, None, Some(first), &mut size) != 0 {
            return (false, false);
        }
        let mut current = first;
        while !current.is_null() {
            let adapter = &*current;
            if adapter.OperStatus.0 == IF_OPER_UP {
                let description = adapter.Description.to_string().unwrap_or_default().to_lowercase();
                let virtual_adapter = ["virtual", "vmware", "hyper-v", "vpn", "loopback", "tap-", "wan miniport", "tailscale"]
                    .iter()
                    .any(|needle| description.contains(needle));
                if !virtual_adapter {
                    wired |= adapter.IfType == IF_TYPE_ETHERNET;
                    wifi |= adapter.IfType == IF_TYPE_WIFI;
                }
            }
            current = adapter.Next;
        }
    }
    (wired, wifi)
}

/// `netsh` output is localized, but the "SSID" label is not.
fn current_ssid() -> Option<String> {
    let output = capture("netsh", &["wlan", "show", "interfaces"])?;
    output.lines().find_map(|line| {
        let (label, value) = line.split_once(':')?;
        (label.trim() == "SSID").then(|| value.trim().to_string()).filter(|v| !v.is_empty())
    })
}

fn radio(kind: RadioKind) -> Option<Radio> {
    let radios = Radio::GetRadiosAsync().ok()?.join().ok()?;
    (0..radios.Size().ok()?).filter_map(|i| radios.GetAt(i).ok()).find(|r| r.Kind().ok() == Some(kind))
}

pub fn network() -> NetworkInfo {
    let (wired, wifi) = adapters_up();
    let wifi_on = radio(RadioKind::WiFi).and_then(|r| r.State().ok()).map(|state| state == RadioState::On);
    if wifi {
        return NetworkInfo { kind: NetworkKind::Wifi, name: current_ssid().unwrap_or_default(), wifi_on };
    }
    NetworkInfo { kind: if wired { NetworkKind::Wired } else { NetworkKind::Offline }, name: String::new(), wifi_on }
}

pub fn set_wifi(on: bool) {
    set_radio(RadioKind::WiFi, on);
}

pub fn set_bluetooth(on: bool) {
    set_radio(RadioKind::Bluetooth, on);
}

fn set_radio(kind: RadioKind, on: bool) {
    if let Some(radio) = radio(kind)
        && let Ok(operation) = radio.SetStateAsync(if on { RadioState::On } else { RadioState::Off })
    {
        let _ = operation.join();
    }
}

fn device_names(selector: windows::core::Result<HSTRING>) -> Vec<String> {
    let Ok(selector) = selector else { return Vec::new() };
    let Ok(found) = DeviceInformation::FindAllAsyncAqsFilter(&selector).and_then(|op| op.join()) else {
        return Vec::new();
    };
    let size = found.Size().unwrap_or(0);
    (0..size)
        .filter_map(|i| found.GetAt(i).ok())
        .filter_map(|device| device.Name().ok().map(|name| name.to_string()))
        .filter(|name| !name.is_empty())
        .collect()
}

pub fn bluetooth() -> BluetoothInfo {
    let powered = radio(RadioKind::Bluetooth).and_then(|r| r.State().ok()).map(|state| state == RadioState::On);
    let mut connected: HashSet<String> = HashSet::new();
    if powered == Some(true) {
        connected.extend(device_names(WinBluetoothDevice::GetDeviceSelectorFromConnectionStatus(
            BluetoothConnectionStatus::Connected,
        )));
        connected.extend(device_names(BluetoothLEDevice::GetDeviceSelectorFromConnectionStatus(
            BluetoothConnectionStatus::Connected,
        )));
    }
    let mut devices: Vec<BluetoothDevice> =
        connected.into_iter().map(|name| BluetoothDevice { name, connected: true }).collect();
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    BluetoothInfo { powered, devices }
}

// ---------------------------------------------------------------------------------------------------------------------
// Apps and system commands
// ---------------------------------------------------------------------------------------------------------------------

fn collect_shortcuts(dir: &Path, apps: &mut Vec<AppEntry>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_shortcuts(&path, apps);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) {
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let lower = name.to_lowercase();
            if lower.contains("uninstall") || lower.contains("deinstall") {
                continue;
            }
            apps.push(AppEntry { name, launch: path.to_string_lossy().into_owned(), icon: Some(path) });
        }
    }
}

pub fn installed_apps() -> Vec<AppEntry> {
    let mut apps = Vec::new();
    if let Ok(data) = std::env::var("ProgramData") {
        collect_shortcuts(&Path::new(&data).join(r"Microsoft\Windows\Start Menu\Programs"), &mut apps);
    }
    if let Ok(data) = std::env::var("APPDATA") {
        collect_shortcuts(&Path::new(&data).join(r"Microsoft\Windows\Start Menu\Programs"), &mut apps);
    }
    // Store apps have no shortcut on disk, only the shell knows them.
    if let Some(output) = capture(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-StartApps | ConvertTo-Json -Compress"],
    ) && let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim())
    {
        let known: HashSet<String> = apps.iter().map(|a| a.name.to_lowercase()).collect();
        let items = value.as_array().cloned().unwrap_or_else(|| vec![value.clone()]);
        for item in items {
            let (Some(name), Some(id)) = (item["Name"].as_str(), item["AppID"].as_str()) else { continue };
            if !known.contains(&name.to_lowercase()) && !name.to_lowercase().contains("uninstall") {
                apps.push(AppEntry { name: name.to_string(), launch: format!("shell:AppsFolder\\{id}"), icon: None });
            }
        }
    }
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    apps
}

pub fn launch_app(app: &AppEntry) -> bool {
    if app.launch.starts_with("shell:") {
        return spawn_detached("explorer.exe", &[&app.launch]);
    }
    open::that_detached(&app.launch).is_ok()
}

pub fn run_system_command(id: &str) -> bool {
    unsafe {
        match id {
            "lock" => LockWorkStation().is_ok(),
            "sleep" => SetSuspendState(false, false, false),
            "restart" => spawn_detached("shutdown", &["/r", "/t", "0"]),
            "shutdown" => spawn_detached("shutdown", &["/s", "/t", "0"]),
            "logout" => spawn_detached("shutdown", &["/l"]),
            "trash" => SHEmptyRecycleBinW(None, PCWSTR::null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND).is_ok(),
            "settings" => open::that_detached("ms-settings:").is_ok(),
            _ => false,
        }
    }
}

/// The icon of a file or shortcut as RGBA pixels, plus its side length.
pub fn file_icon(path: &Path) -> Option<(Vec<u8>, u32)> {
    unsafe {
        let wide_path = wide(&path.to_string_lossy());
        let mut info = SHFILEINFOW::default();
        let found = SHGetFileInfoW(
            PCWSTR(wide_path.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if found == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let pixels = icon_pixels(info.hIcon);
        let _ = DestroyIcon(info.hIcon);
        pixels
    }
}

unsafe fn icon_pixels(icon: HICON) -> Option<(Vec<u8>, u32)> {
    unsafe {
        let mut icon_info = ICONINFO::default();
        GetIconInfo(icon, &mut icon_info).ok()?;
        let cleanup = |info: &ICONINFO| {
            let _ = DeleteObject(info.hbmColor.into());
            let _ = DeleteObject(info.hbmMask.into());
        };
        let mut bitmap = BITMAP::default();
        let read = GetObjectW(icon_info.hbmColor.into(), size_of::<BITMAP>() as i32, Some((&mut bitmap as *mut BITMAP).cast()));
        if read == 0 || bitmap.bmWidth <= 0 || bitmap.bmHeight <= 0 {
            cleanup(&icon_info);
            return None;
        }
        let side = bitmap.bmWidth as u32;
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: bitmap.bmWidth,
                biHeight: -bitmap.bmHeight,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (side * bitmap.bmHeight as u32 * 4) as usize];
        let dc = GetDC(None);
        let lines = GetDIBits(
            dc,
            icon_info.hbmColor,
            0,
            bitmap.bmHeight as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut header,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        cleanup(&icon_info);
        if lines == 0 {
            return None;
        }
        let has_alpha = pixels.chunks_exact(4).any(|p| p[3] != 0);
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            if !has_alpha {
                pixel[3] = 255;
            }
        }
        Some((pixels, side))
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Files, autostart, secrets, input
// ---------------------------------------------------------------------------------------------------------------------

pub fn reveal_path(path: &Path) {
    spawn_detached("explorer.exe", &[&format!("/select,{}", path.display())]);
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn set_autostart(enabled: bool) -> bool {
    let Some(key) = RegKey::open(HKEY_CURRENT_USER, RUN_KEY, KEY_SET_VALUE) else { return false };
    let name = wide("V-Notch");
    unsafe {
        if enabled {
            let Ok(exe) = std::env::current_exe() else { return false };
            let value = wide(&format!("\"{}\"", exe.display()));
            let bytes = std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2);
            RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)) == ERROR_SUCCESS
        } else {
            let status = RegDeleteValueW(key.0, PCWSTR(name.as_ptr()));
            status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND
        }
    }
}

pub fn is_autostart_enabled() -> bool {
    RegKey::open(HKEY_CURRENT_USER, RUN_KEY, KEY_READ).is_some_and(|key| key.read_string("V-Notch").is_some())
}

fn secrets_path() -> std::path::PathBuf {
    crate::paths::config_dir().join("secrets.json")
}

fn read_secrets() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(secrets_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn dpapi(input: &[u8], protect: bool) -> Option<Vec<u8>> {
    unsafe {
        let blob = CRYPT_INTEGER_BLOB { cbData: input.len() as u32, pbData: input.as_ptr().cast_mut() };
        let mut output = CRYPT_INTEGER_BLOB::default();
        if protect {
            CryptProtectData(&blob, PCWSTR::null(), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output).ok()?;
        } else {
            CryptUnprotectData(&blob, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output).ok()?;
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Some(bytes)
    }
}

/// Stored encrypted with DPAPI, readable by this Windows account only.
pub fn secret_get(name: &str) -> Option<String> {
    let secrets = read_secrets();
    let hex = secrets.get(name)?.as_str()?;
    let bytes: Vec<u8> = (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()).collect();
    String::from_utf8(dpapi(&bytes, false)?).ok()
}

pub fn secret_set(name: &str, value: &str) -> bool {
    let mut secrets = read_secrets();
    if value.is_empty() {
        secrets.remove(name);
    } else {
        let Some(encrypted) = dpapi(value.as_bytes(), true) else { return false };
        let hex: String = encrypted.iter().map(|b| format!("{b:02x}")).collect();
        secrets.insert(name.to_string(), hex.into());
    }
    serde_json::to_string(&secrets).ok().is_some_and(|text| std::fs::write(secrets_path(), text).is_ok())
}

fn key_event(key: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: key, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, ..Default::default() },
        },
    }
}

pub fn clipboard_change_counter() -> Option<u64> {
    Some(u64::from(unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() }))
}

/// Presses Ctrl+C for the active app, so its selection lands on the clipboard.
pub fn copy_selection() {
    let events = [
        key_event(VK_CONTROL, false),
        key_event(VK_C, false),
        key_event(VK_C, true),
        key_event(VK_CONTROL, true),
    ];
    unsafe {
        SendInput(&events, size_of::<INPUT>() as i32);
    }
}

/// Starts dragging files out of the notch. Blocks until the drop on Windows.
pub fn start_file_drag(window: &impl HasWindowHandle, paths: Vec<PathBuf>) -> bool {
    let icon = image_for_drag();
    drag::start_drag(
        window,
        drag::DragItem::Files(paths),
        drag::Image::Raw(icon),
        |_, _| {},
        drag::Options::default(),
    )
    .is_ok()
}

fn image_for_drag() -> Vec<u8> {
    include_bytes!("../../assets/logo.png").to_vec()
}

// ---------------------------------------------------------------------------------------------------------------------
// Lenovo Legion
// ---------------------------------------------------------------------------------------------------------------------

/// Read from the BIOS strings Windows keeps; needs neither WMI nor admin rights.
pub fn is_legion() -> bool {
    RegKey::open(HKEY_LOCAL_MACHINE, r"HARDWARE\DESCRIPTION\System\BIOS", KEY_READ).is_some_and(|key| {
        ["SystemFamily", "SystemVersion", "SystemProductName"].iter().any(|name| {
            key.read_string(name).is_some_and(|value| value.to_lowercase().contains("legion"))
        })
    })
}

/// Sends one request line to the elevated helper and returns its one answer line.
pub fn legion_request(request: &str) -> Option<String> {
    use std::io::{BufRead, BufReader, Write};
    let mut pipe = std::fs::OpenOptions::new().read(true).write(true).open(r"\\.\pipe\vnotch-legion").ok()?;
    pipe.write_all(format!("{request}\n").as_bytes()).ok()?;
    let mut answer = String::new();
    BufReader::new(pipe).read_line(&mut answer).ok()?;
    Some(answer)
}

/// Runs the one-time setup elevated. The scripts ship inside the binary and are unpacked first.
pub fn legion_setup() -> bool {
    let dir = crate::paths::data_dir().join("legion");
    let _ = std::fs::create_dir_all(&dir);
    let helper = dir.join("LegionHelper.ps1");
    let setup = dir.join("LegionSetup.ps1");
    if std::fs::write(&helper, include_str!("../../assets/legion/LegionHelper.ps1")).is_err()
        || std::fs::write(&setup, include_str!("../../assets/legion/LegionSetup.ps1")).is_err()
    {
        return false;
    }
    let Some(sid) = current_user_sid() else { return false };
    let arguments = format!(
        "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\" -User {sid}",
        setup.display()
    );
    shell_execute_runas("powershell.exe", &arguments)
}

fn current_user_sid() -> Option<String> {
    let output = capture(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value"],
    )?;
    let sid = output.trim().to_string();
    (sid.starts_with("S-1-") && sid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).then_some(sid)
}

fn shell_execute_runas(file: &str, parameters: &str) -> bool {
    use windows::Win32::UI::Shell::ShellExecuteW;
    let (verb, file, parameters) = (wide("runas"), wide(file), wide(parameters));
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(parameters.as_ptr()),
            PCWSTR::null(),
            SW_HIDE,
        )
    };
    // Values above 32 mean success, the rest are error codes such as a declined UAC prompt.
    result.0 as isize > 32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_owner_must_still_run() {
        let processes = vec![("discord.exe".to_string(), 10), ("soundrec.exe".to_string(), 20), ("explorer.exe".to_string(), 30)];
        let path_of = |pid: u32| match pid {
            10 => Some(r"C:\Users\me\AppData\Local\Discord\app-1.0.9200\Discord.exe".to_string()),
            20 => Some(r"C:\Program Files\WindowsApps\Microsoft.WindowsSoundRecorder_11.2.0.0_x64__8wekyb3d8bbwe\SoundRec.exe".to_string()),
            _ => Some(r"C:\Windows\explorer.exe".to_string()),
        };
        // A crashed old Discord version stays "in use" in the registry, but it no longer runs.
        assert!(!consent_owner_running(r"C:#Users#me#AppData#Local#Discord#app-1.0.9189#Discord.exe", false, &processes, path_of));
        assert!(consent_owner_running(r"C:#Users#me#AppData#Local#Discord#app-1.0.9200#Discord.exe", false, &processes, path_of));
        assert!(consent_owner_running("Microsoft.WindowsSoundRecorder_8wekyb3d8bbwe", true, &processes, path_of));
        assert!(!consent_owner_running("Microsoft.WindowsCamera_8wekyb3d8bbwe", true, &processes, path_of));
    }

    #[test]
    fn desktop_apps_only_open_processes_with_the_same_exe_name() {
        let processes = vec![("discord.exe".to_string(), 10), ("explorer.exe".to_string(), 30)];
        let opened = std::cell::RefCell::new(Vec::new());
        let path_of = |pid: u32| {
            opened.borrow_mut().push(pid);
            None
        };
        consent_owner_running(r"C:#Apps#Discord.exe", false, &processes, path_of);
        assert_eq!(*opened.borrow(), vec![10]);
    }

    #[test]
    fn this_test_shows_up_in_the_process_list() {
        let me = std::env::current_exe().unwrap();
        let name = me.file_name().unwrap().to_string_lossy().to_lowercase();
        let list = process_list();
        let pid = std::process::id();
        assert!(list.iter().any(|(n, p)| *n == name && *p == pid));
        assert_eq!(process_path(pid).map(|p| p.to_lowercase()), Some(me.to_string_lossy().to_lowercase()));
    }

    #[test]
    fn consent_keys_become_readable_names() {
        assert_eq!(consent_key_name("C:#Program Files#Zoom#bin#Zoom.exe"), "Zoom");
        assert_eq!(consent_key_name("Microsoft.WindowsCamera_8wekyb3d8bbwe"), "WindowsCamera");
    }

    #[test]
    fn app_ids_become_readable_names() {
        assert_eq!(friendly_app_name("Spotify.exe"), "Spotify");
        assert_eq!(friendly_app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "Media Player");
        assert_eq!(friendly_app_name("foo.exe"), "foo");
    }

    #[test]
    fn dpapi_round_trip() {
        let secret = b"hello";
        let encrypted = dpapi(secret, true).unwrap();
        assert_ne!(encrypted, secret);
        assert_eq!(dpapi(&encrypted, false).unwrap(), secret);
    }
}
