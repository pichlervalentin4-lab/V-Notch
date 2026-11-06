//! The controls tab: quick tiles, system stats, timer and ambient noise.

use super::kit::{self, rounded, text_clipped};
use super::{Notch, battery_icon, volume_icon};
use crate::i18n::t;
use crate::icons;
use crate::platform::{self, NetworkKind};
use crate::services::audio::AudioCommand;
use crate::services::sounds::{NoiseKind, Tone};
use crate::services::system::{format_bytes, format_rate};
use crate::services::timer::{self, TimerState};
use crate::settings::Settings;
use crate::theme;
use eframe::egui::{Align2, Color32, Rect, Sense, Ui, pos2, vec2};
use std::time::Instant;

pub struct ControlsUi {
    noise: Option<NoiseKind>,
    noise_volume: f32,
    top_by_memory: bool,
}

impl Default for ControlsUi {
    fn default() -> Self {
        Self { noise: None, noise_volume: 0.4, top_by_memory: false }
    }
}

struct Tile<'a> {
    icon: &'a str,
    title: String,
    subtitle: String,
    on: bool,
    color: Color32,
}

impl Notch {
    pub(super) fn controls_view(&mut self, ui: &mut Ui, body: Rect, _settings: &Settings) {
        let gap = 8.0;
        let tiles_row = Rect::from_min_size(body.min, vec2(body.width(), 62.0));
        self.quick_tiles(ui, tiles_row, gap);

        let middle_top = tiles_row.bottom() + gap;
        let middle = Rect::from_min_size(pos2(body.left(), middle_top), vec2(body.width(), 152.0));
        let parts = kit::columns(middle, 2, gap);
        self.stats_card(ui, parts[0]);
        self.processes_card(ui, parts[1]);

        let bottom = Rect::from_min_max(pos2(body.left(), middle.bottom() + gap), body.max);
        let parts = kit::columns(bottom, 2, gap);
        self.timer_card(ui, parts[0]);
        self.noise_card(ui, parts[1]);
    }

    fn tile(&mut self, ui: &mut Ui, rect: Rect, tile: &Tile<'_>, id: &str) -> bool {
        let response = ui.interact(rect, ui.id().with(id), Sense::click());
        let fill = if response.hovered() { theme::CARD_HOVER } else { theme::CARD };
        ui.painter().rect_filled(rect, rounded(14.0), fill);
        let circle = rect.left_center() + vec2(26.0, 0.0);
        let color = if tile.on { tile.color } else { theme::alpha(Color32::WHITE, 0.15) };
        ui.painter().circle_filled(circle, 16.0, color);
        let tint = if tile.on { Color32::WHITE } else { theme::DIM };
        ui.painter().text(circle, Align2::CENTER_CENTER, tile.icon, theme::icon_fill(16.0), tint);
        let left = rect.left() + 50.0;
        let width = rect.right() - left - 8.0;
        text_clipped(ui, pos2(left, rect.center().y - 8.0), Align2::LEFT_CENTER, &tile.title, theme::semibold(12.5), theme::WHITE, width);
        text_clipped(ui, pos2(left, rect.center().y + 9.0), Align2::LEFT_CENTER, &tile.subtitle, theme::regular(10.5), theme::DIM, width);
        response.clicked()
    }

