//! The audio mixer, laid out like the original's AudioMixerView: a "System" section with the output
//! and input rows and an "Applications" section with one row per app. Every row is a grid of
//! name | volume slider | percent | device box, and the notch grows with the number of rows.

use super::Notch;
use super::kit::{rounded, text_clipped};
use crate::i18n::t;
use crate::icons;
use crate::platform::AudioDeviceInfo;
use crate::services::audio::{AudioCommand, AudioState};
use crate::services::sounds::Tone;
use crate::theme;
use eframe::egui::{Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};

// Column widths and colours from AudioMixerView.xaml.cs.
const COL_NAME: f32 = 196.0;
const COL_PERCENT: f32 = 50.0;
const COL_DEVICE: f32 = 206.0;
/// Room the original's scroll viewer keeps on the right for its scroll bar.
const SCROLL_PAD: f32 = 7.0;
const HEADER_H: f32 = 20.0;
const HEADER_GAP: f32 = 6.0;
const ROW_H: f32 = 34.0;
const ROW_MARGIN: f32 = 4.0;
const SECTION_GAP: f32 = 14.0;
const THUMB: f32 = 15.0;
/// Space above and below the rows inside the open notch (the tab bar sits in the top part).
const CHROME_TOP: f32 = 37.0;
const CHROME_BOTTOM: f32 = 24.0;
pub const MIN_HEIGHT: f32 = 150.0;
pub const MAX_HEIGHT: f32 = 378.0;

const GREEN: Color32 = Color32::from_rgb(0x3F, 0xD1, 0x5B);
const TRACK: Color32 = Color32::from_rgb(0x4D, 0x4D, 0x4D);
const MUTED: Color32 = Color32::from_rgb(0x8C, 0x8C, 0x8C);
const HEADER_TEXT: Color32 = Color32::from_rgb(0x8A, 0x8A, 0x8A);
const COMBO_BG: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 0x33);
const COMBO_BORDER: Color32 = Color32::from_rgba_premultiplied(0x26, 0x26, 0x26, 0x26);
const COMBO_HOVER: Color32 = Color32::from_rgba_premultiplied(0x1F, 0x1F, 0x1F, 0x1F);
const MENU_BG: Color32 = Color32::from_rgba_premultiplied(0x1B, 0x1B, 0x1B, 0xF2);

/// Which device list a drop-down belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum DeviceKind {
    Output,
    Input,
}

#[derive(Default)]
pub struct MixerUi {
    system_collapsed: bool,
    apps_collapsed: bool,
    scroll: f32,
    /// The open device drop-down and the box it hangs from.
    menu: Option<(DeviceKind, Rect)>,
}

/// The x ranges of the four columns of a row starting at `left`, `width` wide.
fn columns(left: f32, width: f32) -> [(f32, f32); 4] {
    let volume = (width - COL_NAME - COL_PERCENT - COL_DEVICE).max(120.0);
    let a = left + COL_NAME;
    let b = a + volume;
    let c = b + COL_PERCENT;
    [(left, a), (a, b), (b, c), (c, c + COL_DEVICE)]
}

/// Height of the rows of both sections.
fn content_height(apps: usize, system_open: bool, apps_open: bool) -> f32 {
    let row = ROW_H + 2.0 * ROW_MARGIN;
    let header = HEADER_H + HEADER_GAP;
    let mut height = header + if system_open { 2.0 * row } else { 0.0 };
    height += SECTION_GAP + header + if apps_open { apps as f32 * row } else { 0.0 };
    height
}

/// Height of the open notch for the mixer, as the original fits it to its rows.
pub fn fit_height(apps: usize, system_open: bool, apps_open: bool) -> f32 {
    (CHROME_TOP + content_height(apps, system_open, apps_open) + CHROME_BOTTOM).clamp(MIN_HEIGHT, MAX_HEIGHT)
}

impl Notch {
    /// Height the mixer wants right now, from the number of apps playing sound.
    pub(super) fn mixer_height(&self) -> f32 {
        let apps = self.hub.audio.state.lock().map(|s| s.sessions.len()).unwrap_or(0);
        fit_height(apps, !self.mixer_ui.system_collapsed, !self.mixer_ui.apps_collapsed)
    }

