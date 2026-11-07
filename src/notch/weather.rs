//! The weather tab, laid out like the original's WeatherView: on the left the place, a drawn sky
//! with the temperature, the rain outlook and four detail tiles; on the right the temperature curve
//! of the next 24 hours and the week as rows with range bars.

use super::kit::{self, rounded, text_clipped};
use super::Notch;
use crate::i18n::{t, tf};
use crate::icons;
use crate::services::weather::{Forecast, HourForecast, condition_icon, condition_key};
use crate::services::weather_service::WeatherStatus;
use crate::settings::Settings;
use crate::theme;
use chrono::{Datelike, NaiveDate, Timelike};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};

// Sizes and colours of the original (WeatherView*.cs, WeatherIcons.cs).
const LEFT_W: f32 = 256.0;
const RIGHT_W: f32 = 404.0;
const CHART_H: f32 = 146.0;
const CHART_PAD: f32 = 14.0;
const CURVE_TOP: f32 = 24.0;
const CURVE_BOTTOM: f32 = 74.0;
const BARS_BOTTOM: f32 = 98.0;
const BARS_H: f32 = 16.0;
const ICONS_TOP: f32 = 102.0;
const TIMES_TOP: f32 = 122.0;
const DAY_ROW_H: f32 = 20.0;
const TILE_W: f32 = 124.0;
const TILE_H: f32 = 64.0;
const MUTED: Color32 = Color32::from_gray(0x8C);
const SOFT: Color32 = Color32::from_rgba_premultiplied(0xC8, 0xC8, 0xC8, 0xC8);
const CARD: f32 = 0x0F as f32 / 255.0;
const RAIN: Color32 = Color32::from_rgb(0x5A, 0xB0, 0xFF);
const SUN: Color32 = Color32::from_rgb(0xFF, 0xC5, 0x31);
const MOON: Color32 = Color32::from_rgb(0xEE, 0xEA, 0xD8);
const CLOUD: Color32 = Color32::from_rgb(0xE9, 0xEE, 0xF5);
const DARK_CLOUD: Color32 = Color32::from_rgb(0x9A, 0xA5, 0xB4);
const BOLT: Color32 = Color32::from_rgb(0xFF, 0xD6, 0x0A);
const TEMPERATURE_STOPS: [(f32, u32); 8] =
    [(-12.0, 0xA5B4FF), (-2.0, 0x6BA8FF), (5.0, 0x4FC3F7), (11.0, 0x4FD1B5), (16.0, 0x9BE15D), (21.0, 0xFFD54A), (27.0, 0xFF9F43), (34.0, 0xFF5A5A)];

