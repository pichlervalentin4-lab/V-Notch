//! The media tab: cover, transport, progress, lyrics. Without a player it turns into clock and calendar.

use super::kit::{self, rounded, text_clipped};
use super::{Notch, format_clock, glyph, volume_icon};
use crate::anim::lerp;
use crate::i18n::t;
use crate::platform::{MediaCommand, MediaInfo};
use crate::services::audio::AudioCommand;
use crate::services::media::LyricsState;
use crate::services::sounds::Tone;
use crate::services::weather_service::WeatherStatus;
use crate::settings::Settings;
use crate::theme;
use chrono::{Datelike, Local, Timelike};
use eframe::egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use std::time::Duration;

/// Places in the open media tab, in points from the notch's top left corner, measured on the
/// original (480 Ã— 147): cover, text column, volume bar and the month-and-clock column.
mod place {
    pub const COVER: (f32, f32, f32) = (12.0, 31.0, 102.0);
    pub const TEXT_LEFT: f32 = 122.0;
    pub const TEXT_RIGHT: f32 = 289.0;
    pub const TITLE_BASELINE: f32 = 42.0;
    pub const ARTIST_BASELINE: f32 = 63.0;
    pub const BAR_Y: f32 = 83.5;
    pub const TIMES_BASELINE: f32 = 103.0;
    pub const CONTROLS_Y: f32 = 118.0;
    pub const VOLUME_X: f32 = 304.5;
    pub const VOLUME_TOP: f32 = 32.0;
    pub const VOLUME_BOTTOM: f32 = 116.0;
    pub const MUTE_Y: f32 = 130.0;
    pub const MONTH_X: f32 = 338.0;
    pub const CLOCK: (f32, f32, f32) = (412.0, 84.0, 46.0);
}

/// Where the content starts below the nav bar when nothing plays.
const NAV_BOTTOM: f32 = 38.0;

/// Inter's ascent is 0.969 em: the top of a galley sits this far above its baseline.
fn top_for_baseline(baseline: f32, size: f32) -> f32 {
    baseline - 0.969 * size
}