    pub(super) fn mixer_view(&mut self, ui: &mut Ui, body: Rect) {
        let state = self.hub.audio.state.lock().map(|s| s.clone()).unwrap_or_default();
        let width = body.width() - SCROLL_PAD;
        let system_open = !self.mixer_ui.system_collapsed;
        let apps_open = !self.mixer_ui.apps_collapsed;
        let total = content_height(state.sessions.len(), system_open, apps_open);
        let overflow = (total - body.height()).max(0.0);
        if overflow > 0.0 && ui.rect_contains_pointer(body) && self.mixer_ui.menu.is_none() {
            self.mixer_ui.scroll -= ui.input(|i| i.smooth_scroll_delta.y);
        }
        self.mixer_ui.scroll = self.mixer_ui.scroll.clamp(0.0, overflow);
        let mut y = body.top() - self.mixer_ui.scroll;
        let notch_clip = ui.clip_rect();
        let mut content = ui.new_child(eframe::egui::UiBuilder::new().max_rect(body));
        content.set_clip_rect(body.intersect(ui.clip_rect()));
        let ui = &mut content;

        // System
        if self.mixer_section(ui, body.left(), width, y, &t("audio.system"), system_open, Some(&t("audio.device")), "system") {
            self.mixer_ui.system_collapsed = system_open;
        }
        y += HEADER_H + HEADER_GAP;
        if system_open {
            let master = state.master.unwrap_or_default();
            let output_name = default_name(&state.outputs).unwrap_or_else(|| t("audio.speakers"));
            let row = Rect::from_min_size(pos2(body.left(), y + ROW_MARGIN), vec2(width, ROW_H));
            self.row_name(ui, row, Some(icons::HEADPHONES), &t("audio.output"));
            if let Some(change) = self.volume_cell(ui, row, master.level, master.muted, "output") {
                self.hub.audio.send(match change {
                    Change::Level(level) => AudioCommand::SetMaster(level),
                    Change::Mute(muted) => AudioCommand::SetMasterMute(muted),
                });
            }
            self.device_box(ui, row, icons::HEADPHONES, &output_name, Some(DeviceKind::Output), state.outputs.len() > 0);
            y += ROW_H + 2.0 * ROW_MARGIN;

            let mic = state.mic.unwrap_or_default();
            let input_name = default_name(&state.inputs).unwrap_or_else(|| t("audio.microphone"));
            let row = Rect::from_min_size(pos2(body.left(), y + ROW_MARGIN), vec2(width, ROW_H));
            self.row_name(ui, row, Some(icons::MICROPHONE), &t("audio.input"));
            if let Some(change) = self.volume_cell(ui, row, mic.level, mic.muted, "input") {
                self.hub.audio.send(match change {
                    Change::Level(level) => AudioCommand::SetMic(level),
                    Change::Mute(muted) => AudioCommand::SetMicMute(muted),
                });
            }
            self.device_box(ui, row, icons::MICROPHONE, &input_name, Some(DeviceKind::Input), state.inputs.len() > 0);
            y += ROW_H + 2.0 * ROW_MARGIN;
        }

        // Applications
        y += SECTION_GAP;
        if self.mixer_section(ui, body.left(), width, y, &t("audio.applications"), apps_open, Some(&t("audio.redirectTo")), "apps") {
            self.mixer_ui.apps_collapsed = apps_open;
        }
        y += HEADER_H + HEADER_GAP;
        if apps_open {
            if state.sessions.is_empty() {
                let message = if cfg!(target_os = "macos") { t("port.mixer.noPerApp") } else { t("port.mixer.empty") };
                text_clipped(ui, pos2(body.left() + 18.0, y + ROW_MARGIN + ROW_H / 2.0), Align2::LEFT_CENTER, &message, theme::bold(12.0), MUTED, width);
            }
            for session in &state.sessions {
                let row = Rect::from_min_size(pos2(body.left(), y + ROW_MARGIN), vec2(width, ROW_H));
                self.row_name(ui, row, None, &session.name);
                if let Some(change) = self.volume_cell(ui, row, session.level, session.muted, ("app", &session.id)) {
                    self.hub.audio.send(match change {
                        Change::Level(level) => AudioCommand::SetSessionVolume(session.id.clone(), level),
                        Change::Mute(muted) => AudioCommand::SetSessionMute(session.id.clone(), muted),
                    });
                }
                self.device_box(ui, row, ICON_REDIRECT, &t("audio.noRedirect"), None, false);
                y += ROW_H + 2.0 * ROW_MARGIN;
            }
        }

        if overflow > 0.0 {
            // The thin scroll bar of the original, 5 px, white at 35 %.
            let track = Rect::from_min_max(pos2(body.right() - 5.0, body.top() + 2.0), pos2(body.right(), body.bottom() - 2.0));
            let thumb_h = (track.height() * body.height() / total).max(24.0);
            let top = track.top() + (track.height() - thumb_h) * (self.mixer_ui.scroll / overflow);
            ui.painter().rect_filled(Rect::from_min_size(pos2(track.left(), top), vec2(5.0, thumb_h)), rounded(2.5), Color32::from_white_alpha(0x59));
        }

        self.device_menu(ui, &state, notch_clip);
    }

