//! The controls tab, built like the original's control center ("system" page): one muted colour
//! from the PC's load for the whole tab, three soft colour fields drifting behind it, and a
//! scrolling column of sections (system tiles, Legion power mode and keys, network, Bluetooth).
//! Like the original, the clock, timer and ambient sounds no longer show here.

use super::kit::{self, rounded, text_clipped};
use super::Notch;
use crate::anim::lerp;
use crate::i18n::t;
use crate::icons;
use crate::platform::{self, NetworkKind};
use crate::services::legion::{LegionCommand, LegionState, LegionStatus, MODE_BALANCED, MODE_CUSTOM, MODE_EXTREME, MODE_PERFORMANCE, MODE_QUIET};
use crate::services::sounds::Tone;
use crate::services::system::{PING_HOST, SystemState, format_rate};
use crate::settings::Settings;
use crate::theme;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shadow, Stroke, Ui, UiBuilder, pos2, vec2};
use std::collections::HashSet;

// Colours of the original (ControlCenterView.xaml.cs and .SystemLook.cs).
const TRACK: Color32 = Color32::from_gray(0x4D);
const MUTED: Color32 = Color32::from_gray(0x8C);
const HEADER_TEXT: Color32 = Color32::from_gray(0x8A);
const PEAK: Color32 = Color32::from_rgba_premultiplied(0xC0, 0xC0, 0xC0, 0xC0);
const MOOD_STOPS: [(f32, Color32); 4] = [
    (0.0, Color32::from_rgb(0x7F, 0xA6, 0xCC)),
    (45.0, Color32::from_rgb(0xA1, 0x9B, 0xD2)),
    (72.0, Color32::from_rgb(0xD2, 0xAE, 0x7C)),
    (100.0, Color32::from_rgb(0xD2, 0x85, 0x7E)),
];
const PING_GOOD: Color32 = Color32::from_rgb(0x8C, 0xC7, 0xBC);
const PING_SLOW: Color32 = Color32::from_rgb(0xD9, 0xC2, 0x7E);
const PING_BAD: Color32 = Color32::from_rgb(0xD9, 0x8A, 0x84);

/// Power modes: icon, name and the colour the active tile glows in.
const MODES: [(i32, &str, &str, Color32); 4] = [
    (MODE_QUIET, icons::MOON, "legion.quiet", Color32::from_rgb(0x86, 0xA8, 0xD8)),
    (MODE_BALANCED, icons::GAUGE, "legion.balanced", Color32::from_rgb(0xD4, 0xD4, 0xDA)),
    (MODE_PERFORMANCE, icons::SPEEDOMETER, "legion.performance", Color32::from_rgb(0xD9, 0x8A, 0x84)),
    (MODE_EXTREME, icons::LIGHTNING, "legion.extreme", Color32::from_rgb(0xAE, 0x9C, 0xD8)),
];
const CUSTOM_GLOW: Color32 = Color32::from_rgb(0xD9, 0xB0, 0x88);

// Columns of a section row: name, then the action (196 in), and a 200-wide box at the right.
const ACTION_X: f32 = 196.0;
const BOX_X: f32 = 467.0;
const BOX_W: f32 = 200.0;
const ROW_H: f32 = 34.0;

