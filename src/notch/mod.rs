//! The notch itself: shape, motion, hover and click logic, the closed pill and its notifications.

mod camera;
mod claude;
mod controls;
pub mod kit;
mod legion;
mod media;
mod mixer;
mod shelf;
mod weather;

use crate::anim::{Spring, SpringConfig, approach, lerp, smoothstep};
use crate::i18n::{t, tf};
use crate::icons;
use crate::platform::{self, MediaCommand};
use crate::services::claude::Status as ClaudeStatus;
use crate::services::sounds::Tone;
use crate::services::weather_service::WeatherStatus;
use crate::services::{Hub, Toast, timer};
use crate::settings::Settings;
use crate::theme;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, Pos2, Rect, Sense, Shadow, Shape, Stroke, TextureHandle, TextureOptions, Ui, UiBuilder, pos2, vec2,
};
use kit::{rounded, text_clipped};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// The window never changes size: it stays large and clear, and clicks pass through outside the notch.
pub const WINDOW_W: f32 = 820.0;
pub const WINDOW_H: f32 = 500.0;
const ISLAND_TOP: f32 = 8.0;
const NAV_H: f32 = 30.0;
const IDLE_SLIM_WIDTH: f32 = 150.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum View {
    Media,
    Shelf,
    Controls,
    Claude,
    Weather,
    Legion,
    Mixer,
    Camera,
}

impl View {
    pub fn token(self) -> &'static str {
        match self {
            View::Media => "Media",
            View::Shelf => "Secondary",
            View::Controls => "Timer",
            View::Claude => "Claude",
            View::Weather => "Weather",
            View::Legion => "Legion",
            View::Mixer => "AudioMixer",
            View::Camera => "Camera",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "Media" => View::Media,
            "Secondary" => View::Shelf,
            "Timer" => View::Controls,
            "Claude" => View::Claude,
            "Weather" => View::Weather,
            "Legion" => View::Legion,
            "AudioMixer" => View::Mixer,
            "Camera" => View::Camera,
            _ => return None,
        })
    }

    pub fn icon(self) -> &'static str {
        match self {
            View::Media => icons::HOUSE,
            View::Shelf => icons::TRAY,
            View::Controls => icons::SLIDERS_HORIZONTAL,
            View::Claude => icons::SPARKLE,
            View::Weather => icons::CLOUD_SUN,
            View::Legion => icons::GAUGE,
            View::Mixer => icons::SPEAKER_HIGH,
            View::Camera => icons::CAMERA,
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            View::Media => "nav.media",
            View::Shelf => "nav.clipboard",
            View::Controls => "nav.controls",
            View::Claude => "nav.claude",
            View::Weather => "port.nav.weather",
            View::Legion => "port.nav.legion",
            View::Mixer => "nav.audio",
            View::Camera => "nav.camera",
        }
    }

    /// Width and height of the opened notch for this view.
    fn size(self) -> (f32, f32) {
        match self {
            View::Media => (540.0, 196.0),
            View::Shelf => (720.0, 320.0),
            View::Controls => (720.0, 400.0),
            View::Claude => (720.0, 400.0),
            View::Weather => (720.0, 380.0),
            View::Legion => (560.0, 300.0),
            View::Mixer => (720.0, 378.0),
            View::Camera => (520.0, 328.0),
        }
    }
}

struct ActiveToast {
    toast: Toast,
    until: Instant,
}

/// What the cursor thread shares with the frame: the notch rectangle and whether the pointer is on it.
#[derive(Default)]
pub struct CursorShare {
    /// Rectangle in physical screen pixels, as fixed-point numbers to fit in atomics.
    left: AtomicU32,
    top: AtomicU32,
    right: AtomicU32,
    bottom: AtomicU32,
    pub inside: AtomicBool,
    pub clicked_outside: AtomicBool,
    pub running: AtomicBool,
    /// While open, every click outside counts, so the pointer is watched closely.
    pub expanded: AtomicBool,
}

const POSITION_OFFSET: f32 = 100_000.0;

fn encode(value: f32) -> u32 {
    (value + POSITION_OFFSET).max(0.0) as u32
}

impl CursorShare {
    fn set_rect(&self, rect: Rect) {
        self.left.store(encode(rect.left()), Ordering::Relaxed);
        self.top.store(encode(rect.top()), Ordering::Relaxed);
        self.right.store(encode(rect.right()), Ordering::Relaxed);
        self.bottom.store(encode(rect.bottom()), Ordering::Relaxed);
    }

    #[cfg(test)]
    fn contains(&self, x: f32, y: f32) -> bool {
        self.distance(x, y) == 0
    }

    /// Distance in pixels from the pointer to the notch, 0 when it is on it.
    fn distance(&self, x: f32, y: f32) -> u32 {
        let (x, y) = (encode(x), encode(y));
        let dx = self.left.load(Ordering::Relaxed).saturating_sub(x).max(x.saturating_sub(self.right.load(Ordering::Relaxed)));
        let dy = self.top.load(Ordering::Relaxed).saturating_sub(y).max(y.saturating_sub(self.bottom.load(Ordering::Relaxed)));
        dx.max(dy)
    }
}

/// How long the cursor thread sleeps: closely near the notch or while it is open, less often
/// when the pointer is far away and a few milliseconds of delay cannot be noticed.
fn cursor_poll_interval(distance: u32, expanded: bool) -> Duration {
    if expanded || distance < 200 { Duration::from_millis(16) } else { Duration::from_millis(60) }
}

/// Polls the global pointer. It keeps running while the window passes clicks through and gets no events.
fn start_cursor_thread(share: Arc<CursorShare>, ctx: egui::Context) {
    let _ = std::thread::Builder::new().name("cursor".into()).spawn(move || {
        let mut was_down = false;
        let mut interval = Duration::from_millis(16);
        loop {
            std::thread::sleep(interval);
            if !share.running.load(Ordering::Relaxed) {
                continue;
            }
            let Some((x, y)) = platform::cursor_position() else { continue };
            let distance = share.distance(x, y);
            interval = cursor_poll_interval(distance, share.expanded.load(Ordering::Relaxed));
            let inside = distance == 0;
            if share.inside.swap(inside, Ordering::Relaxed) != inside {
                ctx.request_repaint();
            }
            let down = platform::primary_button_down();
            if down && !was_down && !inside {
                share.clicked_outside.store(true, Ordering::Relaxed);
                ctx.request_repaint();
            }
            was_down = down;
        }
    });
}

#[derive(Default)]
pub struct Textures {
    covers: HashMap<u64, (TextureHandle, TextureHandle)>,
    files: HashMap<String, Option<TextureHandle>>,
}

