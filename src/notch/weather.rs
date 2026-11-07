//! The weather tab: an animated sky, current conditions, the next 24 hours and the week.

use super::kit::{self, rounded, text_clipped};
use super::Notch;
use crate::i18n::{t, tf};
use crate::icons;
use crate::services::weather::{self, Forecast, HourForecast, condition_icon, condition_key};
use crate::services::weather_service::WeatherStatus;
use crate::settings::Settings;
use crate::theme;
use chrono::{Datelike, NaiveDate};
use eframe::egui::{self, Align2, Color32, Mesh, Pos2, Rect, Shape, Stroke, Ui, pos2, vec2};

impl Notch {
    pub(super) fn weather_view(&mut self, ui: &mut Ui, body: Rect, settings: &Settings) {
        let status = self.hub.weather.status.lock().map(|s| s.clone()).unwrap_or_default();
        match status {
            WeatherStatus::Ready(forecast) => self.forecast_panel(ui, body, &forecast),
            WeatherStatus::Loading => {
                ui.painter().text(body.center(), Align2::CENTER_CENTER, t("audio.loading").replace("Audiomixer", "Wetter"), theme::regular(13.0), theme::DIM);
            }
            WeatherStatus::Failed => message(ui, body, &t("weather.unavailable"), &t("weather.retryLater")),
            WeatherStatus::Off => {
                let blocked = !settings.is_online_allowed() || !settings.allow_online_weather;
                let detail = if blocked { t("port.weather.blocked") } else { t("weather.enableInSettings") };
                message(ui, body, &t("weather.disabled"), &detail);
            }
        }
    }

    fn forecast_panel(&mut self, ui: &mut Ui, body: Rect, forecast: &Forecast) {
        let time = ui.input(|i| i.time) as f32;
        let sky = Rect::from_min_size(body.min, vec2(238.0, body.height()));
        paint_sky(ui, sky, forecast.code, forecast.is_day, time);

        // Current conditions on top of the sky
        let left = sky.left() + 16.0;
        let icon = condition_icon(forecast.code, forecast.is_day);
        ui.painter().text(pos2(left, sky.top() + 20.0), Align2::LEFT_CENTER, &forecast.place, theme::semibold(14.0), Color32::WHITE);
        ui.painter().text(pos2(sky.right() - 16.0, sky.top() + 22.0), Align2::RIGHT_CENTER, icon, theme::icon_fill(22.0), Color32::WHITE);
        ui.painter().text(pos2(left, sky.top() + 62.0), Align2::LEFT_CENTER, format!("{:.0}°", forecast.temperature), theme::bold(44.0), Color32::WHITE);
        text_clipped(ui, pos2(left, sky.top() + 100.0), Align2::LEFT_CENTER, &t(condition_key(forecast.code)), theme::medium(13.0), Color32::WHITE, sky.width() - 32.0);
        let feels = tf("weather.feelsLike", &[&format!("{:.0}°", forecast.feels_like)]);
        ui.painter().text(pos2(left, sky.top() + 120.0), Align2::LEFT_CENTER, feels, theme::regular(11.5), theme::alpha(Color32::WHITE, 0.8));
        if let Some(today) = forecast.days.first() {
            let range = format!("↑ {:.0}°   ↓ {:.0}°", today.max, today.min);
            ui.painter().text(pos2(left, sky.top() + 139.0), Align2::LEFT_CENTER, range, theme::regular(11.5), theme::alpha(Color32::WHITE, 0.8));
        }
        let facts = [
            (icons::WIND, format!("{:.0} km/h", forecast.wind)),
            (icons::DROP, format!("{:.0} %", forecast.humidity)),
            (icons::SUN, format!("UV {:.0}", forecast.days.first().map_or(0.0, |d| d.uv))),
        ];
        let mut y = sky.bottom() - 58.0;
        for (fact_icon, fact) in facts {
            ui.painter().text(pos2(left + 7.0, y), Align2::CENTER_CENTER, fact_icon, theme::icon(13.0), theme::alpha(Color32::WHITE, 0.85));
            ui.painter().text(pos2(left + 22.0, y), Align2::LEFT_CENTER, fact, theme::regular(11.5), theme::alpha(Color32::WHITE, 0.85));
            y += 19.0;
        }

        // Hours chart
        let right = Rect::from_min_max(pos2(sky.right() + 10.0, body.top()), body.max);
        let summary = rain_summary(&forecast.hours);
        text_clipped(ui, pos2(right.left() + 4.0, right.top() + 8.0), Align2::LEFT_CENTER, &summary, theme::medium(12.0), theme::DIM, right.width() - 8.0);
        let chart = Rect::from_min_size(pos2(right.left(), right.top() + 20.0), vec2(right.width(), 118.0));
        temperature_chart(ui, chart, &forecast.hours);

        // Week
        let week = Rect::from_min_max(pos2(right.left(), chart.bottom() + 6.0), right.max);
        week_row(ui, week, forecast);
    }
}