fn rgb(value: u32) -> Color32 {
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// Where a temperature sits on the cold-to-hot scale the curve and the day bars share.
pub fn temperature_color(celsius: f32) -> Color32 {
    let (first_t, first) = TEMPERATURE_STOPS[0];
    if celsius <= first_t {
        return rgb(first);
    }
    for pair in TEMPERATURE_STOPS.windows(2) {
        let ((from_t, from), (to_t, to)) = (pair[0], pair[1]);
        if celsius <= to_t {
            return kit::lerp_color(rgb(from), rgb(to), (celsius - from_t) / (to_t - from_t));
        }
    }
    rgb(TEMPERATURE_STOPS[TEMPERATURE_STOPS.len() - 1].1)
}

impl Notch {
    pub(super) fn weather_view(&mut self, ui: &mut Ui, body: Rect, settings: &Settings) {
        let status = self.hub.weather.status.lock().map(|s| s.clone()).unwrap_or_default();
        match status {
            WeatherStatus::Ready(forecast) => self.forecast_panel(ui, body, &forecast, settings),
            WeatherStatus::Loading => {
                self.hub.weather.wake();
                ui.painter().text(body.center(), Align2::CENTER_CENTER, t("port.weather.loading"), theme::semibold(13.0), MUTED);
            }
            WeatherStatus::Failed => message(ui, body, &t("weather.unavailable"), &t("weather.retryLater")),
            WeatherStatus::Off => {
                let blocked = !settings.is_online_allowed() || !settings.allow_online_weather;
                let detail = if blocked { t("port.weather.blocked") } else { t("weather.enableInSettings") };
                message(ui, body, &t("weather.disabled"), &detail);
            }
        }
    }

    fn forecast_panel(&mut self, ui: &mut Ui, body: Rect, forecast: &Forecast, settings: &Settings) {
        let time = ui.input(|i| i.time) as f32;
        let fahrenheit = settings.use_fahrenheit;
        let left = Rect::from_min_size(body.min, vec2(LEFT_W, body.height()));
        let right = Rect::from_min_size(pos2(body.right() - RIGHT_W, body.top()), vec2(RIGHT_W, body.height()));

        self.place_header(ui, Rect::from_min_size(left.min, vec2(LEFT_W, 18.0)), forecast);
        hero(ui, Rect::from_min_size(left.min + vec2(0.0, 24.0), vec2(LEFT_W, 108.0)), forecast, time);
        nowcast(ui, Rect::from_min_size(left.min + vec2(0.0, 140.0), vec2(LEFT_W, 34.0)), &forecast.hours);
        let tiles_top = left.bottom() - (TILE_H * 2.0 + 8.0);
        detail_tiles(ui, pos2(left.left(), tiles_top), forecast);

        // Right: the next 24 hours, then the week.
        let header = Rect::from_min_size(right.min, vec2(RIGHT_W, 18.0));
        ui.painter().text(header.left_center(), Align2::LEFT_CENTER, t("weather.next24"), theme::semibold(11.5), Color32::WHITE);
        ui.painter().text(header.right_center(), Align2::RIGHT_CENTER, "Open-Meteo", theme::regular(10.0), MUTED);
        let chart = Rect::from_min_size(right.min + vec2(0.0, 22.0), vec2(RIGHT_W, CHART_H));
        temperature_chart(ui, chart, &forecast.hours, fahrenheit);
        let days = Rect::from_min_max(pos2(right.left(), right.bottom() - DAY_ROW_H * 7.0), right.max);
        day_rows(ui, days, forecast, fahrenheit);
    }

    /// A pin, the place in semibold, "Updated 14:05" and a refresh button at the end.
    fn place_header(&mut self, ui: &mut Ui, row: Rect, forecast: &Forecast) {
        let y = row.center().y;
        paint_pin(ui, pos2(row.left() + 4.0, y), Color32::WHITE);
        let place_width = kit::text_width(ui, &forecast.place, theme::semibold(11.5));
        ui.painter().text(pos2(row.left() + 14.0, y), Align2::LEFT_CENTER, &forecast.place, theme::semibold(11.5), Color32::WHITE);
        if !forecast.fetched_at.is_empty() {
            let updated = format!("  ·  {}", tf("weather.updatedAt", &[&forecast.fetched_at]));
            text_clipped(ui, pos2(row.left() + 14.0 + place_width, y), Align2::LEFT_CENTER, &updated, theme::regular(10.5), MUTED, row.width() - place_width - 40.0);
        }
        let button = Rect::from_center_size(pos2(row.right() - 10.0, y), vec2(20.0, 18.0));
        let response = ui.interact(button, ui.id().with("weather-refresh"), Sense::click());
        let ink = if response.hovered() { Color32::WHITE } else { MUTED };
        ui.painter().text(button.center(), Align2::CENTER_CENTER, icons::ARROWS_CLOCKWISE, theme::icon(11.0), ink);
        if response.clicked() {
            self.hub.weather.wake();
        }
    }
}

/// The map pin before the place: a round head over a point, 8 × 10.5.
fn paint_pin(ui: &Ui, center: Pos2, color: Color32) {
    let head = center - vec2(0.0, 1.5);
    ui.painter().add(Shape::convex_polygon(vec![head + vec2(-3.4, 1.4), head + vec2(3.4, 1.4), head + vec2(0.0, 6.6)], color, Stroke::NONE));
    ui.painter().circle_filled(head, 4.0, color);
    ui.painter().circle_filled(head, 1.5, Color32::BLACK);
}

/// The drawn sky (108 × 108) and next to it the temperature, the condition, how it feels and the day's range.
fn hero(ui: &Ui, row: Rect, forecast: &Forecast, time: f32) {
    let sky = Rect::from_min_size(row.min, vec2(108.0, 108.0));
    paint_sky_icon(ui, sky, forecast.code, forecast.is_day, time);
    let left = sky.right() + 10.0;
    let width = row.right() - left;
    let temperature = format!("{:.0}°", forecast.temperature);
    // 50-point light figures in the original; their line box is trimmed by 10 above and 6 below.
    let big = ui.painter().layout_no_wrap(temperature, theme::thin(50.0), Color32::WHITE);
    let lines = [
        (t(condition_key(forecast.code)), theme::semibold(13.5), Color32::WHITE, 0.0),
        (tf("weather.feelsLike", &[&format!("{:.0}°", forecast.feels_like)]), theme::regular(11.0), SOFT, 3.0),
    ];
    let high_low = forecast.days.first().map(|today| tf("weather.highLow", &[&format!("{:.0}", today.max), &format!("{:.0}", today.min)]));
    let big_height = big.size().y - 16.0;
    let total = big_height + 17.0 + 3.0 + 14.0 + if high_low.is_some() { 15.0 } else { 0.0 };
    let mut y = row.center().y - total / 2.0;
    ui.painter().galley(pos2(left - 2.0, y - 10.0), big, Color32::WHITE);
    y += big_height;
    for (text, font, color, gap) in lines {
        y += gap;
        let height = font.size * 1.25;
        text_clipped(ui, pos2(left, y), Align2::LEFT_TOP, &text, font, color, width);
        y += height;
    }
    if let Some(high_low) = high_low {
        text_clipped(ui, pos2(left, y + 1.0), Align2::LEFT_TOP, &high_low, theme::regular(11.0), MUTED, width);
    }
}

/// The original's drawn sky icons: sun or moon, layered clouds shaded from light to dark, rain
/// streaks, snow, fog bands and a bolt, all with soft edges.
fn paint_sky_icon(ui: &Ui, rect: Rect, code: i32, is_day: bool, time: f32) {
    let painter = ui.painter();
    let center = rect.center();
    let clear = matches!(code, 0 | 1);
    let partly = code == 2;
    let fog = matches!(code, 45 | 48);
    let rain = matches!(code, 51..=67 | 80..=82);
    let snow = matches!(code, 71..=77 | 85 | 86);
    let storm = matches!(code, 95..=99);
    let cloudy = !clear;

    // Sun or moon: up left when clouds share the sky, in the middle when it is clear.
    let body_at = if clear { center } else { center + vec2(-18.0, -18.0) };
    let body_size = if clear { 30.0 } else { 20.0 };
    if clear || partly {
        if is_day {
            let pulse = 1.0 + 0.05 * (time * 0.8).sin();
            kit::soft_shape(painter, kit::Soft::Glow, body_at, body_size / 14.0 * pulse, theme::alpha(SUN, 0.45));
            painter.circle_filled(body_at, body_size / 2.0, SUN);
        } else {
            painter.circle_filled(body_at, body_size / 2.0, MOON);
            painter.circle_filled(body_at + vec2(body_size * 0.32, -body_size * 0.18), body_size * 0.42, Color32::BLACK);
        }
    }
    if cloudy {
        let drift = (time * 0.35).sin() * 2.0;
        let (light, dark) = if storm || rain { (kit::lerp_color(CLOUD, DARK_CLOUD, 0.45), DARK_CLOUD.gamma_multiply(0.75)) } else { (CLOUD, DARK_CLOUD) };
        if !partly {
            // A second, darker cloud behind for overcast skies.
            kit::soft_shape(painter, kit::Soft::Cloud, center + vec2(14.0 - drift, -12.0), 0.62, dark.gamma_multiply(0.85));
        }
        // The front cloud: a darker copy a little lower gives it a shaded underside.
        let front = center + vec2(drift - 2.0, 2.0);
        kit::soft_shape(painter, kit::Soft::Cloud, front + vec2(0.0, 3.0), 0.92, dark);
        kit::soft_shape(painter, kit::Soft::Cloud, front, 0.88, light);
    }
    if fog {
        for (index, width) in [52.0_f32, 40.0, 30.0].into_iter().enumerate() {
            let y = center.y + 26.0 + index as f32 * 7.0;
            let shift = (time * 0.6 + index as f32).sin() * 3.0;
            let line = Rect::from_center_size(pos2(center.x - 4.0 + shift, y), vec2(width, 3.4));
            painter.rect_filled(line, rounded(1.7), theme::alpha(CLOUD, 0.55 - index as f32 * 0.1));
        }
    }
    if rain || storm {
        for index in 0..5 {
            let phase = (time * 1.6 + index as f32 * 0.37) % 1.0;
            let x = center.x - 20.0 + index as f32 * 10.0;
            let y = center.y + 22.0 + phase * 14.0;
            let start = pos2(x - phase * 4.0, y);
            painter.line_segment([start, start + vec2(-3.0, 8.0)], Stroke::new(2.0, theme::alpha(RAIN, 1.0 - phase * 0.7)));
        }
    }
    if snow {
        for index in 0..6 {
            let phase = (time * 0.5 + index as f32 * 0.29) % 1.0;
            let x = center.x - 22.0 + index as f32 * 9.0 + (time + index as f32).sin() * 2.0;
            painter.circle_filled(pos2(x, center.y + 22.0 + phase * 16.0), 1.8, theme::alpha(Color32::WHITE, 1.0 - phase * 0.6));
        }
    }
    if storm {
        super::glyph::paint(painter, center + vec2(2.0, 26.0), 22.0, super::glyph::BOLT, BOLT);
    }
}

/// "No rain in the next 12 hours" or when it starts or stops, in a card with the next hours as bars.
fn nowcast(ui: &Ui, card: Rect, hours: &[HourForecast]) {
    ui.painter().rect_filled(card, rounded(10.0), theme::alpha(Color32::WHITE, CARD));
    let (message, wet, snow) = nowcast_text(hours);
    let icon_center = pos2(card.left() + 12.0, card.center().y);
    if snow {
        for (dx, dy) in [(-3.0, -3.0), (3.0, 0.0), (-1.0, 4.0)] {
            ui.painter().circle_filled(icon_center + vec2(dx, dy), 1.8, Color32::WHITE);
        }
    } else {
        for (index, dx) in [-4.0_f32, 0.0, 4.0].into_iter().enumerate() {
            let top = icon_center + vec2(dx, -5.0 + (index % 2) as f32 * 3.0);
            let color = if wet { RAIN } else { theme::alpha(RAIN, 0.45) };
            ui.painter().rect_filled(Rect::from_min_size(top, vec2(2.0, 6.0)), rounded(1.0), color);
        }
    }
    let bars_width = 12.0 * 3.5 + 11.0 * 2.0;
    let label_right = card.right() - 10.0 - bars_width - 8.0;
    text_clipped(ui, pos2(card.left() + 26.0, card.center().y), Align2::LEFT_CENTER, &message, theme::regular(11.5), SOFT, label_right - card.left() - 26.0);
    let mut x = card.right() - 10.0 - bars_width;
    for hour in hours.iter().take(12) {
        let any = hour.rain > 0.0;
        let height = if any { 4.0 + (hour.rain / 4.0).sqrt().min(1.0) * 14.0 } else { 2.0 };
        let bar = Rect::from_min_max(pos2(x, card.center().y + 9.0 - height), pos2(x + 3.5, card.center().y + 9.0));
        ui.painter().rect_filled(bar, rounded(1.0), if any { RAIN } else { theme::alpha(Color32::WHITE, 0.25) });
        x += 3.5 + 2.0;
    }
}

/// The outlook line: when rain (or snow) starts, stops, or that it stays dry for twelve hours.
pub fn nowcast_text(hours: &[HourForecast]) -> (String, bool, bool) {
    let is_wet = |hour: &HourForecast| hour.rain >= 0.2 || (hour.rain_chance >= 50.0 && hour.rain > 0.0);
    let is_snow = |hour: &HourForecast| matches!(hour.code, 71..=77 | 85 | 86);
    let ahead: Vec<&HourForecast> = hours.iter().take(12).collect();
    let clock = |hour: &HourForecast| hour.time.get(11..16).unwrap_or_default().to_owned();
    match ahead.first() {
        Some(first) if is_wet(first) => {
            let kind = if is_snow(first) { "snow" } else { "rain" };
            let text = match ahead.iter().position(|hour| !is_wet(hour)) {
                Some(stop) => tf(&format!("weather.{kind}Until"), &[&clock(ahead[stop])]),
                None => t(&format!("weather.{kind}Continues")),
            };
            (text, true, kind == "snow")
        }
        _ => match ahead.iter().position(|hour| is_wet(hour)) {
            Some(start) => {
                let kind = if is_snow(ahead[start]) { "snow" } else { "rain" };
                (tf(&format!("weather.{kind}From"), &[&clock(ahead[start])]), false, kind == "snow")
            }
            None => (t("weather.dryAhead"), false, false),
        },
    }
}

/// Wind, humidity, UV and sunrise: four 124 × 64 cards with a label, a value and a small drawing.
fn detail_tiles(ui: &Ui, origin: Pos2, forecast: &Forecast) {
    let today = forecast.days.first();
    let at = |column: f32, row: f32| Rect::from_min_size(origin + vec2(column * (TILE_W + 8.0), row * (TILE_H + 8.0)), vec2(TILE_W, TILE_H));

    // Wind with a compass dial whose needle points where the wind blows to.
    let tile = at(0.0, 0.0);
    let gusts = (forecast.gusts >= forecast.wind + 5.0).then(|| tf("weather.gusts", &[&format!("{:.0}", forecast.gusts)]));
    tile_text(ui, tile, &t("weather.wind"), &format!("{:.0} km/h", forecast.wind), gusts.as_deref());
    let dial = pos2(tile.right() - 8.0 - 17.0, tile.center().y);
    ui.painter().circle_stroke(dial, 17.0, Stroke::new(1.2, theme::alpha(Color32::WHITE, 0x30 as f32 / 255.0)));
    for quarter in 0..4 {
        let angle = (quarter as f32 * 90.0).to_radians();
        let direction = vec2(angle.sin(), -angle.cos());
        let (width, color) = if quarter == 0 { (1.6, Color32::WHITE) } else { (1.1, MUTED) };
        ui.painter().line_segment([dial + direction * 13.0, dial + direction * 16.0], Stroke::new(width, color));
    }
    let toward = (forecast.wind_direction + 180.0).to_radians();
    let needle = vec2(toward.sin(), -toward.cos());
    ui.painter().line_segment([dial - needle * 8.0, dial + needle * 8.0], Stroke::new(1.6, Color32::WHITE));
    let tip = dial + needle * 9.0;
    let side = vec2(-needle.y, needle.x);
    ui.painter().add(Shape::convex_polygon(vec![tip, tip - needle * 5.0 + side * 3.0, tip - needle * 5.0 - side * 3.0], Color32::WHITE, Stroke::NONE));

    // Humidity with a blue ring filled to the value and a drop in it.
    let tile = at(1.0, 0.0);
    tile_text(ui, tile, &t("weather.humidity"), &format!("{:.0} %", forecast.humidity), None);
    let ring = pos2(tile.right() - 8.0 - 17.0, tile.center().y);
    let radius = (32.0 - 3.6) / 2.0;
    ui.painter().circle_stroke(ring, radius, Stroke::new(3.6, theme::alpha(Color32::WHITE, 0x30 as f32 / 255.0)));
    arc(ui, ring, radius, 3.6, forecast.humidity / 100.0, RAIN);
    ui.painter().text(ring, Align2::CENTER_CENTER, icons::DROP, theme::icon_fill(11.0), RAIN);

    // UV with its level and a green-to-violet scale with a dot at the value.
    let tile = at(0.0, 1.0);
    let uv = today.map_or(0.0, |d| d.uv);
    let level = match uv.round() as i32 {
        ..=2 => "weather.uv.low",
        3..=5 => "weather.uv.moderate",
        6..=7 => "weather.uv.high",
        8..=10 => "weather.uv.veryHigh",
        _ => "weather.uv.extreme",
    };
    tile_text(ui, tile, &t("weather.uv"), &format!("{:.0} · {}", uv.round(), t(level)), None);
    let bar = Rect::from_min_size(pos2(tile.left() + 10.0, tile.bottom() - 7.0 - 4.0), vec2(TILE_W - 18.0, 4.0));
    let scale = [(0.0, 0x3FD15B), (0.3, 0xFFD60A), (0.55, 0xFF9F0A), (0.8, 0xFF453A), (1.0, 0xBF5AF2)];
    for pair in scale.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let part = Rect::from_min_max(pos2(bar.left() + bar.width() * from.0, bar.top()), pos2(bar.left() + bar.width() * to.0, bar.bottom()));
        kit::horizontal_gradient_rect(ui, part, 0.0, rgb(from.1), rgb(to.1));
    }
    let dot_x = bar.left() + bar.width() * (uv / 11.0).clamp(0.0, 1.0);
    ui.painter().circle_filled(pos2(dot_x, bar.center().y), 4.0, Color32::WHITE);
    ui.painter().circle_stroke(pos2(dot_x, bar.center().y), 4.0, Stroke::new(1.4, Color32::BLACK));

    // Sunrise and sunset with the sun on its arc.
    let tile = at(1.0, 1.0);
    let clock = |text: &str| text.get(11..16).unwrap_or("–").to_owned();
    let (rise, set) = today.map_or(("–".to_owned(), "–".to_owned()), |d| (clock(&d.sunrise), clock(&d.sunset)));
    tile_text(ui, tile, &t("weather.sunrise"), &rise, Some(&format!("↓ {set}")));
    let base = pos2(tile.right() - 8.0 - 26.0, tile.center().y + 9.0);
    let points: Vec<Pos2> = (0..=24)
        .map(|i| {
            let angle = std::f32::consts::PI * (1.0 - i as f32 / 24.0);
            base + vec2(angle.cos() * 22.0, -angle.sin() * 22.0 * 30.0 / 22.0 * 0.7)
        })
        .collect();
    dashed(ui, &points, Stroke::new(1.2, theme::alpha(Color32::WHITE, 0.35)));
    ui.painter().line_segment([base - vec2(26.0, 0.0), base + vec2(26.0, 0.0)], Stroke::new(1.0, theme::alpha(Color32::WHITE, 0.25)));
    let progress = day_progress(today.map(|d| (d.sunrise.as_str(), d.sunset.as_str())));
    let angle = std::f32::consts::PI * (1.0 - progress);
    let sun = base + vec2(angle.cos() * 22.0, -angle.sin() * 22.0 * 30.0 / 22.0 * 0.7);
    ui.painter().circle_filled(sun, 3.5, SUN);
}

