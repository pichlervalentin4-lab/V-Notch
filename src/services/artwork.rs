//! Album art: decode, crop away video letterboxing, derive the background blur and accent colour.

use eframe::egui::Color32;
use image::{GenericImageView, RgbaImage, imageops::FilterType};
use std::sync::Arc;

#[derive(Clone)]
pub struct Artwork {
    pub key: u64,
    /// Square cover for display.
    pub cover: Arc<RgbaImage>,
    /// Small and heavily blurred, stretched behind the expanded notch.
    pub blur: Arc<RgbaImage>,
    pub accent: Color32,
}

const COVER_SIDE: u32 = 256;
const BLUR_SIDE: u32 = 64;

pub fn process(bytes: &[u8], key: u64) -> Option<Artwork> {
    let decoded = image::load_from_memory(bytes).ok()?;
    let cropped = crop_bars(&decoded.to_rgba8());
    let square = center_square(&cropped);
    let cover = image::imageops::resize(&square, COVER_SIDE, COVER_SIDE, FilterType::Triangle);
    let mut blur = image::imageops::resize(&cover, BLUR_SIDE, BLUR_SIDE, FilterType::Triangle);
    box_blur(&mut blur, 6);
    box_blur(&mut blur, 6);
    let accent = accent_color(&cover);
    Some(Artwork { key, cover: Arc::new(cover), blur: Arc::new(blur), accent })
}

/// Video players hand out thumbnails with black bars around the picture.
pub fn crop_bars(image: &RgbaImage) -> RgbaImage {
    let (width, height) = image.dimensions();
    if width < 8 || height < 8 {
        return image.clone();
    }
    let dark_row = |y: u32| (0..width).step_by(4).all(|x| is_dark(image.get_pixel(x, y).0));
    let dark_column = |x: u32| (0..height).step_by(4).all(|y| is_dark(image.get_pixel(x, y).0));
    let top = (0..height / 3).find(|&y| !dark_row(y)).unwrap_or(0);
    let bottom = (height * 2 / 3..height).rev().find(|&y| !dark_row(y)).map_or(height, |y| y + 1);
    let left = (0..width / 3).find(|&x| !dark_column(x)).unwrap_or(0);
    let right = (width * 2 / 3..width).rev().find(|&x| !dark_column(x)).map_or(width, |x| x + 1);
    image.view(left, top, right - left, bottom - top).to_image()
}

fn is_dark(pixel: [u8; 4]) -> bool {
    pixel[0] < 16 && pixel[1] < 16 && pixel[2] < 16
}

fn center_square(image: &RgbaImage) -> RgbaImage {
    let (width, height) = image.dimensions();
    let side = width.min(height);
    image.view((width - side) / 2, (height - side) / 2, side, side).to_image()
}

/// Fast approximate blur: one horizontal and one vertical pass with a sliding window.
pub fn box_blur(image: &mut RgbaImage, radius: u32) {
    if radius == 0 {
        return;
    }
    let (width, height) = image.dimensions();
    let run = |get: &dyn Fn(u32) -> [u8; 4], length: u32, out: &mut Vec<[u8; 4]>| {
        out.clear();
        let window = radius as i64 * 2 + 1;
        let mut sum = [0_i64; 4];
        for i in -(radius as i64)..=(radius as i64) {
            let pixel = get(i.clamp(0, length as i64 - 1) as u32);
            (0..4).for_each(|c| sum[c] += i64::from(pixel[c]));
        }
        for i in 0..length as i64 {
            out.push([(sum[0] / window) as u8, (sum[1] / window) as u8, (sum[2] / window) as u8, (sum[3] / window) as u8]);
            let enter = get((i + radius as i64 + 1).clamp(0, length as i64 - 1) as u32);
            let leave = get((i - radius as i64).clamp(0, length as i64 - 1) as u32);
            (0..4).for_each(|c| sum[c] += i64::from(enter[c]) - i64::from(leave[c]));
        }
    };
    let mut out = Vec::new();
    for y in 0..height {
        let source = image.clone();
        run(&|x| source.get_pixel(x, y).0, width, &mut out);
        for (x, pixel) in out.iter().enumerate() {
            image.put_pixel(x as u32, y, image::Rgba(*pixel));
        }
    }
    for x in 0..width {
        let source = image.clone();
        run(&|y| source.get_pixel(x, y).0, height, &mut out);
        for (y, pixel) in out.iter().enumerate() {
            image.put_pixel(x, y as u32, image::Rgba(*pixel));
        }
    }
}