/// The colour fields behind the tab, on a 720 × 400 canvas: position, size, strength at the centre
/// and how far, how much bigger and how slowly each one wanders.
const FIELDS: [(f32, f32, f32, f32, f32, f32, f32, f32, f32); 3] = [
    (-110.0, -130.0, 540.0, 360.0, 0.50, 70.0, 30.0, 1.18, 17.0),
    (380.0, 30.0, 470.0, 320.0, 0.32, -80.0, -20.0, 0.92, 23.0),
    (150.0, 170.0, 440.0, 280.0, 0.30, 40.0, -40.0, 1.15, 19.0),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum TopMode {
    Off,
    Cpu,
    Memory,
}

pub struct ControlsUi {
    /// The load the colour shows, eased towards the real one.
    mood_load: f32,
    mood_target: f32,
    /// Where each load bar peaked lately (CPU, GPU, RAM); sinks back slowly.
    peaks: [f32; 3],
    top: TopMode,
    collapsed: HashSet<&'static str>,
}

impl Default for ControlsUi {
    fn default() -> Self {
        Self { mood_load: f32::NAN, mood_target: 0.0, peaks: [0.0; 3], top: TopMode::Off, collapsed: HashSet::new() }
    }
}

/// The tab's colour at a load from 0 to 100, blended between the stops.
pub fn mood_at(load: f32) -> Color32 {
    let load = if load.is_nan() { 0.0 } else { load.clamp(0.0, 100.0) };
    for pair in MOOD_STOPS.windows(2) {
        let ((from_load, from), (to_load, to)) = (pair[0], pair[1]);
        if load <= to_load {
            return kit::lerp_color(from, to, (load - from_load) / (to_load - from_load));
        }
    }
    MOOD_STOPS[MOOD_STOPS.len() - 1].1
}

/// What sets the colour: the busiest of CPU and GPU, and the memory once it fills up.
fn mood_load(state: &SystemState) -> f32 {
    let gpu = state.gpu.as_ref().map_or(0.0, |g| g.usage);
    state.cpu.max(gpu * 0.95).max(memory_percent(state) * 0.7)
}

fn memory_percent(state: &SystemState) -> f32 {
    if state.memory_total == 0 { 0.0 } else { state.memory_used as f32 / state.memory_total as f32 * 100.0 }
}

fn lift(color: Color32, toward_white: f32) -> Color32 {
    kit::lerp_color(color, Color32::WHITE, toward_white)
}

fn dim(color: Color32, factor: f32) -> Color32 {
    let scale = |c: u8| (f32::from(c) * factor) as u8;
    Color32::from_rgb(scale(color.r()), scale(color.g()), scale(color.b()))
}

impl Notch {
    pub(super) fn controls_view(&mut self, ui: &mut Ui, body: Rect, _settings: &Settings) {
        let state = self.hub.system.state.lock().map(|s| s.clone()).unwrap_or_default();
        let legion = self.hub.legion.status.lock().map(|s| s.clone()).unwrap_or_default();

        // Ease the colour towards the load; small wobbles do not move it at all.
        let target = mood_load(&state);
        let controls = &mut self.controls_ui;
        if controls.mood_load.is_nan() {
            controls.mood_load = target;
            controls.mood_target = target;
        } else if (target - controls.mood_target).abs() >= 2.0 {
            controls.mood_target = target;
        }
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        controls.mood_load += (controls.mood_target - controls.mood_load) * (1.0 - (-dt / 0.4).exp());
        let load = controls.mood_load;
        let mood = mood_at(load);

        let notch = Rect::from_min_max(body.min - vec2(20.0, 34.0), body.max + vec2(20.0, 14.0));
        paint_backdrop(ui, notch, [mood, mood_at(load + 22.0), mood_at(load - 18.0)]);

        let mut child = ui.new_child(UiBuilder::new().max_rect(body).id_salt("controls-scroll"));
        child.set_clip_rect(body.expand2(vec2(20.0, 0.0)).intersect(ui.clip_rect()));
        let scroll = &mut child.style_mut().spacing.scroll;
        *scroll = egui::style::ScrollStyle { bar_width: 4.0, bar_inner_margin: 0.0, bar_outer_margin: 0.0, floating: false, ..egui::style::ScrollStyle::solid() };
        child.visuals_mut().widgets.inactive.bg_fill = theme::alpha(Color32::WHITE, 0.45);
        child.visuals_mut().widgets.hovered.bg_fill = theme::alpha(Color32::WHITE, 0.6);
        child.visuals_mut().widgets.active.bg_fill = theme::alpha(Color32::WHITE, 0.75);
        child.visuals_mut().extreme_bg_color = Color32::TRANSPARENT;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let width = ui.available_width() - 8.0;
            let ink = lift(mood, 0.3);
            if self.section_header(ui, "system", &t("control.system"), "", 0.0, ink, width) {
                self.stat_tiles(ui, &state, &legion, mood, width);
            }
            if let LegionStatus::Ready(legion_state) = &legion {
                if self.section_header(ui, "legion.mode", &t("legion.mode"), "", 14.0, ink, width) {
                    self.power_modes(ui, legion_state, width);
                }
                if self.section_header(ui, "legion.keys", &t("legion.keys"), "", 6.0, ink, width) {
                    self.key_tiles(ui, legion_state, width);
                }
            } else if matches!(legion, LegionStatus::NeedsSetup | LegionStatus::SettingUp)
                && self.section_header(ui, "legion.mode", &t("legion.mode"), "", 14.0, ink, width)
            {
                self.legion_setup_row(ui, &legion, width);
            }
            if self.section_header(ui, "network", &t("control.network"), &t("control.status"), 14.0, ink, width) {
                self.network_rows(ui, &state, mood, width);
            }
            if self.section_header(ui, "bluetooth", "Bluetooth", &t("control.status"), 14.0, ink, width) {
                self.bluetooth_rows(ui, &state, mood, width);
            }
            ui.add_space(6.0);
        });
    }

    /// A chevron and a 15-point bold title; a click folds the section. Returns whether it is open.
    fn section_header(&mut self, ui: &mut Ui, key: &'static str, title: &str, right: &str, top: f32, ink: Color32, width: f32) -> bool {
        let (rect, _) = ui.allocate_exact_size(vec2(width, top + 18.0 + 6.0), Sense::hover());
        let row = Rect::from_min_size(pos2(rect.left(), rect.top() + top), vec2(width, 18.0));
        let open = !self.controls_ui.collapsed.contains(key);
        let title_width = kit::text_width(ui, title, theme::bold(15.0));
        let hit = Rect::from_min_size(row.min, vec2(16.0 + title_width, row.height()));
        let response = ui.interact(hit, ui.id().with(("section", key)), Sense::click());
        if response.clicked() {
            if open {
                self.controls_ui.collapsed.insert(key);
            } else {
                self.controls_ui.collapsed.remove(key);
            }
            self.hub.play(Tone::Tap);
        }
        let turn = ui.ctx().animate_bool_with_time(response.id.with("turn"), open, 0.22);
        let angle = lerp(-90.0_f32, 0.0, turn).to_radians();
        let center = pos2(row.left() + 4.0, row.center().y + 0.5);
        let corners = [vec2(-4.0, -2.5), vec2(4.0, -2.5), vec2(0.0, 2.5)].map(|p| {
            let (sin, cos) = angle.sin_cos();
            center + vec2(p.x * cos - p.y * sin, p.x * sin + p.y * cos)
        });
        ui.painter().add(egui::Shape::convex_polygon(corners.to_vec(), ink, Stroke::NONE));
        ui.painter().text(pos2(row.left() + 16.0, row.center().y), Align2::LEFT_CENTER, title, theme::bold(15.0), Color32::WHITE);
        if open && !right.is_empty() {
            let x = rect.left() + BOX_X + BOX_W / 2.0;
            ui.painter().text(pos2(x, row.center().y), Align2::CENTER_CENTER, right, theme::bold(12.0), HEADER_TEXT);
        }
        open
    }

    /// CPU, GPU, RAM, ping and network: five cards 8 apart, each with a label in the tab's colour,
    /// a big number and a load bar or a detail line.
    fn stat_tiles(&mut self, ui: &mut Ui, state: &SystemState, legion: &LegionStatus, mood: Color32, width: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 64.0 + 2.0), Sense::hover());
        let row = Rect::from_min_size(rect.min, vec2(width, 64.0));
        let cells = kit::columns(row, 5, 8.0);
        let ink = lift(mood, 0.3);
        let (cpu_temp, gpu_temp) = match legion {
            LegionStatus::Ready(legion) => (Some(legion.cpu_temp as f32), Some(legion.gpu_temp as f32)),
            _ => (None, state.gpu.as_ref().and_then(|g| g.temperature)),
        };
        let gpu = state.gpu.as_ref().map(|g| g.usage);
        let ram = memory_percent(state);
        let loads = [Some(state.cpu), gpu, Some(ram)];
        for (index, load) in loads.iter().enumerate() {
            let peak = &mut self.controls_ui.peaks[index];
            let value = load.unwrap_or(0.0);
            // A peak mark that sinks back six points every three seconds, like a level meter.
            *peak = if value >= *peak { value } else { (*peak - 2.0 * ui.input(|i| i.stable_dt).min(0.1)).max(value) };
        }

        let tiles: [(&str, Option<f32>); 3] = [("CPU", cpu_temp), ("GPU", gpu_temp), ("RAM", None)];
        for (index, (label, temperature)) in tiles.into_iter().enumerate() {
            let card = cells[index];
            let response = self.stat_card(ui, card, index);
            let mut x = card.left() + 12.0;
            let label_galley = ui.painter().layout_no_wrap(label.to_owned(), theme::bold(11.0), ink);
            let label_width = label_galley.size().x;
            ui.painter().galley(pos2(x, card.top() + 9.0), label_galley, ink);
            x += label_width;
            if let Some(temperature) = temperature {
                ui.painter().text(pos2(x, card.top() + 9.0), Align2::LEFT_TOP, format!(" · {temperature:.0}°"), theme::bold(11.0), MUTED);
            }
            let value = loads[index];
            let text = value.map_or_else(|| "–".to_owned(), |v| format!("{v:.0} %"));
            ui.painter().text(pos2(card.left() + 12.0, card.top() + 25.0), Align2::LEFT_TOP, text, theme::bold(15.0), Color32::WHITE);
            let bar = Rect::from_min_max(pos2(card.left() + 12.0, card.top() + 50.0), pos2(card.right() - 12.0, card.top() + 53.0));
            ui.painter().rect_filled(bar, rounded(1.5), TRACK);
            let fill = value.unwrap_or(0.0).clamp(0.0, 100.0) / 100.0;
            if fill > 0.0 {
                let filled = Rect::from_min_size(bar.min, vec2((bar.width() * fill).max(3.0), bar.height()));
                kit::horizontal_gradient_rect(ui, filled, 1.5, mood, dim(mood, 0.62));
            }
            let peak = self.controls_ui.peaks[index].clamp(0.0, 100.0) / 100.0;
            if value.is_some() && peak > 0.02 {
                let px = bar.left() + bar.width() * peak;
                ui.painter().rect_filled(Rect::from_center_size(pos2(px, bar.center().y), vec2(2.0, 7.0)), rounded(1.0), PEAK);
            }
            if response.clicked() && index != 1 {
                let mode = if index == 0 { TopMode::Cpu } else { TopMode::Memory };
                self.controls_ui.top = if self.controls_ui.top == mode { TopMode::Off } else { mode };
                self.hub.play(Tone::Tap);
            }
        }

        // Ping: a coloured dot after the label, the round trip and the host under it.
        let card = cells[3];
        let _ = self.stat_card(ui, card, 3);
        let label_width = kit::text_width(ui, "Ping", theme::bold(11.0));
        ui.painter().text(pos2(card.left() + 12.0, card.top() + 9.0), Align2::LEFT_TOP, "Ping", theme::bold(11.0), ink);
        if let Some(ms) = state.ping_ms {
            let color = if ms < 60.0 { PING_GOOD } else if ms < 150.0 { PING_SLOW } else { PING_BAD };
            ui.painter().circle_filled(pos2(card.left() + 12.0 + label_width + 7.0, card.top() + 16.0), 3.0, color);
        }
        let ping = state.ping_ms.map_or_else(|| "–".to_owned(), |ms| format!("{ms:.0} ms"));
        ui.painter().text(pos2(card.left() + 12.0, card.top() + 25.0), Align2::LEFT_TOP, ping, theme::bold(15.0), Color32::WHITE);
        ui.painter().text(pos2(card.left() + 12.0, card.top() + 45.0), Align2::LEFT_TOP, PING_HOST, theme::bold(10.5), MUTED);

        // Network: download big, upload small, each with its arrow.
        let card = cells[4];
        if self.stat_card(ui, card, 4).clicked() {
            platform::open_network_settings();
        }
        ui.painter().text(pos2(card.left() + 12.0, card.top() + 9.0), Align2::LEFT_TOP, t("control.network"), theme::bold(11.0), ink);
        let down = format!("↓ {}", format_rate(state.net_down));
        text_clipped(ui, pos2(card.left() + 12.0, card.top() + 26.0), Align2::LEFT_TOP, &down, theme::bold(13.5), Color32::WHITE, card.width() - 20.0);
        let up = format!("↑ {}", format_rate(state.net_up));
        text_clipped(ui, pos2(card.left() + 12.0, card.top() + 45.0), Align2::LEFT_TOP, &up, theme::bold(10.5), MUTED, card.width() - 20.0);

        if self.controls_ui.top != TopMode::Off {
            let rows = if self.controls_ui.top == TopMode::Cpu { &state.top_cpu } else { &state.top_memory };
            for process in rows.iter().take(4) {
                let (rect, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
                text_clipped(ui, pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, &process.name, theme::bold(12.0), Color32::WHITE, ACTION_X + 120.0);
                let value = if self.controls_ui.top == TopMode::Cpu {
                    format!("{:.1} %", process.cpu)
                } else {
                    crate::services::system::format_bytes(process.memory)
                };
                ui.painter().text(pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, value, theme::bold(11.0), MUTED);
            }
            ui.add_space(4.0);
        }
    }

    /// The card behind a stat tile: 6 % white, a bit more under the pointer, 12-point corners.
    fn stat_card(&mut self, ui: &mut Ui, card: Rect, index: usize) -> egui::Response {
        let response = ui.interact(card, ui.id().with(("stat", index)), Sense::click());
        let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.15);
        let grow = if index == 3 { 0.0 } else { hover * 1.0 };
        ui.painter().rect_filled(card.expand(grow), rounded(12.0), theme::alpha(Color32::WHITE, lerp(0.059, 0.11, hover)));
        response
    }

    /// Four power-mode tiles; the active one is washed in its colour, outlined and glowing.
    fn power_modes(&mut self, ui: &mut Ui, state: &LegionState, width: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 72.0 + 8.0), Sense::hover());
        let row = Rect::from_min_size(rect.min, vec2(width, 72.0));
        for (card, (mode, icon, key, glow)) in kit::columns(row, MODES.len(), 8.0).into_iter().zip(MODES) {
            let active = state.mode == mode;
            let response = ui.interact(card, ui.id().with(("mode", mode)), Sense::click());
            let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.15);
            if active {
                let shadow = Shadow { offset: [0, 0], blur: 18, spread: 0, color: theme::alpha(glow, 0.3) };
                ui.painter().add(shadow.as_shape(card, rounded(12.0)));
                ui.painter().rect_filled(card, rounded(12.0), Color32::from_rgba_unmultiplied(glow.r(), glow.g(), glow.b(), 0x38));
                ui.painter().rect_stroke(card, rounded(12.0), Stroke::new(1.2, glow), egui::StrokeKind::Inside);
            } else {
                ui.painter().rect_filled(card, rounded(12.0), theme::alpha(Color32::WHITE, lerp(0.059, 0.11, hover)));
            }
            let ink = if active { glow } else { MUTED };
            ui.painter().text(pos2(card.left() + 12.0, card.top() + 10.0), Align2::LEFT_TOP, icon, theme::icon(17.0), ink);
            text_clipped(ui, pos2(card.left() + 12.0, card.top() + 37.0), Align2::LEFT_TOP, &t(key), theme::bold(12.5), Color32::WHITE, card.width() - 20.0);
            if active {
                text_clipped(ui, pos2(card.left() + 12.0, card.top() + 53.0), Align2::LEFT_TOP, &t("legion.active"), theme::bold(11.0), lift(glow, 0.1), card.width() - 20.0);
            }
            if response.clicked() && !active {
                self.hub.legion.send(LegionCommand::Mode(mode));
                self.hub.play(Tone::Tap);
            }
        }
        if state.mode == MODE_CUSTOM {
            let (rect, _) = ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
            text_clipped(ui, pos2(rect.left() + 2.0, rect.center().y), Align2::LEFT_CENTER, &t("legion.custom"), theme::bold(11.0), CUSTOM_GLOW, width);
        }
    }

    /// The Windows key and touchpad: a wide card each, washed in the active mode's colour while on.
    fn key_tiles(&mut self, ui: &mut Ui, state: &LegionState, width: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 48.0 + 8.0), Sense::hover());
        let row = Rect::from_min_size(rect.min, vec2(width, 48.0));
        let glow = MODES.iter().find(|(mode, ..)| *mode == state.mode).map_or(CUSTOM_GLOW, |m| m.3);
        let cells = kit::columns(row, 2, 8.0);
        let keys = [
            (cells[0], icons::KEYBOARD, t("legion.winKey"), !state.win_key_locked, "win"),
            (cells[1], icons::LAPTOP, t("legion.touchpad"), !state.touchpad_locked, "touchpad"),
        ];
        for (card, icon, name, on, id) in keys {
            let response = ui.interact(card, ui.id().with(("key", id)), Sense::click());
            let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.15);
            let fill = if on { Color32::from_rgba_unmultiplied(glow.r(), glow.g(), glow.b(), 0x38) } else { theme::alpha(Color32::WHITE, lerp(0.059, 0.11, hover)) };
            ui.painter().rect_filled(card, rounded(12.0), fill);
            let ink = if on { glow } else { MUTED };
            ui.painter().text(pos2(card.left() + 21.0, card.center().y), Align2::CENTER_CENTER, icon, theme::icon(18.0), ink);
            let left = card.left() + 46.0;
            text_clipped(ui, pos2(left, card.center().y - 8.0), Align2::LEFT_CENTER, &name, theme::bold(12.5), Color32::WHITE, card.width() - 60.0);
            let status = t(if on { "control.on" } else { "legion.locked" });
            text_clipped(ui, pos2(left, card.center().y + 8.0), Align2::LEFT_CENTER, &status, theme::bold(11.0), MUTED, card.width() - 60.0);
            if response.clicked() {
                let command = if id == "win" { LegionCommand::WinKey(on) } else { LegionCommand::Touchpad(on) };
                self.hub.legion.send(command);
                self.hub.play(Tone::Tap);
            }
        }
    }

    fn legion_setup_row(&mut self, ui: &mut Ui, status: &LegionStatus, width: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
        let hint = t(if matches!(status, LegionStatus::SettingUp) { "legion.settingUp" } else { "legion.setupHint" });
        text_clipped(ui, pos2(rect.left() + 2.0, rect.center().y), Align2::LEFT_CENTER, &hint, theme::bold(11.5), MUTED, BOX_X - 10.0);
        if matches!(status, LegionStatus::NeedsSetup) {
            let button = Rect::from_min_size(pos2(rect.left() + BOX_X, rect.center().y - 13.0), vec2(BOX_W, 26.0));
            if combo(ui, button, None, &t("legion.setup"), false, "legion-setup").clicked() {
                self.hub.legion.send(LegionCommand::Setup);
            }
        }
    }

    fn network_rows(&mut self, ui: &mut Ui, state: &SystemState, mood: Color32, width: f32) {
        let network = &state.network;
        if network.kind == NetworkKind::Wired {
            let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
            row_name(ui, rect, icons::DESKTOP, &t("control.wired"), Color32::WHITE);
            ui.painter().text(pos2(rect.left() + ACTION_X, rect.center().y), Align2::LEFT_CENTER, t("control.connected"), theme::bold(11.5), MUTED);
            let label = if network.name.is_empty() { t("control.wired") } else { network.name.clone() };
            let button = Rect::from_min_size(pos2(rect.left() + BOX_X, rect.center().y - 13.0), vec2(BOX_W, 26.0));
            if combo(ui, button, Some(icons::DESKTOP), &label, false, "wired").clicked() {
                platform::open_network_settings();
            }
        }
        let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
        let wifi_on = network.wifi_on.unwrap_or(network.kind == NetworkKind::Wifi);
        row_name(ui, rect, icons::WIFI_HIGH, &t("control.wifi"), if wifi_on { lift(mood, 0.3) } else { MUTED });
        if let Some(on) = network.wifi_on {
            if switch(ui, rect, on, mood, "wifi-switch") && !self.hub.demo {
                spawn_toggle(move || platform::set_wifi(!on));
                self.hub.play(Tone::Tap);
            }
        }
        let connected = network.kind == NetworkKind::Wifi;
        let label = if connected && !network.name.is_empty() {
            network.name.clone()
        } else if wifi_on {
            t("control.notConnected")
        } else {
            t("control.wifiOff")
        };
        let button = Rect::from_min_size(pos2(rect.left() + BOX_X, rect.center().y - 13.0), vec2(BOX_W, 26.0));
        if combo(ui, button, Some(icons::WIFI_HIGH), &label, wifi_on, "wifi-box").clicked() {
            platform::open_network_settings();
        }
    }

    fn bluetooth_rows(&mut self, ui: &mut Ui, state: &SystemState, mood: Color32, width: f32) {
        let bluetooth = &state.bluetooth;
        let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
        let on = bluetooth.powered.unwrap_or(false);
        row_name(ui, rect, icons::BLUETOOTH, "Bluetooth", if on { lift(mood, 0.3) } else { MUTED });
        match bluetooth.powered {
            Some(powered) => {
                if switch(ui, rect, powered, mood, "bt-switch") && !self.hub.demo {
                    spawn_toggle(move || platform::set_bluetooth(!powered));
                    self.hub.play(Tone::Tap);
                }
            }
            None => {
                ui.painter().text(pos2(rect.left() + ACTION_X, rect.center().y), Align2::LEFT_CENTER, t("control.noAdapter"), theme::bold(11.5), MUTED);
            }
        }
        if !on {
            return;
        }
        if bluetooth.devices.is_empty() {
            let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
            text_clipped(ui, pos2(rect.left() + 26.0, rect.center().y), Align2::LEFT_CENTER, &t("control.noDevices"), theme::bold(11.5), MUTED, width);
        }
        for (index, device) in bluetooth.devices.iter().enumerate() {
            let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
            let ink = if device.connected { Color32::WHITE } else { MUTED };
            row_name(ui, rect, icons::HEADPHONES, &device.name, ink);
            let status = t(if device.connected { "control.connected" } else { "control.notConnected" });
            ui.painter().text(pos2(rect.left() + ACTION_X, rect.center().y), Align2::LEFT_CENTER, status, theme::bold(11.5), MUTED);
            let _ = index;
        }
    }
}

