//! Ties the windows, the tray and the shortcuts together.

use crate::i18n;
use crate::notch::{Notch, View};
use crate::services::hotkeys::{HotkeyAction, Hotkeys};
use crate::services::tray::{Tray, TrayAction};
use crate::services::{Hub, Toast};
use crate::settings::SharedSettings;
use crate::settings_ui::{Section, SettingsWindow};
use crate::spotlight::Spotlight;
use crate::translate::Translator;
use crate::theme;
use eframe::egui::{self, ViewportCommand, WindowLevel};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What a test run should do; all empty for a normal start.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub demo: bool,
    pub view: Option<String>,
    pub spotlight: bool,
    pub spotlight_ai: bool,
    pub settings_section: Option<String>,
    pub island: bool,
    pub toast: Option<String>,
    pub collapsed_media: bool,
    pub screenshot: Option<PathBuf>,
    pub exit_after: Option<f32>,
    pub language: Option<String>,
}

pub struct VNotchApp {
    hub: Arc<Hub>,
    notch: Notch,
    spotlight: Spotlight,
    settings_window: SettingsWindow,
    translator: Translator,
    hotkeys: Option<Hotkeys>,
    tray: Option<Tray>,
    options: Options,
    started: Instant,
    level_on_bottom: Option<bool>,
    update_checked: bool,
}

impl VNotchApp {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: SharedSettings, options: Options) -> Self {
        let ctx = cc.egui_ctx.clone();
        theme::install_fonts(&ctx);
        theme::apply_style(&ctx);
        let hub = Hub::start(ctx.clone(), settings.clone(), options.demo);
        let mut notch = Notch::new(hub.clone(), &ctx);
        let mut spotlight = Spotlight::new(hub.clone());
        let mut settings_window = SettingsWindow::new(hub.clone());
        let translator = Translator::new(hub.clone());

        let (hotkeys, tray) = if options.demo { (None, None) } else { (Some(Hotkeys::new(&ctx)), Tray::start(&ctx)) };

        if settings.read(|s| s.enable_hello_greeting) && !options.demo {
            hub.toast(Toast::Hello);
        }

        // A test run opens exactly what it wants to look at.
        if let Some(view) = options.view.as_deref().and_then(View::from_token) {
            notch.open(view);
        }
        if options.spotlight || options.spotlight_ai {
            spotlight.keep_open = true;
            if options.spotlight_ai {
                spotlight.show_ai_demo();
            } else {
                spotlight.show();
            }
        }
        if let Some(section) = options.settings_section.as_deref() {
            settings_window.show();
            settings_window.section = section_from_name(section);
        }
        if let Some(toast) = options.toast.as_deref() {
            hub.toast(demo_toast(toast));
        }
        Self {
            hub,
            notch,
            spotlight,
            settings_window,
            translator,
            hotkeys,
            tray,
            options,
            started: Instant::now(),
            level_on_bottom: None,
            update_checked: false,
        }
    }

    fn handle_shortcuts(&mut self) {
        let Some(hotkeys) = self.hotkeys.as_mut() else { return };
        let settings = self.hub.settings.get();
        hotkeys.apply(&settings);
        for action in hotkeys.poll() {
            match action {
                HotkeyAction::Spotlight => self.spotlight.toggle(),
                HotkeyAction::Clipboard => self.notch.open(View::Shelf),
                HotkeyAction::Translate => self.translator.trigger(&settings),
                HotkeyAction::ToggleInvisible => {
                    self.hub.settings.update(|s| s.invisible_mode = !s.invisible_mode);
                }
            }
        }
    }

    fn handle_tray(&mut self, ctx: &egui::Context) {
        let Some(tray) = self.tray.as_ref() else { return };
        let actions: Vec<TrayAction> = tray.actions.try_iter().collect();
        for action in actions {
            match action {
                TrayAction::ToggleVisible => self.notch.visible = !self.notch.visible,
                TrayAction::Settings => self.settings_window.show(),
                TrayAction::Spotlight => self.spotlight.show(),
                TrayAction::ResetPosition => self.notch.reset_position(),
                TrayAction::Restart => {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).spawn();
                    }
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                TrayAction::Exit => ctx.send_viewport_cmd(ViewportCommand::Close),
            }
        }
    }

    /// Ends a test run after the requested time. TODO: rebuild `--screenshot` on egui 0.37's `Context::request_screenshot`.
    fn drive_test_run(&mut self) {
        if self.options.exit_after.is_some_and(|limit| self.started.elapsed().as_secs_f32() > limit) {
            std::process::exit(0);
        }
    }
}

