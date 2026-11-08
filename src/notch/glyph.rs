//! The original's vector icons. Each is SVG path data, drawn once per pixel size into a white
//! texture and tinted when painted, so it stays sharp at every size and display scale.

use super::kit::cached_texture;
use eframe::egui::{self, Color32, Pos2, Rect, pos2, vec2};
use resvg::{tiny_skia, usvg};

#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    name: &'static str,
    /// Path data, or whole SVG markup (starting with `<`) for icons made of several shapes.
    data: &'static str,
    /// Turned by this many degrees around its centre, as some icons are in the original.
    rotate: f32,
}

impl Glyph {
    const fn new(name: &'static str, data: &'static str) -> Self {
        Glyph { name, data, rotate: 0.0 }
    }

    const fn turned(self, degrees: f32) -> Self {
        Glyph { rotate: degrees, ..self }
    }
}

/// The original's WeatherGlyph: a sun up left, its rays cut back 1.2 around the cloud that covers it.
pub const WEATHER: Glyph = Glyph::new(
    "weather",
    concat!(
        r##"<defs><mask id="m" maskUnits="userSpaceOnUse" x="-5" y="-5" width="30" height="30">"##,
        r##"<rect x="-5" y="-5" width="30" height="30" fill="#fff"/>"##,
        r##"<path d="M6.5,17 L15.5,17 A3.5,3.5 0 1 0 15.25,10.01 A4.5,4.5 0 0 0 6.53,11 A3,3 0 0 0 6.5,17 Z" fill="#000" stroke="#000" stroke-width="2.4"/>"##,
        r##"</mask></defs><g mask="url(#m)"><circle cx="7" cy="7.4" r="3" fill="#fff"/>"##,
        r##"<path d="M11.6,7.4L12.9,7.4M10.2527,10.6527L11.1719,11.5719M7,12L7,13.3M3.7473,10.6527L2.8281,11.5719"##,
        r##"M2.4,7.4L1.1,7.4M3.7473,4.1473L2.8281,3.2281M7,2.8L7,1.5M10.2527,4.1473L11.1719,3.2281" stroke="#fff" stroke-width="1.5" stroke-linecap="round"/></g>"##,
        r##"<path d="M6.5,17 L15.5,17 A3.5,3.5 0 1 0 15.25,10.01 A4.5,4.5 0 0 0 6.53,11 A3,3 0 0 0 6.5,17 Z" fill="#fff"/>"##,
    ),
);

/// The grey camera in the middle of the camera tab (Ionicons, as in the original).
pub const CAMERA_LENS: Glyph = Glyph::new(
    "camera-lens",
    "M256,208A64,64,0,1,0,320,272,64,64,0,0,0,256,208Z M432,144H373c-3,0-6.72-1.94-9.62-5L337.44,98.06a15.52,15.52,0,0,0-1.37-1.85C327.11,85.76,315,80,302,80H210c-13,0-25.11,5.76-34.07,16.21a15.52,15.52,0,0,0-1.37,1.85l-25.94,41c-2.22,2.42-5.34,5-8.62,5v-8a16,16,0,0,0-16-16H100a16,16,0,0,0-16,16v8H80a48.05,48.05,0,0,0-48,48V384a48.05,48.05,0,0,0,48,48H432a48.05,48.05,0,0,0,48-48V192A48.05,48.05,0,0,0,432,144ZM256,368a96,96,0,1,1,96-96A96.11,96.11,0,0,1,256,368Z",
);

/// The warning triangle of the camera tab's error overlay, drawn as an outline.
pub const WARNING: Glyph = Glyph::new(
    "warning",
    r##"<path d="M12 9v4m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 17c-.77 1.333.192 3 1.732 3z" fill="none" stroke="#fff" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>"##,
);