/// Label (10, muted), value (15 semibold) and an optional detail line (10, soft) in a detail card.
fn tile_text(ui: &Ui, tile: Rect, label: &str, value: &str, detail: Option<&str>) {
    ui.painter().rect_filled(tile, rounded(12.0), theme::alpha(Color32::WHITE, CARD));
    let left = tile.left() + 10.0;
    let width = TILE_W - 18.0 - 38.0;
    text_clipped(ui, pos2(left, tile.top() + 7.0), Align2::LEFT_TOP, label, theme::regular(10.0), MUTED, width);
    text_clipped(ui, pos2(left, tile.top() + 7.0 + 13.0 + 1.0), Align2::LEFT_TOP, value, theme::semibold(15.0), Color32::WHITE, TILE_W - 18.0);
    if let Some(detail) = detail {
        text_clipped(ui, pos2(left, tile.top() + 7.0 + 13.0 + 1.0 + 19.0), Align2::LEFT_TOP, detail, theme::regular(10.0), SOFT, width + 10.0);
    }
}

/// How far the sun is between rise (0) and set (1) now.
fn day_progress(times: Option<(&str, &str)>) -> f32 {
    let minutes = |text: &str| -> Option<f32> {
        let (hour, minute) = (text.get(11..13)?.parse::<f32>().ok()?, text.get(14..16)?.parse::<f32>().ok()?);
        Some(hour * 60.0 + minute)
    };
    let Some((rise, set)) = times.and_then(|(rise, set)| Some((minutes(rise)?, minutes(set)?))) else { return 0.5 };
    let now = chrono::Local::now();
    let current = now.hour() as f32 * 60.0 + now.minute() as f32;
    ((current - rise) / (set - rise).max(1.0)).clamp(0.0, 1.0)
}

