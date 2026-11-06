//! The audio mixer: master volume, one slider per app, device switching.

use super::kit::{self, rounded, text_clipped};
use super::{Notch, volume_icon};
use crate::i18n::t;
use crate::icons;
use crate::platform::AudioDeviceInfo;
use crate::services::audio::AudioCommand;
use crate::services::sounds::Tone;
use crate::theme;
use eframe::egui::{Align2, Rect, Sense, Ui, pos2, vec2};

impl Notch {
    pub(super) fn mixer_view(&mut self, ui: &mut Ui, body: Rect) {
        let state = self.hub.audio.state.lock().map(|s| s.clone()).unwrap_or_default();
        let accent = self.hub.media.state.lock().ok().and_then(|s| s.art.as_ref().map(|a| a.accent)).unwrap_or(theme::YELLOW);
        let split = body.left() + 430.0;
        let apps = Rect::from_min_max(body.min, pos2(split - 6.0, body.bottom()));
        let devices = Rect::from_min_max(pos2(split + 6.0, body.top()), body.max);

        // Master
        let master_card = Rect::from_min_size(apps.min, vec2(apps.width(), 56.0));
        kit::card(ui, master_card, 14.0, theme::CARD);
        let master = state.master.unwrap_or_default();
        let icon = Rect::from_center_size(master_card.left_center() + vec2(26.0, 0.0), vec2(30.0, 30.0));
        if kit::icon_button_fill(ui, icon, volume_icon(master.level, master.muted), 17.0, theme::WHITE, "master-mute").clicked() {
            self.hub.audio.send(AudioCommand::SetMasterMute(!master.muted));
        }
        text_clipped(ui, pos2(icon.right() + 8.0, master_card.top() + 16.0), Align2::LEFT_CENTER, &t("audio.system"), theme::semibold(12.5), theme::WHITE, 160.0);
        let percent = format!("{:.0} %", master.level * 100.0);
        ui.painter().text(pos2(master_card.right() - 14.0, master_card.top() + 16.0), Align2::RIGHT_CENTER, percent, theme::medium(11.5), theme::DIM);
        let bar = Rect::from_min_max(pos2(icon.right() + 8.0, master_card.bottom() - 19.0), pos2(master_card.right() - 14.0, master_card.bottom() - 13.0));
        if let Some(value) = kit::slider(ui, bar, if master.muted { 0.0 } else { master.level }, accent, "master") {
            self.hub.audio.send(AudioCommand::SetMaster(value));
        }

        // Applications
        let list_top = master_card.bottom() + 8.0;
        text_clipped(ui, pos2(apps.left() + 4.0, list_top + 7.0), Align2::LEFT_CENTER, &t("audio.applications"), theme::semibold(11.5), theme::DIM, 200.0);
        let mut y = list_top + 20.0;
        if state.sessions.is_empty() {
            let message = if cfg!(target_os = "macos") { t("port.mixer.noPerApp") } else { t("port.mixer.empty") };
            ui.painter().text(pos2(apps.center().x, y + 40.0), Align2::CENTER_CENTER, message, theme::regular(12.0), theme::FAINT);
        }
        for session in &state.sessions {
            if y + 44.0 > apps.bottom() {
                break;
            }
            let row = Rect::from_min_size(pos2(apps.left(), y), vec2(apps.width(), 42.0));
            kit::card(ui, row, 10.0, theme::CARD);
            let icon = Rect::from_center_size(row.left_center() + vec2(22.0, 0.0), vec2(26.0, 26.0));
            if kit::icon_button_fill(ui, icon, volume_icon(session.level, session.muted), 14.0, theme::DIM, ("session-mute", &session.id)).clicked() {
                self.hub.audio.send(AudioCommand::SetSessionMute(session.id.clone(), !session.muted));
            }
            text_clipped(ui, pos2(icon.right() + 6.0, row.top() + 13.0), Align2::LEFT_CENTER, &session.name, theme::medium(11.5), theme::WHITE, 200.0);
            let bar = Rect::from_min_max(pos2(icon.right() + 6.0, row.bottom() - 15.0), pos2(row.right() - 14.0, row.bottom() - 10.0));
            let shown = if session.muted { 0.0 } else { session.level };
            // A faint second bar shows the app's actual output level.
            if session.peak > 0.005 {
                let level = Rect::from_min_size(bar.min - vec2(0.0, 3.0), vec2(bar.width() * (session.peak * shown).clamp(0.0, 1.0), 1.5));
                ui.painter().rect_filled(level, rounded(1.0), theme::alpha(accent, 0.7));
            }
            if let Some(value) = kit::slider(ui, bar, shown, theme::WHITE, ("session", &session.id)) {
                self.hub.audio.send(AudioCommand::SetSessionVolume(session.id.clone(), value));
            }
            y += 46.0;
        }

        // Devices
        let mut y = devices.top();
        y = self.device_list(ui, devices, y, &t("audio.output"), &state.outputs, icons::SPEAKER_HIGH);
        y += 6.0;
        let _ = self.device_list(ui, devices, y, &t("audio.input"), &state.inputs, icons::MICROPHONE);
    }

    fn device_list(&mut self, ui: &mut Ui, area: Rect, mut y: f32, title: &str, devices: &[AudioDeviceInfo], icon: &str) -> f32 {
        text_clipped(ui, pos2(area.left() + 4.0, y + 8.0), Align2::LEFT_CENTER, title, theme::semibold(11.5), theme::DIM, area.width());
        y += 20.0;
        if devices.is_empty() {
            text_clipped(ui, pos2(area.left() + 4.0, y + 12.0), Align2::LEFT_CENTER, &t("control.noDevices"), theme::regular(11.5), theme::FAINT, area.width());
            return y + 26.0;
        }
        for (index, device) in devices.iter().take(5).enumerate() {
            let row = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), 32.0));
            let response = ui.interact(row, ui.id().with((title, index)), Sense::click());
            let fill = if device.is_default {
                theme::alpha(theme::ACCENT, 0.35)
            } else if response.hovered() {
                theme::CARD_HOVER
            } else {
                theme::CARD
            };
            ui.painter().rect_filled(row, rounded(9.0), fill);
            ui.painter().text(row.left_center() + vec2(16.0, 0.0), Align2::CENTER_CENTER, icon, theme::icon(13.0), theme::DIM);
            text_clipped(ui, row.left_center() + vec2(30.0, 0.0), Align2::LEFT_CENTER, &device.name, theme::regular(11.5), theme::WHITE, row.width() - 54.0);
            if device.is_default {
                ui.painter().text(row.right_center() - vec2(14.0, 0.0), Align2::CENTER_CENTER, icons::CHECK, theme::icon(12.0), theme::WHITE);
            }
            if response.clicked() && !device.is_default {
                self.hub.audio.send(AudioCommand::SetDefaultDevice(device.id.clone()));
                self.hub.play(Tone::Tap);
            }
            y += 36.0;
        }
        y
    }
}