    /// "▾ System        Volume        Device". Returns true when the title was clicked.
    #[allow(clippy::too_many_arguments)]
    fn mixer_section(&mut self, ui: &mut Ui, left: f32, width: f32, top: f32, title: &str, open: bool, device_label: Option<&str>, id: &str) -> bool {
        let cols = columns(left, width);
        let center_y = top + HEADER_H / 2.0;
        let title_w = super::kit::text_width(ui, title, theme::bold(15.0));
        let hit = Rect::from_min_max(pos2(left, top), pos2(left + 16.0 + title_w, top + HEADER_H));
        let response = ui.interact(hit, ui.id().with(("mixer-section", id)), Sense::click());
        // The chevron: a small 8 x 5 triangle, turned to the right when the section is closed.
        let c = pos2(left + 4.0, center_y + 0.5);
        let points = if open {
            vec![c + vec2(-4.0, -2.5), c + vec2(4.0, -2.5), c + vec2(0.0, 2.5)]
        } else {
            vec![c + vec2(-2.5, -4.0), c + vec2(2.5, 0.0), c + vec2(-2.5, 4.0)]
        };
        ui.painter().add(Shape::convex_polygon(points, HEADER_TEXT, Stroke::NONE));
        ui.painter().text(pos2(left + 16.0, center_y), Align2::LEFT_CENTER, title, theme::bold(15.0), theme::WHITE);
        if open {
            if id == "system" {
                let (a, b) = cols[1];
                ui.painter().text(pos2((a + b) / 2.0, center_y), Align2::CENTER_CENTER, t("audio.volume"), theme::bold(12.0), HEADER_TEXT);
            }
            if let Some(label) = device_label {
                let (a, b) = cols[3];
                text_clipped(ui, pos2((a + b) / 2.0, center_y), Align2::CENTER_CENTER, label, theme::bold(12.0), HEADER_TEXT, COL_DEVICE);
            }
        }
        if response.clicked() {
            self.hub.play(Tone::Tap);
            self.mixer_ui.menu = None;
        }
        response.clicked()
    }

    /// Icon and name in the first column. Apps without an icon get the generic window glyph.
    fn row_name(&self, ui: &Ui, row: Rect, icon: Option<&str>, name: &str) {
        let (left, right) = columns(row.left(), row.width())[0];
        let y = row.center().y;
        match icon {
            Some(icon) => {
                ui.painter().text(pos2(left + 8.0, y), Align2::CENTER_CENTER, icon, theme::icon_fill(16.0), theme::WHITE);
                text_clipped(ui, pos2(left + 26.0, y), Align2::LEFT_CENTER, name, theme::bold(13.5), theme::WHITE, right - left - 30.0);
            }
            None => {
                ui.painter().text(pos2(left + 9.0, y), Align2::CENTER_CENTER, icons::APP_WINDOW, theme::icon_fill(15.0), theme::WHITE);
                text_clipped(ui, pos2(left + 26.0, y), Align2::LEFT_CENTER, name, theme::bold(13.5), theme::WHITE, 130.0);
            }
        }
    }