impl Textures {
    pub fn image_file(&mut self, ctx: &egui::Context, path: &std::path::Path) -> Option<TextureHandle> {
        let key = path.to_string_lossy().into_owned();
        if let Some(found) = self.files.get(&key) {
            return found.clone();
        }
        let handle = image::open(path).ok().map(|image| {
            let small = image.thumbnail(160, 160).to_rgba8();
            let size = [small.width() as usize, small.height() as usize];
            let pixels = egui::ColorImage::from_rgba_unmultiplied(size, small.as_raw());
            ctx.load_texture(key.clone(), pixels, TextureOptions::LINEAR)
        });
        if self.files.len() > 256 {
            self.files.clear();
        }
        self.files.insert(key, handle.clone());
        handle
    }

    pub fn app_icon(&mut self, ctx: &egui::Context, path: &std::path::Path) -> Option<TextureHandle> {
        let key = format!("icon:{}", path.display());
        if let Some(found) = self.files.get(&key) {
            return found.clone();
        }
        let handle = platform::file_icon(path).map(|(pixels, side)| {
            let image = egui::ColorImage::from_rgba_unmultiplied([side as usize, side as usize], &pixels);
            ctx.load_texture(key.clone(), image, TextureOptions::LINEAR)
        });
        self.files.insert(key, handle.clone());
        handle
    }
}

pub struct Notch {
    pub hub: Arc<Hub>,
    pub view: View,
    pub expanded: bool,
    pub visible: bool,
    width: Spring,
    height: Spring,
    radius: Spring,
    top: Spring,
    content: f32,
    hover: f32,
    view_alpha: f32,
    toast: Option<ActiveToast>,
    cursor: Arc<CursorShare>,
    passthrough: Option<bool>,
    /// Why the last frame asked for the next one; counted by `--stats`.
    pub repaint_reason: &'static str,
    prepared: bool,
    placed_on: Option<usize>,
    window_origin: Pos2,
    hover_since: Option<Instant>,
    left_since: Option<Instant>,
    idle_hidden: bool,
    swipe: f32,
    swipe_cooldown: f32,
    last_rect: Rect,
    pub textures: Textures,
    pub lyrics_open: bool,
    pub shelf_ui: shelf::ShelfUi,
    pub claude_ui: claude::ClaudeUi,
    pub camera_ui: camera::CameraUi,
    pub controls_ui: controls::ControlsUi,
    pub request_settings: bool,
    pub request_spotlight: bool,
    limits: Option<crate::services::claude::Limits>,
    limits_checked: Option<Instant>,
    last_claude_waiting: usize,
    sweep_at: Instant,
    frames: u64,
}

impl Notch {
    pub fn new(hub: Arc<Hub>, ctx: &egui::Context) -> Self {
        let settings = hub.settings.get();
        let cursor = Arc::new(CursorShare::default());
        start_cursor_thread(cursor.clone(), ctx.clone());
        let (w, h) = Self::compact_base(&settings);
        Self {
            hub,
            view: View::Media,
            expanded: false,
            visible: true,
            width: Spring::new(w),
            height: Spring::new(h),
            radius: Spring::new(settings.corner_radius),
            top: Spring::new(if settings.enable_dynamic_island_mode { ISLAND_TOP } else { 0.0 }),
            content: 0.0,
            hover: 0.0,
            view_alpha: 1.0,
            toast: None,
            cursor,
            passthrough: None,
            repaint_reason: "start",
            prepared: false,
            placed_on: None,
            window_origin: Pos2::ZERO,
            hover_since: None,
            left_since: None,
            idle_hidden: false,
            swipe: 0.0,
            swipe_cooldown: 0.0,
            last_rect: Rect::NOTHING,
            textures: Textures::default(),
            lyrics_open: false,
            shelf_ui: shelf::ShelfUi::default(),
            claude_ui: claude::ClaudeUi::default(),
            camera_ui: camera::CameraUi::default(),
            controls_ui: controls::ControlsUi::default(),
            request_settings: false,
            request_spotlight: false,
            limits: None,
            limits_checked: None,
            last_claude_waiting: 0,
            sweep_at: Instant::now(),
            frames: 0,
        }
    }

    fn compact_base(settings: &Settings) -> (f32, f32) {
        if settings.enable_dynamic_island_mode {
            (settings.dynamic_island_width, settings.dynamic_island_height)
        } else {
            (settings.width, settings.height)
        }
    }

    pub fn visible_tabs(&self, settings: &Settings) -> Vec<View> {
        settings
            .nav_tab_order
            .split(',')
            .filter_map(View::from_token)
            .filter(|view| *view == View::Media || settings.tab_visible(view.token()))
            .filter(|view| *view != View::Camera || !settings.hide_camera)
            .filter(|view| *view != View::Legion || self.legion_available())
            .collect()
    }

    fn legion_available(&self) -> bool {
        use crate::services::legion::LegionStatus;
        self.hub.legion.status.lock().is_ok_and(|status| *status != LegionStatus::Unsupported)
    }

    pub fn open(&mut self, view: View) {
        if self.view != view {
            self.view_alpha = 0.0;
        }
        self.view = view;
        self.expanded = true;
        self.toast = None;
    }

    pub fn close(&mut self) {
        self.expanded = false;
        self.lyrics_open = false;
    }

    pub fn toggle(&mut self, settings: &Settings) {
        if self.expanded {
            self.close();
            self.hub.play(Tone::SoftClick);
        } else {
            let view = if settings.reopen_last_view_on_expand { self.view } else { View::Media };
            self.open(view);
            self.hub.play(Tone::Click);
        }
    }

    pub fn reset_position(&mut self) {
        self.placed_on = None;
    }

    fn target_size(&self, settings: &Settings, activity: &Activity) -> (f32, f32, f32) {
        let (base_w, base_h) = Self::compact_base(settings);
        if self.expanded {
            let (w, h) = open_size(self.view, self.claude_ui.wanted_height);
            let radius = if settings.enable_dynamic_island_mode { 32.0 } else { 24.0 };
            return (w, h, radius);
        }
        let collapsed_radius = if settings.enable_dynamic_island_mode { base_h / 2.0 } else { settings.corner_radius };
        if let Some(active) = &self.toast {
            let (w, h) = toast_size(&active.toast, base_w, base_h);
            return (w, h, (collapsed_radius + 8.0).min(h / 2.0));
        }
        let hover_grow = 1.0 + 0.04 * self.hover;
        let width = if activity.media_playing || activity.has_media {
            (base_w + 50.0).max(280.0) + activity.extra_width
        } else if activity.extra_width > 0.0 {
            base_w + activity.extra_width
        } else if settings.slim_idle_notch && !settings.enable_dynamic_island_mode {
            IDLE_SLIM_WIDTH.max(base_w * 0.65)
        } else {
            base_w
        };
        (width * hover_grow, base_h * (1.0 + 0.06 * self.hover), collapsed_radius)
    }

