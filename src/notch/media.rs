//! The media tab: cover, transport, progress, lyrics. Without a player it turns into clock and calendar.

use super::kit::{self, rounded, text_clipped};
use super::{Notch, format_clock, volume_icon};
use crate::i18n::t;
use crate::icons;
use crate::platform::{MediaCommand, MediaInfo};
use crate::services::audio::AudioCommand;
use crate::services::media::LyricsState;
use crate::services::sounds::Tone;
use crate::services::weather_service::WeatherStatus;
use crate::settings::Settings;
use crate::theme;
use chrono::{Datelike, Local, Timelike};
use eframe::egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use std::time::Duration;

impl Notch {
    pub(super) fn media_view(&mut self, ui: &mut Ui, body: Rect, settings: &Settings) {
        let (info, lyrics, accent) = match self.hub.media.state.lock() {
            Ok(state) => (state.info.clone(), state.lyrics.clone(), state.art.as_ref().map(|art| art.accent)),
            Err(_) => return,
        };
        let Some(info) = info else {
            self.idle_home(ui, body, settings);
            return;
        };
        let accent = accent.unwrap_or(theme::WHITE);
        let art_side = body.height().min(132.0);
        let art = Rect::from_min_size(body.min, vec2(art_side, art_side));
        match self.current_art_textures(ui.ctx()) {
            Some((cover, _)) => {
                ui.painter().add(
                    eframe::egui::epaint::RectShape::filled(art, rounded(14.0), Color32::WHITE)
                        .with_texture(cover.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
                );
            }
            None => {
                ui.painter().rect_filled(art, rounded(14.0), theme::CARD_HOVER);
                ui.painter().text(art.center(), Align2::CENTER_CENTER, icons::MUSIC_NOTES, theme::icon(38.0), theme::FAINT);
            }
        }

        let area = Rect::from_min_max(pos2(art.right() + 16.0, body.top()), body.max);
        if self.lyrics_open {
            self.lyrics_panel(ui, area, &info, &lyrics, accent);
        } else {
            self.track_panel(ui, area, &info, accent);
        }

        let has_lyrics = matches!(lyrics, LyricsState::Found(_));
        if has_lyrics || self.lyrics_open {
            let button = Rect::from_center_size(pos2(body.right() - 12.0, body.top() + 12.0), vec2(24.0, 24.0));
            let color = if self.lyrics_open { accent } else { theme::DIM };
            let response = kit::icon_button(ui, button, icons::TEXT_AA, 14.0, color, "lyrics").on_hover_text(t("port.media.lyrics"));
            if response.clicked() {
                self.lyrics_open = !self.lyrics_open;
                self.hub.play(Tone::Tap);
            }
        }
    }

    fn track_panel(&mut self, ui: &mut Ui, area: Rect, info: &MediaInfo, accent: Color32) {
        let time = ui.input(|i| i.time);
        let title_rect = Rect::from_min_size(pos2(area.left(), area.top() + 2.0), vec2(area.width() - 30.0, 22.0));
        kit::marquee(ui, title_rect, &info.title, theme::semibold(16.0), theme::WHITE, time);
        let artist = if info.artist.is_empty() { info.source_app.as_str() } else { info.artist.as_str() };
        let artist_rect = Rect::from_min_size(pos2(area.left(), area.top() + 25.0), vec2(area.width() - 30.0, 18.0));
        kit::marquee(ui, artist_rect, artist, theme::regular(12.5), theme::DIM, time);

        // Progress
        let position = info.current_position();
        let duration = info.duration;
        let bar = Rect::from_min_size(pos2(area.left(), area.top() + 58.0), vec2(area.width(), 5.0));
        let ratio = if duration.is_zero() { 0.0 } else { position.as_secs_f32() / duration.as_secs_f32() };
        if let Some(value) = kit::slider(ui, bar, ratio, accent, "seek")
            && info.can_seek
            && !duration.is_zero()
            && ui.input(|i| i.pointer.any_released())
        {
            self.hub.media.send(MediaCommand::Seek(duration.mul_f32(value)));
        }
        let times_y = bar.bottom() + 11.0;
        ui.painter().text(pos2(bar.left(), times_y), Align2::LEFT_CENTER, format_clock(position), theme::regular(10.5), theme::FAINT);
        if !duration.is_zero() {
            ui.painter().text(pos2(bar.right(), times_y), Align2::RIGHT_CENTER, format_clock(duration), theme::regular(10.5), theme::FAINT);
        }

        // Transport
        let y = area.top() + 100.0;
        let center = area.left() + 62.0;
        let controls: [(f32, &str, f32, MediaCommand, &str); 3] = [
            (center - 42.0, icons::SKIP_BACK, 18.0, MediaCommand::Previous, "media.previous"),
            (center, if info.playing { icons::PAUSE } else { icons::PLAY }, 24.0, MediaCommand::PlayPause, if info.playing { "media.pause" } else { "media.play" }),
            (center + 42.0, icons::SKIP_FORWARD, 18.0, MediaCommand::Next, "media.next"),
        ];
        for (x, icon, size, command, tip) in controls {
            let side = if size > 20.0 { 38.0 } else { 32.0 };
            let rect = Rect::from_center_size(pos2(x, y), vec2(side, side));
            let response = kit::icon_button_fill(ui, rect, icon, size, theme::WHITE, tip).on_hover_text(t(tip));
            if response.clicked() {
                self.hub.media.send(command);
                self.hub.play(Tone::Click);
            }
        }

        // Volume
        let master = self.hub.audio.state.lock().ok().and_then(|s| s.master);
        if let Some(master) = master {
            let icon_rect = Rect::from_center_size(pos2(area.left() + 150.0, y), vec2(26.0, 26.0));
            let response = kit::icon_button_fill(ui, icon_rect, volume_icon(master.level, master.muted), 15.0, theme::DIM, "mute").on_hover_text(t("media.mute"));
            if response.clicked() {
                self.hub.audio.send(AudioCommand::SetMasterMute(!master.muted));
            }
            let bar_rect = Rect::from_min_max(pos2(icon_rect.right() + 4.0, y - 2.5), pos2(area.right() - 2.0, y + 2.5));
            if bar_rect.width() > 40.0
                && let Some(value) = kit::slider(ui, bar_rect, if master.muted { 0.0 } else { master.level }, theme::WHITE, "volume")
            {
                self.hub.audio.send(AudioCommand::SetMaster(value));
            }
        }
    }

    fn lyrics_panel(&mut self, ui: &mut Ui, area: Rect, info: &MediaInfo, lyrics: &LyricsState, accent: Color32) {
        let clip = ui.painter().with_clip_rect(area);
        match lyrics {
            LyricsState::Found(found) => {
                let position = info.current_position();
                let index = found.line_at(position + Duration::from_millis(200));
                let line_height = 25.0;
                let center_y = area.center().y;
                // Smooth scroll toward the current line.
                let current = index.map_or(-1.0, |i| i as f32);
                let scroll = ui.ctx().animate_value_with_time(ui.id().with("lyric-scroll"), current, 0.28);
                let first = (scroll - 3.0).floor().max(0.0) as usize;
                for (row, line) in found.lines.iter().enumerate().skip(first).take(8) {
                    let distance = row as f32 - scroll;
                    let y = center_y + distance * line_height;
                    if y < area.top() - line_height || y > area.bottom() + line_height {
                        continue;
                    }
                    let focus = (1.0 - distance.abs() / 2.5).clamp(0.0, 1.0);
                    let is_current = Some(row) == index;
                    let color = if is_current { accent } else { theme::alpha(theme::WHITE, 0.22 + 0.4 * focus) };
                    let font = if is_current { theme::semibold(15.5) } else { theme::regular(13.5) };
                    let text = if line.text.is_empty() { "♪" } else { line.text.as_str() };
                    let mut job = eframe::egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
                    job.wrap = eframe::egui::text::TextWrapping::truncate_at_width(area.width() - 30.0);
                    let galley = clip.layout_job(job);
                    clip.galley(pos2(area.left(), y - galley.size().y / 2.0), galley, color);
                }
            }
            LyricsState::Searching => {
                ui.painter().text(area.center(), Align2::CENTER_CENTER, t("lyrics.searching"), theme::regular(13.0), theme::DIM);
            }
            _ => {
                ui.painter().text(area.center(), Align2::CENTER_CENTER, t("port.media.noLyrics"), theme::regular(13.0), theme::DIM);
            }
        }
    }

    fn clock_face(&self, ui: &Ui, area: Rect, style: &str) {
        let now = Local::now();
        match style {
            "digital" => {
                ui.painter().text(area.center(), Align2::CENTER_CENTER, now.format("%H:%M").to_string(), theme::bold(44.0), theme::WHITE);
            }
            "word" => {
                let text = word_clock(now.hour(), now.minute());
                kit::text_wrapped(ui, area.min + vec2(0.0, 20.0), &text, theme::semibold(17.0), theme::WHITE, area.width(), 4);
            }
            _ => analog_clock(ui, area.center(), area.height().min(area.width()) / 2.0 - 2.0, now.hour12().1, now.minute(), now.second()),
        }
    }

    /// Nothing is playing: time, date, calendar and the weather in one line.
    fn idle_home(&mut self, ui: &mut Ui, body: Rect, settings: &Settings) {
        let side = body.height().min(134.0);
        let clock_area = Rect::from_min_size(body.min, vec2(side, side));
        self.clock_face(ui, clock_area, &settings.clock_page_style);
        let now = Local::now();
        let text_left = clock_area.right() + 18.0;
        ui.painter().text(pos2(text_left, body.top() + 16.0), Align2::LEFT_CENTER, now.format("%H:%M").to_string(), theme::bold(26.0), theme::WHITE);
        let date = format_date(&now);
        text_clipped(ui, pos2(text_left, body.top() + 44.0), Align2::LEFT_CENTER, &date, theme::regular(12.5), theme::DIM, 150.0);
        if let Ok(WeatherStatus::Ready(forecast)) = self.hub.weather.status.lock().map(|s| s.clone()) {
            let icon = crate::services::weather::condition_icon(forecast.code, forecast.is_day);
            ui.painter().text(pos2(text_left + 9.0, body.top() + 76.0), Align2::CENTER_CENTER, icon, theme::icon(18.0), theme::WHITE);
            ui.painter().text(
                pos2(text_left + 26.0, body.top() + 76.0),
                Align2::LEFT_CENTER,
                format!("{:.0}°  {}", forecast.temperature, forecast.place),
                theme::medium(12.5),
                theme::DIM,
            );
        }
        ui.painter().text(pos2(text_left, body.top() + 108.0), Align2::LEFT_CENTER, t("port.media.nothingPlaying"), theme::regular(12.0), theme::FAINT);

        let calendar = Rect::from_min_max(pos2(body.right() - 168.0, body.top()), body.max);
        month_calendar(ui, calendar, now.date_naive());
    }
}

fn format_date(now: &chrono::DateTime<Local>) -> String {
    const DAYS: [&str; 7] = ["port.day.mon", "port.day.tue", "port.day.wed", "port.day.thu", "port.day.fri", "port.day.sat", "port.day.sun"];
    let day = t(DAYS[now.weekday().num_days_from_monday() as usize]);
    format!("{day}, {}", now.format("%d.%m.%Y"))
}

fn analog_clock(ui: &Ui, center: Pos2, radius: f32, hour: u32, minute: u32, second: u32) {
    let painter = ui.painter();
    painter.circle_filled(center, radius, theme::CARD);
    for tick in 0..60_u32 {
        let major = tick % 5 == 0;
        let angle = tick as f32 / 60.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        let outer = center + Vec2::angled(angle) * (radius - 4.0);
        let inner = center + Vec2::angled(angle) * (radius - if major { 11.0 } else { 7.0 });
        let color = if major { theme::WHITE } else { theme::FAINT };
        painter.line_segment([inner, outer], Stroke::new(if major { 1.6 } else { 0.8 }, color));
    }
    let hand = |fraction: f32, length: f32, width: f32, color: Color32| {
        let angle = fraction * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        painter.line_segment([center, center + Vec2::angled(angle) * length], Stroke::new(width, color));
    };
    let minutes = minute as f32 + second as f32 / 60.0;
    hand((hour as f32 + minutes / 60.0) / 12.0, radius * 0.5, 3.2, theme::WHITE);
    hand(minutes / 60.0, radius * 0.74, 2.2, theme::WHITE);
    hand(second as f32 / 60.0, radius * 0.82, 1.0, theme::ORANGE);
    painter.circle_filled(center, 3.0, theme::ORANGE);
}

fn month_calendar(ui: &Ui, area: Rect, today: chrono::NaiveDate) {
    const HEAD: [&str; 7] = ["M", "D", "M", "D", "F", "S", "S"];
    let cell = vec2(area.width() / 7.0, 18.0);
    let top = area.top() + 12.0;
    for (index, label) in HEAD.iter().enumerate() {
        let x = area.left() + cell.x * (index as f32 + 0.5);
        ui.painter().text(pos2(x, top), Align2::CENTER_CENTER, *label, theme::medium(10.0), theme::FAINT);
    }
    let Some(first) = today.with_day(1) else { return };
    let offset = first.weekday().num_days_from_monday() as i32;
    for day in 1..=days_in_month(today.year(), today.month()) {
        let index = offset + day as i32 - 1;
        let (row, column) = (index / 7, index % 7);
        let center = pos2(area.left() + cell.x * (column as f32 + 0.5), top + 19.0 + row as f32 * cell.y);
        let is_today = day == today.day();
        if is_today {
            ui.painter().circle_filled(center, 8.5, theme::RED);
        }
        let color = if is_today { Color32::WHITE } else if column >= 5 { theme::FAINT } else { theme::DIM };
        ui.painter().text(center, Align2::CENTER_CENTER, day.to_string(), theme::regular(10.5), color);
    }
    let _ = ui.interact(area, ui.id().with("calendar"), Sense::hover());
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    let next = if month == 12 { chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1) } else { chrono::NaiveDate::from_ymd_opt(year, month + 1, 1) };
    next.and_then(|date| date.pred_opt()).map_or(30, |date| date.day())
}

