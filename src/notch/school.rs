//! The original's home and school tabs (ControlCenterView.Pi.cs / .Home.cs). Both sit on the same
//! drifting colour fields as the controls tab, each in its own muted colour: home in night indigo,
//! school in a colour for how soon things are due. The original reads both from its author's
//! Raspberry Pi; here the school tab reads the school's Moodle directly. Without a data source a
//! tab shows the original's one muted status row.

use super::controls::paint_backdrop;
use super::kit::{lerp_color, text_clipped};
use super::weather::weekday_short;
use super::{Notch, View, glyph};
use crate::i18n::{t, tf};
use crate::icons;
use crate::services::moodle::{Deadline, MoodleStatus, short_course};
use crate::settings::Settings;
use crate::theme;
use chrono::{Datelike, Local, Timelike};
use eframe::egui::{self, Align2, Color32, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};

/// HomeNight in the original: the room with every lamp off.
const HOME_NIGHT: [Color32; 3] = [Color32::from_rgb(0x7C, 0x7F, 0xB8), Color32::from_rgb(0xA1, 0x9B, 0xD2), Color32::from_rgb(0x5F, 0x7F, 0xA8)];
/// How soon things are due sets the colour: sage while there is time, sand within three days, rosé within a day.
const SCHOOL_CALM: [Color32; 3] = [Color32::from_rgb(0x86, 0xB6, 0xB0), Color32::from_rgb(0x9B, 0xB0, 0xCC), Color32::from_rgb(0x7F, 0xA6, 0xCC)];
const SCHOOL_SOON: [Color32; 3] = [Color32::from_rgb(0xD2, 0xAE, 0x7C), Color32::from_rgb(0xD2, 0x95, 0x7E), Color32::from_rgb(0x86, 0xB6, 0xB0)];
const SCHOOL_NOW: [Color32; 3] = [Color32::from_rgb(0xD2, 0x85, 0x7E), Color32::from_rgb(0xC9, 0x8A, 0xB0), Color32::from_rgb(0xD2, 0xAE, 0x7C)];
const DUE_SOON: Color32 = Color32::from_rgb(0xD2, 0xAE, 0x7C);
const DUE_TODAY: Color32 = Color32::from_rgb(0xD2, 0x85, 0x7E);
const MUTED: Color32 = Color32::from_gray(0x8C);
const TRACK: Color32 = Color32::from_gray(0x4D);
const HEADER_TEXT: Color32 = Color32::from_gray(0x8A);
/// The hero ring: 84 px, a 7 px stroke, full at two weeks or more.
const HERO_RING: f32 = 84.0;
const HERO_STROKE: f32 = 7.0;
const HERO_SPAN_HOURS: f32 = 14.0 * 24.0;
/// Rows after the hero, as in the original.
const MAX_DEADLINES: usize = 4;
const ROW_H: f32 = 34.0;
const ROW_STEP: f32 = 42.0;
const ACTION_X: f32 = 196.0;

/// The status line the home tab shows while it has nothing else, as the original picks it.
pub fn status_key(view: View, settings: &Settings) -> &'static str {
    if settings.enable_local_only_mode {
        "pi.localOnly"
    } else if view == View::Home && !settings.pi_dashboard_url.trim().is_empty() {
        "port.pi.unsupported"
    } else {
        "pi.notConfigured"
    }
}

/// The colours for what is due next.
fn school_mood(hours_left: Option<f32>) -> [Color32; 3] {
    match hours_left {
        Some(hours) if hours < 24.0 => SCHOOL_NOW,
        Some(hours) if hours < 72.0 => SCHOOL_SOON,
        _ => SCHOOL_CALM,
    }
}

/// "Today 23:59", "Tomorrow 08:00", "In 3 days".
fn due_text(deadline: &Deadline, now: chrono::DateTime<Local>) -> String {
    let at = format!("{:02}:{:02}", deadline.due.hour(), deadline.due.minute());
    let days = (deadline.due.date_naive() - now.date_naive()).num_days();
    match days {
        0 => tf("pi.dueToday", &[&at]),
        1 => tf("pi.dueTomorrow", &[&at]),
        _ => {
            let left = (deadline.due - now).num_hours() as f32 / 24.0;
            tf("pi.dueInDays", &[&(left.ceil() as i64).max(1).to_string()])
        }
    }
}

fn hours_left(deadline: &Deadline, now: chrono::DateTime<Local>) -> f32 {
    (deadline.due - now).num_minutes() as f32 / 60.0
}

