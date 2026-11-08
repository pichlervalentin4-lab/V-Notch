//! User settings: same options and defaults as the original, adapted for this port.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

pub const CURRENT_VERSION: i32 = 2;

/// Order and tokens of the tabs in the notch. "Secondary" is the shelf and clipboard, "Timer" the controls page.
pub const DEFAULT_TABS: &str = "Media,Secondary,Timer,Claude,Pi,School,Weather,Legion,AudioMixer,Camera";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub settings_version: i32,
    pub language: String,
    pub last_run_version: String,

    // Look
    pub width: f32,
    pub height: f32,
    pub corner_radius: f32,
    pub dynamic_island_width: f32,
    pub dynamic_island_height: f32,
    pub enable_dynamic_island_mode: bool,
    pub opacity: f32,
    pub enable_shadow: bool,
    pub enable_glow_on_hover: bool,
    pub enable_blur_effects: bool,
    pub show_media_art_background: bool,
    pub media_blur_dark_overlay: f32,
    pub slim_idle_notch: bool,
    pub clock_page_style: String,
    pub enable_glass_style: bool,
    pub glass_tint: f32,

    // Motion
    pub enable_animations: bool,
    pub enable_bounce_effect: bool,
    pub animation_speed: f32,

    // Behaviour
    pub auto_start: bool,
    pub stay_behind_windows: bool,
    pub invisible_mode: bool,
    pub enable_hover_expand: bool,
    pub enable_cursor_bypass: bool,
    pub disable_mouse_leave_auto_close: bool,
    pub keep_media_pinned_on_track_change: bool,
    pub reopen_last_view_on_expand: bool,
    pub hover_expand_delay: u32,
    pub hover_collapse_delay: u32,
    pub hover_zone_margin: f32,
    pub hide_on_exclusive_fullscreen: bool,
    pub hide_on_windowed_fullscreen: bool,
    pub enable_idle_auto_hide: bool,
    pub idle_auto_hide_delay: u32,
    pub enable_gesture_controls: bool,
    pub enable_hello_greeting: bool,
    pub monitor_index: usize,
    pub hide_camera: bool,
    pub nav_tab_order: String,
    pub visible_nav_tabs: String,
    pub invisible_hotkey: String,

    // Sound
    pub enable_interaction_sounds: bool,
    pub interaction_sound_volume: f32,
    pub play_claude_sounds: bool,

    // Notifications
    pub show_music_notifications: bool,
    pub show_system_notifications: bool,
    pub show_battery_indicator: bool,
    pub notification_duration: u32,

    // Clipboard
    pub clipboard_capture_text: bool,
    pub clipboard_capture_images: bool,
    pub clipboard_capture_files: bool,
    pub clipboard_auto_delete_sensitive: bool,
    pub clipboard_max_items: usize,
    pub clipboard_hotkey: String,

    // Spotlight
    pub enable_spotlight: bool,
    pub spotlight_hotkey: String,
    pub spotlight_default_ai: bool,
    pub spotlight_ai_provider: String,
    pub spotlight_openai_model: String,
    pub spotlight_gemini_model: String,
    pub spotlight_claude_model: String,
    pub spotlight_deepseek_model: String,
    pub spotlight_ollama_model: String,
    pub spotlight_ollama_url: String,
    pub spotlight_ai_words_per_second: u32,
    pub save_ai_chat_history: bool,
    pub enable_spotlight_history: bool,

    // Lyrics and media lookups
    pub enable_online_lyrics: bool,
    pub enable_online_artwork_lookup: bool,

    // Weather
    pub enable_weather: bool,
    pub manual_city: String,
    pub use_fahrenheit: bool,

    // Live translation
    pub enable_live_translation: bool,
    pub translation_hotkey: String,
    pub translation_source_language: String,
    pub translation_target_language: String,
    pub translation_engine: String,
    pub translation_ollama_model: String,
    pub enable_currency_conversion: bool,
    pub translation_target_currency: String,
    pub translation_ignore_fullscreen: bool,
    pub translation_excluded_apps: String,

    // Privacy
    pub enable_local_only_mode: bool,
    pub allow_online_ai: bool,
    pub allow_online_weather: bool,
    pub enable_privacy_indicators: bool,
    pub enable_diagnostic_logging: bool,

    // Updates
    pub auto_check_updates: bool,
    pub include_prerelease_updates: bool,

    // Devices
    pub camera_device: String,

    // School and home tabs
    /// The original's Raspberry Pi dashboard (e.g. http://100.x.y.z:4000 over Tailscale); empty hides its data.
    pub pi_dashboard_url: String,
    /// The school's Moodle, e.g. https://moodle.example.at. Its token lives in the secret store.
    pub school_moodle_url: String,
    /// WebUntis server (e.g. example.webuntis.com), school login name and user; the password lives in the secret store.
    pub school_untis_server: String,
    pub school_untis_school: String,
    pub school_untis_user: String,
    pub battery_device_id: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            settings_version: CURRENT_VERSION,
            language: "de".into(),
            last_run_version: String::new(),

            width: 230.0,
            height: 34.0,
            corner_radius: 10.0,
            dynamic_island_width: 220.0,
            dynamic_island_height: 40.0,
            enable_dynamic_island_mode: false,
            opacity: 1.0,
            enable_shadow: true,
            enable_glow_on_hover: true,
            enable_blur_effects: true,
            show_media_art_background: true,
            media_blur_dark_overlay: 0.0,
            slim_idle_notch: true,
            clock_page_style: "analog".into(),
            enable_glass_style: false,
            glass_tint: 0.55,

            enable_animations: true,
            enable_bounce_effect: true,
            animation_speed: 2.0,

            auto_start: false,
            stay_behind_windows: false,
            invisible_mode: false,
            enable_hover_expand: false,
            enable_cursor_bypass: false,
            disable_mouse_leave_auto_close: true,
            keep_media_pinned_on_track_change: true,
            reopen_last_view_on_expand: false,
            hover_expand_delay: 0,
            hover_collapse_delay: 500,
            hover_zone_margin: 60.0,
            hide_on_exclusive_fullscreen: false,
            hide_on_windowed_fullscreen: false,
            enable_idle_auto_hide: false,
            idle_auto_hide_delay: 5000,
            enable_gesture_controls: true,
            enable_hello_greeting: true,
            monitor_index: 0,
            hide_camera: false,
            nav_tab_order: DEFAULT_TABS.into(),
            visible_nav_tabs: DEFAULT_TABS.into(),
            invisible_hotkey: String::new(),

            enable_interaction_sounds: true,
            interaction_sound_volume: 0.55,
            play_claude_sounds: true,

            show_music_notifications: true,
            show_system_notifications: true,
            show_battery_indicator: true,
            notification_duration: 5000,

            clipboard_capture_text: true,
            clipboard_capture_images: true,
            clipboard_capture_files: true,
            clipboard_auto_delete_sensitive: false,
            clipboard_max_items: 200,
            clipboard_hotkey: String::new(),

            enable_spotlight: true,
            spotlight_hotkey: "Alt+Space".into(),
            spotlight_default_ai: false,
            spotlight_ai_provider: "Claude".into(),
            spotlight_openai_model: String::new(),
            spotlight_gemini_model: String::new(),
            spotlight_claude_model: String::new(),
            spotlight_deepseek_model: String::new(),
            spotlight_ollama_model: String::new(),
            spotlight_ollama_url: "http://127.0.0.1:11434".into(),
            spotlight_ai_words_per_second: 9,
            save_ai_chat_history: true,
            enable_spotlight_history: true,

            enable_online_lyrics: true,
            enable_online_artwork_lookup: true,

            enable_weather: true,
            manual_city: "Traun".into(),
            use_fahrenheit: false,

            enable_live_translation: false,
            translation_hotkey: "Ctrl+Shift+T".into(),
            translation_source_language: "auto".into(),
            translation_target_language: "de".into(),
            translation_engine: "ollama".into(),
            translation_ollama_model: "qwen3:4b".into(),
            enable_currency_conversion: true,
            translation_target_currency: "EUR".into(),
            translation_ignore_fullscreen: true,
            translation_excluded_apps: "cs2;csgo;VALORANT-Win64-Shipping;GenshinImpact".into(),

            enable_local_only_mode: false,
            allow_online_ai: true,
            allow_online_weather: true,
            enable_privacy_indicators: true,
            enable_diagnostic_logging: true,

            auto_check_updates: true,
            include_prerelease_updates: false,

            camera_device: String::new(),
            pi_dashboard_url: String::new(),
            school_moodle_url: String::new(),
            school_untis_server: String::new(),
            school_untis_school: String::new(),
            school_untis_user: String::new(),
            battery_device_id: String::new(),
        }
    }
}