    fn quick_tiles(&mut self, ui: &mut Ui, row: Rect, gap: f32) {
        let parts = kit::columns(row, 4, gap);
        let (network, bluetooth) = self.hub.system.state.lock().map(|s| (s.network.clone(), s.bluetooth.clone())).unwrap_or_default();
        let (master, mic) = self.hub.audio.state.lock().map(|s| (s.master, s.mic)).unwrap_or_default();

        // Wi-Fi
        let wifi_on = network.wifi_on.unwrap_or(network.kind == NetworkKind::Wifi);
        let (icon, title, subtitle) = match network.kind {
            NetworkKind::Wired => (icons::GLOBE, t("control.wired"), t("control.connected")),
            NetworkKind::Wifi => {
                let name = if network.name.is_empty() { t("control.connected") } else { network.name.clone() };
                (icons::WIFI_HIGH, t("control.wifi"), name)
            }
            NetworkKind::Offline => (
                if wifi_on { icons::WIFI_HIGH } else { icons::WIFI_SLASH },
                t("control.wifi"),
                t(if network.wifi_on == Some(false) { "control.off" } else { "control.notConnected" }),
            ),
        };
        let tile = Tile { icon, title, subtitle, on: wifi_on || network.kind == NetworkKind::Wired, color: theme::ACCENT };
        if self.tile(ui, parts[0], &tile, "tile-wifi") && network.wifi_on.is_some() && !self.hub.demo {
            spawn_toggle(move || platform::set_wifi(!wifi_on));
            self.hub.play(Tone::Tap);
        }

        // Bluetooth
        let bt_on = bluetooth.powered.unwrap_or(false);
        let subtitle = match (bluetooth.powered, bluetooth.devices.first()) {
            (None, _) => t("control.noAdapter"),
            (Some(false), _) => t("control.off"),
            (Some(true), Some(device)) => device.name.clone(),
            (Some(true), None) => t("control.on"),
        };
        let tile = Tile { icon: icons::BLUETOOTH, title: "Bluetooth".into(), subtitle, on: bt_on, color: theme::ACCENT };
        if self.tile(ui, parts[1], &tile, "tile-bt") && bluetooth.powered.is_some() && !self.hub.demo {
            spawn_toggle(move || platform::set_bluetooth(!bt_on));
            self.hub.play(Tone::Tap);
        }

        // Sound
        let (level, muted) = master.map_or((0.0, true), |m| (m.level, m.muted));
        let subtitle = if muted { t("control.microphone.muted") } else { format!("{:.0} %", level * 100.0) };
        let tile = Tile { icon: volume_icon(level, muted), title: t("control.audio"), subtitle, on: !muted, color: theme::ORANGE };
        if self.tile(ui, parts[2], &tile, "tile-sound") {
            self.hub.audio.send(AudioCommand::SetMasterMute(!muted));
            self.hub.play(Tone::Tap);
        }

        // Microphone
        let mic_muted = mic.is_none_or(|m| m.muted);
        let subtitle = t(if mic.is_none() {
            "control.noAdapter"
        } else if mic_muted {
            "control.microphone.muted"
        } else {
            "control.on"
        });
        let icon = if mic_muted { icons::MICROPHONE_SLASH } else { icons::MICROPHONE };
        let tile = Tile { icon, title: t("control.microphone"), subtitle, on: !mic_muted, color: theme::RED };
        if self.tile(ui, parts[3], &tile, "tile-mic") && mic.is_some() {
            self.hub.audio.send(AudioCommand::SetMicMute(!mic_muted));
            self.hub.play(Tone::Tap);
        }
    }