impl Notch {
    pub(super) fn pi_view(&mut self, ui: &mut Ui, body: Rect, notch: Rect, settings: &Settings) {
        if self.view == View::Home {
            paint_backdrop(ui, notch, HOME_NIGHT);
            status_row(ui, body, &t(status_key(View::Home, settings)));
            return;
        }
        let status = self.hub.moodle.status.lock().map(|s| s.clone()).unwrap_or_default();
        let now = Local::now();
        let open: Vec<Deadline> = match &status {
            MoodleStatus::Ready(list) => list.iter().filter(|d| d.due >= now && !d.overdue).cloned().collect(),
            _ => Vec::new(),
        };
        let mood = school_mood(open.first().map(|d| hours_left(d, now)));
        paint_backdrop(ui, notch, mood);

        let message = match &status {
            _ if settings.enable_local_only_mode => Some(t("pi.localOnly")),
            MoodleStatus::NotConfigured => Some(t("port.school.noMoodle")),
            MoodleStatus::NoToken => Some(t("port.school.noToken")),
            MoodleStatus::Loading => Some(t("pi.loading")),
            MoodleStatus::Failed(error) => Some(tf("port.school.failed", &[error])),
            MoodleStatus::Ready(_) => None,
        };
        if let Some(message) = message {
            status_row(ui, body, &message);
            return;
        }
        let Some(next) = open.first() else {
            icon_row(ui, body.top() + 4.0, body.left(), icons::CHECK, &t("school.nothingDue"), MUTED, None);
            return;
        };
        self.deadline_hero(ui, body, next, now, mood[0]);

        let mut y = body.top() + HERO_RING + 6.0 + 12.0;
        if open.len() > 1 {
            section_title(ui, body.left(), y, &t("school.deadlines"));
            y += 18.0 + 6.0;
            for deadline in open.iter().skip(1).take(MAX_DEADLINES) {
                let left = hours_left(deadline, now);
                let urgency = if left < 24.0 {
                    DUE_TODAY
                } else if left < 72.0 {
                    DUE_SOON
                } else {
                    MUTED
                };
                let status = format!("{} · {}", short_course(&deadline.course), due_text(deadline, now));
                let row = icon_row(ui, y + 4.0, body.left(), icons::CALENDAR_BLANK, &deadline.name, Color32::WHITE, Some((&status, urgency)));
                link(ui, row, &deadline.url, ("deadline", deadline.id));
                y += ROW_STEP;
                if y + ROW_H > body.bottom() {
                    break;
                }
            }
        }
    }

    /// The next deadline, big: a ring that empties as the due date comes closer with the days (or hours)
    /// left in it, the task's name, and its course and date in the tab's colour.
    fn deadline_hero(&mut self, ui: &mut Ui, body: Rect, deadline: &Deadline, now: chrono::DateTime<Local>, mood: Color32) {
        let left = hours_left(deadline, now);
        let share = (left / HERO_SPAN_HOURS).clamp(0.02, 1.0);
        let center = pos2(body.left() + HERO_RING / 2.0, body.top() + HERO_RING / 2.0);
        let radius = (HERO_RING - HERO_STROKE) / 2.0;
        ui.painter().circle_stroke(center, radius, Stroke::new(HERO_STROKE, theme::alpha(TRACK, 0.6)));
        // The ring sweeps to its share when the tab opens.
        let sweep = ui.ctx().animate_value_with_time(ui.id().with(("hero", deadline.id)), share, 1.1);
        arc(ui, center, radius, sweep, Stroke::new(HERO_STROKE, mood));
        let (number, unit) = if left < 24.0 {
            ((left.max(0.0) as i64).to_string(), t("school.hours"))
        } else {
            (((left / 24.0).ceil() as i64).to_string(), t("school.days"))
        };
        ui.painter().text(center - vec2(0.0, 6.0), Align2::CENTER_CENTER, number, theme::bold(20.0), Color32::WHITE);
        ui.painter().text(center + vec2(0.0, 12.0), Align2::CENTER_CENTER, unit, theme::bold(10.0), MUTED);

        let x = body.left() + HERO_RING + 16.0;
        let width = body.right() - x;
        let title = text_clipped(ui, pos2(x, center.y - 10.0), Align2::LEFT_CENTER, &deadline.name, theme::bold(16.0), Color32::WHITE, width);
        let day = weekday_short(deadline.due.weekday());
        let meta = format!(
            "{} · {} {}.{}. · {:02}:{:02}",
            short_course(&deadline.course),
            day,
            deadline.due.day(),
            deadline.due.month(),
            deadline.due.hour(),
            deadline.due.minute()
        );
        text_clipped(ui, pos2(x, center.y + 12.0), Align2::LEFT_CENTER, &meta, theme::bold(12.0), lerp_color(mood, Color32::WHITE, 0.35), width);
        link(ui, title.union(Rect::from_center_size(center, vec2(HERO_RING, HERO_RING))), &deadline.url, ("hero-link", deadline.id));
    }
}