fn arc(ui: &Ui, center: Pos2, radius: f32, width: f32, progress: f32, color: Color32) {
    let progress = progress.clamp(0.0, 1.0);
    if progress <= 0.0 {
        return;
    }
    let steps = ((progress * 48.0) as usize).max(2);
    let points: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * progress * i as f32 / steps as f32;
            center + radius * vec2(angle.cos(), angle.sin())
        })
        .collect();
    ui.painter().circle_filled(points[0], width / 2.0, color);
    ui.painter().circle_filled(points[points.len() - 1], width / 2.0, color);
    ui.painter().add(Shape::line(points, Stroke::new(width, color)));
}

fn dashed(ui: &Ui, points: &[Pos2], stroke: Stroke) {
    ui.painter().add(Shape::dashed_line(points, stroke, 2.5, 3.0));
}

/// The next 24 hours: a smooth curve coloured by temperature with a soft fill under it, a dashed
/// zero line, rain bars, and every third hour its temperature, icon and time.
fn temperature_chart(ui: &Ui, chart: Rect, hours: &[HourForecast], fahrenheit: bool) {
    if hours.len() < 2 {
        return;
    }
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for hour in hours {
        low = low.min(hour.temperature);
        high = high.max(hour.temperature);
    }
    if high - low < 4.0 {
        let middle = (high + low) / 2.0;
        (low, high) = (middle - 2.0, middle + 2.0);
    }
    let x_of = |index: usize| chart.left() + CHART_PAD + (chart.width() - 2.0 * CHART_PAD) * index as f32 / (hours.len() - 1) as f32;
    let y_of = |temperature: f32| chart.top() + CURVE_TOP + (high - temperature) / (high - low) * (CURVE_BOTTOM - CURVE_TOP);
    let celsius = |value: f32| if fahrenheit { (value - 32.0) * 5.0 / 9.0 } else { value };

    // The zero line, when zero is in sight.
    if low <= 0.0 && high >= 0.0 {
        let y = y_of(0.0);
        let line = [pos2(chart.left() + CHART_PAD, y), pos2(chart.right() - CHART_PAD - 12.0, y)];
        ui.painter().add(Shape::dashed_line(&line, Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xA5, 0xB4, 0xFF, 0x50)), 2.0, 4.0));
        ui.painter().text(pos2(chart.right() - CHART_PAD, y), Align2::RIGHT_CENTER, "0°", theme::regular(9.0), Color32::from_rgba_unmultiplied(0xA5, 0xB4, 0xFF, 0xA0));
    }

    // Rain bars standing on their baseline.
    let step = x_of(1) - x_of(0);
    let bar_width = (step * 0.42).clamp(3.0, 9.0);
    for (index, hour) in hours.iter().enumerate() {
        if hour.rain <= 0.0 && hour.rain_chance < 30.0 {
            continue;
        }
        let amount = if hour.rain > 0.0 { (hour.rain / 4.0).sqrt().min(1.0) } else { hour.rain_chance / 300.0 };
        let height = (2.0 + amount * (BARS_H - 2.0)).min(BARS_H);
        let bar = Rect::from_min_max(pos2(x_of(index) - bar_width / 2.0, chart.top() + BARS_BOTTOM - height), pos2(x_of(index) + bar_width / 2.0, chart.top() + BARS_BOTTOM));
        ui.painter().rect_filled(bar, rounded(2.0_f32.min(bar_width / 2.0)), Color32::from_rgba_unmultiplied(0x5A, 0xB0, 0xFF, 0xC0));
    }

    // The curve: a monotone cubic through the hours, so it never overshoots a value.
    let points: Vec<(f32, f32, f32)> = hours.iter().enumerate().map(|(i, h)| (x_of(i), y_of(h.temperature), celsius(h.temperature))).collect();
    let samples = monotone_curve(&points, 8);
    let bottom = chart.top() + CURVE_BOTTOM + 14.0;
    let mut fill = egui::epaint::Mesh::default();
    for (x, y, temperature) in &samples {
        let color = temperature_color(*temperature);
        fill.colored_vertex(pos2(*x, *y), Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 0x48));
        fill.colored_vertex(pos2(*x, bottom), Color32::TRANSPARENT);
    }
    for i in 0..samples.len().saturating_sub(1) as u32 {
        let (top, low_vertex, next_top, next_low) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
        fill.add_triangle(top, low_vertex, next_top);
        fill.add_triangle(next_top, low_vertex, next_low);
    }
    ui.painter().add(fill);
    for pair in samples.windows(2) {
        let ((x0, y0, t0), (x1, y1, t1)) = (pair[0], pair[1]);
        let color = temperature_color((t0 + t1) / 2.0);
        ui.painter().line_segment([pos2(x0, y0), pos2(x1, y1)], Stroke::new(2.6, color));
    }
    for (x, y, temperature) in [samples[0], samples[samples.len() - 1]] {
        ui.painter().circle_filled(pos2(x, y), 1.3, temperature_color(temperature));
    }

    // Every third hour from now: the temperature above a small dot, the icon and the time below.
    for (index, hour) in hours.iter().enumerate() {
        let hour_of_day = hour.time.get(11..13).and_then(|h| h.parse::<u32>().ok()).unwrap_or(0);
        let first = index == 0;
        if index % 3 != 0 || index + 1 >= hours.len() {
            continue;
        }
        let (x, y) = (x_of(index), y_of(hour.temperature));
        ui.painter().circle_filled(pos2(x, y), 2.5, Color32::WHITE);
        ui.painter().circle_stroke(pos2(x, y), 2.5, Stroke::new(1.2, Color32::BLACK));
        ui.painter().text(pos2(x, y - 6.0), Align2::CENTER_BOTTOM, format!("{:.0}°", hour.temperature), theme::semibold(11.0), Color32::WHITE);
        small_icon(ui, pos2(x, chart.top() + ICONS_TOP + 8.0), hour.code, hour.is_day, 15.0);
        let label = if first { t("weather.now") } else { format!("{hour_of_day:02}") };
        let color = if first { SOFT } else { MUTED };
        ui.painter().text(pos2(x, chart.top() + TIMES_TOP), Align2::CENTER_TOP, label, theme::regular(10.0), color);
    }
}