/// The most saturated bright colour of the cover, lifted enough to read on black.
pub fn accent_color(image: &RgbaImage) -> Color32 {
    let (mut best, mut best_score) = ([200.0_f32, 200.0, 205.0], -1.0_f32);
    let step = (image.width() / 24).max(1) as usize;
    for y in (0..image.height()).step_by(step) {
        for x in (0..image.width()).step_by(step) {
            let pixel = image.get_pixel(x, y).0;
            let [r, g, b] = [f32::from(pixel[0]), f32::from(pixel[1]), f32::from(pixel[2])];
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
            let brightness = max / 255.0;
            let score = saturation * (0.35 + brightness);
            if brightness > 0.25 && score > best_score {
                best = [r, g, b];
                best_score = score;
            }
        }
    }
    let max = best[0].max(best[1]).max(best[2]).max(1.0);
    let lift = if max < 190.0 { 190.0 / max } else { 1.0 };
    Color32::from_rgb(
        (best[0] * lift).min(255.0) as u8,
        (best[1] * lift).min(255.0) as u8,
        (best[2] * lift).min(255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn filled(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba(color))
    }

    #[test]
    fn black_bars_are_removed() {
        let mut image = filled(160, 90, [0, 0, 0, 255]);
        for y in 0..90 {
            for x in 35..125 {
                image.put_pixel(x, y, Rgba([200, 50, 50, 255]));
            }
        }
        assert_eq!(crop_bars(&image).dimensions(), (90, 90));
    }

    #[test]
    fn an_all_black_image_stays_whole() {
        assert_eq!(crop_bars(&filled(90, 90, [0, 0, 0, 255])).dimensions(), (90, 90));
    }

    #[test]
    fn square_is_centered() {
        let square = center_square(&filled(200, 100, [9, 9, 9, 255]));
        assert_eq!(square.dimensions(), (100, 100));
    }

    #[test]
    fn blur_keeps_a_flat_image_flat() {
        let mut image = filled(16, 16, [100, 150, 200, 255]);
        box_blur(&mut image, 3);
        assert_eq!(image.get_pixel(8, 8).0, [100, 150, 200, 255]);
    }

    #[test]
    fn blur_spreads_a_bright_spot() {
        let mut image = filled(17, 17, [0, 0, 0, 255]);
        image.put_pixel(8, 8, Rgba([255, 255, 255, 255]));
        box_blur(&mut image, 2);
        assert!(image.get_pixel(8, 8).0[0] < 255);
        assert!(image.get_pixel(9, 9).0[0] > 0);
        assert_eq!(image.get_pixel(0, 0).0[0], 0);
    }

    #[test]
    fn accent_prefers_the_vivid_colour_and_stays_readable() {
        let mut image = filled(64, 64, [20, 20, 20, 255]);
        for y in 0..8 {
            for x in 0..64 {
                image.put_pixel(x, y, Rgba([230, 30, 40, 255]));
            }
        }
        let accent = accent_color(&image);
        assert!(accent.r() > 200 && accent.g() < 80);
        // Dark but saturated covers get lifted.
        let dim = accent_color(&filled(64, 64, [80, 10, 10, 255]));
        assert!(dim.r() >= 180);
    }

    #[test]
    fn broken_bytes_give_no_artwork() {
        assert!(process(b"not an image", 1).is_none());
    }

    #[test]
    fn process_produces_all_parts() {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(filled(300, 200, [40, 120, 220, 255]))
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        let art = process(&bytes, 7).unwrap();
        assert_eq!(art.key, 7);
        assert_eq!(art.cover.dimensions(), (COVER_SIDE, COVER_SIDE));
        assert_eq!(art.blur.dimensions(), (BLUR_SIDE, BLUR_SIDE));
    }
}