/// Opens Moodle's page of the item on a click.
fn link(ui: &mut Ui, rect: Rect, url: &str, id: impl std::hash::Hash + std::fmt::Debug) {
    if url.is_empty() {
        return;
    }
    let response = ui.interact(rect, ui.id().with(id), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        let _ = open::that_detached(url);
    }
}

/// An arc from twelve o'clock clockwise over `share` of the circle, with round ends.
fn arc(ui: &Ui, center: egui::Pos2, radius: f32, share: f32, stroke: Stroke) {
    let steps = ((share * 96.0).ceil() as usize).max(2);
    let sweep = share.min(0.999) * std::f32::consts::TAU;
    let points: Vec<_> = (0..=steps)
        .map(|i| {
            let angle = -std::f32::consts::FRAC_PI_2 + sweep * i as f32 / steps as f32;
            center + vec2(angle.cos(), angle.sin()) * radius
        })
        .collect();
    let (first, last) = (points[0], points[points.len() - 1]);
    ui.painter().add(Shape::line(points, stroke));
    ui.painter().circle_filled(first, stroke.width / 2.0, stroke.color);
    ui.painter().circle_filled(last, stroke.width / 2.0, stroke.color);
}

/// "▾ Deadlines" in the original's section style (always open here).
fn section_title(ui: &Ui, left: f32, top: f32, title: &str) {
    let y = top + 9.5;
    let c = pos2(left + 4.0, y);
    ui.painter().add(Shape::convex_polygon(vec![c + vec2(-4.0, -2.5), c + vec2(4.0, -2.5), c + vec2(0.0, 2.5)], HEADER_TEXT, Stroke::NONE));
    ui.painter().text(pos2(left + 16.0, y), Align2::LEFT_CENTER, title, theme::bold(15.0), Color32::WHITE);
}

/// A row of the original's grid: icon and name in the first column, a status in the second.
fn icon_row(ui: &Ui, top: f32, left: f32, icon: &str, name: &str, color: Color32, status: Option<(&str, Color32)>) -> Rect {
    let rect = Rect::from_min_size(pos2(left, top), vec2(680.0, ROW_H));
    let y = rect.center().y;
    ui.painter().text(pos2(left + 8.0, y), Align2::CENTER_CENTER, icon, theme::icon(16.0), color);
    let name_width = if status.is_some() { ACTION_X - 34.0 } else { 600.0 };
    text_clipped(ui, pos2(left + 26.0, y), Align2::LEFT_CENTER, name, theme::bold(13.5), color, name_width);
    if let Some((text, tint)) = status {
        text_clipped(ui, pos2(left + ACTION_X, y), Align2::LEFT_CENTER, text, theme::bold(12.0), tint, 460.0);
    }
    rect
}

/// A muted row: the warning triangle and one line of grey text (the original's NewRow(muted: true)).
fn status_row(ui: &Ui, body: Rect, text: &str) {
    let muted = Color32::from_rgb(0x9A, 0x9A, 0x9F);
    let y = body.top() + 21.0;
    glyph::paint(ui.painter(), pos2(body.left() + 7.5, y), 15.0, glyph::WARNING, muted);
    text_clipped(ui, pos2(body.left() + 26.0, y), Align2::LEFT_CENTER, text, theme::bold(13.5), muted, body.width() - 30.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn due_in(hours: i64, now: chrono::DateTime<Local>) -> Deadline {
        Deadline { id: 1, name: "HÜ".into(), course: "DBI 3AHIT".into(), due: now + chrono::Duration::hours(hours), url: String::new(), module: "assign".into(), overdue: false }
    }

    #[test]
    fn status_follows_the_settings() {
        let mut settings = Settings::default();
        assert_eq!(status_key(View::Home, &settings), "pi.notConfigured");
        settings.pi_dashboard_url = "http://100.1.2.3:4000".into();
        assert_eq!(status_key(View::Home, &settings), "port.pi.unsupported");
        settings.enable_local_only_mode = true;
        assert_eq!(status_key(View::Home, &settings), "pi.localOnly");
    }

    #[test]
    fn colour_follows_what_is_due() {
        assert_eq!(school_mood(None), SCHOOL_CALM);
        assert_eq!(school_mood(Some(200.0)), SCHOOL_CALM);
        assert_eq!(school_mood(Some(48.0)), SCHOOL_SOON);
        assert_eq!(school_mood(Some(3.0)), SCHOOL_NOW);
    }

    #[test]
    fn due_texts() {
        crate::i18n::set_language("en");
        let now = Local::now().date_naive().and_hms_opt(9, 0, 0).unwrap().and_local_timezone(Local).single().unwrap();
        assert!(due_text(&due_in(3, now), now).starts_with("Today"));
        assert!(due_text(&due_in(24, now), now).starts_with("Tomorrow"));
        assert_eq!(due_text(&due_in(24 * 4, now), now), "In 4 days");
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
