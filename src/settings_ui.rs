//! The settings window. Changes apply at once and are saved a moment later.

use crate::i18n::{self, t, tf};
use crate::icons;
use crate::notch::kit::{rounded, text_clipped};
use crate::notch::View;
use crate::platform;
use crate::services::Hub;
use crate::services::updates::{RELEASES_PAGE, UpdateStatus};
use crate::settings::{self, DEFAULT_TABS, Settings};
use crate::spotlight::ai;
use crate::theme;
use eframe::egui::{self, Align2, Color32, RichText, ScrollArea, Sense, Ui, ViewportBuilder, ViewportId, vec2};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Appearance,
    Behavior,
    Tabs,
    Devices,
    Spotlight,
    Translation,
    Privacy,
    Updates,
    About,
}

const SECTIONS: [(Section, &str, &str); 9] = [
    (Section::Appearance, "settings.nav.appearance", icons::PAINT_BRUSH),
    (Section::Behavior, "settings.nav.behavior", icons::CURSOR_CLICK),
    (Section::Tabs, "settings.navTabs", icons::LIST),
    (Section::Devices, "settings.nav.devices", icons::LAPTOP),
    (Section::Spotlight, "settings.nav.spotlight", icons::MAGNIFYING_GLASS),
    (Section::Translation, "translation.title", icons::TRANSLATE),
    (Section::Privacy, "settings.nav.privacy", icons::SHIELD_CHECK),
    (Section::Updates, "settings.nav.updates", icons::ARROWS_CLOCKWISE),
    (Section::About, "port.settings.about", icons::INFO),
];

const SAVE_DELAY: Duration = Duration::from_millis(500);

pub struct SettingsWindow {
    hub: Arc<Hub>,
    pub open: bool,
    pub section: Section,
    draft: Settings,
    dirty_since: Option<Instant>,
    keys: HashMap<String, String>,
    notice: Option<String>,
    confirm_reset: bool,
    viewport_id: ViewportId,
    pub clear_history_requested: bool,
    school: SchoolForm,
}

/// The Moodle fields of the settings. The token and the password are never part of the settings
/// file: the token goes to the secret store, the password only into one login request.
#[derive(Default)]
struct SchoolForm {
    token: String,
    username: String,
    password: String,
    /// Whether the secret store holds a token; read once, not every frame.
    has_token: Option<bool>,
    message: Option<String>,
    login: Option<Arc<std::sync::Mutex<Option<Result<String, String>>>>>,
}

impl SettingsWindow {
    pub fn new(hub: Arc<Hub>) -> Self {
        let draft = hub.settings.get();
        Self {
            hub,
            open: false,
            section: Section::Appearance,
            draft,
            dirty_since: None,
            keys: HashMap::new(),
            notice: None,
            confirm_reset: false,
            viewport_id: ViewportId::from_hash_of("v-notch-settings"),
            clear_history_requested: false,
            school: SchoolForm::default(),
        }
    }

    pub fn show(&mut self) {
        if !self.open {
            self.draft = self.hub.settings.get();
            self.keys = ai::PROVIDERS
                .iter()
                .filter(|provider| ai::needs_key(provider))
                .map(|provider| ((*provider).to_string(), platform::secret_get(&ai::secret_name(provider)).unwrap_or_default()))
                .collect();
        }
        self.open = true;
    }

    /// Draws the window into a ui of the caller. Picture runs use this, because egui
    /// cannot capture an immediate viewport.
    pub fn draw_embedded(&mut self, ui: &mut Ui) {
        self.window(ui);
    }

