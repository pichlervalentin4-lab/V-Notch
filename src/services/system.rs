//! System stats, battery, network, Bluetooth, privacy indicators and fullscreen detection.

use super::{Shared, Toast, push_toast};
use crate::platform::{self, BluetoothInfo, FullscreenKind, NetworkInfo, PrivacyInfo};
use crate::settings::SharedSettings;
use eframe::egui;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatteryInfo {
    pub percent: f32,
    pub charging: bool,
    pub full: bool,
    pub time_left: Option<Duration>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcessLoad {
    pub name: String,
    pub cpu: f32,
    pub memory: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuInfo {
    pub usage: f32,
    pub temperature: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemState {
    pub cpu: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub net_down: u64,
    pub net_up: u64,
    pub gpu: Option<GpuInfo>,
    pub top_cpu: Vec<ProcessLoad>,
    pub top_memory: Vec<ProcessLoad>,
    pub battery: Option<BatteryInfo>,
    pub network: NetworkInfo,
    pub bluetooth: BluetoothInfo,
    pub privacy: PrivacyInfo,
    pub fullscreen: FullscreenKind,
    pub idle_seconds: f32,
    /// Round trip to 1.1.1.1 in milliseconds, like the original's ping tile (a TCP handshake here,
    /// which needs no administrator rights the way ICMP sockets do).
    pub ping_ms: Option<f32>,
}

/// The host the ping tile measures, shown under its value.
pub const PING_HOST: &str = "1.1.1.1";

fn measure_ping() -> Option<f32> {
    let address = std::net::SocketAddr::from(([1, 1, 1, 1], 443));
    let started = Instant::now();
    std::net::TcpStream::connect_timeout(&address, Duration::from_millis(1500)).ok()?;
    Some(started.elapsed().as_secs_f32() * 1000.0)
}

pub struct SystemService {
    pub state: Mutex<SystemState>,
    /// Set while the controls tab is open: then the expensive numbers are worth collecting.
    pub details_wanted: AtomicBool,
    /// Set after a radio or device was switched: network and Bluetooth are read again right away.
    pub reread_now: AtomicBool,
    /// The Bluetooth switch: one switch at a time, and the last wish wins.
    pub bluetooth_wish: Mutex<RadioWish>,
}

/// How long the switch shows the wished state after the radio was set, until Windows reports it.
const SETTLE: Duration = Duration::from_secs(4);
/// The shortest time between two real switches of the radio.
const MIN_GAP: Duration = Duration::from_millis(2500);
/// Readings without a radio that are taken for a radio in the middle of switching.
const MAX_MISSES: u8 = 3;

/// Clicks on a radio switch while it is still switching: only one switch runs, and what the user
/// clicked last is applied after it. Keeps the wished state for the switch to show meanwhile.
#[derive(Debug, Default)]
pub struct RadioWish {
    pending: Option<bool>,
    running: bool,
    target: Option<bool>,
    settled_at: Option<Instant>,
}

impl RadioWish {
    /// Records a wish. True when no switch is running, so the caller starts one.
    pub fn want(&mut self, on: bool) -> bool {
        self.pending = Some(on);
        self.target = Some(on);
        self.settled_at = None;
        !std::mem::replace(&mut self.running, true)
    }

    /// The next state to apply; `None` ends the running switch.
    pub fn next(&mut self, now: Instant) -> Option<bool> {
        let next = self.pending.take();
        if next.is_none() {
            self.running = false;
            self.settled_at = Some(now);
        }
        next
    }

    /// While switching or settling, the state the switch shows instead of the last reading.
    pub fn shown(&self, now: Instant) -> Option<bool> {
        self.busy(now).then_some(self.target).flatten()
    }

    pub fn busy(&self, now: Instant) -> bool {
        self.running || self.settled_at.is_some_and(|at| now.duration_since(at) < SETTLE)
    }
}

/// A reading without a radio right after it was there is Windows in the middle of switching: the
/// last good reading stays for a few readings (and all the time while switching).
fn steady_bluetooth(previous: &BluetoothInfo, read: BluetoothInfo, switching: bool, misses: &mut u8) -> BluetoothInfo {
    if read.powered.is_none() && previous.powered.is_some() && (switching || *misses < MAX_MISSES) {
        *misses = misses.saturating_add(1);
        return previous.clone();
    }
    *misses = 0;
    read
}

fn read_battery() -> Option<BatteryInfo> {
    let manager = starship_battery::Manager::new().ok()?;
    let battery = manager.batteries().ok()?.next()?.ok()?;
    let state = battery.state();
    Some(BatteryInfo {
        percent: battery.state_of_charge().value * 100.0,
        charging: matches!(state, starship_battery::State::Charging),
        full: matches!(state, starship_battery::State::Full),
        time_left: battery
            .time_to_empty()
            .map(|time| Duration::from_secs_f32(time.value.max(0.0))),
    })
}

fn read_gpu() -> Option<GpuInfo> {
    // Works for NVIDIA on every OS; other GPUs simply show no value.
    if let Some(output) = platform::capture(
        "nvidia-smi",
        &["--query-gpu=utilization.gpu,temperature.gpu", "--format=csv,noheader,nounits"],
    ) {
        let line = output.lines().next()?;
        let (usage, temperature) = line.split_once(',')?;
        return Some(GpuInfo { usage: usage.trim().parse().ok()?, temperature: temperature.trim().parse().ok() });
    }
    #[cfg(target_os = "linux")]
    for card in 0..4 {
        if let Ok(text) = std::fs::read_to_string(format!("/sys/class/drm/card{card}/device/gpu_busy_percent")) {
            return Some(GpuInfo { usage: text.trim().parse().ok()?, temperature: None });
        }
    }
    None
}

fn top_processes(system: &System) -> (Vec<ProcessLoad>, Vec<ProcessLoad>) {
    let mut merged: std::collections::HashMap<String, ProcessLoad> = std::collections::HashMap::new();
    let cores = system.cpus().len().max(1) as f32;
    for process in system.processes().values() {
        let name = process.name().to_string_lossy().trim_end_matches(".exe").to_string();
        let entry = merged.entry(name.clone()).or_insert(ProcessLoad { name, cpu: 0.0, memory: 0 });
        entry.cpu += process.cpu_usage() / cores;
        entry.memory += process.memory();
    }
    let mut all: Vec<ProcessLoad> = merged.into_values().collect();
    all.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
    let by_cpu = all.iter().take(4).cloned().collect();
    all.sort_by_key(|p| std::cmp::Reverse(p.memory));
    let by_memory = all.into_iter().take(4).collect();
    (by_cpu, by_memory)
}

impl SystemService {
    pub fn start(ctx: egui::Context, settings: SharedSettings, toasts: Shared<VecDeque<Toast>>, demo: bool) -> Arc<Self> {
        let service = Arc::new(Self { state: Mutex::new(SystemState::default()), details_wanted: AtomicBool::new(false),
            reread_now: AtomicBool::new(false),
            bluetooth_wish: Mutex::new(RadioWish::default()),
        });
        if demo {
            service.demo_fill();
            return service;
        }
        let worker = service.clone();
        let _ = std::thread::Builder::new().name("system".into()).spawn(move || worker.run(&ctx, &settings, &toasts));
        service
    }

    fn run(&self, ctx: &egui::Context, settings: &SharedSettings, toasts: &Shared<VecDeque<Toast>>) {
        let refresh = RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing().with_usage())
            .with_memory(MemoryRefreshKind::nothing().with_ram());
        let Ok(mut system) = System::new_with_specifics(refresh) else { return };
        let Ok(mut networks) = Networks::new_with_refreshed_list() else { return };
        let mut last_tick = Instant::now();
        let mut last_slow = Instant::now() - Duration::from_secs(60);
        let mut last_battery: Option<BatteryInfo> = None;
        let mut last_devices: Option<Vec<String>> = None;
        let mut bt_misses = 0_u8;
        let mut ticks = 0_u32;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            ticks = ticks.wrapping_add(1);
            let details = self.details_wanted.load(Ordering::Relaxed);
            let mut next = self.state.lock().map(|s| s.clone()).unwrap_or_default();

            next.fullscreen = platform::fullscreen_state();
            next.idle_seconds = platform::idle_seconds();
            let privacy_on = settings.read(|s| s.enable_privacy_indicators);
            next.privacy = if privacy_on { platform::privacy() } else { PrivacyInfo::default() };

            if details {
                system.refresh_specifics(refresh);
                next.cpu = system.global_cpu_usage();
                next.memory_used = system.used_memory();
                next.memory_total = system.total_memory();
                networks.refresh(true);
                let elapsed = last_tick.elapsed().as_secs_f64().max(0.1);
                let (mut down, mut up) = (0_u64, 0_u64);
                for (_, data) in &networks {
                    down += data.received();
                    up += data.transmitted();
                }
                next.net_down = (down as f64 / elapsed) as u64;
                next.net_up = (up as f64 / elapsed) as u64;
                if ticks % 3 == 0 {
                    system.refresh_processes_specifics(
                        ProcessesToUpdate::All,
                        true,
                        ProcessRefreshKind::nothing().with_cpu().with_memory(),
                    );
                    (next.top_cpu, next.top_memory) = top_processes(&system);
                    next.gpu = read_gpu();
                    next.ping_ms = measure_ping();
                }
            }
            last_tick = Instant::now();

            let slow_every = if details { 5 } else { 30 };
            if last_slow.elapsed() > Duration::from_secs(slow_every) || self.reread_now.swap(false, Ordering::Relaxed) {
                last_slow = Instant::now();
                next.battery = read_battery();
                next.network = platform::network();
                let read = platform::bluetooth();
                let switching = self.bluetooth_wish.lock().is_ok_and(|w| w.busy(Instant::now()));
                next.bluetooth = steady_bluetooth(&next.bluetooth, read, switching, &mut bt_misses);

                let notify = settings.read(|s| s.show_system_notifications);
                if notify && let (Some(now), Some(before)) = (&next.battery, &last_battery)
                    && now.charging != before.charging
                {
                    push_toast(toasts, Toast::Battery { percent: now.percent, charging: now.charging });
                }
                last_battery.clone_from(&next.battery);

                let connected: Vec<String> =
                    next.bluetooth.devices.iter().filter(|d| d.connected).map(|d| d.name.clone()).collect();
                if notify && let Some(before) = &last_devices {
                    for name in connected.iter().filter(|name| !before.contains(name)) {
                        push_toast(toasts, Toast::Bluetooth { name: name.clone(), connected: true });
                    }
                    for name in before.iter().filter(|name| !connected.contains(name)) {
                        push_toast(toasts, Toast::Bluetooth { name: name.clone(), connected: false });
                    }
                }
                last_devices = Some(connected);
            }

            if let Ok(mut state) = self.state.lock() {
                // The idle time grows every second; the notch picks it up on its own once-a-second
                // frame, so only other changes need an extra one.
                state.idle_seconds = next.idle_seconds;
                let changed = *state != next;
                *state = next;
                if changed {
                    ctx.request_repaint();
                }
            }
        }
    }

    /// Switches Bluetooth on or off. Clicks while a switch runs only change what is applied next,
    /// so fast clicking ends in the state clicked last instead of a pile of half-done switches.
    pub fn switch_bluetooth(self: &Arc<Self>, on: bool) {
        let start = self.bluetooth_wish.lock().is_ok_and(|mut wish| wish.want(on));
        if !start {
            return;
        }
        let service = self.clone();
        let _ = std::thread::Builder::new().name("bluetooth-switch".into()).spawn(move || {
            let mut last: Option<Instant> = None;
            loop {
                // The driver needs a breather between two switches; clicks meanwhile only change the wish.
                if let Some(at) = last {
                    std::thread::sleep(MIN_GAP.saturating_sub(at.elapsed()));
                }
                let Some(on) = service.bluetooth_wish.lock().ok().and_then(|mut wish| wish.next(Instant::now())) else { break };
                platform::set_bluetooth(on);
                last = Some(Instant::now());
            }
            // Windows takes a moment to report the new state: read again a few times.
            for wait in [300, 700, 1500, 2500] {
                std::thread::sleep(Duration::from_millis(wait));
                service.refresh_soon();
            }
        });
    }

    /// Reads network and Bluetooth again on the next tick, e.g. after a switch.
    pub fn refresh_soon(&self) {
        self.reread_now.store(true, Ordering::Relaxed);
    }

    fn demo_fill(&self) {
        use crate::platform::{BluetoothDevice, NetworkKind};
        let load = |name: &str, cpu: f32, memory: u64| ProcessLoad { name: name.to_string(), cpu, memory };
        if let Ok(mut state) = self.state.lock() {
            *state = SystemState {
                cpu: 23.0,
                memory_used: 11 * 1024 * 1024 * 1024,
                memory_total: 32 * 1024 * 1024 * 1024,
                net_down: 2_400_000,
                net_up: 180_000,
                gpu: Some(GpuInfo { usage: 41.0, temperature: Some(58.0) }),
                top_cpu: vec![load("v-notch", 3.1, 90 << 20), load("chrome", 2.2, 1400 << 20), load("cargo", 1.4, 300 << 20)],
                top_memory: vec![load("chrome", 2.2, 1400 << 20), load("code", 0.8, 900 << 20), load("v-notch", 3.1, 90 << 20)],
                battery: Some(BatteryInfo { percent: 76.0, charging: false, full: false, time_left: Some(Duration::from_secs(9000)) }),
                network: NetworkInfo { kind: NetworkKind::Wifi, name: "Heimnetz".into(), wifi_on: Some(true) },
                bluetooth: BluetoothInfo {
                    powered: Some(true),
                    devices: vec![
                        BluetoothDevice { name: "Kopfhörer".into(), connected: true, audio: true },
                        BluetoothDevice { name: "iPhone".into(), connected: false, audio: false },
                    ],
                },
                privacy: PrivacyInfo { mic: vec!["Discord".into()], camera: Vec::new(), camera_detectable: true },
                fullscreen: FullscreenKind::None,
                idle_seconds: 0.0,
                ping_ms: Some(16.0),
            };
        }
    }
}

/// "2.4 MB/s"
pub fn format_rate(bytes_per_second: u64) -> String {
    format!("{}/s", format_bytes(bytes_per_second))
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_readable() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_rate(1024), "1.0 KB/s");
    }

    #[test]
    fn fast_clicks_end_in_the_last_wish() {
        let t0 = Instant::now();
        let mut wish = RadioWish::default();
        assert!(wish.want(false), "the first click starts a switch");
        assert_eq!(wish.next(t0), Some(false));
        // Three more clicks while the radio is still switching: none starts a second switch.
        assert!(!wish.want(true));
        assert!(!wish.want(false));
        assert!(!wish.want(true));
        assert_eq!(wish.shown(t0), Some(true));
        assert_eq!(wish.next(t0), Some(true), "only the last wish is applied");
        assert_eq!(wish.next(t0), None);
        assert!(wish.busy(t0), "the switch shows the wish until Windows reports it");
        assert_eq!(wish.shown(t0 + Duration::from_secs(1)), Some(true));
        assert_eq!(wish.shown(t0 + SETTLE + Duration::from_millis(1)), None);
        assert!(wish.want(false), "after it ended, a click starts a new switch");
    }

    #[test]
    fn a_missing_radio_while_switching_is_not_unavailable() {
        let on = BluetoothInfo { powered: Some(true), devices: Vec::new() };
        let gone = BluetoothInfo::default();
        let mut misses = 0;
        assert_eq!(steady_bluetooth(&on, gone.clone(), false, &mut misses), on);
        assert_eq!(steady_bluetooth(&on, gone.clone(), false, &mut misses), on);
        assert_eq!(steady_bluetooth(&on, gone.clone(), false, &mut misses), on);
        assert_eq!(steady_bluetooth(&on, gone.clone(), false, &mut misses), gone, "gone for good after three readings");
        misses = 9;
        assert_eq!(steady_bluetooth(&on, gone.clone(), true, &mut misses), on, "never while switching");
        let off = BluetoothInfo { powered: Some(false), devices: Vec::new() };
        assert_eq!(steady_bluetooth(&on, off.clone(), false, &mut misses), off);
        assert_eq!(misses, 0);
    }
}