fn message(ui: &Ui, body: Rect, title: &str, detail: &str) {
    ui.painter().text(body.center() - vec2(0.0, 24.0), Align2::CENTER_CENTER, icons::CLOUD_SLASH, theme::icon(30.0), theme::FAINT);
    ui.painter().text(body.center() + vec2(0.0, 6.0), Align2::CENTER_CENTER, title, theme::medium(13.0), theme::DIM);
    text_clipped(ui, body.center() + vec2(0.0, 26.0), Align2::CENTER_CENTER, detail, theme::regular(11.5), theme::FAINT, body.width() - 20.0);
}

/// Two colours for the vertical sky gradient.
pub fn sky_colors(code: i32, is_day: bool) -> (Color32, Color32) {
    let storm = matches!(code, 95..=99);
    let wet = matches!(code, 51..=67 | 80..=82);
    let snow = matches!(code, 71..=77 | 85 | 86);
    let grey = matches!(code, 3 | 45 | 48);
    match (is_day, storm, wet, snow, grey) {
        (_, true, ..) => (Color32::from_rgb(38, 36, 62), Color32::from_rgb(20, 20, 34)),
        (true, _, true, ..) => (Color32::from_rgb(70, 92, 120), Color32::from_rgb(38, 52, 74)),
        (true, _, _, true, _) => (Color32::from_rgb(130, 158, 190), Color32::from_rgb(78, 100, 130)),
        (true, _, _, _, true) => (Color32::from_rgb(112, 128, 146), Color32::from_rgb(70, 82, 98)),
        (true, ..) => (Color32::from_rgb(58, 136, 222), Color32::from_rgb(34, 86, 170)),
        (false, _, true, ..) => (Color32::from_rgb(28, 36, 54), Color32::from_rgb(14, 18, 30)),
        (false, ..) => (Color32::from_rgb(20, 28, 62), Color32::from_rgb(8, 10, 28)),
    }
}

