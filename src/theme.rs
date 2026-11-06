//! Fonts, colours and text sizes shared by all windows.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId};
use std::sync::Arc;

pub const WHITE: Color32 = Color32::from_rgb(245, 245, 247);
pub const DIM: Color32 = Color32::from_rgb(160, 160, 168);
pub const FAINT: Color32 = Color32::from_rgb(105, 105, 112);
pub const CARD: Color32 = Color32::from_rgb(28, 28, 30);
pub const CARD_HOVER: Color32 = Color32::from_rgb(40, 40, 44);
pub const LINE: Color32 = Color32::from_rgb(52, 52, 56);
pub const ACCENT: Color32 = Color32::from_rgb(10, 132, 255);
pub const GREEN: Color32 = Color32::from_rgb(48, 209, 88);
pub const ORANGE: Color32 = Color32::from_rgb(255, 159, 10);
pub const RED: Color32 = Color32::from_rgb(255, 69, 58);
pub const YELLOW: Color32 = Color32::from_rgb(255, 214, 10);

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn icon(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("ph".into()))
}

pub fn icon_fill(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("ph-fill".into()))
}

/// System fonts that add the scripts Inter lacks. Titles and lyrics can be in any language.
fn system_fallbacks() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &[
            "C:\\Windows\\Fonts\\segoeui.ttf",
            "C:\\Windows\\Fonts\\msyh.ttc",
            "C:\\Windows\\Fonts\\YuGothR.ttc",
            "C:\\Windows\\Fonts\\malgun.ttf",
            "C:\\Windows\\Fonts\\Nirmala.ttf",
            "C:\\Windows\\Fonts\\seguisym.ttf",
        ]
    }
    #[cfg(target_os = "macos")]
    {
        &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",
            "/System/Library/Fonts/Kohinoor.ttc",
            "/System/Library/Fonts/GeezaPro.ttc",
        ]
    }
    #[cfg(target_os = "linux")]
    {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansDevanagari-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]
    }
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, data: &'static [u8]| {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(data)));
    };
    add("inter", include_bytes!("../assets/fonts/Inter-Regular.ttf"));
    add("inter-medium", include_bytes!("../assets/fonts/Inter-Medium.ttf"));
    add("inter-semibold", include_bytes!("../assets/fonts/Inter-SemiBold.ttf"));
    add("inter-bold", include_bytes!("../assets/fonts/Inter-Bold.ttf"));
    add("ph", include_bytes!("../assets/fonts/Phosphor.ttf"));
    add("ph-fill", include_bytes!("../assets/fonts/Phosphor-Fill.ttf"));

    let mut fallback_names = Vec::new();
    for (index, path) in system_fallbacks().iter().enumerate() {
        if let Ok(bytes) = std::fs::read(path) {
            let name = format!("system-{index}");
            fonts.font_data.insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
            fallback_names.push(name);
        }
    }
    let built_in = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();

    let chain = |first: &str| {
        let mut chain = vec![first.to_owned()];
        chain.extend(fallback_names.iter().cloned());
        chain.extend(built_in.iter().cloned());
        chain.push("ph".to_owned());
        chain
    };
    fonts.families.insert(FontFamily::Proportional, chain("inter"));
    fonts.families.insert(FontFamily::Name("medium".into()), chain("inter-medium"));
    fonts.families.insert(FontFamily::Name("semibold".into()), chain("inter-semibold"));
    fonts.families.insert(FontFamily::Name("bold".into()), chain("inter-bold"));
    fonts.families.insert(FontFamily::Name("ph".into()), vec!["ph".to_owned()]);
    fonts.families.insert(FontFamily::Name("ph-fill".into()), vec!["ph-fill".to_owned()]);
    ctx.set_fonts(fonts);
}

pub fn apply_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = Color32::from_rgb(14, 14, 16);
    visuals.panel_fill = Color32::from_rgb(14, 14, 16);
    visuals.extreme_bg_color = Color32::from_rgb(24, 24, 27);
    visuals.selection.bg_fill = ACCENT.linear_multiply(0.45);
    visuals.hyperlink_color = ACCENT;
    visuals.override_text_color = Some(WHITE);
    ctx.set_visuals(visuals);
    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.interaction.tooltip_delay = 0.4;
    });
}

/// Multiplies only the alpha, keeping the colour itself.
pub fn alpha(color: Color32, factor: f32) -> Color32 {
    color.gamma_multiply(factor.clamp(0.0, 1.0))
}