/// Puts `new` (comma-separated tokens) right after `anchor` in a tab list, or at its end; tokens
/// already in the list stay where they are.
fn insert_after(list: &str, anchor: &str, new: &str) -> String {
    let mut tokens: Vec<String> = list.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect();
    let mut at = tokens.iter().position(|t| t == anchor).map_or(tokens.len(), |i| i + 1);
    for token in new.split(',') {
        if !tokens.iter().any(|t| t == token) {
            tokens.insert(at, token.to_string());
            at += 1;
        }
    }
    tokens.join(",")
}

impl Settings {
    /// Pulls every value back into the range the UI expects, whatever the file said.
    pub fn sanitize(&mut self) {
        let fallback = Settings::default();
        let clamp = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.width = clamp(self.width, 120.0, 460.0, fallback.width);
        self.height = clamp(self.height, 24.0, 60.0, fallback.height);
        self.corner_radius = clamp(self.corner_radius, 0.0, 30.0, fallback.corner_radius);
        self.dynamic_island_width = clamp(self.dynamic_island_width, 120.0, 460.0, fallback.dynamic_island_width);
        self.dynamic_island_height = clamp(self.dynamic_island_height, 24.0, 60.0, fallback.dynamic_island_height);
        self.opacity = clamp(self.opacity, 0.2, 1.0, 1.0);
        self.media_blur_dark_overlay = clamp(self.media_blur_dark_overlay, 0.0, 1.0, fallback.media_blur_dark_overlay);
        self.glass_tint = clamp(self.glass_tint, 0.0, 1.0, fallback.glass_tint);
        self.animation_speed = clamp(self.animation_speed, 0.25, 4.0, fallback.animation_speed);
        self.hover_zone_margin = clamp(self.hover_zone_margin, 0.0, 200.0, fallback.hover_zone_margin);
        self.interaction_sound_volume = clamp(self.interaction_sound_volume, 0.0, 1.0, fallback.interaction_sound_volume);
        self.hover_expand_delay = self.hover_expand_delay.min(5000);
        self.hover_collapse_delay = self.hover_collapse_delay.min(10000);
        self.idle_auto_hide_delay = self.idle_auto_hide_delay.clamp(500, 600_000);
        self.notification_duration = self.notification_duration.clamp(500, 60_000);
        self.clipboard_max_items = self.clipboard_max_items.clamp(10, 5000);
        self.spotlight_ai_words_per_second = self.spotlight_ai_words_per_second.clamp(1, 200);
        if !crate::i18n::LANGUAGES.iter().any(|(code, _)| *code == self.language) {
            self.language = fallback.language;
        }
        if !matches!(self.spotlight_ai_provider.as_str(), "OpenAI" | "Gemini" | "Claude" | "DeepSeek" | "Ollama") {
            self.spotlight_ai_provider = fallback.spotlight_ai_provider;
        }
        if !matches!(self.clock_page_style.as_str(), "analog" | "digital" | "word") {
            self.clock_page_style = fallback.clock_page_style;
        }
        // Version 2 brought the original's home and school tabs: show them after Claude, as there.
        if self.settings_version < 2 {
            self.nav_tab_order = insert_after(&self.nav_tab_order, "Claude", "Pi,School");
            self.visible_nav_tabs = insert_after(&self.visible_nav_tabs, "Claude", "Pi,School");
        }
        // Tabs of a removed feature, or tabs from an older file, must not break the bar.
        let known = |token: &str| DEFAULT_TABS.split(',').any(|t| t == token);
        let keep = |list: &str| list.split(',').map(str::trim).filter(|t| known(t)).collect::<Vec<_>>().join(",");
        let mut order = keep(&self.nav_tab_order);
        for token in DEFAULT_TABS.split(',') {
            if !order.split(',').any(|t| t == token) {
                if !order.is_empty() {
                    order.push(',');
                }
                order.push_str(token);
            }
        }
        self.nav_tab_order = order;
        self.visible_nav_tabs = keep(&self.visible_nav_tabs);
        self.settings_version = CURRENT_VERSION;
    }