    /// Speaker icon, green slider with a white knob, and the percentage.
    fn volume_cell(&mut self, ui: &mut Ui, row: Rect, level: f32, muted: bool, id: impl std::hash::Hash + std::fmt::Debug + Copy) -> Option<Change> {
        let cols = columns(row.left(), row.width());
        let (a, b) = cols[1];
        let y = row.center().y;
        let mut change = None;

        let speaker = Rect::from_center_size(pos2(a + 7.0, y), vec2(18.0, 22.0));
        let speaker_response = ui.interact(speaker, ui.id().with(("mixer-mute", id)), Sense::click());
        let glyph = if muted { icons::SPEAKER_X } else { icons::SPEAKER_HIGH };
        let color = if speaker_response.hovered() { theme::WHITE } else { MUTED };
        ui.painter().text(speaker.center(), Align2::CENTER_CENTER, glyph, theme::icon(14.0), color);
        if speaker_response.clicked() {
            change = Some(Change::Mute(!muted));
        }

        let area = Rect::from_min_max(pos2(a + 24.0, y - 15.0), pos2(b, y + 15.0));
        let response = ui.interact(area, ui.id().with(("mixer-volume", id)), Sense::click_and_drag());
        let active = response.dragged() || response.is_pointer_button_down_on();
        let hover = ui.ctx().animate_bool_with_time(ui.id().with(("mixer-hover", id)), active || response.hovered(), 0.3);
        let shown = if muted { 0.0 } else { level.clamp(0.0, 1.0) };
        let usable = (area.width() - THUMB).max(0.0);
        let thumb_x = area.left() + shown * usable;
        let bar_h = 4.0 * (1.0 + 0.75 * hover);
        let track = Rect::from_center_size(pos2(area.center().x, y), vec2(area.width(), bar_h));
        ui.painter().rect_filled(track, rounded(bar_h / 2.0), TRACK);
        let fill = Rect::from_min_max(track.min, pos2(thumb_x + THUMB / 2.0, track.bottom()));
        ui.painter().rect_filled(fill, rounded(bar_h / 2.0), GREEN);
        let knob = THUMB / 2.0 * (1.0 + 0.18 * hover);
        ui.painter().circle_filled(pos2(thumb_x + THUMB / 2.0, y), knob, theme::WHITE);
        if active && let Some(pointer) = response.interact_pointer_pos() {
            let value = ((pointer.x - area.left() - THUMB / 2.0) / usable.max(1.0)).clamp(0.0, 1.0);
            change = Some(Change::Level(value));
        }

        let percent = format!("{}%", (shown * 100.0).round() as i32);
        ui.painter().text(pos2(cols[2].1 - 4.0, y), Align2::RIGHT_CENTER, percent, theme::bold(12.5), MUTED);
        change
    }

    /// The bordered box in the last column: the current device, or "No redirect" for apps.
    fn device_box(&mut self, ui: &mut Ui, row: Rect, icon: &str, text: &str, kind: Option<DeviceKind>, interactive: bool) {
        let (a, b) = columns(row.left(), row.width())[3];
        let rect = Rect::from_min_max(pos2(a, row.center().y - 14.0), pos2(b - 6.0, row.center().y + 14.0));
        let response = ui.interact(rect, ui.id().with(("mixer-device", kind)), if interactive { Sense::click() } else { Sense::hover() });
        let open = kind.is_some() && self.mixer_ui.menu.is_some_and(|(k, _)| Some(k) == kind);
        let fill = if interactive && (response.hovered() || open) { COMBO_HOVER } else { COMBO_BG };
        ui.painter().rect(rect, rounded(6.0), fill, Stroke::new(1.0, COMBO_BORDER), eframe::egui::StrokeKind::Inside);
        let y = rect.center().y;
        if icon == ICON_REDIRECT {
            paint_redirect_icon(ui, pos2(rect.left() + 14.5, y), MUTED);
        } else {
            ui.painter().text(pos2(rect.left() + 14.5, y), Align2::CENTER_CENTER, icon, theme::icon(13.0), MUTED);
        }
        let label_color = if interactive { theme::WHITE } else { MUTED };
        let caret_room = if interactive { 18.0 } else { 6.0 };
        text_clipped(ui, pos2(rect.left() + 27.0, y), Align2::LEFT_CENTER, text, theme::bold(12.0), label_color, rect.width() - 27.0 - caret_room);
        if interactive {
            // Up/down chevrons squeezed into 8 x 8, as in the original's combo box.
            let c = pos2(rect.right() - 10.0, y);
            let stroke = Stroke::new(1.3, MUTED);
            ui.painter().line(vec![c + vec2(-3.0, -3.0), c + vec2(0.0, -0.2), c + vec2(3.0, -3.0)], stroke);
            ui.painter().line(vec![c + vec2(-3.0, 3.0), c + vec2(0.0, 0.2), c + vec2(3.0, 3.0)], stroke);
        }
        if response.clicked()
            && let Some(kind) = kind
        {
            self.mixer_ui.menu = if open { None } else { Some((kind, rect)) };
            self.hub.play(Tone::Tap);
        }
    }