    fn pull_toast(&mut self, settings: &Settings) {
        let now = Instant::now();
        if self.toast.as_ref().is_some_and(|active| now >= active.until) {
            self.toast = None;
        }
        // A new volume level takes over the running volume pop-up right away.
        let replace_volume = self.toast.as_ref().is_some_and(|a| matches!(a.toast, Toast::Volume { .. }));
        if self.toast.is_some() && !replace_volume {
            return;
        }
        let Ok(mut queue) = self.hub.toasts.lock() else { return };
        let next = if replace_volume {
            let position = queue.iter().position(|t| matches!(t, Toast::Volume { .. }));
            position.and_then(|i| queue.remove(i))
        } else {
            queue.pop_front()
        };
        drop(queue);
        let Some(toast) = next else { return };
        if self.expanded && !matches!(toast, Toast::TimerDone) {
            return;
        }
        let duration = match toast {
            Toast::Volume { .. } => Duration::from_millis(1500),
            Toast::Clipboard { .. } => Duration::from_millis(2200),
            Toast::Hello => Duration::from_millis(2600),
            Toast::TimerDone => Duration::from_secs(8),
            _ => Duration::from_millis(u64::from(settings.notification_duration).min(6000)),
        };
        self.toast = Some(ActiveToast { toast, until: now + duration });
    }