// Nav bar icons, from MainWindow.xaml and the glyph classes of the original.
pub const HOME: Glyph = Glyph::new("home", "M8 0L0 6V8H1V15H4V10H7V15H15V8H16V6L14 4.5V1H11V2.25L8 0ZM9 10H12V13H9V10Z");
pub const TRAY: Glyph = Glyph::new(
    "tray",
    "M479.66,268.7l-32-151.81C441.48,83.77,417.68,64,384,64H128c-16.8,0-31,4.69-42.1,13.94s-18.37,22.31-21.58,38.89l-32,151.87A16.65,16.65,0,0,0,32,272V384a64,64,0,0,0,64,64H416a64,64,0,0,0,64-64V272A16.65,16.65,0,0,0,479.66,268.7Zm-384-145.4c0-.1,0-.19,0-.28,3.55-18.43,13.81-27,32.29-27H384c18.61,0,28.87,8.55,32.27,26.91,0,.13.05.26.07.39l26.93,127.88a4,4,0,0,1-3.92,4.82H320a15.92,15.92,0,0,0-16,15.82,48,48,0,1,1-96,0A15.92,15.92,0,0,0,192,256H72.65a4,4,0,0,1-3.92-4.82Z",
);
pub const TOGGLES: Glyph = Glyph::new(
    "toggles",
    "M6 3H14A4 4 0 0 1 14 11H6A4 4 0 0 1 6 3ZM6 5A2 2 0 1 0 6 9A2 2 0 1 0 6 5ZM6 12H14A4 4 0 0 1 14 20H6A4 4 0 0 1 6 12ZM14 14A2 2 0 1 0 14 18A2 2 0 1 0 14 14Z",
);
pub const CLAUDE: Glyph = Glyph::new(
    "claude",
    "M10,0.4 L10.54,7.97 L14.8,1.69 L11.48,8.52 L18.31,5.2 L12.03,9.46 L19.6,10 L12.03,10.54 L18.31,14.8 L11.48,11.48 L14.8,18.31 L10.54,12.03 L10,19.6 L9.46,12.03 L5.2,18.31 L8.52,11.48 L1.69,14.8 L7.97,10.54 L0.4,10 L7.97,9.46 L1.69,5.2 L8.52,8.52 L5.2,1.69 L9.46,7.97 Z",
);
pub const SPEAKER: Glyph = Glyph::new(
    "speaker",
    "M13.5 2.5C13.5 2.1 13.05 1.86 12.72 2.09L6.8 6.2H3.5C2.95 6.2 2.5 6.65 2.5 7.2V12.8C2.5 13.35 2.95 13.8 3.5 13.8H6.8L12.72 17.91C13.05 18.14 13.5 17.9 13.5 17.5V2.5ZM16.04 6.05C15.74 5.79 15.28 5.82 15.02 6.13C14.76 6.43 14.79 6.89 15.1 7.15C16.0 7.93 16.5 8.93 16.5 10C16.5 11.07 16.0 12.07 15.1 12.85C14.79 13.11 14.76 13.57 15.02 13.87C15.28 14.18 15.74 14.21 16.04 13.95C17.25 12.91 18 11.5 18 10C18 8.5 17.25 7.09 16.04 6.05Z",
);
pub const CAMERA: Glyph = Glyph::new(
    "camera",
    "M28,4 L25,4 L24,2 C23.411,0.837 23.104,0 22,0 L10,0 C8.896,0 8.53,0.954 8,2 L7,4 L4,4 C1.791,4 0,5.791 0,8 L0,24 C0,26.209 1.791,28 4,28 L28,28 C30.209,28 32,26.209 32,24 L32,8 C32,5.791 30.209,4 28,4 Z M16,24 C11.582,24 8,20.418 8,16 C8,11.582 11.582,8 16,8 C20.418,8 24,11.582 24,16 C24,20.418 20.418,24 16,24 Z M16,10 C12.687,10 10,12.687 10,16 C10,19.313 12.687,22 16,22 C19.313,22 22,19.313 22,16 C22,12.687 19.313,10 16,10 L16,10 Z",
);
pub const GEAR: Glyph = Glyph::new(
    "gear",
    "M 28.0000 50.8281 C 28.6094 50.8281 29.1719 50.7578 29.8047 50.7110 L 31.1641 53.2891 C 31.4219 53.8047 31.9375 54.0859 32.5704 53.9922 C 33.1563 53.8750 33.5782 53.4532 33.6485 52.8203 L 34.0704 49.9844 C 35.2188 49.6797 36.3672 49.2344 37.4922 48.7422 L 39.6016 50.6406 C 40.0469 51.0625 40.6094 51.1563 41.1953 50.8281 C 41.7110 50.5469 41.9219 49.9844 41.8047 49.3750 L 41.2188 46.5625 C 42.1797 45.8828 43.1407 45.1094 44.0078 44.2422 L 46.6328 45.3438 C 47.2189 45.5781 47.7579 45.4610 48.2034 44.9219 C 48.6018 44.5000 48.6483 43.8906 48.3205 43.3750 L 46.7970 40.9375 C 47.5002 39.9532 48.0627 38.8984 48.6018 37.7735 L 51.4611 37.9141 C 52.0703 37.9375 52.5858 37.6094 52.7970 37.0235 C 52.9842 36.4375 52.7970 35.8750 52.3045 35.5 L 50.0783 33.7188 C 50.3831 32.5937 50.6173 31.3750 50.7109 30.1094 L 53.4296 29.2657 C 54.0157 29.0547 54.3906 28.6094 54.3906 27.9766 C 54.3906 27.3672 54.0157 26.9219 53.4296 26.7110 L 50.7109 25.8437 C 50.6173 24.5781 50.3831 23.4063 50.0783 22.2578 L 52.3045 20.4532 C 52.7970 20.1016 52.9842 19.5625 52.7970 18.9766 C 52.5858 18.3906 52.0703 18.0625 51.4611 18.0859 L 48.6018 18.2032 C 48.0627 17.0781 47.5002 16.0235 46.7970 15.0391 L 48.3205 12.6016 C 48.6483 12.1094 48.6018 11.5000 48.2034 11.0781 C 47.7579 10.5391 47.2189 10.3984 46.6328 10.6563 L 44.0078 11.7110 C 43.1407 10.8906 42.1797 10.0937 41.2188 9.4141 L 41.8047 6.6250 C 41.9219 5.9922 41.7110 5.4297 41.1953 5.1484 C 40.6094 4.8437 40.0469 4.8906 39.6016 5.3594 L 37.4922 7.2110 C 36.3672 6.7188 35.2188 6.3203 34.0704 5.9922 L 33.6485 3.1563 C 33.5782 2.5469 33.1563 2.1016 32.5704 2.0078 C 31.9375 1.9141 31.4219 2.1719 31.1641 2.6875 L 29.8047 5.2657 C 29.1719 5.2188 28.6094 5.1719 28.0000 5.1719 C 27.3672 5.1719 26.8047 5.2188 26.1485 5.2657 L 24.8126 2.6875 C 24.5547 2.1719 24.0391 1.9141 23.3829 2.0078 C 22.7969 2.1016 22.3985 2.5469 22.3047 3.1563 L 21.9063 5.9688 C 20.7344 6.3203 19.5860 6.7188 18.4844 7.2110 L 16.3751 5.3594 C 15.9063 4.8906 15.3438 4.8437 14.7813 5.1484 C 14.2657 5.4297 14.0547 5.9922 14.1719 6.6250 L 14.7578 9.4141 C 13.7735 10.0937 12.8126 10.8906 11.9453 11.7110 L 9.3438 10.6563 C 8.7578 10.3984 8.1953 10.5391 7.7500 11.0781 C 7.3751 11.5000 7.3282 12.1094 7.6329 12.5781 L 9.1797 15.0391 C 8.4766 16.0235 7.9141 17.0781 7.3751 18.2032 L 4.4922 18.0859 C 3.8829 18.0625 3.3672 18.3906 3.1797 18.9766 C 2.9922 19.5625 3.1563 20.1016 3.6485 20.4532 L 5.8985 22.2578 C 5.5938 23.4063 5.3360 24.5781 5.2657 25.8437 L 2.5469 26.7110 C 1.9375 26.8984 1.6094 27.3437 1.6094 27.9766 C 1.6094 28.6094 1.9375 29.0547 2.5469 29.2657 L 5.2657 30.1328 C 5.3360 31.3750 5.5938 32.5937 5.8985 33.7188 L 3.6485 35.5 C 3.1563 35.8750 2.9922 36.4375 3.1797 37.0235 C 3.3672 37.6094 3.8829 37.9375 4.4922 37.9141 L 7.3751 37.7735 C 7.9141 38.8984 8.4766 39.9532 9.1563 40.9375 L 7.6563 43.3750 C 7.3047 43.8906 7.3516 44.5000 7.7500 44.9219 C 8.1953 45.4610 8.7578 45.5781 9.3438 45.3438 L 11.9453 44.2422 C 12.8126 45.1094 13.7735 45.8828 14.7578 46.5625 L 14.1719 49.3750 C 14.0547 49.9844 14.2657 50.5469 14.7813 50.8281 C 15.3672 51.1563 15.9297 51.0625 16.3751 50.6406 L 18.4610 48.7422 C 19.5860 49.2344 20.7344 49.6797 21.9063 49.9844 L 22.3047 52.8203 C 22.3985 53.4532 22.7969 53.8750 23.3829 53.9922 C 24.0391 54.0859 24.5547 53.8047 24.8126 53.2891 L 26.1485 50.7110 C 26.7813 50.7578 27.3672 50.8281 28.0000 50.8281 Z M 33.8829 26.5000 C 32.7813 23.5469 30.6251 21.9297 27.9297 21.9297 C 27.5313 21.9297 27.0626 21.9766 26.3126 22.1406 L 19.5860 10.6563 C 22.0938 9.4141 24.9531 8.7344 28.0000 8.7344 C 38.2188 8.7344 46.2344 16.4922 46.9611 26.5000 Z M 8.9688 28.0000 C 8.9688 21.4844 12.0391 15.7422 16.8907 12.2735 L 23.6641 23.8281 C 22.3985 25.1406 21.8126 26.5703 21.8126 28.0937 C 21.8126 29.5703 22.3751 30.9297 23.6641 32.2891 L 16.7266 43.6328 C 11.9922 40.1406 8.9688 34.4688 8.9688 28.0000 Z M 25.1172 28.0703 C 25.1172 26.4766 26.4766 25.2344 27.9766 25.2344 C 29.5704 25.2344 30.8829 26.4766 30.8829 28.0703 C 30.8829 29.6406 29.5704 30.9297 27.9766 30.9297 C 26.4766 30.9297 25.1172 29.6406 25.1172 28.0703 Z M 28.0000 47.2656 C 24.8829 47.2656 21.9766 46.5391 19.4219 45.2735 L 26.3126 34 C 27.0391 34.1875 27.5313 34.2344 27.9297 34.2344 C 30.6485 34.2344 32.8047 32.5703 33.8829 29.5469 L 46.9611 29.5469 C 46.2109 39.5078 38.1953 47.2656 28.0000 47.2656 Z",
)
.turned(45.0);