impl Notch {
    pub(super) fn media_view(&mut self, ui: &mut Ui, body: Rect, settings: &Settings) {
        let (info, lyrics, accent) = match self.hub.media.state.lock() {
            Ok(state) => (state.info.clone(), state.lyrics.clone(), state.art.as_ref().map(|art| art.accent)),
            Err(_) => return,
        };
        let Some(info) = info else {
            let inner = Rect::from_min_max(body.min + vec2(16.0, NAV_BOTTOM), body.max - vec2(16.0, 12.0));
            self.idle_home(ui, inner, settings);
            return;
        };
        // While it plays, bars and times take the colour of the artwork, like the original.
        let playing = ui.ctx().animate_bool_with_time(ui.id().with("media-playing"), info.playing, 0.35);
        let tint = kit::lerp_color(Color32::WHITE, accent.unwrap_or(Color32::WHITE), playing);
        let origin = body.min;
        let at = |x: f32, y: f32| origin + vec2(x, y);

        let (cover_x, cover_y, cover_side) = place::COVER;
        let cover = Rect::from_min_size(at(cover_x, cover_y), vec2(cover_side, cover_side));
        let textures = self.current_art_textures(ui.ctx());
        if let Some((_, blur)) = &textures
            && playing > 0.01
            && settings.show_media_art_background
        {
            let fade = playing * (1.0 - settings.media_blur_dark_overlay).clamp(0.0, 1.0);
            paint_art_glow(ui, blur.id(), body, 20.0, fade);
        }
        match &textures {
            Some((art, _)) => {
                ui.painter().add(
                    eframe::egui::epaint::RectShape::filled(cover, rounded(14.0), Color32::WHITE)
                        .with_texture(art.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
                );
            }
            None => glyph::paint(ui.painter(), cover.center(), 32.0, glyph::MUSIC, theme::alpha(Color32::WHITE, 0.4)),
        }

        self.track_panel(ui, origin, &info, tint);
        self.volume_column(ui, origin, tint);

        // Month and clock, or the lyrics in their place.
        let column = Rect::from_min_max(at(318.0, 31.0), at(472.0, 137.0));
        let response = ui.interact(column, ui.id().with("media-right"), Sense::click());
        let has_lyrics = matches!(lyrics, LyricsState::Found(_));
        if response.clicked() && (has_lyrics || self.lyrics_open) {
            self.lyrics_open = !self.lyrics_open;
            self.hub.play(Tone::Tap);
        }
        if self.lyrics_open {
            self.lyrics_panel(ui, column, &info, &lyrics, tint);
        } else {
            let now = Local::now();
            let month = month_short(now.month());
            let size = 15.0;
            let galley = ui.painter().layout_no_wrap(month, theme::bold(size), Color32::WHITE);
            let pos = pos2(origin.x + place::MONTH_X - galley.size().x / 2.0, origin.y + top_for_baseline(89.5, size));
            ui.painter().galley(pos, galley, Color32::WHITE);
            let (cx, cy, side) = place::CLOCK;
            analog_clock(ui, at(cx, cy), side - 4.0, &now);
        }
    }

    fn track_panel(&mut self, ui: &mut Ui, origin: Pos2, info: &MediaInfo, tint: Color32) {
        let time = ui.input(|i| i.time);
        let width = place::TEXT_RIGHT - place::TEXT_LEFT;
        let left = origin.x + place::TEXT_LEFT;

        // Title (14 bold) and artist (11 bold, 70 % white), each over a soft black copy one point lower.
        let title_top = origin.y + top_for_baseline(place::TITLE_BASELINE, 14.0);
        let title_rect = Rect::from_min_size(pos2(left, title_top), vec2(width, 18.0));
        kit::marquee(ui, title_rect.translate(vec2(0.0, 1.0)), &info.title, theme::bold(14.0), theme::alpha(Color32::BLACK, 0.5), time);
        let overflows = kit::marquee(ui, title_rect, &info.title, theme::bold(14.0), Color32::WHITE, time);
        if overflows {
            // The original fades the last fifth of the title out.
            let fade = Rect::from_min_max(pos2(title_rect.right() - width * 0.2, title_rect.top()), title_rect.max + vec2(1.0, 2.0));
            kit::horizontal_fade(ui, fade, Color32::TRANSPARENT, Color32::BLACK);
        }
        let artist = if info.artist.is_empty() { info.source_app.as_str() } else { info.artist.as_str() };
        let artist_rect = Rect::from_min_size(pos2(left, origin.y + top_for_baseline(place::ARTIST_BASELINE, 11.0)), vec2(width, 14.0));
        kit::marquee(ui, artist_rect, artist, theme::bold(11.0), theme::alpha(Color32::WHITE, 0.7), time);

        // Progress: a 4-point bar, filled from white to the tint, times below in 10.5 bold.
        let position = info.current_position();
        let duration = info.duration;
        let bar = Rect::from_min_max(pos2(left, origin.y + place::BAR_Y - 2.0), pos2(origin.x + place::TEXT_RIGHT, origin.y + place::BAR_Y + 2.0));
        let ratio = if duration.is_zero() { 0.0 } else { (position.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0) };
        let response = ui.interact(bar.expand2(vec2(0.0, 8.0)), ui.id().with("seek"), Sense::click_and_drag());
        let dragging = (response.dragged() || response.is_pointer_button_down_on()) && info.can_seek && !duration.is_zero();
        let shown = match (dragging, response.interact_pointer_pos()) {
            (true, Some(pointer)) => ((pointer.x - bar.left()) / bar.width()).clamp(0.0, 1.0),
            _ => ratio,
        };
        if dragging && ui.input(|i| i.pointer.any_released()) {
            self.hub.media.send(MediaCommand::Seek(duration.mul_f32(shown)));
        }
        let grow = ui.ctx().animate_bool_with_time(response.id.with("grow"), response.hovered() || dragging, 0.15);
        let bar = bar.expand2(vec2(0.0, grow));
        ui.painter().rect_filled(bar, rounded(bar.height() / 2.0), theme::alpha(Color32::WHITE, 0.157));
        if shown > 0.0 {
            let fill = Rect::from_min_size(bar.min, vec2((bar.width() * shown).max(bar.height()), bar.height()));
            kit::horizontal_gradient_rect(ui, fill, bar.height() / 2.0, tint, kit::lerp_color(theme::alpha(Color32::WHITE, 0.7), tint, 0.85));
        }
        let times_font = theme::bold(10.5);
        let times_color = kit::lerp_color(theme::alpha(Color32::WHITE, 0.7), tint, 0.9);
        let times_top = origin.y + top_for_baseline(place::TIMES_BASELINE, 10.5);
        ui.painter().text(pos2(bar.left(), times_top), Align2::LEFT_TOP, format_clock(position.min(duration.max(position))), times_font.clone(), times_color);
        if !duration.is_zero() {
            ui.painter().text(pos2(bar.right(), times_top), Align2::RIGHT_TOP, format_clock(duration), times_font, times_color);
        }

        // Previous, play or pause, next: spread over the progress bar's width.
        let y = origin.y + place::CONTROLS_Y;
        let play = if info.playing { (glyph::PAUSE, "media.pause", 11.0) } else { (glyph::PLAY, "media.play", 12.8) };
        let controls = [
            (bar.left() + 16.0, glyph::PREVIOUS, 15.0, MediaCommand::Previous, "media.previous"),
            (bar.center().x, play.0, play.2, MediaCommand::PlayPause, play.1),
            (bar.right() - 16.0, glyph::NEXT, 15.0, MediaCommand::Next, "media.next"),
        ];
        for (x, icon, size, command, tip) in controls {
            let rect = Rect::from_center_size(pos2(x, y), vec2(32.0, 26.0));
            let response = ui.interact(rect, ui.id().with(tip), Sense::click());
            let scale = if response.is_pointer_button_down_on() { 0.85 } else { 1.0 };
            let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.12);
            glyph::paint(ui.painter(), rect.center() + vec2(0.0, 1.2), size * scale, icon, theme::alpha(Color32::BLACK, 0.45));
            glyph::paint(ui.painter(), rect.center(), size * scale, icon, theme::alpha(Color32::WHITE, lerp(0.92, 1.0, hover)));
            if response.on_hover_text(t(tip)).clicked() {
                self.hub.media.send(command);
                self.hub.play(Tone::Click);
            }
        }
    }

