//! Small drawing and input helpers shared by the notch views.

use crate::anim::lerp;
use crate::theme;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, pos2, vec2};

pub fn rounded(radius: f32) -> CornerRadius {
    CornerRadius::same(radius.clamp(0.0, 255.0) as u8)
}

pub fn card(ui: &Ui, rect: Rect, radius: f32, fill: Color32) {
    ui.painter().rect_filled(rect, rounded(radius), fill);
}

/// Cuts text to `max_width`, ending in an ellipsis, and paints it.
pub fn text_clipped(ui: &Ui, pos: Pos2, align: Align2, text: &str, font: FontId, color: Color32, max_width: f32) -> Rect {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(1.0));
    let galley = ui.painter().layout_job(job);
    let rect = align.anchor_size(pos, galley.size());
    let size = rect.size();
    ui.painter().galley(rect.min, galley, color);
    Rect::from_min_size(rect.min, size)
}

/// Wrapped text limited to `lines` lines. Returns the height used.
pub fn text_wrapped(ui: &Ui, pos: Pos2, text: &str, font: FontId, color: Color32, width: f32, lines: usize) -> f32 {
    let mut job = egui::text::LayoutJob::simple(text.to_string(), font, color, width.max(1.0));
    job.wrap.max_rows = lines;
    job.wrap.break_anywhere = false;
    let galley = ui.painter().layout_job(job);
    let height = galley.size().y;
    ui.painter().galley(pos, galley, color);
    height
}

pub fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x
}

/// Text that gently scrolls back and forth when it does not fit.
pub fn marquee(ui: &Ui, rect: Rect, text: &str, font: FontId, color: Color32, time: f64) -> bool {
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let overflow = galley.size().x - rect.width();
    let y = rect.center().y - galley.size().y / 2.0;
    if overflow <= 0.0 {
        ui.painter().galley(pos2(rect.left(), y), galley, color);
        return false;
    }
    let travel = overflow + 12.0;
    let seconds = (travel / 28.0).max(1.0) as f64;
    let cycle = seconds * 2.0 + 4.0;
    let phase = time % cycle;
    let offset = if phase < 2.0 {
        0.0
    } else if phase < 2.0 + seconds {
        (phase - 2.0) / seconds
    } else if phase < 4.0 + seconds {
        1.0
    } else {
        1.0 - (phase - 4.0 - seconds) / seconds
    };
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.galley(pos2(rect.left() - offset as f32 * travel, y), galley, color);
    true
}

/// A round icon button; it lights up on hover and reports clicks.
pub fn icon_button(ui: &mut Ui, rect: Rect, icon: &str, size: f32, color: Color32, id: impl std::hash::Hash + std::fmt::Debug) -> Response {
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    let scale = if response.is_pointer_button_down_on() { 0.9 } else { 1.0 };
    if response.hovered() {
        ui.painter().rect_filled(rect, rounded(rect.height() / 2.0), theme::alpha(Color32::WHITE, 0.12));
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, theme::icon(size * scale), color);
    response
}

/// Same, with the filled icon set.
pub fn icon_button_fill(ui: &mut Ui, rect: Rect, icon: &str, size: f32, color: Color32, id: impl std::hash::Hash + std::fmt::Debug) -> Response {
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    let scale = if response.is_pointer_button_down_on() { 0.9 } else { 1.0 };
    if response.hovered() {
        ui.painter().rect_filled(rect, rounded(rect.height() / 2.0), theme::alpha(Color32::WHITE, 0.12));
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, theme::icon_fill(size * scale), color);
    response
}

/// A horizontal value bar. Returns the new value while the user drags or clicks it.
pub fn slider(ui: &mut Ui, rect: Rect, value: f32, color: Color32, id: impl std::hash::Hash + std::fmt::Debug) -> Option<f32> {
    let response = ui.interact(rect.expand2(vec2(0.0, 6.0)), ui.id().with(id), Sense::click_and_drag());
    let active = response.dragged() || response.is_pointer_button_down_on();
    let height = if active || response.hovered() { rect.height() + 2.0 } else { rect.height() };
    let bar = Rect::from_center_size(rect.center(), vec2(rect.width(), height));
    let radius = rounded(height / 2.0);
    ui.painter().rect_filled(bar, radius, theme::alpha(Color32::WHITE, 0.16));
    let filled = bar.width() * value.clamp(0.0, 1.0);
    if filled > 0.5 {
        let fill = Rect::from_min_size(bar.min, vec2(filled.max(height), height));
        ui.painter().rect_filled(fill, radius, color);
    }
    if active && let Some(pointer) = response.interact_pointer_pos() {
        return Some(((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0));
    }
    None
}

pub fn toggle(ui: &mut Ui, rect: Rect, on: bool, color: Color32, id: impl std::hash::Hash + std::fmt::Debug) -> Response {
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    let t = ui.ctx().animate_bool(response.id.with("t"), on);
    let track = if on { theme::alpha(color, lerp(0.4, 1.0, t)) } else { theme::alpha(Color32::WHITE, 0.2) };
    ui.painter().rect_filled(rect, rounded(rect.height() / 2.0), track);
    let radius = rect.height() / 2.0 - 2.0;
    let x = lerp(rect.left() + radius + 2.0, rect.right() - radius - 2.0, t);
    ui.painter().circle_filled(pos2(x, rect.center().y), radius, Color32::WHITE);
    response
}

/// A pill-shaped text button.
pub fn pill_button(ui: &mut Ui, rect: Rect, text: &str, fill: Color32, color: Color32, id: impl std::hash::Hash + std::fmt::Debug) -> Response {
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    let fill = if response.hovered() { fill.gamma_multiply(1.25) } else { fill };
    ui.painter().rect_filled(rect, rounded(rect.height() / 2.0), fill);
    text_clipped(ui, rect.center(), Align2::CENTER_CENTER, text, theme::medium(12.0), color, rect.width() - 12.0);
    response
}

/// A round progress ring, as used for the timer.
pub fn ring(ui: &Ui, center: Pos2, radius: f32, width: f32, progress: f32, color: Color32) {
    ui.painter().circle_stroke(center, radius, Stroke::new(width, theme::alpha(Color32::WHITE, 0.14)));
    let progress = progress.clamp(0.0, 1.0);
    if progress <= 0.0 {
        return;
    }
    let steps = ((progress * 64.0) as usize).max(2);
    let points: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * progress * i as f32 / steps as f32;
            center + radius * Vec2::angled(angle)
        })
        .collect();
    ui.painter().add(egui::Shape::line(points, Stroke::new(width, color)));
}

/// Divides a rect into `count` equal columns with `gap` between them.
pub fn columns(rect: Rect, count: usize, gap: f32) -> Vec<Rect> {
    let count = count.max(1);
    let width = (rect.width() - gap * (count - 1) as f32) / count as f32;
    (0..count)
        .map(|i| Rect::from_min_size(pos2(rect.left() + (width + gap) * i as f32, rect.top()), vec2(width, rect.height())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_fill_the_rect_exactly() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(100.0, 50.0));
        let parts = columns(rect, 3, 5.0);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].left(), 10.0);
        assert!((parts[2].right() - 110.0).abs() < 0.001);
        assert_eq!(parts[1].height(), 50.0);
        assert!((parts[1].left() - parts[0].right() - 5.0).abs() < 0.001);
    }

    #[test]
    fn zero_columns_do_not_divide_by_zero() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 10.0));
        assert_eq!(columns(rect, 0, 5.0).len(), 1);
    }
}
