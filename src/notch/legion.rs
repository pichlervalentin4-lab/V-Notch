//! The Legion tab: power modes, temperatures, Windows key and touchpad locks.

use super::kit::{self, rounded, text_clipped};
use super::Notch;
use crate::i18n::{t, tf};
use crate::icons;
use crate::services::legion::{LegionCommand, LegionState, LegionStatus, MODE_BALANCED, MODE_CUSTOM, MODE_EXTREME, MODE_PERFORMANCE, MODE_QUIET};
use crate::services::sounds::Tone;
use crate::theme;
use eframe::egui::{Align2, Color32, Rect, Sense, Ui, pos2, vec2};

const MODES: [(i32, &str, &str, Color32); 4] = [
    (MODE_QUIET, "legion.quiet", icons::LEAF, Color32::from_rgb(48, 160, 255)),
    (MODE_BALANCED, "legion.balanced", icons::GAUGE, Color32::from_rgb(235, 235, 240)),
    (MODE_PERFORMANCE, "legion.performance", icons::LIGHTNING, Color32::from_rgb(255, 69, 58)),
    (MODE_EXTREME, "legion.extreme", icons::FIRE, Color32::from_rgb(191, 90, 242)),
];

impl Notch {
    pub(super) fn legion_view(&mut self, ui: &mut Ui, body: Rect) {
        let status = self.hub.legion.status.lock().map(|s| s.clone()).unwrap_or_default();
        match status {
            LegionStatus::Ready(state) => self.legion_ready(ui, body, &state),
            LegionStatus::NeedsSetup | LegionStatus::Unsupported => {
                let center = body.center();
                ui.painter().text(center - vec2(0.0, 44.0), Align2::CENTER_CENTER, icons::GAUGE, theme::icon(30.0), theme::DIM);
                kit::text_wrapped(ui, pos2(body.left() + 40.0, center.y - 22.0), &t("legion.setupHint"), theme::regular(12.5), theme::DIM, body.width() - 80.0, 3);
                let button = Rect::from_center_size(center + vec2(0.0, 40.0), vec2(280.0, 28.0));
                if kit::pill_button(ui, button, &t("legion.setup"), theme::ACCENT, Color32::WHITE, "legion-setup").clicked() {
                    self.hub.legion.send(LegionCommand::Setup);
                }
            }
            LegionStatus::SettingUp => {
                ui.painter().text(body.center(), Align2::CENTER_CENTER, t("legion.settingUp"), theme::regular(13.0), theme::DIM);
            }
        }
    }

    fn legion_ready(&mut self, ui: &mut Ui, body: Rect, state: &LegionState) {
        text_clipped(ui, pos2(body.left() + 4.0, body.top() + 9.0), Align2::LEFT_CENTER, &t("legion.mode"), theme::semibold(12.0), theme::DIM, 200.0);
        let temperatures = tf("legion.temps", &[&state.cpu_temp.to_string(), &state.gpu_temp.to_string()]);
        ui.painter().text(pos2(body.right() - 4.0, body.top() + 9.0), Align2::RIGHT_CENTER, temperatures, theme::medium(12.0), theme::WHITE);

        let row = Rect::from_min_size(pos2(body.left(), body.top() + 24.0), vec2(body.width(), 110.0));
        for (rect, (mode, key, icon, color)) in kit::columns(row, MODES.len(), 8.0).into_iter().zip(MODES) {
            let active = state.mode == mode;
            let response = ui.interact(rect, ui.id().with(("mode", mode)), Sense::click());
            let fill = if active {
                theme::alpha(color, 0.3)
            } else if response.hovered() {
                theme::CARD_HOVER
            } else {
                theme::CARD
            };
            ui.painter().rect_filled(rect, rounded(14.0), fill);
            let tint = if active { color } else { theme::DIM };
            ui.painter().text(rect.center() - vec2(0.0, 18.0), Align2::CENTER_CENTER, icon, theme::icon_fill(26.0), tint);
            text_clipped(ui, rect.center() + vec2(0.0, 14.0), Align2::CENTER_CENTER, &t(key), theme::semibold(12.0), theme::WHITE, rect.width() - 10.0);
            let hint = t(if active { "legion.active" } else { "legion.tapToSet" });
            text_clipped(ui, rect.center() + vec2(0.0, 32.0), Align2::CENTER_CENTER, &hint, theme::regular(10.0), theme::FAINT, rect.width() - 10.0);
            if response.clicked() && !active {
                self.hub.legion.send(LegionCommand::Mode(mode));
                self.hub.play(Tone::Tap);
            }
        }

        let mut y = row.bottom() + 8.0;
        if state.mode == MODE_CUSTOM {
            text_clipped(ui, pos2(body.left() + 4.0, y + 6.0), Align2::LEFT_CENTER, &t("legion.custom"), theme::regular(11.0), theme::ORANGE, body.width() - 8.0);
            y += 18.0;
        }

        text_clipped(ui, pos2(body.left() + 4.0, y + 9.0), Align2::LEFT_CENTER, &t("legion.keys"), theme::semibold(12.0), theme::DIM, 200.0);
        let keys = Rect::from_min_size(pos2(body.left(), y + 22.0), vec2(body.width(), 46.0));
        let parts = kit::columns(keys, 2, 8.0);
        let locks = [
            (parts[0], icons::KEYBOARD, t("legion.winKey"), state.win_key_locked, "lock-win"),
            (parts[1], icons::CURSOR_CLICK, t("legion.touchpad"), state.touchpad_locked, "lock-touch"),
        ];
        for (rect, icon, label, locked, id) in locks {
            kit::card(ui, rect, 12.0, theme::CARD);
            ui.painter().text(rect.left_center() + vec2(20.0, 0.0), Align2::CENTER_CENTER, icon, theme::icon(16.0), theme::DIM);
            text_clipped(ui, rect.left_center() + vec2(38.0, -7.0), Align2::LEFT_CENTER, &label, theme::medium(12.0), theme::WHITE, rect.width() - 100.0);
            let state_text = t(if locked { "legion.locked" } else { "control.on" });
            text_clipped(ui, rect.left_center() + vec2(38.0, 9.0), Align2::LEFT_CENTER, &state_text, theme::regular(10.5), theme::DIM, rect.width() - 100.0);
            let switch = Rect::from_center_size(rect.right_center() - vec2(30.0, 0.0), vec2(38.0, 22.0));
            if kit::toggle(ui, switch, !locked, theme::GREEN, id).clicked() {
                let command = if id == "lock-win" { LegionCommand::WinKey(!locked) } else { LegionCommand::Touchpad(!locked) };
                self.hub.legion.send(command);
                self.hub.play(Tone::Tap);
            }
        }
    }
}