/// Icon and name at the start of a section row.
fn row_name(ui: &Ui, rect: Rect, icon: &str, name: &str, icon_color: Color32) {
    ui.painter().text(pos2(rect.left() + 8.0, rect.center().y), Align2::CENTER_CENTER, icon, theme::icon(16.0), icon_color);
    text_clipped(ui, pos2(rect.left() + 26.0, rect.center().y), Align2::LEFT_CENTER, name, theme::bold(12.5), Color32::WHITE, ACTION_X - 34.0);
}

/// The original's switch in the action column, in the tab's colour, with "On" or "Off" beside it.
fn switch(ui: &mut Ui, row: Rect, on: bool, mood: Color32, id: &str) -> bool {
    let rect = Rect::from_min_size(pos2(row.left() + ACTION_X, row.center().y - 10.0), vec2(34.0, 20.0));
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    let t_on = ui.ctx().animate_bool_with_time(response.id.with("t"), on, 0.2);
    let track = kit::lerp_color(theme::alpha(Color32::WHITE, 0.18), mood, t_on);
    ui.painter().rect_filled(rect, rounded(10.0), track);
    let knob_x = lerp(rect.left() + 10.0, rect.right() - 10.0, t_on);
    ui.painter().circle_filled(pos2(knob_x, rect.center().y), 8.0, Color32::WHITE);
    let label = t(if on { "control.on" } else { "control.off" });
    ui.painter().text(pos2(rect.right() + 10.0, rect.center().y), Align2::LEFT_CENTER, label, theme::bold(11.5), MUTED);
    response.clicked()
}