    pub fn show_viewport(&mut self, ctx: &egui::Context) {
        if !self.open {
            self.flush();
            return;
        }
        let builder = ViewportBuilder::default()
            .with_title(t("settings.windowTitle"))
            .with_inner_size([820.0, 600.0])
            .with_min_inner_size([680.0, 460.0])
            .with_resizable(true);
        let id = self.viewport_id;
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                self.open = false;
            }
            self.window(ui);
        });
        if self.dirty_since.is_some_and(|since| since.elapsed() >= SAVE_DELAY) {
            self.flush();
        } else if self.dirty_since.is_some() {
            ctx.request_repaint_after(SAVE_DELAY);
        }
    }

    fn flush(&mut self) {
        if self.dirty_since.take().is_some() {
            let draft = self.draft.clone();
            self.hub.settings.update(|settings| *settings = draft);
        }
    }

    fn window(&mut self, ui: &mut Ui) {
        let before = self.draft.clone();
        egui::Panel::left("settings-nav").exact_size(186.0).frame(egui::Frame::new().fill(Color32::from_rgb(20, 20, 23)).inner_margin(10)).show_inside(ui, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new("V-Notch").font(theme::bold(16.0)));
            ui.add_space(10.0);
            for (section, key, icon) in SECTIONS {
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                let selected = self.section == section;
                let fill = if selected {
                    theme::CARD_HOVER
                } else if response.hovered() {
                    theme::CARD
                } else {
                    Color32::TRANSPARENT
                };
                ui.painter().rect_filled(rect, rounded(8.0), fill);
                ui.painter().text(rect.left_center() + vec2(16.0, 0.0), Align2::CENTER_CENTER, icon, theme::icon(15.0), if selected { theme::WHITE } else { theme::DIM });
                text_clipped(ui, rect.left_center() + vec2(32.0, 0.0), Align2::LEFT_CENTER, &t(key), theme::medium(12.5), theme::WHITE, rect.width() - 36.0);
                if response.clicked() {
                    self.section = section;
                }
            }
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::from_rgb(14, 14, 16)).inner_margin(18)).show_inside(ui, |ui| {
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(ui.available_width() - 12.0);
                if let Some(notice) = &self.notice {
                    ui.label(RichText::new(notice).font(theme::regular(12.0)).color(theme::GREEN));
                    ui.add_space(6.0);
                }
                match self.section {
                    Section::Appearance => self.appearance(ui),
                    Section::Behavior => self.behavior(ui),
                    Section::Tabs => self.tabs(ui),
                    Section::Devices => self.devices(ui),
                    Section::Spotlight => self.spotlight(ui),
                    Section::Translation => self.translation(ui),
                    Section::Privacy => self.privacy(ui),
                    Section::Updates => self.updates(ui),
                    Section::About => self.about(ui),
                }
            });
        });

        if self.draft != before {
            self.draft.sanitize();
            if self.draft.language != before.language {
                i18n::set_language(&self.draft.language);
            }
            if self.draft.auto_start != before.auto_start {
                platform::set_autostart(self.draft.auto_start);
            }
            if self.draft.enable_diagnostic_logging != before.enable_diagnostic_logging {
                crate::vlog::set_enabled(self.draft.enable_diagnostic_logging);
            }
            if self.draft.manual_city != before.manual_city || self.draft.enable_weather != before.enable_weather {
                self.hub.weather.refresh_now();
            }
            let draft = self.draft.clone();
            self.hub.settings.set_live(draft);
            self.dirty_since = Some(Instant::now());
        }
    }

    fn appearance(&mut self, ui: &mut Ui) {
        heading(ui, &t("settings.appearance"));
        let s = &mut self.draft;
        language_row(ui, s);
        toggle_row(ui, &t("settings.dynamicIslandMode"), &mut s.enable_dynamic_island_mode);
        if s.enable_dynamic_island_mode {
            slider_row(ui, &t("settings.dynamicIslandWidth"), &mut s.dynamic_island_width, 120.0..=460.0, " px");
            slider_row(ui, &t("settings.dynamicIslandHeight"), &mut s.dynamic_island_height, 24.0..=60.0, " px");
        } else {
            slider_row(ui, &t("settings.width"), &mut s.width, 120.0..=460.0, " px");
            slider_row(ui, &t("settings.height"), &mut s.height, 24.0..=60.0, " px");
            slider_row(ui, &t("settings.cornerRadius"), &mut s.corner_radius, 0.0..=30.0, " px");
        }
        slider_row(ui, &t("settings.opacity"), &mut s.opacity, 0.2..=1.0, "");
        toggle_row(ui, &t("port.settings.slimIdle"), &mut s.slim_idle_notch);
        toggle_row(ui, &t("settings.liquidGlass"), &mut s.enable_glass_style);
        if s.enable_glass_style {
            slider_row(ui, &t("settings.glass.opacity"), &mut s.glass_tint, 0.0..=1.0, "");
        }
        toggle_row(ui, &t("settings.mediaArtBackground"), &mut s.show_media_art_background);
        toggle_row(ui, &t("settings.enableBlurEffects"), &mut s.enable_blur_effects);
        slider_row(ui, &t("settings.lyricsDarkOverlay"), &mut s.media_blur_dark_overlay, 0.0..=1.0, "");
        toggle_row(ui, &t("port.settings.shadow"), &mut s.enable_shadow);
        toggle_row(ui, &t("port.settings.glow"), &mut s.enable_glow_on_hover);
        row(ui, &t("settings.clockPageStyle"), |ui| {
            egui::ComboBox::from_id_salt("clock-style")
                .selected_text(match s.clock_page_style.as_str() {
                    "digital" => t("settings.clockPageStyle.digital"),
                    "word" => t("settings.clockPageStyle.wordclock"),
                    _ => t("settings.clockPageStyle.analog"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.clock_page_style, "analog".to_string(), t("settings.clockPageStyle.analog"));
                    ui.selectable_value(&mut s.clock_page_style, "digital".to_string(), t("settings.clockPageStyle.digital"));
                    ui.selectable_value(&mut s.clock_page_style, "word".to_string(), t("settings.clockPageStyle.wordclock"));
                });
        });

        section_gap(ui);
        heading(ui, &t("port.settings.motion"));
        toggle_row(ui, &t("port.settings.animations"), &mut s.enable_animations);
        toggle_row(ui, &t("port.settings.bounce"), &mut s.enable_bounce_effect);
        slider_row(ui, &t("port.settings.speed"), &mut s.animation_speed, 0.25..=4.0, "×");
    }

    fn behavior(&mut self, ui: &mut Ui) {
        heading(ui, &t("settings.behavior"));
        let s = &mut self.draft;
        toggle_row(ui, &t("settings.autoStart"), &mut s.auto_start);
        toggle_row(ui, &t("settings.stayBehindWindows"), &mut s.stay_behind_windows);
        toggle_row(ui, &t("settings.helloGreeting"), &mut s.enable_hello_greeting);
        toggle_row(ui, &t("settings.hideExclusiveFs"), &mut s.hide_on_exclusive_fullscreen);
        toggle_row(ui, &t("settings.hideWindowedFs"), &mut s.hide_on_windowed_fullscreen);
        toggle_row(ui, &t("settings.hoverExpand"), &mut s.enable_hover_expand);
        if s.enable_hover_expand {
            let mut delay = s.hover_expand_delay as f32;
            if slider_row(ui, &t("settings.expandDelay"), &mut delay, 0.0..=2000.0, " ms") {
                s.hover_expand_delay = delay as u32;
            }
        }
        toggle_row(ui, &t("settings.disableAutoClose"), &mut s.disable_mouse_leave_auto_close);
        toggle_row(ui, &t("settings.reopenLastView"), &mut s.reopen_last_view_on_expand);
        toggle_row(ui, &t("settings.keepMediaPinned"), &mut s.keep_media_pinned_on_track_change);
        toggle_row(ui, &t("settings.idleAutoHide"), &mut s.enable_idle_auto_hide);
        if s.enable_idle_auto_hide {
            let mut delay = s.idle_auto_hide_delay as f32 / 1000.0;
            if slider_row(ui, &t("settings.idleAutoHideDelay"), &mut delay, 1.0..=120.0, " s") {
                s.idle_auto_hide_delay = (delay * 1000.0) as u32;
            }
        }
        toggle_row(ui, &t("port.settings.gestures"), &mut s.enable_gesture_controls);
        text_row(ui, &t("settings.privacy.invisible.shortcut"), &mut s.invisible_hotkey, "Ctrl+Alt+H");

        section_gap(ui);
        heading(ui, &t("port.settings.notifications"));
        toggle_row(ui, &t("settings.systemNotify"), &mut s.show_system_notifications);
        toggle_row(ui, &t("settings.showBattery"), &mut s.show_battery_indicator);
        toggle_row(ui, &t("settings.interactionSounds"), &mut s.enable_interaction_sounds);
        slider_row(ui, &t("settings.interactionSoundVolume"), &mut s.interaction_sound_volume, 0.0..=1.0, "");
        toggle_row(ui, &t("port.settings.claudeSounds"), &mut s.play_claude_sounds);

        section_gap(ui);
        heading(ui, &t("settings.trayAdvanced"));
        toggle_row(ui, &t("settings.trayCaptureText"), &mut s.clipboard_capture_text);
        toggle_row(ui, &t("settings.trayCaptureImages"), &mut s.clipboard_capture_images);
        toggle_row(ui, &t("settings.trayCaptureFiles"), &mut s.clipboard_capture_files);
        toggle_row(ui, &t("settings.trayDeleteSensitive"), &mut s.clipboard_auto_delete_sensitive);
        text_row(ui, &t("settings.clipboardHotkey"), &mut s.clipboard_hotkey, "Ctrl+Shift+V");
    }

    fn tabs(&mut self, ui: &mut Ui) {
        heading(ui, &t("settings.navTabs"));
        let s = &mut self.draft;
        let order: Vec<String> = s.nav_tab_order.split(',').map(str::to_string).collect();
        let mut new_order = order.clone();
        let mut new_visible: Vec<String> = s.visible_nav_tabs.split(',').map(str::to_string).collect();
        for (index, token) in order.iter().enumerate() {
            let Some(view) = View::from_token(token) else { continue };
            ui.horizontal(|ui| {
                ui.label(RichText::new(view.icon()).font(theme::icon(15.0)));
                let mut shown = new_visible.contains(token) || view == View::Media;
                let enabled = view != View::Media;
                ui.add_enabled_ui(enabled, |ui| {
                    if ui.checkbox(&mut shown, RichText::new(t(view.label_key())).font(theme::regular(13.0))).changed() {
                        if shown {
                            new_visible.push(token.clone());
                        } else {
                            new_visible.retain(|t| t != token);
                        }
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(index + 1 < order.len(), egui::Button::new(RichText::new(icons::CARET_DOWN).font(theme::icon(13.0)))).on_hover_text(t("settings.tab.moveDown")).clicked() {
                        new_order.swap(index, index + 1);
                    }
                    if ui.add_enabled(index > 0, egui::Button::new(RichText::new(icons::CARET_UP).font(theme::icon(13.0)))).on_hover_text(t("settings.tab.moveUp")).clicked() {
                        new_order.swap(index, index - 1);
                    }
                });
            });
        }
        s.nav_tab_order = new_order.join(",");
        s.visible_nav_tabs = new_visible.join(",");
        ui.add_space(8.0);
        if ui.button(t("settings.tab.reset")).clicked() {
            s.nav_tab_order = DEFAULT_TABS.into();
            s.visible_nav_tabs = DEFAULT_TABS.into();
        }
        ui.add_space(12.0);
        text_row(ui, &t("port.settings.piUrl"), &mut s.pi_dashboard_url, "http://100.x.y.z:4000");
        ui.label(RichText::new(t("port.settings.piUrlHint")).font(theme::regular(11.5)).color(theme::DIM));
        self.school_settings(ui);
    }

    /// Moodle for the school tab: the address, and a token pasted or fetched with name and password.
    fn school_settings(&mut self, ui: &mut Ui) {
        use crate::services::moodle;
        section_gap(ui);
        heading(ui, &t("port.settings.moodle"));
        text_row(ui, &t("port.settings.moodleUrl"), &mut self.draft.school_moodle_url, "https://moodle.example.at");
        let hub = self.hub.clone();
        let base = moodle::normalize_base(&self.draft.school_moodle_url);
        let form = &mut self.school;
        let has_token = *form.has_token.get_or_insert_with(|| platform::secret_get(moodle::TOKEN_SECRET).is_some_and(|t| !t.is_empty()));

        // A login that finished: keep the token, forget the password.
        if let Some(pending) = form.login.clone()
            && let Some(result) = pending.lock().ok().and_then(|mut r| r.take())
        {
            form.login = None;
            form.password.clear();
            form.message = Some(match result {
                Ok(token) if platform::secret_set(moodle::TOKEN_SECRET, &token) => {
                    form.has_token = Some(true);
                    hub.moodle.refresh();
                    t("port.settings.moodleSaved")
                }
                Ok(_) => t("error.apiKeyEncrypt"),
                Err(error) => tf("port.school.failed", &[&error]),
            });
        }

        let status = if has_token { t("port.settings.moodleHasToken") } else { t("port.settings.moodleNoToken") };
        row(ui, &t("port.settings.moodleToken"), |ui| {
            if has_token && ui.button(t("port.settings.moodleForget")).clicked() {
                platform::secret_set(moodle::TOKEN_SECRET, "");
                form.has_token = Some(false);
                form.message = None;
                hub.moodle.refresh();
            }
            ui.label(RichText::new(status).font(theme::regular(12.0)).color(if has_token { theme::GREEN } else { theme::DIM }));
        });
        row(ui, &t("port.settings.moodlePaste"), |ui| {
            let save = ui.add_enabled(!form.token.trim().is_empty(), egui::Button::new(t("port.settings.moodleSave"))).clicked();
            ui.add(egui::TextEdit::singleline(&mut form.token).password(true).desired_width(200.0));
            if save {
                if platform::secret_set(moodle::TOKEN_SECRET, form.token.trim()) {
                    form.has_token = Some(true);
                    form.message = Some(t("port.settings.moodleSaved"));
                    hub.moodle.refresh();
                } else {
                    form.message = Some(t("error.apiKeyEncrypt"));
                }
                form.token.clear();
            }
        });
        hint(ui, &t("port.settings.moodlePasteHint"));
        ui.add_space(6.0);
        row(ui, &t("port.settings.moodleUser"), |ui| {
            ui.add(egui::TextEdit::singleline(&mut form.username).desired_width(200.0));
        });
        let busy = form.login.is_some();
        row(ui, &t("port.settings.moodlePassword"), |ui| {
            let ready = !busy && !base.is_empty() && !form.username.trim().is_empty() && !form.password.is_empty();
            let label = if busy { t("pi.loading") } else { t("port.settings.moodleSignIn") };
            if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                let result = Arc::new(std::sync::Mutex::new(None));
                form.login = Some(result.clone());
                form.message = None;
                let user = form.username.trim().to_string();
                let password = std::mem::take(&mut form.password);
                let ctx = ui.ctx().clone();
                let base = base.clone();
                std::thread::spawn(move || {
                    let token = moodle::fetch_token(&base, &user, &password);
                    if let Ok(mut slot) = result.lock() {
                        *slot = Some(token);
                    }
                    ctx.request_repaint();
                });
            }
            ui.add(egui::TextEdit::singleline(&mut form.password).password(true).desired_width(200.0));
        });
        hint(ui, &t("port.settings.moodleSignInHint"));
        if let Some(message) = &form.message {
            ui.label(RichText::new(message).font(theme::regular(12.0)));
        }
    }

    fn devices(&mut self, ui: &mut Ui) {
        heading(ui, &t("settings.nav.devices"));
        let s = &mut self.draft;
        let monitors = ui.ctx().input(|_| ());
        let _ = monitors;
        let mut monitor = s.monitor_index as f32;
        if slider_row(ui, &t("settings.activeMonitor"), &mut monitor, 0.0..=7.0, "") {
            s.monitor_index = monitor as usize;
        }
        toggle_row(ui, &t("settings.hideCamera"), &mut s.hide_camera);
        section_gap(ui);
        heading(ui, &t("settings.nav.system"));
        text_row(ui, &t("settings.manualCity"), &mut s.manual_city, "Traun");
        toggle_row(ui, &t("settings.enableWeather"), &mut s.enable_weather);
        toggle_row(ui, &t("port.settings.fahrenheit"), &mut s.use_fahrenheit);
        hint(ui, &t("port.settings.cityHint"));
    }

    fn spotlight(&mut self, ui: &mut Ui) {
        heading(ui, &t("settings.spotlight"));
        let s = &mut self.draft;
        toggle_row(ui, &t("settings.enableSpotlight"), &mut s.enable_spotlight);
        text_row(ui, &t("port.settings.spotlightHotkey"), &mut s.spotlight_hotkey, "Alt+Space");
        toggle_row(ui, &t("settings.spotlight.defaultMode"), &mut s.spotlight_default_ai);
        toggle_row(ui, &t("settings.privacy.spotlightHistory"), &mut s.enable_spotlight_history);
        section_gap(ui);
        heading(ui, "AI");
        row(ui, &t("settings.spotlightAi.provider"), |ui| {
            egui::ComboBox::from_id_salt("ai-provider").selected_text(s.spotlight_ai_provider.clone()).show_ui(ui, |ui| {
                for provider in ai::PROVIDERS {
                    ui.selectable_value(&mut s.spotlight_ai_provider, provider.to_string(), provider);
                }
            });
        });
        let provider = s.spotlight_ai_provider.clone();
        let hint_model = ai::default_model(&provider);
        let model = match provider.as_str() {
            "Claude" => &mut s.spotlight_claude_model,
            "OpenAI" => &mut s.spotlight_openai_model,
            "Gemini" => &mut s.spotlight_gemini_model,
            "DeepSeek" => &mut s.spotlight_deepseek_model,
            _ => &mut s.spotlight_ollama_model,
        };
        text_row(ui, &t("settings.spotlightAi.model"), model, hint_model);
        if ai::needs_key(&provider) {
            let key = self.keys.entry(provider.clone()).or_default();
            let mut changed = false;
            row(ui, &t("settings.spotlightAi.key"), |ui| {
                let response = ui.add(egui::TextEdit::singleline(key).password(true).desired_width(280.0));
                changed = response.lost_focus();
            });
            if changed {
                if !platform::secret_set(&ai::secret_name(&provider), key.trim()) {
                    self.notice = Some(t("error.apiKeyEncrypt"));
                }
            }
        } else {
            text_row(ui, &t("port.settings.ollamaUrl"), &mut s.spotlight_ollama_url, "http://127.0.0.1:11434");
        }
        hint(ui, &t("settings.spotlightAi.disclaimer"));
        toggle_row(ui, &t("settings.privacy.chatHistory"), &mut s.save_ai_chat_history);
    }

    fn translation(&mut self, ui: &mut Ui) {
        heading(ui, &t("translation.title"));
        let s = &mut self.draft;
        toggle_row(ui, &t("port.settings.translationOn"), &mut s.enable_live_translation);
        text_row(ui, &t("port.settings.translationHotkey"), &mut s.translation_hotkey, "Ctrl+Shift+T");
        row(ui, &t("translation.target"), |ui| {
            egui::ComboBox::from_id_salt("translate-target")
                .selected_text(i18n::LANGUAGES.iter().find(|(id, _)| *id == s.translation_target_language).map_or("?", |(_, name)| name))
                .show_ui(ui, |ui| {
                    for (id, name) in i18n::LANGUAGES {
                        ui.selectable_value(&mut s.translation_target_language, (*id).to_string(), *name);
                    }
                });
        });
        row(ui, &t("port.settings.translationEngine"), |ui| {
            ui.selectable_value(&mut s.translation_engine, "ollama".to_string(), "Ollama");
            ui.selectable_value(&mut s.translation_engine, "ai".to_string(), "AI");
        });
        if s.translation_engine == "ollama" {
            text_row(ui, &t("settings.spotlightAi.model"), &mut s.translation_ollama_model, "qwen3:4b");
            hint(ui, &t("port.settings.ollamaHint"));
        }
        toggle_row(ui, &t("port.settings.currency"), &mut s.enable_currency_conversion);
        text_row(ui, &t("port.settings.targetCurrency"), &mut s.translation_target_currency, "EUR");
        toggle_row(ui, &t("port.settings.ignoreFullscreen"), &mut s.translation_ignore_fullscreen);
        text_row(ui, &t("port.settings.excludedApps"), &mut s.translation_excluded_apps, "");
    }

    fn privacy(&mut self, ui: &mut Ui) {
        heading(ui, &t("settings.header.privacy"));
        let s = &mut self.draft;
        toggle_row(ui, &t("settings.privacy.localOnly"), &mut s.enable_local_only_mode);
        ui.add_enabled_ui(!s.enable_local_only_mode, |ui| {
            toggle_row(ui, &t("settings.privacy.ai"), &mut s.allow_online_ai);
            toggle_row(ui, &t("settings.privacy.weather"), &mut s.allow_online_weather);
            toggle_row(ui, &t("settings.privacy.onlineLyrics"), &mut s.enable_online_lyrics);
            toggle_row(ui, &t("settings.privacy.autoUpdates"), &mut s.auto_check_updates);
        });
        toggle_row(ui, &t("settings.privacy.indicators"), &mut s.enable_privacy_indicators);
        toggle_row(ui, &t("settings.privacy.invisible"), &mut s.invisible_mode);
        toggle_row(ui, &t("settings.privacy.logging"), &mut s.enable_diagnostic_logging);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(t("settings.privacy.clearLog")).clicked() {
                let _ = std::fs::remove_file(crate::vlog::path());
                self.notice = Some(t("settings.privacy.clearedLog"));
            }
            if ui.button(t("settings.privacy.clearSpotlight")).clicked() {
                self.clear_history_requested = true;
                self.notice = Some(t("settings.privacy.clearedSpotlight"));
            }
        });
    }

    fn updates(&mut self, ui: &mut Ui) {
        heading(ui, &t("settings.updates"));
        let s = &mut self.draft;
        row(ui, &tf("settings.currentVersion", &[env!("CARGO_PKG_VERSION")]), |_ui| {});
        toggle_row(ui, &t("settings.privacy.autoUpdates"), &mut s.auto_check_updates);
        toggle_row(ui, &t("settings.updates.prerelease"), &mut s.include_prerelease_updates);
        let include = s.include_prerelease_updates;
        ui.add_space(6.0);
        if ui.button(t("settings.checkUpdate")).clicked() {
            self.hub.updates.check(include);
        }
        let status = self.hub.updates.status.lock().map(|s| s.clone()).unwrap_or_default();
        match status {
            UpdateStatus::Unknown => {}
            UpdateStatus::Checking => {
                ui.label(t("settings.checkingUpdates"));
            }
            UpdateStatus::UpToDate => {
                ui.label(RichText::new(t("settings.upToDate")).color(theme::GREEN));
            }
            UpdateStatus::Available { version, url } => {
                ui.label(RichText::new(tf("settings.updateAvailable", &[&version.to_string()])).color(theme::ORANGE));
                if ui.button(t("settings.download")).clicked() {
                    let _ = open::that_detached(url);
                }
            }
            UpdateStatus::Failed => {
                ui.label(RichText::new(t("error.updateFailed")).color(theme::RED));
            }
        }
        ui.add_space(6.0);
        ui.hyperlink_to(t("settings.btn.changelog"), RELEASES_PAGE);

        section_gap(ui);
        heading(ui, &t("settings.section.backup"));
        ui.horizontal(|ui| {
            if ui.button(t("settings.exportSettings.btn")).clicked() {
                self.notice = Some(export_settings(&self.draft));
            }
            if ui.button(t("settings.importSettings.btn")).clicked() {
                match import_settings() {
                    Ok(imported) => {
                        self.draft = imported;
                        i18n::set_language(&self.draft.language);
                        self.notice = Some(t("settings.import.success"));
                    }
                    Err(error) => self.notice = Some(tf("settings.import.error", &[&error])),
                }
            }
        });
        hint(ui, &t("port.settings.backupHint"));

        section_gap(ui);
        if self.confirm_reset {
            ui.label(RichText::new(t("settings.reset.confirm")).color(theme::ORANGE));
            ui.horizontal(|ui| {
                if ui.button(t("dialog.confirm")).clicked() {
                    self.draft = Settings::default();
                    i18n::set_language(&self.draft.language);
                    self.confirm_reset = false;
                }
                if ui.button(t("dialog.cancel")).clicked() {
                    self.confirm_reset = false;
                }
            });
        } else if ui.button(t("settings.reset")).clicked() {
            self.confirm_reset = true;
        }
    }

    fn about(&mut self, ui: &mut Ui) {
        heading(ui, "V-Notch");
        ui.label(tf("settings.currentVersion", &[env!("CARGO_PKG_VERSION")]));
        ui.add_space(6.0);
        ui.label(t("port.settings.aboutText"));
        ui.add_space(6.0);
        ui.hyperlink("https://github.com/pichlervalentin4-lab/V-Notch");
        ui.hyperlink("https://github.com/DavidLeitnerHTL/V-Notch");
        ui.hyperlink("https://github.com/w0otdev/V-Notch");
        section_gap(ui);
        if ui.button(t("port.settings.openLog")).clicked() {
            let _ = open::that_detached(crate::vlog::path());
        }
        if ui.button(t("port.settings.openFolder")).clicked() {
            let _ = open::that_detached(crate::paths::config_dir());
        }
    }
}

