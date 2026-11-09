//! Background work. Each service owns its thread and publishes snapshots the UI reads every frame.

pub mod artwork;
pub mod audio;
pub mod claude;
pub mod clipboard;
pub mod http;
pub mod hotkeys;
pub mod legion;
pub mod lyrics;
pub mod media;
pub mod moodle;
pub mod shelf;
// The original's control center took over the controls tab and hides the focus timer and the
// ambient sounds; both services stay, unused by the UI for now, so they can come back.
#[allow(dead_code)]
pub mod sounds;
pub mod system;
#[allow(dead_code)]
pub mod timer;
pub mod teams;
pub mod untis;
pub mod tray;
pub mod updates;
pub mod weather;
pub mod weather_service;

use crate::settings::SharedSettings;
use eframe::egui;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub type Shared<T> = Arc<Mutex<T>>;

pub fn shared<T>(value: T) -> Shared<T> {
    Arc::new(Mutex::new(value))
}

/// A short message the pill shows and then drops.
#[derive(Clone, Debug, PartialEq)]
pub enum Toast {
    Hello,
    Music { title: String, artist: String },
    Volume { level: f32, muted: bool },
    Clipboard { preview: String, kind: clipboard::ClipKind },
    Battery { percent: f32, charging: bool },
    Bluetooth { name: String, connected: bool },
    /// A Teams notification from Windows: sender (or chat) and text.
    Teams { sender: String, text: String },
    TimerDone,
    ClaudeWaiting { session: String },
    ClaudeDone { session: String },
    Info { icon: &'static str, text: String },
}

/// Everything the windows need, shared with the background threads.
pub struct Hub {
    pub ctx: egui::Context,
    pub settings: SharedSettings,
    pub media: Arc<media::MediaService>,
    pub audio: Arc<audio::AudioService>,
    pub system: Arc<system::SystemService>,
    pub weather: Arc<weather_service::WeatherService>,
    pub clipboard: Arc<clipboard::ClipboardService>,
    pub shelf: Shared<shelf::Shelf>,
    pub timer: Shared<timer::Countdown>,
    pub sounds: sounds::Sounds,
    pub claude: Arc<claude::Tracker>,
    pub claude_listening: bool,
    pub legion: Arc<legion::LegionService>,
    pub moodle: Arc<moodle::MoodleService>,
    pub untis: Arc<untis::UntisService>,
    pub teams: Arc<teams::TeamsService>,
    pub updates: Arc<updates::UpdateService>,
    pub toasts: Shared<VecDeque<Toast>>,
    pub demo: bool,
}

impl Hub {
    pub fn start(ctx: egui::Context, settings: SharedSettings, demo: bool) -> Arc<Self> {
        let toasts = shared(VecDeque::new());
        let claude = Arc::new(claude::Tracker::default());
        let mut claude_listening = false;
        if !demo {
            match claude::start_server(claude.clone(), ctx.clone()) {
                Ok(()) => claude_listening = true,
                Err(error) => crate::vlog!("CLAUDE", "hook port unavailable: {error}"),
            }
        }
        let hub = Self {
            media: media::MediaService::start(ctx.clone(), settings.clone(), demo),
            audio: audio::AudioService::start(ctx.clone(), toasts.clone(), demo),
            system: system::SystemService::start(ctx.clone(), settings.clone(), toasts.clone(), demo),
            weather: weather_service::WeatherService::start(ctx.clone(), settings.clone(), demo),
            clipboard: clipboard::ClipboardService::start(settings.clone(), ctx.clone(), demo),
            shelf: shared(shelf::Shelf::load()),
            timer: shared(timer::Countdown::default()),
            sounds: sounds::Sounds::start(),
            claude,
            claude_listening,
            legion: legion::LegionService::start(ctx.clone(), demo),
            moodle: moodle::MoodleService::start(ctx.clone(), settings.clone(), demo),
            untis: untis::UntisService::start(ctx.clone(), settings.clone(), demo),
            teams: teams::TeamsService::start(ctx.clone(), settings.clone(), demo),
            updates: updates::UpdateService::new(ctx.clone()),
            toasts,
            demo,
            ctx,
            settings,
        };
        if demo {
            demo_seed(&hub);
        }
        teams::watch_notifications(hub.toasts.clone(), hub.settings.clone(), hub.ctx.clone(), demo);
        Arc::new(hub)
    }

    pub fn toast(&self, toast: Toast) {
        push_toast(&self.toasts, toast);
        self.ctx.request_repaint();
    }

    /// Plays an interaction sound if the user wants those.
    pub fn play(&self, tone: sounds::Tone) {
        let (enabled, volume) = self.settings.read(|s| (s.enable_interaction_sounds, s.interaction_sound_volume));
        if enabled && !self.demo {
            self.sounds.play(tone, volume);
        }
    }
}

pub fn push_toast(toasts: &Shared<VecDeque<Toast>>, toast: Toast) {
    if let Ok(mut queue) = toasts.lock() {
        // A volume change replaces the previous one instead of queueing behind it.
        if matches!(toast, Toast::Volume { .. }) {
            queue.retain(|t| !matches!(t, Toast::Volume { .. }));
        }
        if queue.len() < 8 {
            queue.push_back(toast);
        }
    }
}

/// Fills the app with made-up data, so the UI can be shown and tested without touching real state.
fn demo_seed(hub: &Hub) {
    use serde_json::json;
    let session = |id: &str, event: &str, extra: serde_json::Value| {
        let mut payload = json!({"session_id": id, "hook_event_name": event, "cwd": format!("/work/{id}")});
        if let (Some(target), Some(source)) = (payload.as_object_mut(), extra.as_object()) {
            target.extend(source.clone());
        }
        payload
    };
    hub.claude.handle(&session("v-notch", "UserPromptSubmit", json!({"prompt": "Baue den Notch in Rust nach"})));
    hub.claude.handle(&session("v-notch", "PreToolUse", json!({"tool_name": "Bash", "tool_input": {"command": "cargo test"}})));
    hub.claude.handle(&session("website", "UserPromptSubmit", json!({"prompt": "Räume die Startseite auf"})));
    hub.claude.handle(&session("website", "Stop", json!({"last_assistant_message": "Die Startseite ist aufgeräumt."})));
    // The receiver is dropped on purpose: nothing is waiting for an answer in the demo.
    drop(hub.claude.handle(&session("api", "PermissionRequest", json!({
        "tool_name": "Bash",
        "tool_input": {"command": "git push origin main"},
        "permission_suggestions": [{"rules": [{"toolName": "Bash", "ruleContent": "git push:*"}]}]
    }))));

    if let Ok(mut store) = hub.clipboard.store.lock() {
        for (index, text) in ["https://github.com/pichlervalentin4-lab/V-Notch", "fn main() {\n    println!(\"Hallo\");\n}", "#30d158", "Einkaufsliste: Milch, Brot, Käse"].iter().enumerate() {
            store.add(
                clipboard::ClipEntry {
                    kind: clipboard::classify_text(text),
                    text: (*text).to_string(),
                    hash: index as u64 + 1,
                    created_unix: clipboard::unix_now(),
                    ..Default::default()
                },
                200,
            );
        }
    }
    if let Ok(mut shelf) = hub.shelf.lock() {
        shelf.demo_fill();
    }
}
