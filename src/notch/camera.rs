//! The camera tab: a mirror for a quick look at yourself. Only runs while the tab is open.

use super::kit::{self, rounded};
use super::Notch;
use crate::i18n::t;
use crate::icons;
use crate::settings::Settings;
use crate::theme;
use eframe::egui::{Align2, Color32, Rect, Ui, vec2};

#[derive(Default)]
pub struct CameraUi {
    pub retry: u32,
}

impl Notch {
    pub(super) fn camera_view(&mut self, ui: &mut Ui, body: Rect, _settings: &Settings) {
        kit::card(ui, body, 16.0, theme::CARD);
        ui.painter().rect_filled(body.shrink(1.0), rounded(15.0), Color32::from_rgb(12, 12, 14));
        let center = body.center();
        ui.painter().text(center - vec2(0.0, 26.0), Align2::CENTER_CENTER, icons::CAMERA, theme::icon(34.0), theme::FAINT);
        ui.painter().text(center + vec2(0.0, 8.0), Align2::CENTER_CENTER, t("notch.camera.error"), theme::medium(13.0), theme::DIM);
        ui.painter().text(center + vec2(0.0, 28.0), Align2::CENTER_CENTER, t("port.camera.hint"), theme::regular(11.5), theme::FAINT);
    }
}
