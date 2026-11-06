//! Master volume, per-app mixer, devices and the microphone.

use super::{Shared, Toast, push_toast};
use crate::platform::{AudioBackend, AudioDeviceInfo, AudioSessionInfo, VolumeInfo};
use eframe::egui;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum AudioCommand {
    SetMaster(f32),
    SetMasterMute(bool),
    SetSessionVolume(String, f32),
    SetSessionMute(String, bool),
    SetDefaultDevice(String),
    SetMicMute(bool),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioState {
    pub master: Option<VolumeInfo>,
    pub mic: Option<VolumeInfo>,
    pub peak: f32,
    pub sessions: Vec<AudioSessionInfo>,
    pub outputs: Vec<AudioDeviceInfo>,
    pub inputs: Vec<AudioDeviceInfo>,
}

pub struct AudioService {
    pub state: Mutex<AudioState>,
    /// Set while the mixer is on screen: then the app list refreshes quickly.
    pub mixer_visible: AtomicBool,
    /// Set while something shows the level meter or visualizer.
    pub meter_wanted: AtomicBool,
    commands: Mutex<Sender<AudioCommand>>,
}

impl AudioService {
    pub fn start(ctx: egui::Context, toasts: Shared<VecDeque<Toast>>, demo: bool) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel();
        let service = Arc::new(Self {
            state: Mutex::new(AudioState::default()),
            mixer_visible: AtomicBool::new(false),
            meter_wanted: AtomicBool::new(false),
            commands: Mutex::new(sender),
        });
        if demo {
            service.demo_fill();
            return service;
        }
        let worker = service.clone();
        let _ = std::thread::Builder::new().name("audio".into()).spawn(move || {
            // COM objects stay on this thread, so the backend is created here.
            let backend = AudioBackend::new();
            let mut last_master: Option<VolumeInfo> = None;
            let mut last_full = Instant::now() - Duration::from_secs(60);
            let mut ignore_toast_until = Instant::now() + Duration::from_secs(2);
            loop {
                let mixer = worker.mixer_visible.load(Ordering::Relaxed);
                let meter = worker.meter_wanted.load(Ordering::Relaxed);
                let wait = if mixer || meter { Duration::from_millis(50) } else { Duration::from_millis(400) };
                if let Ok(command) = receiver.recv_timeout(wait) {
                    // What the user set in the notch needs no separate volume pop-up.
                    ignore_toast_until = Instant::now() + Duration::from_millis(800);
                    apply(&backend, command);
                    while let Ok(next) = receiver.try_recv() {
                        apply(&backend, next);
                    }
                }

                let master = backend.master();
                if let (Some(now), Some(before)) = (master, last_master)
                    && (now != before)
                    && Instant::now() > ignore_toast_until
                {
                    push_toast(&toasts, Toast::Volume { level: now.level, muted: now.muted });
                    ctx.request_repaint();
                }
                last_master = master;

                let full_due = last_full.elapsed() > if mixer { Duration::from_millis(500) } else { Duration::from_secs(5) };
                let mut changed = false;
                if let Ok(mut state) = worker.state.lock() {
                    changed |= state.master != master;
                    state.master = master;
                    state.peak = if meter || mixer { backend.master_peak() } else { 0.0 };
                    if mixer {
                        // Levels need to move while the mixer is open, not just on a full refresh.
                        let sessions = backend.sessions();
                        changed |= state.sessions != sessions;
                        state.sessions = sessions;
                    }
                    if full_due {
                        let mic = backend.mic();
                        let outputs = backend.output_devices();
                        let inputs = backend.input_devices();
                        changed |= state.mic != mic || state.outputs != outputs || state.inputs != inputs;
                        state.mic = mic;
                        state.outputs = outputs;
                        state.inputs = inputs;
                        last_full = Instant::now();
                    }
                }
                if changed {
                    ctx.request_repaint();
                }
            }
        });
        service
    }

    pub fn send(&self, command: AudioCommand) {
        if let Ok(sender) = self.commands.lock() {
            let _ = sender.send(command);
        }
    }

    fn demo_fill(&self) {
        let session = |name: &str, level: f32, peak: f32| AudioSessionInfo {
            id: name.to_string(),
            name: name.to_string(),
            level,
            muted: false,
            peak,
        };
        let device = |name: &str, is_default: bool| AudioDeviceInfo { id: name.to_string(), name: name.to_string(), is_default };
        if let Ok(mut state) = self.state.lock() {
            *state = AudioState {
                master: Some(VolumeInfo { level: 0.62, muted: false }),
                mic: Some(VolumeInfo { level: 0.8, muted: false }),
                peak: 0.5,
                sessions: vec![session("Spotify", 0.8, 0.6), session("Chrome", 0.45, 0.2), session("Discord", 1.0, 0.0)],
                outputs: vec![device("Lautsprecher (Realtek)", true), device("Kopfhörer (USB)", false)],
                inputs: vec![device("Mikrofon (Realtek)", true)],
            };
        }
    }
}

fn apply(backend: &AudioBackend, command: AudioCommand) {
    match command {
        AudioCommand::SetMaster(level) => backend.set_master(level),
        AudioCommand::SetMasterMute(muted) => backend.set_master_mute(muted),
        AudioCommand::SetSessionVolume(id, level) => backend.set_session_volume(&id, level),
        AudioCommand::SetSessionMute(id, muted) => backend.set_session_mute(&id, muted),
        AudioCommand::SetDefaultDevice(id) => backend.set_default_device(&id),
        AudioCommand::SetMicMute(muted) => backend.set_mic_muted(muted),
    }
}
