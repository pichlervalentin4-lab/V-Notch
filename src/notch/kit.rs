//! Small drawing and input helpers shared by the notch views.

use crate::theme;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Ui, Vec2, pos2, vec2};

pub fn rounded(radius: f32) -> CornerRadius {
    CornerRadius::same(radius.clamp(0.0, 255.0) as u8)
}

pub fn card(ui: &Ui, rect: Rect, radius: f32, fill: Color32) {
    ui.painter().rect_filled(rect, rounded(radius), fill);
}

/// A texture made once and kept for the whole run: gradients and soft shapes.
pub(super) fn cached_texture(ctx: &egui::Context, key: &str, make: impl FnOnce() -> egui::ColorImage) -> egui::TextureId {
    let id = egui::Id::new(("v-notch-texture", key));
    if let Some(handle) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return handle.id();
    }
    let handle = ctx.load_texture(key, make(), egui::TextureOptions::LINEAR);
    let texture = handle.id();
    ctx.data_mut(|d| d.insert_temp(id, handle));
    texture
}

/// A rounded rectangle with a horizontal gradient from `left` to `right`.
pub fn horizontal_gradient_rect(ui: &Ui, rect: Rect, radius: f32, left: Color32, right: Color32) {
    let key = format!("hgradient-{:?}-{:?}", left.to_array(), right.to_array());
    let texture = cached_texture(ui.ctx(), &key, || {
        const COLUMNS: usize = 128;
        let pixels = (0..COLUMNS).map(|column| lerp_color(left, right, column as f32 / (COLUMNS - 1) as f32)).collect();
        egui::ColorImage::new([COLUMNS, 1], pixels)
    });
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().add(egui::epaint::RectShape::filled(rect, rounded(radius), Color32::WHITE).with_texture(texture, uv));
}

/// A plain horizontal fade over `rect`, e.g. to let text run out into the black notch.
pub fn horizontal_fade(ui: &Ui, rect: Rect, left: Color32, right: Color32) {
    horizontal_gradient_rect(ui, rect, 0.0, left, right);
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_unmultiplied(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()), mix(a.a(), b.a()))
}

/// Soft white shapes that are drawn tinted. One texture each, so overlapping parts do not add up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Soft {
    /// A round glow that fades out towards the edge.
    Glow,
    /// A cloud made of three puffs with an even inside and a soft rim.
    Cloud,
}

impl Soft {
    /// Size in points when drawn at scale 1.
    pub fn size(self) -> Vec2 {
        match self {
            Soft::Glow => vec2(64.0, 64.0),
            Soft::Cloud => vec2(90.0, 60.0),
        }
    }

    /// Alpha in 0..1 at a point given in points, with the shape at its natural size.
    pub fn alpha_at(self, x: f32, y: f32) -> f32 {
        match self {
            Soft::Glow => {
                let distance = ((x - 32.0).powi(2) + (y - 32.0).powi(2)).sqrt() / 32.0;
                let fade = (1.0 - distance).clamp(0.0, 1.0);
                fade * fade * (3.0 - 2.0 * fade)
            }
            Soft::Cloud => {
                // Three puffs on a flat base; the union keeps one even alpha inside.
                const PUFFS: [(f32, f32, f32); 3] = [(23.0, 38.0, 16.0), (45.0, 30.0, 22.0), (67.0, 38.0, 15.0)];
                const BASE: f32 = 50.0;
                let puffs = PUFFS.iter().map(|&(cx, cy, r)| ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r).fold(f32::MAX, f32::min);
                // A rounded bar along the base, so the bottom is flat instead of bumpy.
                let bar = rounded_box_distance(x - 45.0, y - (BASE - 6.0), 31.0, 6.0, 6.0);
                let inside = puffs.min(bar).max(y - BASE);
                (0.5 - inside / 2.0).clamp(0.0, 1.0)
            }
        }
    }

    fn image(self) -> egui::ColorImage {
        const SCALE: f32 = 3.0;
        let size = self.size() * SCALE;
        let (width, height) = (size.x as usize, size.y as usize);
        let mut rgba = Vec::with_capacity(width * height * 4);
        for py in 0..height {
            for px in 0..width {
                let alpha = self.alpha_at((px as f32 + 0.5) / SCALE, (py as f32 + 0.5) / SCALE);
                rgba.extend_from_slice(&[255, 255, 255, (alpha * 255.0).round() as u8]);
            }
        }
        egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba)
    }
}

/// Signed distance from a point (relative to the centre) to a box with rounded corners.
fn rounded_box_distance(x: f32, y: f32, half_width: f32, half_height: f32, radius: f32) -> f32 {
    let qx = x.abs() - half_width + radius;
    let qy = y.abs() - half_height + radius;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

/// Draws a soft shape centred on `center`, scaled and tinted.
pub fn soft_shape(painter: &egui::Painter, shape: Soft, center: Pos2, scale: f32, tint: Color32) {
    let texture = cached_texture(painter.ctx(), &format!("soft-{shape:?}"), || shape.image());
    let rect = Rect::from_center_size(center, shape.size() * scale);
    painter.image(texture, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
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

/// A pill-shaped text button.
pub fn pill_button(ui: &mut Ui, rect: Rect, text: &str, fill: Color32, color: Color32, id: impl std::hash::Hash + std::fmt::Debug) -> Response {
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    let fill = if response.hovered() { fill.gamma_multiply(1.25) } else { fill };
    ui.painter().rect_filled(rect, rounded(rect.height() / 2.0), fill);
    text_clipped(ui, rect.center(), Align2::CENTER_CENTER, text, theme::medium(12.0), color, rect.width() - 12.0);
    response
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
    fn soft_shapes_are_even_inside_and_fade_out() {
        // The cloud is fully opaque wherever puffs overlap, so tinting gives one even colour.
        assert_eq!(Soft::Cloud.alpha_at(44.0, 31.0), 1.0);
        assert_eq!(Soft::Cloud.alpha_at(33.0, 36.0), 1.0);
        assert_eq!(Soft::Cloud.alpha_at(1.0, 1.0), 0.0);
        let size = Soft::Cloud.size();
        for x in [0.0, size.x] {
            for y in [0.0, size.y] {
                assert_eq!(Soft::Cloud.alpha_at(x, y), 0.0, "corner {x},{y} must be clear");
            }
        }
        assert!((Soft::Glow.alpha_at(32.0, 32.0) - 1.0).abs() < 1e-6);
        assert_eq!(Soft::Glow.alpha_at(0.0, 32.0), 0.0);
        assert!(Soft::Glow.alpha_at(32.0, 20.0) > Soft::Glow.alpha_at(32.0, 10.0));
    }

    #[test]
    fn lerp_color_hits_both_ends() {
        let (a, b) = (Color32::from_rgb(10, 20, 30), Color32::from_rgb(200, 100, 0));
        assert_eq!(lerp_color(a, b, 0.0), a);
        assert_eq!(lerp_color(a, b, 1.0), b);
    }

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