    fn apply_window_state(&mut self, ctx: &egui::Context, frame: &eframe::Frame, settings: &Settings, full_hide: bool) {
        if !self.prepared {
            platform::prepare_notch_window(frame);
            self.prepared = true;
            self.cursor.running.store(true, Ordering::Relaxed);
        }
        // Centre the window on the chosen monitor. winit knows the monitors, in physical pixels.
        if self.placed_on != Some(settings.monitor_index)
            && let Some(window) = frame.winit_window()
        {
            let monitors: Vec<_> = window.available_monitors().collect();
            let monitor = monitors.get(settings.monitor_index).or_else(|| monitors.first()).cloned().or_else(|| window.primary_monitor());
            if let Some(monitor) = monitor {
                let scale = monitor.scale_factor() as f32;
                let (x, y) = (monitor.position().x as f32, monitor.position().y as f32);
                let width = monitor.size().width as f32;
                let left = x + (width - WINDOW_W * scale) / 2.0;
                let origin = pos2(left, y);
                self.window_origin = origin;
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos2(left / scale, y / scale)));
                self.placed_on = Some(settings.monitor_index);
            }
        }
        let scale = ctx.pixels_per_point();
        let visual = if full_hide { Rect::NOTHING } else { self.last_rect };
        let hit = visual.expand(if self.expanded { 2.0 } else { 6.0 });
        let origin = self.window_origin.to_vec2();
        let rect_px = Rect::from_min_max((hit.min.to_vec2() * platform_scale(scale)).to_pos2() + origin, (hit.max.to_vec2() * platform_scale(scale)).to_pos2() + origin);
        self.cursor.set_rect(if full_hide || !visual.is_positive() { Rect::from_min_size(pos2(-99_000.0, -99_000.0), vec2(1.0, 1.0)) } else { rect_px });
        self.cursor.expanded.store(self.expanded, Ordering::Relaxed);

        let inside = self.cursor.inside.load(Ordering::Relaxed);
        let dragging = ctx.input(|i| i.pointer.any_down() && i.pointer.is_decidedly_dragging());
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let wanted = !(inside || dragging || hovering_files) || settings.enable_cursor_bypass && !self.expanded;
        if self.passthrough != Some(wanted) {
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(wanted));
            self.passthrough = Some(wanted);
        }
    }

    /// One frame of the notch window.
    pub fn ui(&mut self, ui: &mut Ui, frame: &eframe::Frame) {
        let ctx = ui.ctx().clone();
        let settings = self.hub.settings.get();
        let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1);
        self.frames += 1;
        let now = Instant::now();

        self.background_chores(&ctx, &settings);

        let activity = self.collect_activity(&settings);
        let system = self.hub.system.state.lock().map(|s| (s.fullscreen, s.idle_seconds)).unwrap_or_default();
        let fullscreen_hide = match system.0 {
            platform::FullscreenKind::Exclusive => settings.hide_on_exclusive_fullscreen,
            platform::FullscreenKind::Windowed => settings.hide_on_windowed_fullscreen,
            platform::FullscreenKind::None => false,
        };
        let idle_hide = settings.enable_idle_auto_hide
            && !self.expanded
            && !activity.has_media
            && self.toast.is_none()
            && system.1 > settings.idle_auto_hide_delay as f32 / 1000.0;
        self.idle_hidden = idle_hide;
        let hidden = !self.visible || settings.invisible_mode || fullscreen_hide || idle_hide;

        self.apply_window_state(&ctx, frame, &settings, hidden);

        self.pull_toast(&settings);
        self.handle_timer(&settings, &ctx);

        let inside = self.cursor.inside.load(Ordering::Relaxed);
        self.handle_hover(inside, &settings, now);
        if self.cursor.clicked_outside.swap(false, Ordering::Relaxed) && self.expanded && settings.disable_mouse_leave_auto_close {
            self.close();
        }

        let (target_w, target_h, target_r) = self.target_size(&settings, &activity);
        let config = if settings.enable_animations {
            SpringConfig::new(settings.animation_speed, settings.enable_bounce_effect)
        } else {
            SpringConfig::INSTANT
        };
        self.width.set_target(target_w);
        self.height.set_target(target_h);
        self.radius.set_target(target_r);
        self.top.set_target(if settings.enable_dynamic_island_mode { ISLAND_TOP } else { 0.0 });
        self.width.step(dt, config);
        self.height.step(dt, config);
        self.radius.step(dt, config);
        self.top.step(dt, config);
        self.hover = approach(self.hover, if inside && !self.expanded { 1.0 } else { 0.0 }, 14.0, dt);
        self.view_alpha = approach(self.view_alpha, 1.0, 12.0, dt);

        let (compact_w, compact_h) = Self::compact_base(&settings);
        // Measured against the height this tab really opens to: the Claude tab's height follows its content.
        let open_progress = open_progress(self.height.value, compact_h, open_size(self.view, self.claude_ui.wanted_height).1);
        let content_target = if self.expanded { smoothstep(0.55, 0.95, open_progress) } else { 0.0 };
        self.content = approach(self.content, content_target, 18.0, dt);
        let _ = compact_w;

        let center_x = WINDOW_W / 2.0;
        let rect = Rect::from_min_size(
            pos2(center_x - self.width.value / 2.0, self.top.value),
            vec2(self.width.value.max(1.0), self.height.value.max(1.0)),
        );
        self.last_rect = if hidden { Rect::NOTHING } else { rect };

        if hidden {
            ctx.request_repaint_after(Duration::from_millis(500));
            return;
        }

        self.paint_shell(ui, rect, &settings);
        self.interact(ui, rect, &settings);

        let inner_alpha = 1.0 - smoothstep(0.0, 0.5, open_progress);
        if inner_alpha > 0.01 && (!self.expanded || self.content < 0.5) {
            let mut child = ui.new_child(UiBuilder::new().max_rect(rect).id_salt("compact"));
            child.set_clip_rect(rect);
            child.multiply_opacity(inner_alpha);
            self.paint_collapsed(&mut child, rect, &settings, &activity);
        }
        if self.expanded && self.content > 0.01 {
            let mut child = ui.new_child(UiBuilder::new().max_rect(rect).id_salt("expanded"));
            child.set_clip_rect(rect);
            child.multiply_opacity(self.content);
            self.paint_expanded(&mut child, rect, &settings, frame);
        }

        self.handle_file_hover(&ctx);

        let animating = !self.width.is_settled()
            || !self.height.is_settled()
            || !self.radius.is_settled()
            || !self.top.is_settled()
            || (self.hover - if inside && !self.expanded { 1.0 } else { 0.0 }).abs() > 0.002
            || (self.content - content_target).abs() > 0.002
            || self.view_alpha < 0.999;
        if animating {
            self.repaint_reason = "animation";
            ctx.request_repaint();
        } else {
            let (wait, reason) = self.next_repaint(&activity);
            self.repaint_reason = reason;
            ctx.request_repaint_after(wait);
        }
    }

    /// How long the notch may wait before it draws again, and why (shown by `--stats`).
    fn next_repaint(&self, activity: &Activity) -> (Duration, &'static str) {
        let ms = Duration::from_millis;
        if self.toast.is_some() {
            return (ms(100), "toast");
        }
        if self.expanded {
            return match self.view {
                View::Media => (ms(if self.lyrics_open { 33 } else { 100 }), "media tab"),
                View::Camera => (ms(33), "camera tab"),
                View::Mixer => (ms(50), "mixer tab"),
                View::Weather => (ms(40), "weather tab"),
                _ => (ms(250), "other tab"),
            };
        }
        if activity.media_playing {
            return (ms(33), "visualizer");
        }
        // Both only pulse slowly, so a lower frame rate looks the same.
        if activity.claude_busy {
            return (ms(66), "claude busy");
        }
        if activity.mic_active {
            return (ms(100), "microphone");
        }
        (ms(1000), "idle")
    }

    fn background_chores(&mut self, ctx: &egui::Context, settings: &Settings) {
        use crate::services::legion::LegionStatus;
        let showing = |view: View| self.expanded && self.view == view;
        self.hub.audio.mixer_visible.store(showing(View::Mixer), Ordering::Relaxed);
        self.hub.system.details_wanted.store(showing(View::Controls), Ordering::Relaxed);
        self.hub.legion.visible.store(showing(View::Legion), Ordering::Relaxed);
        let playing = self.hub.media.state.lock().is_ok_and(|s| s.info.as_ref().is_some_and(|i| i.playing));
        self.hub.audio.meter_wanted.store(playing && !self.expanded, Ordering::Relaxed);

        if self.sweep_at.elapsed() > Duration::from_secs(15) {
            self.sweep_at = Instant::now();
            self.hub.claude.sweep();
        }

        let (waiting, finished) = self.hub.claude.take_attention();
        if waiting || finished {
            let (sessions, pending) = self.hub.claude.snapshot();
            let name = |status| sessions.iter().find(|s| s.status == status).map(|s| s.name()).unwrap_or_default();
            if settings.play_claude_sounds {
                self.hub.play(if waiting { Tone::Bong } else { Tone::Finish });
            }
            if waiting {
                // Claude cannot go on without an answer: open right where the answer is given.
                if !pending.is_empty() && !self.expanded && settings.tab_visible("Claude") {
                    self.open(View::Claude);
                } else {
                    self.hub.toast(Toast::ClaudeWaiting { session: name(ClaudeStatus::Waiting) });
                }
            } else if !self.expanded {
                self.hub.toast(Toast::ClaudeDone { session: name(ClaudeStatus::Done) });
            }
        }
        let _ = self.last_claude_waiting;

        if let Ok(mut latest) = self.hub.clipboard.latest.lock()
            && let Some((_, preview, kind)) = latest.take()
            && settings.show_system_notifications
        {
            self.hub.toast(Toast::Clipboard { preview, kind });
        }

        // Only asked while the Claude tab is open; the endpoint is rate limited.
        if self.expanded && self.view == View::Claude && !self.hub.demo {
            let due = self.limits_checked.is_none_or(|at| at.elapsed() > Duration::from_secs(120));
            if due && settings.is_online_allowed() {
                self.limits_checked = Some(Instant::now());
                let share = self.claude_ui.limits_share.clone();
                let repaint = ctx.clone();
                let _ = std::thread::Builder::new().name("claude-limits".into()).spawn(move || {
                    let limits = crate::services::claude::fetch_limits();
                    if let Ok(mut slot) = share.lock() {
                        *slot = limits;
                    }
                    repaint.request_repaint();
                });
            }
        }
        if let Ok(slot) = self.claude_ui.limits_share.lock() {
            self.limits.clone_from(&slot);
        }
        let _ = LegionStatus::Unsupported;
    }

    fn handle_timer(&mut self, settings: &Settings, ctx: &egui::Context) {
        let finished = self.hub.timer.lock().is_ok_and(|mut timer| timer.tick(Instant::now()));
        if finished {
            self.hub.toast(Toast::TimerDone);
            if settings.enable_interaction_sounds {
                self.hub.play(Tone::Finish);
            }
            ctx.request_repaint();
        }
    }

    fn handle_hover(&mut self, inside: bool, settings: &Settings, now: Instant) {
        if inside {
            self.left_since = None;
            let since = *self.hover_since.get_or_insert(now);
            if settings.enable_hover_expand
                && !self.expanded
                && now.duration_since(since) >= Duration::from_millis(u64::from(settings.hover_expand_delay))
                && self.toast.is_none()
            {
                self.open(if settings.reopen_last_view_on_expand { self.view } else { View::Media });
            }
        } else {
            self.hover_since = None;
            let since = *self.left_since.get_or_insert(now);
            if self.expanded
                && !settings.disable_mouse_leave_auto_close
                && now.duration_since(since) >= Duration::from_millis(u64::from(settings.hover_collapse_delay))
            {
                self.close();
            }
        }
    }

    /// Dragging files over the closed notch opens the shelf; dropping stages them.
    fn handle_file_hover(&mut self, ctx: &egui::Context) {
        let (hovering, dropped) = ctx.input(|i| {
            let dropped: Vec<_> = i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect();
            (!i.raw.hovered_files.is_empty(), dropped)
        });
        if hovering && (!self.expanded || self.view != View::Shelf) {
            self.open(View::Shelf);
        }
        if !dropped.is_empty() {
            if let Ok(mut shelf) = self.hub.shelf.lock() {
                for path in &dropped {
                    shelf.add(path);
                }
            }
            self.hub.play(Tone::Tap);
            self.open(View::Shelf);
        }
    }

    fn paint_shell(&mut self, ui: &mut Ui, rect: Rect, settings: &Settings) {
        let radius = self.radius.value.max(0.0);
        let island = settings.enable_dynamic_island_mode;
        let opacity = if settings.enable_glass_style { settings.glass_tint } else { settings.opacity };
        let fill = theme::alpha(Color32::BLACK, opacity);
        let corners = if island {
            rounded(radius)
        } else {
            CornerRadius { nw: 0, ne: 0, sw: radius.min(255.0) as u8, se: radius.min(255.0) as u8 }
        };

        if settings.enable_shadow {
            let strength = lerp(0.35, 0.55, self.content);
            let shadow = Shadow {
                offset: [0, 6 + (self.content * 6.0) as i8],
                blur: 18 + (self.content * 22.0) as u8,
                spread: 0,
                color: theme::alpha(Color32::BLACK, strength),
            };
            ui.painter().add(shadow.as_shape(rect, corners));
        }
        if settings.enable_glow_on_hover && self.hover > 0.01 {
            let glow = Shadow {
                offset: [0, 0],
                blur: 14,
                spread: 0,
                color: theme::alpha(Color32::WHITE, 0.10 * self.hover),
            };
            ui.painter().add(glow.as_shape(rect, corners));
        }

        ui.painter().rect_filled(rect, corners, fill);

        if !island {
            // The little concave ears where the notch meets the top edge of the screen.
            let ear = (radius * 0.75).clamp(4.0, 10.0);
            ear_shape(ui, rect.left_top(), ear, -1.0, fill);
            ear_shape(ui, rect.right_top(), ear, 1.0, fill);
        }

        if settings.enable_glass_style {
            let line = theme::alpha(Color32::WHITE, 0.16);
            ui.painter().rect_stroke(rect, corners, Stroke::new(1.0, line), egui::StrokeKind::Inside);
        }

        if self.content > 0.01 && self.expanded && self.view == View::Media && settings.show_media_art_background {
            self.paint_art_background(ui, rect, corners, settings);
        }
    }

    fn paint_art_background(&mut self, ui: &mut Ui, rect: Rect, corners: CornerRadius, settings: &Settings) {
        let Some((_, blur)) = self.current_art_textures(ui.ctx()) else { return };
        let tint = theme::alpha(Color32::WHITE, self.content * if settings.enable_blur_effects { 0.55 } else { 0.0 });
        let shape = egui::epaint::RectShape::filled(rect, corners, tint)
            .with_texture(blur.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
        ui.painter().add(shape);
        let overlay = theme::alpha(Color32::BLACK, settings.media_blur_dark_overlay * self.content);
        ui.painter().rect_filled(rect, corners, overlay);
    }

    pub fn current_art_textures(&mut self, ctx: &egui::Context) -> Option<(TextureHandle, TextureHandle)> {
        let art = self.hub.media.state.lock().ok()?.art.clone()?;
        if let Some(found) = self.textures.covers.get(&art.key) {
            return Some(found.clone());
        }
        let load = |name: String, image: &image::RgbaImage| {
            let size = [image.width() as usize, image.height() as usize];
            ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()), TextureOptions::LINEAR)
        };
        let pair = (load(format!("cover-{}", art.key), &art.cover), load(format!("blur-{}", art.key), &art.blur));
        if self.textures.covers.len() > 8 {
            self.textures.covers.clear();
        }
        self.textures.covers.insert(art.key, pair.clone());
        Some(pair)
    }

    fn interact(&mut self, ui: &mut Ui, rect: Rect, settings: &Settings) {
        // Background layer: views add their own widgets on top of this.
        let response = ui.interact(rect, ui.id().with("notch-bg"), Sense::click());
        if response.clicked() {
            if self.toast.is_some() && !self.expanded {
                self.toast_click();
            } else if !self.expanded {
                self.toggle(settings);
            } else if ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| p.y < rect.top() + NAV_H + 8.0) {
                self.toggle(settings);
            }
        }
        if response.middle_clicked() {
            self.hub.media.send(MediaCommand::PlayPause);
            self.hub.play(Tone::Tap);
        }
        if response.secondary_clicked() {
            self.request_settings = true;
        }

        if settings.enable_gesture_controls && ui.rect_contains_pointer(rect) {
            self.handle_gestures(ui);
        }
    }

    fn handle_gestures(&mut self, ui: &mut Ui) {
        const SWIPE_DISTANCE: f32 = 110.0;
        let (delta, dt) = ui.input(|i| (i.smooth_scroll_delta, i.stable_dt));
        self.swipe_cooldown = (self.swipe_cooldown - dt).max(0.0);
        if self.expanded && self.view != View::Media {
            return;
        }
        if delta.x.abs() > delta.y.abs() {
            self.swipe += delta.x;
        } else {
            self.swipe *= 0.8;
        }
        if self.swipe.abs() > SWIPE_DISTANCE && self.swipe_cooldown <= 0.0 {
            let command = if self.swipe > 0.0 { MediaCommand::Previous } else { MediaCommand::Next };
            self.hub.media.send(command);
            self.hub.play(Tone::Tap);
            self.swipe = 0.0;
            self.swipe_cooldown = 0.6;
        }
        if !self.expanded && delta.y < -30.0 && delta.y.abs() > delta.x.abs() && self.swipe_cooldown <= 0.0 {
            self.open(View::Shelf);
            self.swipe_cooldown = 0.6;
        }
    }

    fn toast_click(&mut self) {
        let Some(active) = self.toast.take() else { return };
        match active.toast {
            Toast::Clipboard { .. } => self.open(View::Shelf),
            Toast::Music { .. } => self.open(View::Media),
            Toast::ClaudeWaiting { .. } | Toast::ClaudeDone { .. } => self.open(View::Claude),
            Toast::TimerDone => {
                if let Ok(mut timer) = self.hub.timer.lock() {
                    timer.reset();
                }
            }
            Toast::Volume { .. } => self.open(View::Mixer),
            _ => {}
        }
    }

    fn collect_activity(&mut self, settings: &Settings) -> Activity {
        let mut activity = Activity::default();
        if let Ok(state) = self.hub.media.state.lock()
            && let Some(info) = &state.info
        {
            activity.has_media = true;
            activity.media_playing = info.playing;
            activity.accent = state.art.as_ref().map(|art| art.accent);
        }
        if let Ok(timer) = self.hub.timer.lock()
            && timer.is_active()
        {
            activity.timer = Some(timer.left(Instant::now()));
            activity.extra_width += 74.0;
        }
        let (sessions, pending) = self.hub.claude.snapshot();
        activity.claude_waiting = pending.len() + sessions.iter().filter(|s| s.status == ClaudeStatus::Waiting).count().min(1) * usize::from(pending.is_empty());
        activity.claude_working = sessions.iter().filter(|s| s.status == ClaudeStatus::Working).count();
        activity.claude_busy = activity.claude_waiting > 0 || activity.claude_working > 0;
        if activity.claude_busy {
            activity.extra_width += 34.0;
        }
        if let Ok(system) = self.hub.system.state.lock() {
            activity.mic_active = !system.privacy.mic.is_empty();
            activity.camera_active = !system.privacy.camera.is_empty();
        }
        if activity.mic_active || activity.camera_active {
            activity.extra_width += 16.0;
        }
        let _ = settings;
        activity
    }

    fn paint_collapsed(&mut self, ui: &mut Ui, rect: Rect, settings: &Settings, activity: &Activity) {
        if let Some(active) = &self.toast {
            let toast = active.toast.clone();
            self.paint_toast(ui, rect, &toast);
            return;
        }
        let time = ui.input(|i| i.time);
        let center_y = rect.center().y;
        let inset = if settings.enable_dynamic_island_mode { rect.height() * 0.3 } else { 12.0 };
        let mut left = rect.left() + inset;
        let mut right = rect.right() - inset;
        let art_side = (rect.height() - 10.0).clamp(14.0, 28.0);

        if activity.has_media {
            if let Some((cover, _)) = self.current_art_textures(ui.ctx()) {
                let art = Rect::from_center_size(pos2(left + art_side / 2.0, center_y), vec2(art_side, art_side));
                ui.painter().add(
                    egui::epaint::RectShape::filled(art, rounded(5.0), Color32::WHITE)
                        .with_texture(cover.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
                );
            } else {
                let art = Rect::from_center_size(pos2(left + art_side / 2.0, center_y), vec2(art_side, art_side));
                ui.painter().rect_filled(art, rounded(5.0), theme::CARD_HOVER);
                ui.painter().text(art.center(), Align2::CENTER_CENTER, icons::MUSIC_NOTE, theme::icon(12.0), theme::DIM);
            }
            left += art_side + 8.0;
            let bars = activity.accent.unwrap_or(theme::WHITE);
            let peak = self.hub.audio.state.lock().map_or(0.0, |s| s.peak);
            paint_visualizer(ui, pos2(right - 20.0, center_y), activity.media_playing, peak, bars, time);
            right -= 28.0;
        } else if settings.slim_idle_notch || rect.width() > 180.0 {
            let clock = chrono::Local::now().format("%H:%M").to_string();
            text_clipped(ui, pos2(left, center_y), Align2::LEFT_CENTER, &clock, theme::semibold(13.0), theme::WHITE, 60.0);
            left += 48.0;
            if let Ok(WeatherStatus::Ready(forecast)) = self.hub.weather.status.lock().map(|s| s.clone()) {
                let icon = crate::services::weather::condition_icon(forecast.code, forecast.is_day);
                let temperature = format!("{:.0}°", forecast.temperature);
                let width = kit::text_width(ui, &temperature, theme::medium(12.0));
                ui.painter().text(pos2(right, center_y), Align2::RIGHT_CENTER, &temperature, theme::medium(12.0), theme::DIM);
                ui.painter().text(pos2(right - width - 4.0, center_y), Align2::RIGHT_CENTER, icon, theme::icon(14.0), theme::DIM);
                right -= width + 22.0;
            }
        }

        // Live activities sit between the cover and the bars, as far right as there is room.
        if activity.claude_busy {
            paint_claude_orb(ui, pos2(right - 10.0, center_y), activity, time);
            right -= 26.0;
        }
        if let Some(left_time) = activity.timer {
            let text = timer::format_left(left_time);
            ui.painter().text(pos2(right, center_y), Align2::RIGHT_CENTER, icons::TIMER, theme::icon(13.0), theme::ORANGE);
            let width = kit::text_width(ui, &text, theme::medium(12.0));
            ui.painter().text(pos2(right - 16.0, center_y), Align2::RIGHT_CENTER, &text, theme::medium(12.0), theme::ORANGE);
            right -= width + 24.0;
        }
        if activity.mic_active || activity.camera_active {
            let breathe = 0.65 + 0.35 * (time as f32 * 3.0).sin();
            let color = if activity.camera_active { theme::GREEN } else { theme::ORANGE };
            ui.painter().circle_filled(pos2(right - 4.0, center_y), 3.2, theme::alpha(color, breathe));
            right -= 14.0;
        }
        let _ = (left, right);
    }

    fn paint_toast(&mut self, ui: &mut Ui, rect: Rect, toast: &Toast) {
        let y = rect.center().y;
        let left = rect.left() + 18.0;
        let right = rect.right() - 18.0;
        let row = |ui: &mut Ui, icon: &str, color: Color32, title: &str, subtitle: &str| {
            ui.painter().text(pos2(left + 9.0, y), Align2::CENTER_CENTER, icon, theme::icon_fill(18.0), color);
            let text_left = left + 28.0;
            let width = right - text_left;
            if subtitle.is_empty() {
                text_clipped(ui, pos2(text_left, y), Align2::LEFT_CENTER, title, theme::semibold(13.0), theme::WHITE, width);
            } else {
                text_clipped(ui, pos2(text_left, y - 8.0), Align2::LEFT_CENTER, title, theme::semibold(13.0), theme::WHITE, width);
                text_clipped(ui, pos2(text_left, y + 9.0), Align2::LEFT_CENTER, subtitle, theme::regular(11.5), theme::DIM, width);
            }
        };
        match toast {
            Toast::Hello => {
                let hour = chrono::Timelike::hour(&chrono::Local::now());
                let key = if hour < 11 {
                    "greeting.morning"
                } else if hour < 18 {
                    "greeting.afternoon"
                } else {
                    "greeting.evening"
                };
                row(ui, icons::SPARKLE, theme::YELLOW, &t(key), &t("greeting.enjoyDay"));
            }
            Toast::Music { title, artist } => {
                let accent = self.hub.media.state.lock().ok().and_then(|s| s.art.as_ref().map(|a| a.accent)).unwrap_or(theme::WHITE);
                row(ui, icons::MUSIC_NOTES, accent, title, artist);
            }
            Toast::Volume { level, muted } => {
                let icon = volume_icon(*level, *muted);
                let color = if *muted { theme::DIM } else { theme::YELLOW };
                ui.painter().text(pos2(left + 9.0, y), Align2::CENTER_CENTER, icon, theme::icon_fill(17.0), color);
                let bar = Rect::from_min_max(pos2(left + 28.0, y - 3.0), pos2(right - 36.0, y + 3.0));
                ui.painter().rect_filled(bar, rounded(3.0), theme::alpha(Color32::WHITE, 0.18));
                let filled = bar.width() * if *muted { 0.0 } else { level.clamp(0.0, 1.0) };
                if filled > 0.5 {
                    let bar_fill = Rect::from_min_size(bar.min, vec2(filled.max(6.0), bar.height()));
                    ui.painter().rect_filled(bar_fill, rounded(3.0), color);
                }
                let percent = format!("{}", (level * 100.0).round() as i32);
                ui.painter().text(pos2(right, y), Align2::RIGHT_CENTER, percent, theme::medium(11.5), theme::DIM);
            }
            Toast::Clipboard { preview, kind } => row(ui, kind.icon(), theme::ACCENT, &t("clipboard.copy"), preview),
            Toast::Battery { percent, charging } => {
                let title = t(if *charging { "battery.charging" } else { "battery.onBattery" });
                let icon = if *charging { icons::BATTERY_CHARGING } else { battery_icon(*percent) };
                let color = if *charging { theme::GREEN } else { theme::WHITE };
                row(ui, icon, color, &title, &format!("{percent:.0} %"));
            }
            Toast::Bluetooth { name, connected } => {
                let status = t(if *connected { "bluetooth.connected" } else { "bluetooth.disconnected" });
                row(ui, icons::BLUETOOTH, theme::ACCENT, name, &status);
            }
            Toast::TimerDone => row(ui, icons::TIMER, theme::ORANGE, &t("port.timer.done"), &t("timer.dismiss")),
            Toast::ClaudeWaiting { session } => row(ui, icons::SPARKLE, theme::ORANGE, &t("claude.status.waiting"), session),
            Toast::ClaudeDone { session } => row(ui, icons::SPARKLE, theme::GREEN, &t("control.claude.done"), session),
            Toast::Info { icon, text } => row(ui, icon, theme::WHITE, text, ""),
        }
    }

    fn paint_nav(&mut self, ui: &mut Ui, rect: Rect, settings: &Settings) {
        let tabs = self.visible_tabs(settings);
        let mut x = rect.left() + 14.0;
        let y = rect.top() + NAV_H / 2.0 + 3.0;
        for (index, view) in tabs.iter().enumerate() {
            let cell = Rect::from_center_size(pos2(x + 11.0, y), vec2(24.0, 24.0));
            let selected = *view == self.view;
            let color = if selected { theme::WHITE } else { theme::FAINT };
            let response = if selected {
                kit::icon_button_fill(ui, cell, view.icon(), 15.0, color, ("tab", index))
            } else {
                kit::icon_button(ui, cell, view.icon(), 15.0, color, ("tab", index))
            };
            let response = response.on_hover_text(t(view.label_key()));
            if response.clicked() && !selected {
                self.view_alpha = 0.0;
                self.view = *view;
                self.lyrics_open = false;
                self.hub.play(Tone::Tap);
            }
            x += 27.0;
        }
        let mut right = rect.right() - 14.0;
        let gear = Rect::from_center_size(pos2(right - 11.0, y), vec2(24.0, 24.0));
        if kit::icon_button(ui, gear, icons::GEAR_SIX, 15.0, theme::DIM, "nav-settings").on_hover_text(t("tray.settings")).clicked() {
            self.request_settings = true;
        }
        right -= 28.0;
        if settings.enable_spotlight {
            let search = Rect::from_center_size(pos2(right - 11.0, y), vec2(24.0, 24.0));
            if kit::icon_button(ui, search, icons::MAGNIFYING_GLASS, 15.0, theme::DIM, "nav-spotlight").clicked() {
                self.request_spotlight = true;
            }
            right -= 30.0;
        }
        if settings.show_battery_indicator {
            let battery = self.hub.system.state.lock().ok().and_then(|s| s.battery.clone());
            if let Some(battery) = battery {
                let text = format!("{:.0}%", battery.percent);
                ui.painter().text(pos2(right, y), Align2::RIGHT_CENTER, &text, theme::medium(11.0), theme::DIM);
                let width = kit::text_width(ui, &text, theme::medium(11.0));
                let icon = if battery.charging { icons::BATTERY_CHARGING } else { battery_icon(battery.percent) };
                let color = if battery.charging { theme::GREEN } else { theme::DIM };
                ui.painter().text(pos2(right - width - 4.0, y), Align2::RIGHT_CENTER, icon, theme::icon(15.0), color);
            }
        }
    }

    fn paint_expanded(&mut self, ui: &mut Ui, rect: Rect, settings: &Settings, frame: &eframe::Frame) {
        self.paint_nav(ui, rect, settings);
        let body = Rect::from_min_max(pos2(rect.left() + 16.0, rect.top() + NAV_H + 8.0), pos2(rect.right() - 16.0, rect.bottom() - 12.0));
        let mut child = ui.new_child(UiBuilder::new().max_rect(body).id_salt(self.view));
        child.set_clip_rect(rect);
        child.multiply_opacity(self.view_alpha);
        match self.view {
            View::Media => self.media_view(&mut child, body, settings),
            View::Shelf => self.shelf_view(&mut child, body, settings, frame),
            View::Controls => self.controls_view(&mut child, body, settings),
            View::Claude => self.claude_view(&mut child, body, settings),
            View::Weather => self.weather_view(&mut child, body, settings),
            View::Legion => self.legion_view(&mut child, body),
            View::Mixer => self.mixer_view(&mut child, body),
            View::Camera => self.camera_view(&mut child, body, settings),
        }
    }
}