/// Samples a monotone cubic (Fritsch–Carlson) through `points` (x, y, value), `per_segment` per gap.
fn monotone_curve(points: &[(f32, f32, f32)], per_segment: usize) -> Vec<(f32, f32, f32)> {
    let n = points.len();
    if n < 2 {
        return points.to_vec();
    }
    let slopes: Vec<f32> = points.windows(2).map(|p| (p[1].1 - p[0].1) / (p[1].0 - p[0].0).max(0.001)).collect();
    let mut tangents = vec![0.0_f32; n];
    tangents[0] = slopes[0];
    tangents[n - 1] = slopes[n - 2];
    for i in 1..n - 1 {
        tangents[i] = if slopes[i - 1] * slopes[i] <= 0.0 { 0.0 } else { (slopes[i - 1] + slopes[i]) / 2.0 };
    }
    for i in 0..n - 1 {
        if slopes[i] == 0.0 {
            tangents[i] = 0.0;
            tangents[i + 1] = 0.0;
            continue;
        }
        let (a, b) = (tangents[i] / slopes[i], tangents[i + 1] / slopes[i]);
        let length = (a * a + b * b).sqrt();
        if length > 3.0 {
            let scale = 3.0 / length;
            tangents[i] = scale * a * slopes[i];
            tangents[i + 1] = scale * b * slopes[i];
        }
    }
    let mut samples = Vec::with_capacity((n - 1) * per_segment + 1);
    for i in 0..n - 1 {
        let ((x0, y0, v0), (x1, y1, v1)) = (points[i], points[i + 1]);
        let h = x1 - x0;
        for step in 0..per_segment {
            let s = step as f32 / per_segment as f32;
            let (s2, s3) = (s * s, s * s * s);
            let y = (2.0 * s3 - 3.0 * s2 + 1.0) * y0 + (s3 - 2.0 * s2 + s) * h * tangents[i] + (-2.0 * s3 + 3.0 * s2) * y1 + (s3 - s2) * h * tangents[i + 1];
            samples.push((x0 + h * s, y, v0 + (v1 - v0) * s));
        }
    }
    samples.push(points[n - 1]);
    samples
}