/// The original's drop-down box: a dark field with a hairline border, an icon and a label, and a
/// small × at the end when there is something to disconnect.
fn combo(ui: &mut Ui, rect: Rect, icon: Option<&str>, label: &str, clear: bool, id: &str) -> egui::Response {
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    let fill = if response.hovered() { theme::alpha(Color32::WHITE, 0x1F as f32 / 255.0) } else { theme::alpha(Color32::BLACK, 0x33 as f32 / 255.0) };
    ui.painter().rect_filled(rect, rounded(6.0), fill);
    ui.painter().rect_stroke(rect, rounded(6.0), Stroke::new(1.0, theme::alpha(Color32::WHITE, 0x26 as f32 / 255.0)), egui::StrokeKind::Inside);
    let mut left = rect.left() + 10.0;
    if let Some(icon) = icon {
        ui.painter().text(pos2(left + 6.0, rect.center().y), Align2::CENTER_CENTER, icon, theme::icon(13.0), theme::alpha(Color32::WHITE, 0.7));
        left += 18.0;
    }
    let right = if clear { rect.right() - 22.0 } else { rect.right() - 10.0 };
    text_clipped(ui, pos2(left, rect.center().y), Align2::LEFT_CENTER, label, theme::bold(11.5), Color32::WHITE, right - left);
    if clear {
        ui.painter().text(pos2(rect.right() - 12.0, rect.center().y), Align2::CENTER_CENTER, icons::X, theme::icon(10.0), MUTED);
    }
    response
}