fn paint_sky(ui: &Ui, rect: Rect, code: i32, is_day: bool, time: f32) {
    let (top, bottom) = sky_colors(code, is_day);
    // A vertical gradient as a mesh, then the rounded corners cut out by painting over them would look
    // wrong on the art background, so the mesh is simply inset by the corner radius at top and bottom.
    let radius = 14.0;
    let mut mesh = Mesh::default();
    let steps = 6;
    let corner_inset = |y: f32| {
        let distance = if y < radius { radius - y } else if y > rect.height() - radius { y - (rect.height() - radius) } else { 0.0 };
        radius - (radius * radius - distance * distance).max(0.0).sqrt()
    };
    let rows = 28 + steps;
    for row in 0..=rows {
        let y = rect.height() * row as f32 / rows as f32;
        let inset = corner_inset(y);
        let t = row as f32 / rows as f32;
        let color = lerp_color(top, bottom, t);
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(rect.left() + inset, rect.top() + y), color);
        mesh.colored_vertex(pos2(rect.right() - inset, rect.top() + y), color);
        if row > 0 {
            mesh.add_triangle(base - 2, base - 1, base);
            mesh.add_triangle(base - 1, base, base + 1);
        }
    }
    ui.painter().add(Shape::mesh(mesh));

    let painter = ui.painter().with_clip_rect(rect.shrink(1.0).intersect(ui.clip_rect()));
    let wet = matches!(code, 51..=67 | 80..=82 | 95..=99);
    let snowing = matches!(code, 71..=77 | 85 | 86);
    let cloudy = matches!(code, 2 | 3 | 45 | 48) || wet || snowing;

    if !is_day && !cloudy {
        for index in 0..26 {
            let x = rect.left() + (hash01(index, 1) * rect.width());
            let y = rect.top() + (hash01(index, 2) * rect.height() * 0.8);
            let twinkle = 0.4 + 0.6 * (time * (0.8 + hash01(index, 3) * 2.0) + index as f32).sin().abs();
            painter.circle_filled(pos2(x, y), 0.7 + hash01(index, 4) * 0.7, theme::alpha(Color32::WHITE, twinkle * 0.8));
        }
    }
    if is_day && !cloudy {
        let sun = pos2(rect.right() - 46.0, rect.top() + 56.0);
        let glow = 22.0 + 2.5 * (time * 0.8).sin();
        for (factor, alpha) in [(2.4, 0.06), (1.7, 0.1), (1.2, 0.2)] {
            painter.circle_filled(sun, glow * factor, theme::alpha(Color32::from_rgb(255, 224, 130), alpha));
        }
        painter.circle_filled(sun, 18.0, Color32::from_rgb(255, 214, 90));
    }
    if cloudy {
        let tint = if is_day { 0.28 } else { 0.1 };
        for index in 0..4 {
            let speed = 5.0 + index as f32 * 2.5;
            let span = rect.width() + 120.0;
            let x = rect.left() - 60.0 + (time * speed + index as f32 * 67.0) % span;
            let y = rect.top() + 34.0 + index as f32 * 22.0;
            for (dx, dy, r) in [(0.0, 0.0, 20.0), (22.0, -6.0, 26.0), (48.0, 2.0, 18.0)] {
                painter.circle_filled(pos2(x + dx, y + dy), r, theme::alpha(Color32::WHITE, tint));
            }
        }
    }
    if wet {
        for index in 0..46 {
            let x = rect.left() + hash01(index, 5) * (rect.width() + 20.0);
            let speed = 150.0 + hash01(index, 6) * 90.0;
            let y = rect.top() + (time * speed + hash01(index, 7) * 400.0) % rect.height();
            let start = pos2(x, y);
            painter.line_segment([start, start + vec2(-2.0, 9.0)], Stroke::new(1.0, theme::alpha(Color32::WHITE, 0.32)));
        }
    }
    if snowing {
        for index in 0..34 {
            let speed = 16.0 + hash01(index, 8) * 22.0;
            let y = rect.top() + (time * speed + hash01(index, 9) * 400.0) % rect.height();
            let sway = (time * 1.2 + index as f32).sin() * 6.0;
            let x = rect.left() + hash01(index, 10) * rect.width() + sway;
            painter.circle_filled(pos2(x, y), 1.0 + hash01(index, 11) * 1.4, theme::alpha(Color32::WHITE, 0.8));
        }
    }
    if matches!(code, 95..=99) {
        let flash = ((time * 0.7).sin() * (time * 2.3).sin()).max(0.0).powi(8);
        if flash > 0.01 {
            painter.rect_filled(rect, rounded(radius), theme::alpha(Color32::WHITE, flash * 0.35));
        }
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t) as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// A stable pseudo-random number in 0..1, so stars and drops keep their place between frames.
fn hash01(index: u32, salt: u32) -> f32 {
    let value = index.wrapping_mul(2_654_435_761).wrapping_add(salt.wrapping_mul(40_503)).rotate_left(13).wrapping_mul(2_166_136_261);
    (value >> 8) as f32 / 16_777_216.0
}

fn temperature_chart(ui: &mut Ui, chart: Rect, hours: &[HourForecast]) {
    if hours.len() < 2 {
        return;
    }
    kit::card(ui, chart, 12.0, theme::CARD);
    let plot = Rect::from_min_max(chart.min + vec2(14.0, 26.0), chart.max - vec2(14.0, 26.0));
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for hour in hours {
        low = low.min(hour.temperature);
        high = high.max(hour.temperature);
    }
    let range = (high - low).max(1.0);
    let point = |index: usize, hour: &HourForecast| -> Pos2 {
        let x = plot.left() + plot.width() * index as f32 / (hours.len() - 1) as f32;
        let y = plot.bottom() - plot.height() * (hour.temperature - low) / range;
        pos2(x, y)
    };

    // Rain chance as soft bars behind the line.
    for (index, hour) in hours.iter().enumerate() {
        if hour.rain_chance >= 20.0 {
            let x = plot.left() + plot.width() * index as f32 / (hours.len() - 1) as f32;
            let height = (chart.height() - 16.0) * (hour.rain_chance / 100.0) * 0.8;
            let bar = Rect::from_min_max(pos2(x - 4.0, chart.bottom() - 16.0 - height), pos2(x + 4.0, chart.bottom() - 16.0));
            ui.painter().rect_filled(bar, rounded(2.0), theme::alpha(theme::ACCENT, 0.3));
        }
    }

    let line: Vec<Pos2> = hours.iter().enumerate().map(|(i, h)| point(i, h)).collect();
    ui.painter().add(Shape::line(line, Stroke::new(2.0, theme::ORANGE)));

    let step = (hours.len() / 6).max(1);
    for (index, hour) in hours.iter().enumerate().step_by(step) {
        let at = point(index, hour);
        ui.painter().circle_filled(at, 2.5, Color32::WHITE);
        ui.painter().text(at - vec2(0.0, 10.0), Align2::CENTER_BOTTOM, format!("{:.0}°", hour.temperature), theme::medium(10.5), theme::WHITE);
        let label = hour.time.get(11..13).unwrap_or_default();
        ui.painter().text(pos2(at.x, chart.bottom() - 9.0), Align2::CENTER_CENTER, label, theme::regular(10.0), theme::FAINT);
    }
}

