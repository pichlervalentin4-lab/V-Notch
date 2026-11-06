//! Global keyboard shortcuts.

use crate::settings::Settings;
use eframe::egui;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::mpsc::{self, Receiver};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyAction {
    Spotlight,
    Clipboard,
    Translate,
    ToggleInvisible,
}

pub struct Hotkeys {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<HotKey>,
    actions: HashMap<u32, HotkeyAction>,
    events: Receiver<u32>,
    signature: String,
}

/// Accepts "Alt+Space", "ctrl+shift+t" and the like. An empty or unknown text means no shortcut.
pub fn parse(text: &str) -> Option<HotKey> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    HotKey::from_str(text).ok()
}

impl Hotkeys {
    /// Must run on the main thread, inside the event loop.
    pub fn new(ctx: &egui::Context) -> Self {
        let manager = match GlobalHotKeyManager::new() {
            Ok(manager) => Some(manager),
            Err(error) => {
                crate::vlog!("HOTKEYS", "unavailable: {error}");
                None
            }
        };
        let (sender, events) = mpsc::channel();
        let ctx = ctx.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() == HotKeyState::Pressed {
                let _ = sender.send(event.id());
                ctx.request_repaint();
            }
        }));
        Self { manager, registered: Vec::new(), actions: HashMap::new(), events, signature: String::new() }
    }

    /// Re-registers when the shortcuts in the settings changed.
    pub fn apply(&mut self, settings: &Settings) {
        let mut wanted = Vec::new();
        if settings.enable_spotlight {
            wanted.push((settings.spotlight_hotkey.as_str(), HotkeyAction::Spotlight));
        }
        wanted.push((settings.clipboard_hotkey.as_str(), HotkeyAction::Clipboard));
        wanted.push((settings.invisible_hotkey.as_str(), HotkeyAction::ToggleInvisible));
        if settings.enable_live_translation {
            wanted.push((settings.translation_hotkey.as_str(), HotkeyAction::Translate));
        }
        let signature = wanted.iter().map(|(text, action)| format!("{action:?}={text}")).collect::<Vec<_>>().join(";");
        if signature == self.signature {
            return;
        }
        self.signature = signature;
        let Some(manager) = self.manager.as_ref() else { return };
        for hotkey in self.registered.drain(..) {
            let _ = manager.unregister(hotkey);
        }
        self.actions.clear();
        for (text, action) in wanted {
            let Some(hotkey) = parse(text) else { continue };
            match manager.register(hotkey) {
                Ok(()) => {
                    self.actions.insert(hotkey.id(), action);
                    self.registered.push(hotkey);
                }
                Err(error) => crate::vlog!("HOTKEYS", "{text}: {error}"),
            }
        }
    }

    pub fn poll(&self) -> Vec<HotkeyAction> {
        self.events.try_iter().filter_map(|id| self.actions.get(&id).copied()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_shortcuts_parse() {
        let settings = Settings::default();
        assert!(parse(&settings.spotlight_hotkey).is_some());
        assert!(parse(&settings.translation_hotkey).is_some());
        assert!(parse("ctrl+shift+t").is_some());
    }

    #[test]
    fn empty_and_nonsense_mean_no_shortcut() {
        assert!(parse("").is_none());
        assert!(parse("   ").is_none());
        assert!(parse("Banana+Q").is_none());
    }
}