/// Media transport, from MediaPlaybackView.xaml. The original stretches play and pause to fill a
/// 48 × 46 box, which makes them wider than their paths; the scale keeps that look.
pub const PREVIOUS: Glyph = Glyph::new(
    "previous",
    "M30.71,229.47l188.87-113a30.54,30.54,0,0,1,31.09-.39,33.74,33.74,0,0,1,16.76,29.47V224.6L448.15,116.44a30.54,30.54,0,0,1,31.09-.39A33.74,33.74,0,0,1,496,145.52v221A33.73,33.73,0,0,1,479.24,396a30.54,30.54,0,0,1-31.09-.39L267.43,287.4v79.08A33.73,33.73,0,0,1,250.67,396a30.54,30.54,0,0,1-31.09-.39l-188.87-113a31.27,31.27,0,0,1,0-53Z",
);
pub const NEXT: Glyph = Glyph::new(
    "next",
    "M481.29,229.47l-188.87-113a30.54,30.54,0,0,0-31.09-.39,33.74,33.74,0,0,0-16.76,29.47V224.6L63.85,116.44a30.54,30.54,0,0,0-31.09-.39A33.74,33.74,0,0,0,16,145.52v221A33.74,33.74,0,0,0,32.76,396a30.54,30.54,0,0,0,31.09-.39L244.57,287.4v79.08A33.74,33.74,0,0,0,261.33,396a30.54,30.54,0,0,0,31.09-.39l188.87-113a31.27,31.27,0,0,0,0-53Z",
);
pub const PLAY: Glyph = Glyph::new(
    "play",
    r##"<path transform="scale(1.894,1)" fill="#fff" d="M9.629,44.68c-1.154,1.16-2.895,1.51-4.407,0.885c-1.513-0.623-2.5-2.1-2.5-3.735V4.043c0-1.637,0.987-3.112,2.5-3.736c1.513-0.625,3.253-0.275,4.407,0.885l17.862,17.951c2.088,2.098,2.088,5.488,0,7.585L9.629,44.68z"/>"##,
);
pub const PAUSE: Glyph = Glyph::new(
    "pause",
    r##"<path transform="scale(1.4,1)" fill="#fff" d="M172,3605 C171.448,3605 171,3605.448 171,3606 L171,3612 C171,3612.552 171.448,3613 172,3613 C172.552,3613 173,3612.552 173,3612 L173,3606 C173,3605.448 172.552,3605 172,3605 M177,3606 L177,3612 C177,3612.552 176.552,3613 176,3613 C175.448,3613 175,3612.552 175,3612 L175,3606 C175,3605.448 175.448,3605 176,3605 C176.552,3605 177,3605.448 177,3606"/>"##,
);
/// The music note shown in the cover's place when there is no artwork.
pub const MUSIC: Glyph = Glyph::new(
    "music",
    "M421.84,37.37a25.86,25.86,0,0,0-22.6-4.46L199.92,86.49A32.3,32.3,0,0,0,176,118v226c0,6.74-4.36,12.56-11.11,14.83l-.12.05-52,18C92.88,383.53,80,402,80,423.91a55.54,55.54,0,0,0,23.23,45.63A54.78,54.78,0,0,0,135.34,480a55.82,55.82,0,0,0,17.75-2.93l.38-.13L175.31,469A47.84,47.84,0,0,0,208,423.91v-212c0-7.29,4.77-13.21,12.16-15.07l.21-.06L395,150.14a4,4,0,0,1,5,3.86V295.93c0,6.75-4.25,12.38-11.11,14.68l-.25.09-50.89,18.11A49.09,49.09,0,0,0,304,375.92a55.67,55.67,0,0,0,23.23,45.8,54.63,54.63,0,0,0,49.88,7.35l.36-.12L399.31,421A47.83,47.83,0,0,0,432,375.92V58A25.74,25.74,0,0,0,421.84,37.37Z",
);

