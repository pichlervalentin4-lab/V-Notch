//! The tray icon and its menu. `tray-icon` on Windows and macOS, StatusNotifierItem on Linux.

use crate::i18n::t;
use eframe::egui;
use std::sync::mpsc::{self, Receiver};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    ToggleVisible,
    Settings,
    Spotlight,
    ResetPosition,
    Restart,
    Exit,
}

const ITEMS: [(TrayAction, &str); 6] = [
    (TrayAction::ToggleVisible, "tray.hide"),
    (TrayAction::Spotlight, "port.tray.spotlight"),
    (TrayAction::Settings, "tray.settings"),
    (TrayAction::ResetPosition, "tray.reset"),
    (TrayAction::Restart, "tray.restart"),
    (TrayAction::Exit, "tray.exit"),
];

fn logo_rgba() -> Option<(Vec<u8>, u32, u32)> {
    let image = image::load_from_memory(include_bytes!("../../assets/logo.png")).ok()?;
    let small = image.resize_exact(32, 32, image::imageops::FilterType::Triangle).to_rgba8();
    let (width, height) = small.dimensions();
    Some((small.into_raw(), width, height))
}

pub struct Tray {
    pub actions: Receiver<TrayAction>,
    #[cfg(not(target_os = "linux"))]
    _icon: tray_icon::TrayIcon,
}

#[cfg(not(target_os = "linux"))]
impl Tray {
    /// Must run on the main thread, once the event loop is running.
    pub fn start(ctx: &egui::Context) -> Option<Self> {
        use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
        let menu = Menu::new();
        let mut ids = Vec::new();
        for (action, key) in ITEMS {
            if action == TrayAction::Exit {
                menu.append(&PredefinedMenuItem::separator()).ok()?;
            }
            let item = MenuItem::new(t(key), true, None);
            ids.push((item.id().clone(), action));
            menu.append(&item).ok()?;
        }
        let (sender, actions) = mpsc::channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some((_, action)) = ids.iter().find(|(id, _)| *id == event.id) {
                let _ = sender.send(*action);
                ctx.request_repaint();
            }
        }));
        let (rgba, width, height) = logo_rgba()?;
        let icon = tray_icon::Icon::from_rgba(rgba, width, height).ok()?;
        let tray = tray_icon::TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("V-Notch")
            .with_icon(icon)
            .build();
        match tray {
            Ok(icon) => Some(Self { actions, _icon: icon }),
            Err(error) => {
                crate::vlog!("TRAY", "{error}");
                None
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod linux_tray {
    use super::*;
    use ksni::blocking::TrayMethods;
    use std::sync::mpsc::Sender;

    pub struct NotchTray {
        pub sender: Sender<TrayAction>,
        pub ctx: egui::Context,
    }

    impl ksni::Tray for NotchTray {
        fn id(&self) -> String {
            "v-notch".into()
        }

        fn title(&self) -> String {
            "V-Notch".into()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            let Some((rgba, width, height)) = logo_rgba() else { return Vec::new() };
            // StatusNotifierItem wants ARGB in network byte order.
            let data = rgba.chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
            vec![ksni::Icon { width: width as i32, height: height as i32, data }]
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            ITEMS
                .iter()
                .map(|(action, key)| {
                    let action = *action;
                    ksni::menu::StandardItem {
                        label: t(key),
                        activate: Box::new(move |tray: &mut Self| {
                            let _ = tray.sender.send(action);
                            tray.ctx.request_repaint();
                        }),
                        ..Default::default()
                    }
                    .into()
                })
                .collect()
        }
    }

    pub fn spawn(sender: Sender<TrayAction>, ctx: egui::Context) -> bool {
        match (NotchTray { sender, ctx }).assume_sni_available(true).spawn() {
            Ok(handle) => {
                // The service lives as long as its handle; the tray stays for the whole run.
                std::mem::forget(handle);
                true
            }
            Err(error) => {
                crate::vlog!("TRAY", "{error}");
                false
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Tray {
    pub fn start(ctx: &egui::Context) -> Option<Self> {
        let (sender, actions) = mpsc::channel();
        linux_tray::spawn(sender, ctx.clone()).then_some(Self { actions })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logo_loads_for_the_tray() {
        let (rgba, width, height) = logo_rgba().unwrap();
        assert_eq!(rgba.len(), (width * height * 4) as usize);
    }

    #[test]
    fn exit_is_always_in_the_menu() {
        assert!(ITEMS.iter().any(|(action, _)| *action == TrayAction::Exit));
    }
}
