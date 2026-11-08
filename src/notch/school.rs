//! The original's home and school tabs (ControlCenterView.Pi.cs / .Home.cs). Both sit on the same
//! drifting colour fields as the controls tab, each in its own muted colour: home in night indigo,
//! school in calm sage. Without a data source they show the original's one muted status row.

use super::controls::paint_backdrop;
use super::{Notch, View, glyph};
use crate::i18n::t;
use crate::settings::Settings;
use crate::theme;
use eframe::egui::{Align2, Color32, Rect, Ui, pos2};

/// HomeNight in the original: the room with every lamp off.
const HOME_NIGHT: [Color32; 3] = [Color32::from_rgb(0x7C, 0x7F, 0xB8), Color32::from_rgb(0xA1, 0x9B, 0xD2), Color32::from_rgb(0x5F, 0x7F, 0xA8)];
/// SchoolCalm in the original: nothing due soon.
const SCHOOL_CALM: [Color32; 3] = [Color32::from_rgb(0x86, 0xB6, 0xB0), Color32::from_rgb(0x9B, 0xB0, 0xCC), Color32::from_rgb(0x7F, 0xA6, 0xCC)];

/// The status line a tab shows while it has nothing else, as the original picks it.
pub fn status_key(view: View, settings: &Settings) -> &'static str {
    if settings.enable_local_only_mode {
        "pi.localOnly"
    } else if view == View::Home && !settings.pi_dashboard_url.trim().is_empty() {
        "port.pi.unsupported"
    } else {
        "pi.notConfigured"
    }
}

impl Notch {
    pub(super) fn pi_view(&mut self, ui: &mut Ui, body: Rect, notch: Rect, settings: &Settings) {
        let colors = if self.view == View::Home { HOME_NIGHT } else { SCHOOL_CALM };
        paint_backdrop(ui, notch, colors);
        status_row(ui, body, &t(status_key(self.view, settings)));
    }
}

/// A muted row: the warning triangle and one line of grey text (the original's NewRow(muted: true)).
fn status_row(ui: &Ui, body: Rect, text: &str) {
    let muted = Color32::from_rgb(0x9A, 0x9A, 0x9F);
    let y = body.top() + 21.0;
    glyph::paint(ui.painter(), pos2(body.left() + 7.5, y), 15.0, glyph::WARNING, muted);
    ui.painter().text(pos2(body.left() + 26.0, y), Align2::LEFT_CENTER, text, theme::bold(13.5), muted);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_follows_the_settings() {
        let mut settings = Settings::default();
        assert_eq!(status_key(View::School, &settings), "pi.notConfigured");
        settings.pi_dashboard_url = "http://100.1.2.3:4000".into();
        assert_eq!(status_key(View::Home, &settings), "port.pi.unsupported");
        settings.enable_local_only_mode = true;
        assert_eq!(status_key(View::Home, &settings), "pi.localOnly");
    }

    #[test]
    fn old_settings_get_the_new_tabs_after_claude() {
        let mut settings = Settings { settings_version: 1, ..Settings::default() };
        settings.nav_tab_order = "Media,Secondary,Timer,Claude,Weather,Legion,AudioMixer,Camera".into();
        settings.visible_nav_tabs = "Media,Claude,Weather".into();
        settings.sanitize();
        assert_eq!(settings.nav_tab_order, "Media,Secondary,Timer,Claude,Pi,School,Weather,Legion,AudioMixer,Camera");
        assert_eq!(settings.visible_nav_tabs, "Media,Claude,Pi,School,Weather");
    }
}