impl eframe::App for VNotchApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_shortcuts();
        self.handle_tray(&ctx);

        let settings = self.hub.settings.get();
        if self.level_on_bottom != Some(settings.stay_behind_windows) {
            self.level_on_bottom = Some(settings.stay_behind_windows);
            let level = if settings.stay_behind_windows { WindowLevel::AlwaysOnBottom } else { WindowLevel::AlwaysOnTop };
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(level));
        }
        if !self.update_checked && !self.options.demo && self.started.elapsed() > Duration::from_secs(20) {
            self.update_checked = true;
            if settings.auto_check_updates && settings.is_online_allowed() {
                self.hub.updates.check(settings.include_prerelease_updates);
            }
        }

        self.notch.ui(ui, frame);

        if std::mem::take(&mut self.notch.request_settings) {
            self.settings_window.show();
        }
        if std::mem::take(&mut self.notch.request_spotlight) {
            self.spotlight.show();
        }
        if std::mem::take(&mut self.settings_window.clear_history_requested) {
            self.spotlight.clear_history();
        }

        let (center_x, top) = ctx.input(|i| {
            let outer = i.viewport().outer_rect.unwrap_or(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0)));
            let height = i.viewport().monitor_size.map_or(800.0, |size| size.y);
            (outer.center().x, outer.top() + height * 0.16)
        });
        self.spotlight.show_viewport(&ctx, center_x, top);
        self.translator.show_viewport(&ctx, center_x, top);
        self.settings_window.show_viewport(&ctx);

        self.drive_test_run();
    }
}

fn section_from_name(name: &str) -> Section {
    match name {
        "behavior" => Section::Behavior,
        "tabs" => Section::Tabs,
        "devices" => Section::Devices,
        "spotlight" => Section::Spotlight,
        "translation" => Section::Translation,
        "privacy" => Section::Privacy,
        "updates" => Section::Updates,
        "about" => Section::About,
        _ => Section::Appearance,
    }
}

fn demo_toast(name: &str) -> Toast {
    use crate::services::clipboard::ClipKind;
    match name {
        "hello" => Toast::Hello,
        "volume" => Toast::Volume { level: 0.62, muted: false },
        "clipboard" => Toast::Clipboard { preview: "https://github.com/pichlervalentin4-lab/V-Notch".into(), kind: ClipKind::Link },
        "battery" => Toast::Battery { percent: 76.0, charging: true },
        "bluetooth" => Toast::Bluetooth { name: "Kopfhörer".into(), connected: true },
        "timer" => Toast::TimerDone,
        "claude" => Toast::ClaudeWaiting { session: "v-notch".into() },
        _ => Toast::Music { title: "Neon Horizon".into(), artist: "Demo Artist".into() },
    }
}

/// Starts the app: applies the language and builds the main window.
pub fn run(settings: SharedSettings, options: Options) -> eframe::Result {
    let (language, logging) = settings.read(|s| (s.language.clone(), s.enable_diagnostic_logging));
    i18n::set_language(options.language.as_deref().unwrap_or(&language));
    crate::vlog::set_enabled(logging);

    let icon = image::load_from_memory(include_bytes!("../assets/logo.png")).ok().map(|image| {
        let small = image.resize_exact(64, 64, image::imageops::FilterType::Triangle).to_rgba8();
        let (width, height) = small.dimensions();
        egui::IconData { rgba: small.into_raw(), width, height }
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("V-Notch")
        .with_app_id("v-notch")
        .with_inner_size([crate::notch::WINDOW_W, crate::notch::WINDOW_H])
        .with_position([200.0, 0.0])
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false)
        .with_active(false)
        .with_resizable(false)
        .with_has_shadow(false)
        .with_drag_and_drop(true);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let native = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native("V-Notch", native, Box::new(move |cc| Ok(Box::new(VNotchApp::new(cc, settings, options)))))
}