fn export_path() -> std::path::PathBuf {
    directories::UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(std::path::Path::to_path_buf))
        .unwrap_or_else(crate::paths::home_dir)
        .join("v-notch-settings.json")
}

/// API keys are not part of the settings file, so they never end up in an export.
fn export_settings(current: &Settings) -> String {
    let path = export_path();
    let saved = serde_json::to_string_pretty(current).map_err(|error| error.to_string()).and_then(|text| std::fs::write(&path, text).map_err(|error| error.to_string()));
    match saved {
        Ok(()) => format!("{} {}", t("settings.export.success"), path.display()),
        Err(error) => tf("error.settingsSave", &[&error]),
    }
}

fn import_settings() -> Result<Settings, String> {
    let path = export_path();
    let text = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut imported = settings::parse(&text).ok_or_else(|| format!("{}: invalid JSON", path.display()))?;
    imported.sanitize();
    Ok(imported)
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::bold(17.0)));
    ui.add_space(8.0);
}

fn section_gap(ui: &mut Ui) {
    ui.add_space(14.0);
    ui.separator();
    ui.add_space(8.0);
}

fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::regular(11.5)).color(theme::DIM));
}

fn row(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        ui.label(RichText::new(label).font(theme::regular(13.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), content);
    });
}

fn toggle_row(ui: &mut Ui, label: &str, value: &mut bool) {
    row(ui, label, |ui| {
        ui.add(egui::Checkbox::without_text(value));
    });
}

fn slider_row(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str) -> bool {
    let mut changed = false;
    row(ui, label, |ui| {
        let slider = egui::Slider::new(value, range).suffix(suffix).max_decimals(2);
        changed = ui.add_sized([250.0, 20.0], slider).changed();
    });
    changed
}

fn text_row(ui: &mut Ui, label: &str, value: &mut String, placeholder: &str) {
    row(ui, label, |ui| {
        ui.add(egui::TextEdit::singleline(value).hint_text(placeholder).desired_width(250.0));
    });
}

fn language_row(ui: &mut Ui, s: &mut Settings) {
    row(ui, &t("settings.language"), |ui| {
        let current = i18n::LANGUAGES.iter().find(|(id, _)| *id == s.language).map_or("Deutsch", |(_, name)| name);
        egui::ComboBox::from_id_salt("language").selected_text(current).show_ui(ui, |ui| {
            for (id, name) in i18n::LANGUAGES {
                ui.selectable_value(&mut s.language, (*id).to_string(), *name);
            }
        });
    });
}