/// Size of the open notch for a tab. The Claude tab grows with its sessions, the others are fixed.
fn open_size(view: View, claude_height: f32) -> (f32, f32) {
    let (w, h) = view.size();
    if view == View::Claude { (w, claude_height.clamp(150.0, 420.0)) } else { (w, h) }
}

/// How far the notch has opened: 0 at the collapsed height, 1 at the tab's open height.
fn open_progress(height: f32, compact_h: f32, open_h: f32) -> f32 {
    ((height - compact_h) / (open_h - compact_h).max(1.0)).clamp(0.0, 1.0)
}

/// On macOS the cursor comes in points; everywhere else in physical pixels, like the monitor layout.
fn platform_scale(pixels_per_point: f32) -> f32 {
    if cfg!(target_os = "macos") { 1.0 } else { pixels_per_point }
}

#[derive(Default)]
struct Activity {
    has_media: bool,
    media_playing: bool,
    accent: Option<Color32>,
    timer: Option<Duration>,
    claude_waiting: usize,
    claude_working: usize,
    claude_busy: bool,
    mic_active: bool,
    camera_active: bool,
    extra_width: f32,
}

fn toast_size(toast: &Toast, base_w: f32, base_h: f32) -> (f32, f32) {
    let tall = base_h.max(46.0) + 6.0;
    match toast {
        Toast::Volume { .. } => ((base_w + 70.0).max(300.0), base_h.max(38.0)),
        Toast::Hello => (300.0, tall),
        _ => (380.0, tall),
    }
}