fn week_row(ui: &mut Ui, area: Rect, forecast: &Forecast) {
    const DAYS: [&str; 7] = ["port.day.mon", "port.day.tue", "port.day.wed", "port.day.thu", "port.day.fri", "port.day.sat", "port.day.sun"];
    let days: Vec<_> = forecast.days.iter().take(7).collect();
    if days.is_empty() {
        return;
    }
    for (rect, day) in kit::columns(area, days.len(), 4.0).into_iter().zip(days.iter().copied()) {
        kit::card(ui, rect, 10.0, theme::CARD);
        let label = NaiveDate::parse_from_str(&day.date, "%Y-%m-%d")
            .map(|date| t(DAYS[date.weekday().num_days_from_monday() as usize]))
            .unwrap_or_default();
        let short: String = label.chars().take(2).collect();
        let x = rect.center().x;
        ui.painter().text(pos2(x, rect.top() + 14.0), Align2::CENTER_CENTER, short, theme::medium(10.5), theme::DIM);
        ui.painter().text(pos2(x, rect.center().y - 1.0), Align2::CENTER_CENTER, condition_icon(day.code, true), theme::icon(18.0), theme::WHITE);
        let range = format!("{:.0}° {:.0}°", day.max, day.min);
        ui.painter().text(pos2(x, rect.bottom() - 13.0), Align2::CENTER_CENTER, range, theme::regular(9.5), theme::DIM);
        let _ = egui::Id::NULL;
    }
    let _ = weather::condition_key;
}

/// "Regen in 3 Stunden", or that it stays dry.
pub fn rain_summary(hours: &[HourForecast]) -> String {
    let is_wet = |hour: &HourForecast| hour.rain_chance >= 50.0 && hour.rain > 0.0;
    let is_snow = |hour: &HourForecast| matches!(hour.code, 71..=77 | 85 | 86);
    match hours.first() {
        Some(first) if is_wet(first) => {
            let kind = if is_snow(first) { "snow" } else { "rain" };
            match hours.iter().position(|hour| !is_wet(hour)) {
                Some(stop) => tf(&format!("weather.{kind}StopsIn"), &[&format!("{stop} h")]),
                None => t(&format!("weather.{kind}Continues")),
            }
        }
        _ => match hours.iter().position(is_wet) {
            Some(start) => {
                let kind = if is_snow(&hours[start]) { "snow" } else { "rain" };
                tf(&format!("weather.{kind}In"), &[&format!("{start} h")])
            }
            None => t("weather.dryAhead"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hour(rain_chance: f32, rain: f32, code: i32) -> HourForecast {
        HourForecast { rain_chance, rain, code, ..Default::default() }
    }

    #[test]
    fn rain_summary_uses_known_keys() {
        crate::i18n::set_language("en");
        let dry = vec![hour(0.0, 0.0, 0); 5];
        assert_ne!(rain_summary(&dry), "weather.dryAhead");
        let soon = vec![hour(0.0, 0.0, 0), hour(80.0, 1.0, 61)];
        assert!(!rain_summary(&soon).starts_with("weather."));
        let now = vec![hour(80.0, 1.0, 61), hour(0.0, 0.0, 0)];
        assert!(!rain_summary(&now).starts_with("weather."));
        let forever = vec![hour(80.0, 1.0, 71); 3];
        assert!(!rain_summary(&forever).starts_with("weather."));
    }

    #[test]
    fn hash_is_stable_and_in_range() {
        for index in 0..500 {
            let value = hash01(index, 3);
            assert!((0.0..1.0).contains(&value));
            assert_eq!(value, hash01(index, 3));
        }
        assert_ne!(hash01(1, 1), hash01(2, 1));
    }

    #[test]
    fn sky_depends_on_day_and_weather() {
        assert_ne!(sky_colors(0, true), sky_colors(0, false));
        assert_ne!(sky_colors(0, true), sky_colors(61, true));
        assert_ne!(sky_colors(95, true), sky_colors(0, true));
    }
}