    fn stats_card(&mut self, ui: &mut Ui, card: Rect) {
        kit::card(ui, card, 14.0, theme::CARD);
        let state = self.hub.system.state.lock().map(|s| s.clone()).unwrap_or_default();
        let left = card.left() + 14.0;
        let right = card.right() - 14.0;
        let mut y = card.top() + 15.0;
        text_clipped(ui, pos2(left, y), Align2::LEFT_CENTER, &t("control.system"), theme::semibold(12.0), theme::DIM, 120.0);
        y += 18.0;
        let bar = |ui: &Ui, y: f32, label: &str, ratio: f32, value: &str, color: Color32| {
            ui.painter().text(pos2(left, y), Align2::LEFT_CENTER, label, theme::medium(11.0), theme::DIM);
            ui.painter().text(pos2(right, y), Align2::RIGHT_CENTER, value, theme::medium(11.0), theme::WHITE);
            let track = Rect::from_min_max(pos2(left + 44.0, y - 2.5), pos2(right - 88.0, y + 2.5));
            ui.painter().rect_filled(track, rounded(2.5), theme::alpha(Color32::WHITE, 0.12));
            let width = track.width() * ratio.clamp(0.0, 1.0);
            if width > 0.5 {
                ui.painter().rect_filled(Rect::from_min_size(track.min, vec2(width.max(5.0), track.height())), rounded(2.5), color);
            }
        };
        bar(ui, y, "CPU", state.cpu / 100.0, &format!("{:.0} %", state.cpu), theme::ACCENT);
        y += 21.0;
        let memory_ratio = if state.memory_total > 0 { state.memory_used as f32 / state.memory_total as f32 } else { 0.0 };
        let memory_text = format!("{} / {}", format_bytes(state.memory_used), format_bytes(state.memory_total));
        bar(ui, y, "RAM", memory_ratio, &memory_text, theme::GREEN);
        y += 21.0;
        if let Some(gpu) = &state.gpu {
            let text = match gpu.temperature {
                Some(temperature) => format!("{:.0} % · {temperature:.0}°", gpu.usage),
                None => format!("{:.0} %", gpu.usage),
            };
            bar(ui, y, "GPU", gpu.usage / 100.0, &text, theme::ORANGE);
        } else {
            ui.painter().text(pos2(left, y), Align2::LEFT_CENTER, "GPU", theme::medium(11.0), theme::DIM);
            ui.painter().text(pos2(right, y), Align2::RIGHT_CENTER, "–", theme::medium(11.0), theme::FAINT);
        }
        y += 21.0;
        ui.painter().text(pos2(left, y), Align2::LEFT_CENTER, t("control.network"), theme::medium(11.0), theme::DIM);
        let rates = format!("↓ {}   ↑ {}", format_rate(state.net_down), format_rate(state.net_up));
        ui.painter().text(pos2(right, y), Align2::RIGHT_CENTER, rates, theme::medium(11.0), theme::WHITE);
        y += 21.0;
        if let Some(battery) = &state.battery {
            let icon = if battery.charging { icons::BATTERY_CHARGING } else { battery_icon(battery.percent) };
            ui.painter().text(pos2(left + 8.0, y), Align2::CENTER_CENTER, icon, theme::icon(15.0), theme::DIM);
            let mut text = format!("{:.0} %", battery.percent);
            if battery.charging {
                text = format!("{text} · {}", t("battery.charging"));
            } else if battery.full {
                text = t("battery.fullyCharged");
            } else if let Some(left_time) = battery.time_left {
                let minutes = left_time.as_secs() / 60;
                text = format!("{text} · {} h {:02} min", minutes / 60, minutes % 60);
            }
            ui.painter().text(pos2(right, y), Align2::RIGHT_CENTER, text, theme::medium(11.0), theme::WHITE);
        }
    }

    fn processes_card(&mut self, ui: &mut Ui, card: Rect) {
        kit::card(ui, card, 14.0, theme::CARD);
        let state = self.hub.system.state.lock().map(|s| (s.top_cpu.clone(), s.top_memory.clone())).unwrap_or_default();
        let left = card.left() + 14.0;
        let right = card.right() - 14.0;
        let by_memory = self.controls_ui.top_by_memory;
        let label = t(if by_memory { "control.top.byMemory" } else { "control.top.byCpu" });
        let header = Rect::from_min_size(pos2(left, card.top() + 6.0), vec2(card.width() - 28.0, 18.0));
        let response = ui.interact(header, ui.id().with("top-toggle"), Sense::click());
        text_clipped(ui, pos2(left, header.center().y), Align2::LEFT_CENTER, &label, theme::semibold(12.0), if response.hovered() { theme::WHITE } else { theme::DIM }, header.width());
        if response.clicked() {
            self.controls_ui.top_by_memory = !by_memory;
        }
        let rows = if by_memory { &state.1 } else { &state.0 };
        let mut y = card.top() + 34.0;
        for process in rows.iter().take(5) {
            text_clipped(ui, pos2(left, y), Align2::LEFT_CENTER, &process.name, theme::regular(11.5), theme::WHITE, card.width() - 100.0);
            let value = if by_memory { format_bytes(process.memory) } else { format!("{:.1} %", process.cpu) };
            ui.painter().text(pos2(right, y), Align2::RIGHT_CENTER, value, theme::medium(11.0), theme::DIM);
            y += 25.0;
        }
    }