fn ear_shape(ui: &Ui, corner: Pos2, size: f32, side: f32, fill: Color32) {
    // A fan from the notch corner over a quarter circle, so the convex tessellator handles it fine.
    let center = pos2(corner.x + side * size, corner.y + size);
    let mut points = vec![pos2(corner.x - side * 1.0, corner.y), pos2(corner.x + side * size, corner.y)];
    for step in 0..=8 {
        let angle = std::f32::consts::FRAC_PI_2 * step as f32 / 8.0;
        points.push(pos2(center.x - side * size * angle.sin(), center.y - size * angle.cos()));
    }
    points.push(pos2(corner.x - side * 1.0, corner.y + size));
    ui.painter().add(Shape::convex_polygon(points, fill, Stroke::NONE));
}

/// Bars that follow the master level where the OS reports one, and a calm idle dance elsewhere.
fn paint_visualizer(ui: &Ui, center: Pos2, playing: bool, peak: f32, color: Color32, time: f64) {
    const BARS: usize = 5;
    let time = time as f32;
    for index in 0..BARS {
        let phase = index as f32 * 1.7;
        let wave = 0.5 + 0.5 * (time * (5.0 + index as f32 * 0.9) + phase).sin() * (time * 2.3 + phase * 0.5).cos();
        let drive = if peak > 0.001 { (peak * 1.6).clamp(0.2, 1.0) } else { 0.7 };
        let level = if playing { 0.2 + wave * drive * 0.8 } else { 0.14 };
        let height = 3.0 + level * 13.0;
        let x = center.x + (index as f32 - (BARS as f32 - 1.0) / 2.0) * 4.2;
        let bar = Rect::from_center_size(pos2(x, center.y), vec2(2.6, height));
        ui.painter().rect_filled(bar, rounded(1.3), color);
    }
}