    /// The drop-down list under a device box. Drawn last so it sits above the rows.
    fn device_menu(&mut self, ui: &mut Ui, state: &AudioState, clip: Rect) {
        let Some((kind, anchor)) = self.mixer_ui.menu else { return };
        let devices: &[AudioDeviceInfo] = match kind {
            DeviceKind::Output => &state.outputs,
            DeviceKind::Input => &state.inputs,
        };
        if devices.is_empty() {
            self.mixer_ui.menu = None;
            return;
        }
        const ITEM_H: f32 = 30.0;
        let height = 8.0 + devices.len() as f32 * (ITEM_H + 2.0);
        let menu = Rect::from_min_size(pos2(anchor.left(), anchor.bottom() + 4.0), vec2(anchor.width(), height));
        let painter = ui.painter().with_clip_rect(clip);
        painter.rect_filled(menu.translate(vec2(0.0, 4.0)).expand(4.0), rounded(13.0), Color32::from_black_alpha(60));
        painter.rect(menu, rounded(9.0), MENU_BG, Stroke::new(1.0, COMBO_BORDER), eframe::egui::StrokeKind::Inside);
        let blocker = ui.interact(menu, ui.id().with("mixer-menu"), Sense::click());
        let mut y = menu.top() + 5.0;
        let mut chosen = None;
        for (index, device) in devices.iter().enumerate() {
            let item = Rect::from_min_size(pos2(menu.left() + 4.0, y), vec2(menu.width() - 8.0, ITEM_H));
            let response = ui.interact(item, ui.id().with(("mixer-menu-item", index)), Sense::click());
            if device.is_default || response.hovered() {
                painter.rect_filled(item, rounded(6.0), COMBO_HOVER);
            }
            let check_room = if device.is_default { 22.0 } else { 0.0 };
            let galley = ui.painter().layout_no_wrap(device.name.clone(), theme::bold(12.0), theme::WHITE);
            let max = item.width() - 16.0 - check_room;
            if galley.size().x <= max {
                painter.galley(pos2(item.left() + 8.0, item.center().y - galley.size().y / 2.0), galley, theme::WHITE);
            } else {
                let clip = Rect::from_min_max(item.min, pos2(item.left() + 8.0 + max, item.bottom()));
                painter.with_clip_rect(clip).galley(pos2(item.left() + 8.0, item.center().y - galley.size().y / 2.0), galley, theme::WHITE);
            }
            if device.is_default {
                painter.text(pos2(item.right() - 12.0, item.center().y), Align2::CENTER_CENTER, icons::CHECK, theme::icon(12.0), GREEN);
            }
            if response.clicked() {
                chosen = Some(device);
            }
            y += ITEM_H + 2.0;
        }
        if let Some(device) = chosen {
            if !device.is_default {
                self.hub.audio.send(AudioCommand::SetDefaultDevice(device.id.clone()));
            }
            self.hub.play(Tone::Tap);
            self.mixer_ui.menu = None;
        } else if !blocker.hovered() && ui.input(|i| i.pointer.primary_clicked()) && !ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| anchor.contains(p)) {
            // A click anywhere else closes the list.
            self.mixer_ui.menu = None;
        }
    }
}

/// What the user did in a volume cell.
enum Change {
    Level(f32),
    Mute(bool),
}

/// Marker for the "No redirect" box's icon, which is painted rather than taken from the icon font.
const ICON_REDIRECT: &str = "redirect";

/// The original's U+E898 glyph: an arrow pointing up at a bar.
fn paint_redirect_icon(ui: &Ui, center: Pos2, color: Color32) {
    let stroke = Stroke::new(1.1, color);
    let painter = ui.painter();
    painter.line_segment([center + vec2(-5.0, -6.0), center + vec2(5.0, -6.0)], stroke);
    painter.line_segment([center + vec2(0.0, -3.5), center + vec2(0.0, 6.0)], stroke);
    painter.line(vec![center + vec2(-3.5, 0.0), center + vec2(0.0, -3.5), center + vec2(3.5, 0.0)], stroke);
}

fn default_name(devices: &[AudioDeviceInfo]) -> Option<String> {
    devices.iter().find(|d| d.is_default).map(|d| d.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_match_the_original_grid() {
        // 682 px scroll viewer minus its 7 px scroll-bar padding.
        let cols = columns(0.0, 675.0);
        assert_eq!(cols[0], (0.0, COL_NAME));
        assert_eq!(cols[1].1 - cols[1].0, 675.0 - COL_NAME - COL_PERCENT - COL_DEVICE);
        assert_eq!(cols[3].1, 675.0);
        // The volume column never gets narrower than the original's minimum.
        assert_eq!(columns(0.0, 400.0)[1].1 - columns(0.0, 400.0)[1].0, 120.0);
    }

    #[test]
    fn height_follows_the_rows() {
        let two = fit_height(2, true, true);
        let three = fit_height(3, true, true);
        assert_eq!(three - two, ROW_H + 2.0 * ROW_MARGIN);
        // Two apps: about as tall as the original's screenshot (295 px).
        assert!((two - 295.0).abs() <= 3.0, "{two}");
        // Many apps stop at the maximum and scroll; everything closed keeps the minimum.
        assert_eq!(fit_height(40, true, true), MAX_HEIGHT);
        assert_eq!(fit_height(0, false, false), MIN_HEIGHT);
    }
}