    /// The original's vertical volume bar right of the controls, the mute button below it.
    fn volume_column(&mut self, ui: &mut Ui, origin: Pos2, tint: Color32) {
        let Some(master) = self.hub.audio.state.lock().ok().and_then(|s| s.master) else { return };
        let track = Rect::from_min_max(
            pos2(origin.x + place::VOLUME_X - 2.0, origin.y + place::VOLUME_TOP),
            pos2(origin.x + place::VOLUME_X + 2.0, origin.y + place::VOLUME_BOTTOM),
        );
        let response = ui.interact(track.expand2(vec2(8.0, 4.0)), ui.id().with("volume"), Sense::click_and_drag());
        if (response.dragged() || response.is_pointer_button_down_on())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let level = ((track.bottom() - pointer.y) / track.height()).clamp(0.0, 1.0);
            self.hub.audio.send(AudioCommand::SetMaster(level));
        }
        let level = if master.muted { 0.0 } else { master.level.clamp(0.0, 1.0) };
        ui.painter().rect_filled(track, rounded(2.0), Color32::from_gray(0x26));
        if level > 0.0 {
            let fill = Rect::from_min_max(pos2(track.left(), track.bottom() - track.height() * level), track.max);
            ui.painter().rect_filled(fill, rounded(2.0), tint);
        }
        let mute = Rect::from_center_size(pos2(track.center().x, origin.y + place::MUTE_Y), vec2(22.0, 22.0));
        let response = ui.interact(mute, ui.id().with("mute"), Sense::click());
        let color = theme::alpha(Color32::WHITE, if response.hovered() { 0.7 } else { 0.45 });
        glyph::paint(ui.painter(), mute.center(), 11.0, glyph::SPEAKER, color);
        if response.on_hover_text(t("media.mute")).clicked() {
            self.hub.audio.send(AudioCommand::SetMasterMute(!master.muted));
        }
        let _ = volume_icon;
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
                    let text = if line.text.is_empty() { "â™ª" } else { line.text.as_str() };
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
            _ => analog_clock(ui, area.center(), area.height().min(area.width()) / 2.0 - 4.0, &now),
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
                format!("{:.0}Â°  {}", forecast.temperature, forecast.place),
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

/// Short month name as the original shows it above the clock.
fn month_short(month: u32) -> String {
    const EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    const DE: [&str; 12] = ["Jan", "Feb", "MÃ¤r", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez"];
    let index = (month.clamp(1, 12) - 1) as usize;
    if crate::i18n::language().starts_with("de") { DE[index] } else { EN[index] }.to_owned()
}

/// The original's AnalogClock: 60 ruler ticks (white every five), rounded hour markers, the day of
/// the month in red at three o'clock, white hour and minute hands and a sweeping red second hand.
/// `radius` is the face radius (the control's half size minus its 4-point padding).
fn analog_clock(ui: &Ui, center: Pos2, radius: f32, now: &chrono::DateTime<Local>) {
    const ACCENT: Color32 = Color32::from_rgb(0xE0, 0x23, 0x1F);
    let painter = ui.painter();
    let r = radius;
    let point = |angle_deg: f32, distance: f32| {
        let angle = angle_deg.to_radians();
        pos2(center.x + angle.sin() * distance, center.y - angle.cos() * distance)
    };
    for tick in 0..60 {
        let five = tick % 5 == 0;
        let angle = tick as f32 * 6.0;
        let inner = if five { r * 0.86 } else { r * 0.91 };
        let (width, color) = if five { (1.4, Color32::WHITE) } else { (1.0, Color32::from_rgb(0x6E, 0x6E, 0x73)) };
        painter.line_segment([point(angle, inner), point(angle, r * 0.97)], Stroke::new(width, color));
    }
    // Hour markers: rounded bars centred at 70 % of the radius; three o'clock gives way to the date.
    let marker_len = r * 0.18;
    let thickness = (r * 0.075).max(2.0);
    for hour in 0..12 {
        if hour == 3 {
            continue;
        }
        let angle = hour as f32 * 30.0;
        let a = point(angle, r * 0.70 - marker_len / 2.0 + thickness / 2.0);
        let b = point(angle, r * 0.70 + marker_len / 2.0 - thickness / 2.0);
        painter.line_segment([a, b], Stroke::new(thickness, Color32::WHITE));
        painter.circle_filled(a, thickness / 2.0, Color32::WHITE);
        painter.circle_filled(b, thickness / 2.0, Color32::WHITE);
    }
    painter.text(pos2(center.x + r * 0.56, center.y), Align2::CENTER_CENTER, now.day().to_string(), theme::bold(r * 0.30), ACCENT);

    let seconds = now.second() as f32 + now.timestamp_subsec_millis() as f32 / 1000.0;
    let minutes = now.minute() as f32 + seconds / 60.0;
    let hours = (now.hour() % 12) as f32 + minutes / 60.0;
    let hand = |angle: f32, length: f32, tail: f32, width: f32, color: Color32| {
        let (tip, end) = (point(angle, length - width / 2.0), point(angle + 180.0, tail - width / 2.0));
        painter.line_segment([end, tip], Stroke::new(width, color));
        painter.circle_filled(tip, width / 2.0, color);
        painter.circle_filled(end, width / 2.0, color);
    };
    hand(hours * 30.0, r * 0.52, r * 0.11, (r * 0.085).max(2.5), Color32::WHITE);
    hand(minutes * 6.0, r * 0.74, r * 0.13, (r * 0.065).max(2.0), Color32::WHITE);
    hand(seconds * 6.0, r * 0.82, r * 0.22, (r * 0.022).max(1.2), ACCENT);
    let hub = (r * 0.07).max(2.5);
    painter.circle_filled(center, hub, ACCENT);
    painter.circle_filled(center, hub * 0.42, Color32::from_gray(0x0A));
}

/// The original's MediaBackground: the blurred artwork stretched over the notch and past it
/// (margins -40, -26, -36, -48 around the content), seen through a radial mask centred at 30 % / 56 %
/// with radius 0.52 and stops 1, 0.6, 0.27, 0 at 0, 0.35, 0.65, 0.95. Drawn as a mesh with the mask
/// in the vertex colours; vertices outside the notch's rounded shape stay clear.
fn paint_art_glow(ui: &Ui, texture: eframe::egui::TextureId, notch: Rect, corner: f32, alpha: f32) {
    use eframe::egui::epaint::{Mesh, Vertex};
    const COLUMNS: u32 = 48;
    const ROWS: u32 = 20;
    let rect = Rect::from_min_max(notch.min + vec2(8.0 - 40.0, 2.0 - 26.0), notch.max + vec2(-8.0 + 36.0, -8.0 + 48.0));
    let mask = |u: f32, v: f32| {
        let distance = (((u - 0.30) / 0.52).powi(2) + ((v - 0.56) / 0.52).powi(2)).sqrt();
        const STOPS: [(f32, f32); 4] = [(0.0, 1.0), (0.35, 0.6), (0.65, 0.27), (0.95, 0.0)];
        STOPS.windows(2).find(|pair| distance <= pair[1].0).map_or(0.0, |pair| {
            let t = (distance - pair[0].0) / (pair[1].0 - pair[0].0);
            pair[0].1 + (pair[1].1 - pair[0].1) * t
        })
    };
    let inside = |p: Pos2| {
        let local = p - notch.min;
        if local.x < 0.0 || local.y < 0.0 || local.x > notch.width() || local.y > notch.height() {
            return 0.0_f32;
        }
        // The rounded bottom corners.
        let cx = if local.x < corner { corner } else if local.x > notch.width() - corner { notch.width() - corner } else { local.x };
        let cy = notch.height() - corner;
        if local.y <= cy || cx == local.x {
            return 1.0;
        }
        let distance = ((local.x - cx).powi(2) + (local.y - cy).powi(2)).sqrt();
        (corner + 0.5 - distance).clamp(0.0, 1.0)
    };
    let mut mesh = Mesh::with_texture(texture);
    for row in 0..=ROWS {
        for column in 0..=COLUMNS {
            let (u, v) = (column as f32 / COLUMNS as f32, row as f32 / ROWS as f32);
            let pos = rect.lerp_inside(vec2(u, v));
            let strength = mask(u, v) * inside(pos) * alpha;
            mesh.vertices.push(Vertex { pos, uv: pos2(u, v), color: Color32::WHITE.gamma_multiply(strength) });
        }
    }
    let width = COLUMNS + 1;
    for row in 0..ROWS {
        for column in 0..COLUMNS {
            let i = row * width + column;
            mesh.add_triangle(i, i + 1, i + width);
            mesh.add_triangle(i + 1, i + width + 1, i + width);
        }
    }
    ui.painter().add(mesh);
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

/// A word clock in German: "Es ist fÃ¼nf nach drei".
pub fn word_clock(hour: u32, minute: u32) -> String {
    const HOURS: [&str; 12] = ["zwÃ¶lf", "eins", "zwei", "drei", "vier", "fÃ¼nf", "sechs", "sieben", "acht", "neun", "zehn", "elf"];
    let rounded = (minute + 2) / 5 * 5;
    let hour = hour + u32::from(rounded >= 25);
    let name = |h: u32| HOURS[(h % 12) as usize];
    let text = match rounded {
        0 | 60 => {
            // A rounded 60 already moved `hour` on above.
            let spoken = if hour % 12 == 1 { "ein" } else { name(hour) };
            format!("{spoken} Uhr")
        }
        5 => format!("fÃ¼nf nach {}", name(hour)),
        10 => format!("zehn nach {}", name(hour)),
        15 => format!("Viertel nach {}", name(hour)),
        20 => format!("zwanzig nach {}", name(hour)),
        25 => format!("fÃ¼nf vor halb {}", name(hour)),
        30 => format!("halb {}", name(hour)),
        35 => format!("fÃ¼nf nach halb {}", name(hour)),
        40 => format!("zwanzig vor {}", name(hour)),
        45 => format!("Viertel vor {}", name(hour)),
        50 => format!("zehn vor {}", name(hour)),
        _ => format!("fÃ¼nf vor {}", name(hour)),
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
        assert_eq!(word_clock(15, 5), "Es ist fÃ¼nf nach drei");
        assert_eq!(word_clock(3, 15), "Es ist Viertel nach drei");
        assert_eq!(word_clock(3, 30), "Es ist halb vier");
        assert_eq!(word_clock(3, 45), "Es ist Viertel vor vier");
        assert_eq!(word_clock(3, 58), "Es ist vier Uhr");
        assert_eq!(word_clock(11, 58), "Es ist zwÃ¶lf Uhr");
        assert_eq!(word_clock(12, 40), "Es ist zwanzig vor eins");
    }
}