fn paint_claude_orb(ui: &Ui, center: Pos2, activity: &Activity, time: f64) {
    let (color, speed) = if activity.claude_waiting > 0 { (theme::ORANGE, 6.0) } else { (Color32::from_rgb(217, 119, 87), 3.0) };
    let pulse = 0.5 + 0.5 * (time as f32 * speed).sin();
    ui.painter().circle_filled(center, 6.0 + pulse * 2.0, theme::alpha(color, 0.25));
    ui.painter().text(center, Align2::CENTER_CENTER, icons::SPARKLE, theme::icon_fill(11.0), color);
    let total = activity.claude_waiting + activity.claude_working;
    if total > 1 {
        let badge = center + vec2(7.0, -6.0);
        ui.painter().circle_filled(badge, 5.0, color);
        ui.painter().text(badge, Align2::CENTER_CENTER, total.to_string(), theme::bold(8.0), Color32::BLACK);
    }
}

pub fn volume_icon(level: f32, muted: bool) -> &'static str {
    if muted || level <= 0.001 {
        icons::SPEAKER_X
    } else if level < 0.34 {
        icons::SPEAKER_NONE
    } else if level < 0.67 {
        icons::SPEAKER_LOW
    } else {
        icons::SPEAKER_HIGH
    }
}

pub fn battery_icon(percent: f32) -> &'static str {
    if percent >= 88.0 {
        icons::BATTERY_FULL
    } else if percent >= 60.0 {
        icons::BATTERY_HIGH
    } else if percent >= 35.0 {
        icons::BATTERY_MEDIUM
    } else if percent >= 12.0 {
        icons::BATTERY_LOW
    } else {
        icons::BATTERY_EMPTY
    }
}