    fn timer_card(&mut self, ui: &mut Ui, card: Rect) {
        kit::card(ui, card, 14.0, theme::CARD);
        let now = Instant::now();
        let countdown = self.hub.timer.lock().map(|t| *t).unwrap_or_default();
        let left = countdown.left(now);
        let total = countdown.duration.as_secs_f32().max(1.0);
        let center = card.left_center() + vec2(44.0, 0.0);
        let radius = (card.height() / 2.0 - 12.0).min(30.0);
        kit::ring(ui, center, radius, 4.0, left.as_secs_f32() / total, theme::ORANGE);
        ui.painter().text(center, Align2::CENTER_CENTER, icons::TIMER, theme::icon(16.0), theme::DIM);

        let text_x = card.left() + 86.0;
        ui.painter().text(pos2(text_x, card.center().y - 12.0), Align2::LEFT_CENTER, timer::format_left(left), theme::semibold(20.0), theme::WHITE);
        let running = countdown.is_running();
        let y = card.center().y + 16.0;
        let mut x = text_x + 12.0;
        let mut button = |ui: &mut Ui, icon: &str, tip: &str, action: u8| {
            let rect = Rect::from_center_size(pos2(x, y), vec2(26.0, 26.0));
            x += 30.0;
            kit::icon_button(ui, rect, icon, 14.0, theme::WHITE, ("timer", action)).on_hover_text(t(tip)).clicked()
        };
        let main_icon = if running { icons::PAUSE } else { icons::PLAY };
        if button(ui, main_icon, "timer.start", 0) {
            if let Ok(mut timer) = self.hub.timer.lock() {
                if running {
                    timer.pause(now);
                } else {
                    timer.start(now);
                }
            }
            self.hub.play(Tone::Tap);
        }
        if button(ui, icons::MINUS, "timer.decrease", 1) && let Ok(mut timer) = self.hub.timer.lock() {
            timer.decrease(now);
        }
        if button(ui, icons::PLUS, "timer.increase", 2) && let Ok(mut timer) = self.hub.timer.lock() {
            timer.increase(now);
        }
        if !matches!(countdown.state, TimerState::Idle) && button(ui, icons::ARROW_COUNTER_CLOCKWISE, "timer.reset", 3) && let Ok(mut timer) = self.hub.timer.lock() {
            timer.reset();
        }
    }

    fn noise_card(&mut self, ui: &mut Ui, card: Rect) {
        kit::card(ui, card, 14.0, theme::CARD);
        let inner = card.shrink(8.0);
        let grid = Rect::from_min_size(inner.min, vec2(inner.width(), inner.height() - 18.0));
        let cell = vec2(grid.width() / 4.0, grid.height() / 2.0);
        for (index, kind) in NoiseKind::ALL.iter().enumerate() {
            let rect = Rect::from_min_size(
                pos2(grid.left() + cell.x * (index % 4) as f32, grid.top() + cell.y * (index / 4) as f32),
                cell,
            )
            .shrink(2.0);
            let selected = self.controls_ui.noise == Some(*kind);
            let response = ui.interact(rect, ui.id().with(("noise", index)), Sense::click());
            let fill = if selected {
                theme::alpha(theme::ACCENT, 0.5)
            } else if response.hovered() {
                theme::CARD_HOVER
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, rounded(8.0), fill);
            let color = if selected { Color32::WHITE } else { theme::DIM };
            ui.painter().text(rect.center() - vec2(0.0, 6.0), Align2::CENTER_CENTER, kind.icon(), theme::icon(14.0), color);
            text_clipped(ui, rect.center() + vec2(0.0, 9.0), Align2::CENTER_CENTER, &t(kind.label_key()), theme::regular(9.5), color, rect.width() - 2.0);
            if response.clicked() {
                if selected {
                    self.controls_ui.noise = None;
                    self.hub.sounds.noise_stop();
                } else {
                    self.controls_ui.noise = Some(*kind);
                    self.hub.sounds.noise_start(*kind, self.controls_ui.noise_volume);
                }
            }
        }
        let bar = Rect::from_min_max(pos2(inner.left() + 4.0, inner.bottom() - 9.0), pos2(inner.right() - 4.0, inner.bottom() - 5.0));
        if let Some(value) = kit::slider(ui, bar, self.controls_ui.noise_volume, theme::WHITE, "noise-volume") {
            self.controls_ui.noise_volume = value;
            self.hub.sounds.noise_volume(value);
        }
    }
}

/// Radio switches can take a second; they never run on the UI thread.
fn spawn_toggle(action: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name("radio-toggle".into()).spawn(action);
}