/// A weather symbol in the original's colours: sun yellow, clouds light grey, rain blue, bolts yellow.
fn small_icon(ui: &Ui, center: Pos2, code: i32, is_day: bool, size: f32) {
    let color = match code {
        0 | 1 if is_day => SUN,
        0 | 1 => MOON,
        51..=67 | 80..=82 => RAIN,
        95..=99 => BOLT,
        _ => CLOUD,
    };
    ui.painter().text(center, Align2::CENTER_CENTER, condition_icon(code, is_day), theme::icon_fill(size), color);
}

/// The week: day, icon, rain chance, low, a range bar coloured from low to high (with today's
/// temperature marked) and high, one 20-point row each.
fn day_rows(ui: &Ui, area: Rect, forecast: &Forecast, fahrenheit: bool) {
    let days: Vec<_> = forecast.days.iter().take(7).collect();
    if days.is_empty() {
        return;
    }
    let celsius = |value: f32| if fahrenheit { (value - 32.0) * 5.0 / 9.0 } else { value };
    let week_low = days.iter().map(|d| d.min).fold(f32::MAX, f32::min);
    let week_high = days.iter().map(|d| d.max).fold(f32::MIN, f32::max);
    let today_date = chrono::Local::now().date_naive();
    for (index, day) in days.iter().enumerate() {
        let row = Rect::from_min_size(pos2(area.left(), area.top() + index as f32 * DAY_ROW_H), vec2(area.width(), DAY_ROW_H));
        let inner = row.shrink2(vec2(4.0, 0.0));
        let y = row.center().y;
        let date = NaiveDate::parse_from_str(&day.date, "%Y-%m-%d").ok();
        let is_today = date == Some(today_date) || (index == 0 && date.is_none());
        let name = if is_today { t("weather.today") } else { date.map(|d| weekday_short(d.weekday())).unwrap_or_default() };
        let font = if is_today { theme::semibold(11.5) } else { theme::medium(11.5) };
        text_clipped(ui, pos2(inner.left(), y), Align2::LEFT_CENTER, &name, font, Color32::WHITE, 46.0);
        small_icon(ui, pos2(inner.left() + 46.0 + 12.0, y), day.code, true, 14.0);
        if day.rain_chance >= 20.0 {
            ui.painter().text(pos2(inner.left() + 46.0 + 24.0, y), Align2::LEFT_CENTER, format!("{:.0} %", day.rain_chance), theme::semibold(10.0), RAIN);
        }
        let low_right = inner.left() + 46.0 + 24.0 + 40.0 + 32.0;
        ui.painter().text(pos2(low_right, y), Align2::RIGHT_CENTER, format!("{:.0}°", day.min), theme::regular(11.0), MUTED);
        ui.painter().text(pos2(inner.right(), y), Align2::RIGHT_CENTER, format!("{:.0}°", day.max), theme::semibold(11.0), Color32::WHITE);
        let track = Rect::from_min_max(pos2(low_right + 9.0, y - 2.5), pos2(inner.right() - 32.0 - 9.0, y + 2.5));
        ui.painter().rect_filled(track, rounded(2.5), theme::alpha(Color32::WHITE, 0x30 as f32 / 255.0 * 0.6));
        let span = (week_high - week_low).max(1.0);
        let start = track.left() + (day.min - week_low) / span * track.width();
        let width = ((day.max - day.min) / span * track.width()).max(6.0);
        let segment = Rect::from_min_size(pos2(start.min(track.right() - width), track.top()), vec2(width, track.height()));
        kit::horizontal_gradient_rect(ui, segment, 2.5, temperature_color(celsius(day.min)), temperature_color(celsius(day.max)));
        if is_today {
            let now_x = track.left() + ((forecast.temperature - week_low) / span).clamp(0.0, 1.0) * track.width();
            ui.painter().circle_filled(pos2(now_x, y), 3.5, Color32::WHITE);
            ui.painter().circle_stroke(pos2(now_x, y), 3.5, Stroke::new(1.6, Color32::BLACK));
        }
    }
}