/// "m:ss", or "h:mm:ss" past the hour.
pub fn format_clock(duration: Duration) -> String {
    let total = duration.as_secs();
    if total >= 3600 {
        format!("{}:{:02}:{:02}", total / 3600, (total % 3600) / 60, total % 60)
    } else {
        format!("{}:{:02}", total / 60, total % 60)
    }
}

pub fn clear_label(count: usize) -> String {
    tf("clipboard.emptyTrayConfirm", &[&count.to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_tokens_round_trip() {
        for view in [View::Media, View::Shelf, View::Controls, View::Claude, View::Weather, View::Legion, View::Mixer, View::Camera] {
            assert_eq!(View::from_token(view.token()), Some(view));
        }
        assert_eq!(View::from_token("Pi"), None);
    }

    #[test]
    fn every_default_tab_maps_to_a_view() {
        for token in crate::settings::DEFAULT_TABS.split(',') {
            assert!(View::from_token(token).is_some(), "{token}");
        }
    }

    #[test]
    fn content_shows_once_the_notch_reaches_the_tab_height() {
        let compact_h = 32.0;
        // An empty Claude tab is only 150 px tall; at that height it is fully open.
        let (_, claude_h) = open_size(View::Claude, 150.0);
        assert_eq!(claude_h, 150.0);
        assert_eq!(open_progress(150.0, compact_h, claude_h), 1.0);
        // With sessions it grows, up to its limit.
        assert_eq!(open_size(View::Claude, 900.0).1, 420.0);
        assert_eq!(open_size(View::Media, 150.0), View::Media.size());
        // Half way there means half open; the collapsed notch is closed.
        let (_, media_h) = open_size(View::Media, 150.0);
        assert!((open_progress((compact_h + media_h) / 2.0, compact_h, media_h) - 0.5).abs() < 1e-6);
        assert_eq!(open_progress(compact_h, compact_h, media_h), 0.0);
    }

    #[test]
    fn all_views_fit_in_the_window() {
        for view in [View::Media, View::Shelf, View::Controls, View::Claude, View::Weather, View::Legion, View::Mixer, View::Camera] {
            let (w, h) = view.size();
            assert!(w + 40.0 <= WINDOW_W, "{view:?} too wide");
            assert!(h + ISLAND_TOP + 40.0 <= WINDOW_H, "{view:?} too tall");
        }
    }

    #[test]
    fn clock_format() {
        assert_eq!(format_clock(Duration::from_secs(62)), "1:02");
        assert_eq!(format_clock(Duration::from_secs(3723)), "1:02:03");
        assert_eq!(format_clock(Duration::ZERO), "0:00");
    }

    #[test]
    fn icons_follow_levels() {
        assert_eq!(volume_icon(0.5, true), icons::SPEAKER_X);
        assert_eq!(volume_icon(0.9, false), icons::SPEAKER_HIGH);
        assert_eq!(battery_icon(100.0), icons::BATTERY_FULL);
        assert_eq!(battery_icon(5.0), icons::BATTERY_EMPTY);
    }

    #[test]
    fn cursor_share_hit_test_handles_negative_screens() {
        let share = CursorShare::default();
        share.set_rect(Rect::from_min_max(pos2(-1000.0, 0.0), pos2(-800.0, 40.0)));
        assert!(share.contains(-900.0, 10.0));
        assert!(!share.contains(-700.0, 10.0));
        assert!(!share.contains(-900.0, 50.0));
    }

    #[test]
    fn cursor_is_watched_closely_only_near_or_open() {
        let share = CursorShare::default();
        share.set_rect(Rect::from_min_max(pos2(500.0, 0.0), pos2(700.0, 40.0)));
        assert_eq!(share.distance(600.0, 20.0), 0);
        assert_eq!(share.distance(600.0, 140.0), 100);
        assert_eq!(share.distance(300.0, 20.0), 200);
        assert_eq!(cursor_poll_interval(100, false), Duration::from_millis(16));
        assert_eq!(cursor_poll_interval(900, false), Duration::from_millis(60));
        assert_eq!(cursor_poll_interval(900, true), Duration::from_millis(16));
    }
}