/// Three soft colour fields drifting slowly behind the tab, as the original's MoodBackdrop: radial
/// gradients (strength at the centre, 55 % of it halfway out, nothing at the rim) on a 720 × 400
/// canvas centred on the notch.
pub(super) fn paint_backdrop(ui: &Ui, notch: Rect, colors: [Color32; 3]) {
    use eframe::egui::epaint::{Mesh, Vertex};
    let time = ui.input(|i| i.time) as f32;
    let canvas = pos2(notch.center().x - 360.0, notch.top());
    let wave = |seconds: f32| (1.0 - (time / seconds * std::f32::consts::TAU).cos()) / 2.0;
    let painter = ui.painter();
    for ((x, y, w, h, strength, dx, dy, ds, seconds), color) in FIELDS.into_iter().zip(colors) {
        let scale = lerp(1.0, ds, wave(seconds * 0.87));
        let center = canvas + vec2(x + w / 2.0 + dx * wave(seconds), y + h / 2.0 + dy * wave(seconds * 1.13));
        let (rx, ry) = (w / 2.0 * scale, h / 2.0 * scale);
        let shade = |alpha: f32| Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (alpha.clamp(0.0, 1.0) * 255.0) as u8);
        let mut mesh = Mesh::default();
        mesh.vertices.push(Vertex { pos: center, uv: egui::epaint::WHITE_UV, color: shade(strength) });
        const SEGMENTS: u32 = 48;
        for ring in [(0.48, strength * 0.55), (1.0, 0.0)] {
            for i in 0..SEGMENTS {
                let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                let pos: Pos2 = center + vec2(angle.cos() * rx * ring.0, angle.sin() * ry * ring.0);
                mesh.vertices.push(Vertex { pos, uv: egui::epaint::WHITE_UV, color: shade(ring.1) });
            }
        }
        for i in 0..SEGMENTS {
            let next = (i + 1) % SEGMENTS;
            mesh.add_triangle(0, 1 + i, 1 + next);
            let (inner, inner_next, outer, outer_next) = (1 + i, 1 + next, 1 + SEGMENTS + i, 1 + SEGMENTS + next);
            mesh.add_triangle(inner, outer, outer_next);
            mesh.add_triangle(inner, outer_next, inner_next);
        }
        painter.add(mesh);
    }
}

/// Radio switches can take a second; they never run on the UI thread.
fn spawn_toggle(action: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name("radio-toggle".into()).spawn(action);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mood_follows_the_stops() {
        assert_eq!(mood_at(0.0), Color32::from_rgb(0x7F, 0xA6, 0xCC));
        assert_eq!(mood_at(100.0), Color32::from_rgb(0xD2, 0x85, 0x7E));
        assert_eq!(mood_at(45.0), Color32::from_rgb(0xA1, 0x9B, 0xD2));
        assert_eq!(mood_at(f32::NAN), mood_at(0.0));
        assert_eq!(mood_at(250.0), mood_at(100.0));
    }

    #[test]
    fn memory_only_counts_once_it_fills_up() {
        let state = SystemState { cpu: 10.0, memory_used: 8, memory_total: 10, ..SystemState::default() };
        assert!((mood_load(&state) - 56.0).abs() < 0.01);
    }
}