pub const BOLT: Glyph = Glyph::new("bolt", "M 13 2 L 3 14 L 8 14 L 7 22 L 17 10 L 12 10 L 13 2 Z");

/// White pixels of `glyph`, fitted into a `px` × `px` square and centred. The original's path
/// markup fills with the even-odd rule, which cuts the holes (the house's window, the toggles' knobs).
pub fn rasterize(glyph: Glyph, px: u32) -> Option<egui::ColorImage> {
    let body = if glyph.data.starts_with('<') {
        glyph.data.to_owned()
    } else {
        format!(r##"<path d="{}" fill="#ffffff" fill-rule="evenodd"/>"##, glyph.data)
    };
    let svg = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1">{body}</svg>"##);
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).ok()?;
    // With strokes, so outlined icons and the sun's rays fit in the square too.
    let bounds = tree.root().abs_stroke_bounding_box();
    let side = px as f32;
    let scale = side / bounds.width().max(bounds.height());
    let dx = (side - bounds.width() * scale) / 2.0 - bounds.left() * scale;
    let dy = (side - bounds.height() * scale) / 2.0 - bounds.top() * scale;
    let fit = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy);
    let transform = if glyph.rotate == 0.0 { fit } else { tiny_skia::Transform::from_rotate_at(glyph.rotate, side / 2.0, side / 2.0).pre_concat(fit) };
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(egui::ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data()))
}