fn weekday_short(day: chrono::Weekday) -> String {
    const DAYS: [&str; 7] = ["port.day.mon", "port.day.tue", "port.day.wed", "port.day.thu", "port.day.fri", "port.day.sat", "port.day.sun"];
    t(DAYS[day.num_days_from_monday() as usize]).chars().take(3).collect()
}

fn message(ui: &Ui, body: Rect, title: &str, detail: &str) {
    ui.painter().text(body.center() - vec2(0.0, 24.0), Align2::CENTER_CENTER, icons::CLOUD_SLASH, theme::icon(30.0), MUTED);
    ui.painter().text(body.center() + vec2(0.0, 8.0), Align2::CENTER_CENTER, title, theme::semibold(15.0), Color32::WHITE);
    text_clipped(ui, body.center() + vec2(0.0, 28.0), Align2::CENTER_CENTER, detail, theme::regular(11.5), MUTED, body.width() - 20.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hour(time: &str, rain_chance: f32, rain: f32, code: i32) -> HourForecast {
        HourForecast { time: time.into(), rain_chance, rain, code, ..Default::default() }
    }

    #[test]
    fn outlook_uses_known_keys() {
        crate::i18n::set_language("en");
        let dry = vec![hour("2026-11-07T10:00", 0.0, 0.0, 0); 12];
        assert_eq!(nowcast_text(&dry).0, "No rain in the next 12 hours");
        let soon = vec![hour("2026-11-07T10:00", 0.0, 0.0, 0), hour("2026-11-07T11:00", 80.0, 1.0, 61)];
        let (text, wet, _) = nowcast_text(&soon);
        assert!(text.contains("11:00") && !wet, "{text}");
        let now = vec![hour("2026-11-07T10:00", 80.0, 1.0, 61), hour("2026-11-07T11:00", 0.0, 0.0, 0)];
        let (text, wet, _) = nowcast_text(&now);
        assert!(text.contains("11:00") && wet, "{text}");
        let snowing = vec![hour("2026-11-07T10:00", 80.0, 1.0, 71); 3];
        let (text, _, snow) = nowcast_text(&snowing);
        assert!(!text.starts_with("weather.") && snow);
    }

    #[test]
    fn temperature_scale_matches_the_original() {
        assert_eq!(temperature_color(-30.0), rgb(0xA5B4FF));
        assert_eq!(temperature_color(21.0), rgb(0xFFD54A));
        assert_eq!(temperature_color(50.0), rgb(0xFF5A5A));
    }

    #[test]
    fn curve_passes_through_every_point_without_overshoot() {
        let points = [(0.0, 10.0, 0.0), (10.0, 20.0, 1.0), (20.0, 20.0, 2.0), (30.0, 5.0, 3.0)];
        let samples = monotone_curve(&points, 6);
        assert_eq!(samples.len(), 3 * 6 + 1);
        for (x, y, _) in &samples {
            assert!((5.0..=20.0).contains(y), "y {y} at x {x} leaves the range");
        }
        assert!(samples.iter().any(|(x, y, _)| *x == 10.0 && (*y - 20.0).abs() < 1e-4));
    }
}
