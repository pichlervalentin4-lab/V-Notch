//! Lenovo Legion power modes and locks, through the elevated helper the original also uses.

use crate::platform;
use eframe::egui;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const HELPER_VERSION: i64 = 2;
pub const MODE_QUIET: i32 = 1;
pub const MODE_BALANCED: i32 = 2;
pub const MODE_PERFORMANCE: i32 = 3;
pub const MODE_EXTREME: i32 = 224;
pub const MODE_CUSTOM: i32 = 255;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LegionState {
    pub mode: i32,
    pub win_key_locked: bool,
    pub touchpad_locked: bool,
    pub cpu_temp: i32,
    pub gpu_temp: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum LegionStatus {
    /// Not a Legion, or not Windows.
    #[default]
    Unsupported,
    /// A Legion, but the helper has not been set up yet.
    NeedsSetup,
    SettingUp,
    Ready(LegionState),
}

#[derive(Clone, Debug, PartialEq)]
pub enum LegionCommand {
    Mode(i32),
    WinKey(bool),
    Touchpad(bool),
    Setup,
}

pub struct LegionService {
    pub status: Mutex<LegionStatus>,
    pub visible: AtomicBool,
    commands: Mutex<Sender<LegionCommand>>,
}

/// The helper's answer; an older helper counts as missing, so the setup is offered again.
pub fn parse_state(answer: &str) -> Option<LegionState> {
    let value: Value = serde_json::from_str(answer.trim()).ok()?;
    if value.get("error").is_some() || value["version"].as_i64() != Some(HELPER_VERSION) {
        return None;
    }
    let number = |key: &str| value[key].as_i64().unwrap_or(-1) as i32;
    Some(LegionState {
        mode: number("mode"),
        win_key_locked: number("winKey") == 1,
        touchpad_locked: number("touchpad") == 1,
        cpu_temp: number("cpuTemp"),
        gpu_temp: number("gpuTemp"),
    })
}

fn command_line(command: &LegionCommand) -> Option<String> {
    Some(match command {
        LegionCommand::Mode(mode) => format!("mode {mode}"),
        LegionCommand::WinKey(locked) => format!("winkey {}", i32::from(*locked)),
        LegionCommand::Touchpad(locked) => format!("touchpad {}", i32::from(*locked)),
        LegionCommand::Setup => return None,
    })
}

impl LegionService {
    pub fn start(ctx: egui::Context, demo: bool) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel();
        let service = Arc::new(Self {
            status: Mutex::new(LegionStatus::Unsupported),
            visible: AtomicBool::new(false),
            commands: Mutex::new(sender),
        });
        if demo {
            if let Ok(mut status) = service.status.lock() {
                *status = LegionStatus::Ready(LegionState {
                    mode: MODE_BALANCED,
                    win_key_locked: false,
                    touchpad_locked: false,
                    cpu_temp: 54,
                    gpu_temp: 48,
                });
            }
            return service;
        }
        if !platform::is_legion() {
            return service;
        }
        let worker = service.clone();
        let _ = std::thread::Builder::new().name("legion".into()).spawn(move || {
            worker.set(LegionStatus::NeedsSetup, &ctx);
            loop {
                let wait = if worker.visible.load(Ordering::Relaxed) { Duration::from_secs(3) } else { Duration::from_secs(30) };
                let command = receiver.recv_timeout(wait).ok();
                match command {
                    Some(LegionCommand::Setup) => {
                        worker.set(LegionStatus::SettingUp, &ctx);
                        if !platform::legion_setup() {
                            worker.set(LegionStatus::NeedsSetup, &ctx);
                            continue;
                        }
                        // The scheduled task needs a moment before its pipe appears.
                        std::thread::sleep(Duration::from_secs(3));
                        worker.poll("state", &ctx);
                    }
                    Some(other) => {
                        if let Some(line) = command_line(&other) {
                            worker.poll(&line, &ctx);
                        }
                    }
                    None => worker.poll("state", &ctx),
                }
            }
        });
        service
    }

    fn poll(&self, request: &str, ctx: &egui::Context) {
        let next = platform::legion_request(request)
            .as_deref()
            .and_then(parse_state)
            .map_or(LegionStatus::NeedsSetup, LegionStatus::Ready);
        self.set(next, ctx);
    }

    fn set(&self, next: LegionStatus, ctx: &egui::Context) {
        if let Ok(mut status) = self.status.lock()
            && *status != next
        {
            *status = next;
            ctx.request_repaint();
        }
    }

    pub fn send(&self, command: LegionCommand) {
        if let Ok(sender) = self.commands.lock() {
            let _ = sender.send(command);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_is_parsed() {
        let state = parse_state(r#"{"version":2,"mode":224,"winKey":1,"touchpad":0,"cpuTemp":61,"gpuTemp":55}"#).unwrap();
        assert_eq!(state.mode, MODE_EXTREME);
        assert!(state.win_key_locked);
        assert!(!state.touchpad_locked);
        assert_eq!(state.cpu_temp, 61);
    }

    #[test]
    fn old_helpers_and_errors_count_as_missing() {
        assert!(parse_state(r#"{"version":1,"mode":1}"#).is_none());
        assert!(parse_state(r#"{"error":"bad value"}"#).is_none());
        assert!(parse_state("garbage").is_none());
    }

    #[test]
    fn commands_match_what_the_helper_expects() {
        assert_eq!(command_line(&LegionCommand::Mode(3)).unwrap(), "mode 3");
        assert_eq!(command_line(&LegionCommand::WinKey(true)).unwrap(), "winkey 1");
        assert_eq!(command_line(&LegionCommand::Touchpad(false)).unwrap(), "touchpad 0");
        assert!(command_line(&LegionCommand::Setup).is_none());
    }
}