/// Paints `glyph` `size` points wide and high, centred on `center`, in `color`.
pub fn paint(painter: &egui::Painter, center: Pos2, size: f32, glyph: Glyph, color: Color32) {
    let ppp = painter.ctx().pixels_per_point();
    let px = (size * ppp).round().max(1.0) as u32;
    let key = format!("glyph-{}-{px}", glyph.name);
    let texture = cached_texture(painter.ctx(), &key, || {
        rasterize(glyph, px).unwrap_or_else(|| egui::ColorImage::filled([1, 1], Color32::TRANSPARENT))
    });
    // On whole physical pixels, so the icon is as crisp as its texture.
    let side = px as f32 / ppp;
    let min = ((center.to_vec2() * ppp - vec2(px as f32, px as f32) / 2.0).round() / ppp).to_pos2();
    let rect = Rect::from_min_size(min, vec2(side, side));
    painter.image(texture, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), color);
}

/// Same as [`paint`], turned by `angle` radians about its centre.
pub fn paint_rotated(painter: &egui::Painter, center: Pos2, size: f32, glyph: Glyph, color: Color32, angle: f32) {
    if angle == 0.0 {
        paint(painter, center, size, glyph, color);
        return;
    }
    let ppp = painter.ctx().pixels_per_point();
    let px = (size * ppp).round().max(1.0) as u32;
    let key = format!("glyph-{}-{px}", glyph.name);
    let texture = cached_texture(painter.ctx(), &key, || {
        rasterize(glyph, px).unwrap_or_else(|| egui::ColorImage::filled([1, 1], Color32::TRANSPARENT))
    });
    let side = px as f32 / ppp;
    let mut mesh = egui::Mesh::with_texture(texture);
    mesh.add_rect_with_uv(Rect::from_center_size(center, vec2(side, side)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), color);
    mesh.rotate(egui::emath::Rot2::from_angle(angle), center);
    painter.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(image: &egui::ColorImage, x: usize, y: usize) -> u8 {
        image.pixels[y * image.size[0] + x].a()
    }

    #[test]
    fn every_glyph_draws_something() {
        for glyph in [HOME, TRAY, TOGGLES, CLAUDE, WEATHER, SPEAKER, CAMERA, CAMERA_LENS, WARNING, GEAR, BOLT] {
            let image = rasterize(glyph, 32).unwrap_or_else(|| panic!("{} did not parse", glyph.name));
            let ink = image.pixels.iter().filter(|p| p.a() > 128).count();
            assert!(ink > 60, "{} has only {ink} solid pixels", glyph.name);
        }
    }

    #[test]
    fn even_odd_cuts_the_holes() {
        // The house's window and the camera lens are holes, the walls around them are filled.
        let house = rasterize(HOME, 64).unwrap();
        assert_eq!(alpha(&house, 42, 46), 0, "window of the house");
        assert_eq!(alpha(&house, 10, 46), 255, "wall of the house");
        let camera = rasterize(CAMERA, 64).unwrap();
        assert!(alpha(&camera, 32, 32) > 200, "the lens centre is filled again (ring inside the hole)");
    }

    #[test]
    fn glyphs_fill_their_square_and_stay_centred() {
        let image = rasterize(TOGGLES, 40).unwrap();
        let columns: Vec<usize> = (0..40).filter(|&x| (0..40).any(|y| alpha(&image, x, y) > 0)).collect();
        let (first, last) = (columns[0], *columns.last().unwrap());
        assert!(first <= 6 && last >= 33, "toggles span {first}..{last}");
        assert!((first as i32 - (39 - last) as i32).abs() <= 1, "toggles are centred");
    }
}