/// A word clock in German: "Es ist fünf nach drei".
pub fn word_clock(hour: u32, minute: u32) -> String {
    const HOURS: [&str; 12] = ["zwölf", "eins", "zwei", "drei", "vier", "fünf", "sechs", "sieben", "acht", "neun", "zehn", "elf"];
    let rounded = (minute + 2) / 5 * 5;
    let hour = hour + u32::from(rounded >= 25);
    let name = |h: u32| HOURS[(h % 12) as usize];
    let text = match rounded {
        0 | 60 => {
            // A rounded 60 already moved `hour` on above.
            let spoken = if hour % 12 == 1 { "ein" } else { name(hour) };
            format!("{spoken} Uhr")
        }
        5 => format!("fünf nach {}", name(hour)),
        10 => format!("zehn nach {}", name(hour)),
        15 => format!("Viertel nach {}", name(hour)),
        20 => format!("zwanzig nach {}", name(hour)),
        25 => format!("fünf vor halb {}", name(hour)),
        30 => format!("halb {}", name(hour)),
        35 => format!("fünf nach halb {}", name(hour)),
        40 => format!("zwanzig vor {}", name(hour)),
        45 => format!("Viertel vor {}", name(hour)),
        50 => format!("zehn vor {}", name(hour)),
        _ => format!("fünf vor {}", name(hour)),
    };
    format!("Es ist {text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_lengths_include_leap_years() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 12), 31);
        assert_eq!(days_in_month(2026, 11), 30);
    }

    #[test]
    fn word_clock_reads_like_german() {
        assert_eq!(word_clock(3, 0), "Es ist drei Uhr");
        assert_eq!(word_clock(1, 0), "Es ist ein Uhr");
        assert_eq!(word_clock(15, 5), "Es ist fünf nach drei");
        assert_eq!(word_clock(3, 15), "Es ist Viertel nach drei");
        assert_eq!(word_clock(3, 30), "Es ist halb vier");
        assert_eq!(word_clock(3, 45), "Es ist Viertel vor vier");
        assert_eq!(word_clock(3, 58), "Es ist vier Uhr");
        assert_eq!(word_clock(11, 58), "Es ist zwölf Uhr");
        assert_eq!(word_clock(12, 40), "Es ist zwanzig vor eins");
    }
}