    pub fn is_online_allowed(&self) -> bool {
        !self.enable_local_only_mode
    }

    pub fn tab_visible(&self, token: &str) -> bool {
        self.visible_nav_tabs.split(',').any(|t| t == token)
    }
}

pub fn file_path() -> PathBuf {
    crate::paths::config_dir().join("settings.json")
}

pub fn load() -> Settings {
    let mut settings = std::fs::read_to_string(file_path())
        .ok()
        .and_then(|text| parse(&text))
        .unwrap_or_default();
    settings.sanitize();
    settings
}

pub fn parse(text: &str) -> Option<Settings> {
    serde_json::from_str::<Settings>(text).ok()
}

/// Atomic, so a crash mid-write can never leave an empty settings file behind.
pub fn save(settings: &Settings) -> std::io::Result<()> {
    let path = file_path();
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)
}

#[derive(Clone)]
pub struct SharedSettings(Arc<RwLock<Settings>>);

impl SharedSettings {
    pub fn new(settings: Settings) -> Self {
        Self(Arc::new(RwLock::new(settings)))
    }

    pub fn get(&self) -> Settings {
        self.0.read().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn read<R>(&self, f: impl FnOnce(&Settings) -> R) -> R {
        match self.0.read() {
            Ok(guard) => f(&guard),
            Err(poisoned) => f(&poisoned.into_inner()),
        }
    }

    /// Applies at once without saving; the settings window saves after a short pause.
    pub fn set_live(&self, settings: Settings) {
        if let Ok(mut guard) = self.0.write() {
            *guard = settings;
        }
    }

    pub fn update(&self, f: impl FnOnce(&mut Settings)) {
        if let Ok(mut guard) = self.0.write() {
            f(&mut guard);
            guard.sanitize();
            if let Err(error) = save(&guard) {
                crate::vlog!("SETTINGS", "save failed: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_already_sane() {
        let mut settings = Settings::default();
        let before = settings.clone();
        settings.sanitize();
        assert_eq!(settings, before);
    }

    #[test]
    fn old_files_keep_their_values_and_gain_new_defaults() {
        let parsed = parse(r#"{"width": 300, "language": "en", "unknown_field": 1}"#).unwrap();
        assert_eq!(parsed.width, 300.0);
        assert_eq!(parsed.language, "en");
        assert!(parsed.enable_spotlight);
    }

    #[test]
    fn values_out_of_range_are_clamped() {
        let mut settings = Settings { width: 99999.0, opacity: f32::NAN, language: "xx".into(), ..Default::default() };
        settings.sanitize();
        assert_eq!(settings.width, 460.0);
        assert_eq!(settings.opacity, 1.0);
        assert_eq!(settings.language, "de");
    }

    #[test]
    fn unknown_tabs_are_dropped_and_missing_tabs_return() {
        let mut settings = Settings { nav_tab_order: "Spotify,Media".into(), visible_nav_tabs: "Spotify,Media".into(), ..Default::default() };
        settings.sanitize();
        assert!(settings.nav_tab_order.starts_with("Media,"));
        assert!(!settings.nav_tab_order.contains("Spotify"));
        assert!(settings.nav_tab_order.contains("Pi,School"));
        assert!(settings.nav_tab_order.contains("Weather"));
        assert_eq!(settings.visible_nav_tabs, "Media");
    }

    #[test]
    fn round_trip_keeps_everything() {
        let settings = Settings { width: 321.0, manual_city: "Traun".into(), ..Default::default() };
        let text = serde_json::to_string(&settings).unwrap();
        assert_eq!(parse(&text).unwrap(), settings);
    }
}
