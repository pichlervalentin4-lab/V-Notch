//! The camera tab. Looks like the original's: one dark card with a grey camera in the middle. The
//! live mirror is not ported yet, so a click shows the original's "cannot detect" overlay.

use super::Notch;
use super::glyph;
use super::kit::rounded;
use crate::i18n::t;
use crate::settings::Settings;
use crate::theme;
use eframe::egui::{Align2, Color32, Rect, Sense, Ui, vec2};

#[derive(Default)]
pub struct CameraUi {
    pub retry: u32,
}

impl Notch {
    pub(super) fn camera_view(&mut self, ui: &mut Ui, body: Rect, _settings: &Settings) {
        // 488 wide and centred, a #1A1A1A card with 12-point corners.
        let card = Rect::from_center_size(body.center(), vec2(body.width().min(488.0), body.height()));
        let response = ui.interact(card, ui.id().with("camera-card"), Sense::click());
        let press = ui.ctx().animate_bool_with_time(response.id.with("press"), response.is_pointer_button_down_on(), 0.1);
        let card = card.shrink(press * 3.0);
        let painter = ui.painter();
        painter.rect_filled(card, rounded(12.0), Color32::from_gray(0x1A));
        glyph::paint(painter, card.center() - vec2(0.0, 2.0), 28.0, glyph::CAMERA_LENS, Color32::from_gray(0x55));
        if response.clicked() {
            self.camera_ui.retry += 1;
        }
        if self.camera_ui.retry > 0 {
            let painter = ui.painter();
            painter.rect_filled(card, rounded(12.0), theme::alpha(Color32::BLACK, 0x60 as f32 / 255.0));
            let center = card.center();
            glyph::paint(painter, center - vec2(0.0, 18.0), 18.0, glyph::WARNING, theme::alpha(Color32::WHITE, 0x73 as f32 / 255.0));
            painter.text(center + vec2(0.0, 2.0), Align2::CENTER_CENTER, t("notch.camera.error"), theme::regular(11.0), Color32::WHITE);
            painter.text(center + vec2(0.0, 17.0), Align2::CENTER_CENTER, t("notch.camera.retry"), theme::regular(10.5), theme::alpha(Color32::WHITE, 0.6));
        }
    }
}
