//! The second tab: the file shelf on the left, the clipboard history on the right.

use super::kit::{self, rounded, text_clipped};
use super::Notch;
use crate::i18n::{t, tf};
use crate::icons;
use crate::platform;
use crate::services::clipboard::{ClipEntry, ClipKind, Store, parse_color};
use crate::services::sounds::Tone;
use crate::settings::Settings;
use crate::theme;
use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, Ui, pos2, vec2};

#[derive(Default)]
pub struct ShelfUi {
    search: String,
    filter: Option<ClipKind>,
    scroll: f32,
    lasso_start: Option<egui::Pos2>,
    confirm_clear: bool,
}

const FILTERS: [(Option<ClipKind>, &str); 6] = [
    (None, icons::LIST),
    (Some(ClipKind::Text), icons::FILE_TEXT),
    (Some(ClipKind::Link), icons::LINK),
    (Some(ClipKind::Code), icons::CODE),
    (Some(ClipKind::Image), icons::IMAGE),
    (Some(ClipKind::Files), icons::FOLDER),
];

impl Notch {
    pub(super) fn shelf_view(&mut self, ui: &mut Ui, body: Rect, settings: &Settings, frame: &eframe::Frame) {
        let split = body.left() + 300.0;
        let left = Rect::from_min_max(body.min, pos2(split - 6.0, body.bottom()));
        let right = Rect::from_min_max(pos2(split + 6.0, body.top()), body.max);
        self.files_panel(ui, left, frame);
        self.clipboard_panel(ui, right, settings, frame);
    }

    fn files_panel(&mut self, ui: &mut Ui, area: Rect, frame: &eframe::Frame) {
        let (hovering_files, ctrl, delete, select_all) = ui.input(|i| {
            (!i.raw.hovered_files.is_empty(), i.modifiers.command, i.key_pressed(egui::Key::Delete), i.modifiers.command && i.key_pressed(egui::Key::A))
        });
        let border = if hovering_files { theme::ACCENT } else { theme::LINE };
        ui.painter().rect_stroke(area, rounded(14.0), Stroke::new(1.2, border), egui::StrokeKind::Inside);

        let files = self.hub.shelf.lock().map(|s| s.files.clone()).unwrap_or_default();
        let count_rect = Rect::from_min_size(pos2(area.right() - 40.0, area.top() + 8.0), vec2(30.0, 18.0));
        ui.painter().rect_stroke(count_rect, rounded(9.0), Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
        ui.painter().text(count_rect.center(), Align2::CENTER_CENTER, files.len().to_string(), theme::medium(10.5), theme::DIM);

        if files.is_empty() {
            ui.painter().text(area.center() - vec2(0.0, 16.0), Align2::CENTER_CENTER, icons::TRAY, theme::icon(32.0), theme::FAINT);
            ui.painter().text(area.center() + vec2(0.0, 16.0), Align2::CENTER_CENTER, t("port.shelf.drop"), theme::regular(12.0), theme::FAINT);
            return;
        }

        let inner = area.shrink(10.0);
        let cell = vec2(64.0, 74.0);
        let columns = ((inner.width() + 6.0) / (cell.x + 6.0)).floor().max(1.0) as usize;
        let lasso = self.shelf_lasso(ui, area);
        for (index, file) in files.iter().enumerate() {
            let (row, column) = (index / columns, index % columns);
            let rect = Rect::from_min_size(
                pos2(inner.left() + column as f32 * (cell.x + 6.0), inner.top() + 16.0 + row as f32 * (cell.y + 6.0)),
                cell,
            );
            if rect.top() > area.bottom() {
                break;
            }
            let response = ui.interact(rect, ui.id().with(("file", index)), Sense::click_and_drag());
            if let Some(lasso) = lasso
                && lasso.intersects(rect)
                && let Ok(mut shelf) = self.hub.shelf.lock()
                && let Some(entry) = shelf.files.get_mut(index)
            {
                entry.selected = true;
            }
            let fill = if file.selected {
                theme::alpha(theme::ACCENT, 0.35)
            } else if response.hovered() {
                theme::CARD_HOVER
            } else {
                theme::CARD
            };
            ui.painter().rect_filled(rect, rounded(10.0), fill);
            let thumb = Rect::from_center_size(rect.center_top() + vec2(0.0, 28.0), vec2(40.0, 40.0));
            let texture = if file.is_image() {
                self.textures.image_file(ui.ctx(), &file.path)
            } else {
                self.textures.app_icon(ui.ctx(), &file.path)
            };
            match texture {
                Some(texture) => {
                    ui.painter().add(
                        egui::epaint::RectShape::filled(thumb, rounded(6.0), Color32::WHITE)
                            .with_texture(texture.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
                    );
                }
                None => {
                    ui.painter().text(thumb.center(), Align2::CENTER_CENTER, icons::FILE, theme::icon(28.0), theme::DIM);
                }
            }
            text_clipped(ui, rect.center_bottom() - vec2(0.0, 11.0), Align2::CENTER_CENTER, &file.name(), theme::regular(10.0), theme::DIM, cell.x - 6.0);

            if response.clicked() && let Ok(mut shelf) = self.hub.shelf.lock() {
                if ctrl {
                    shelf.toggle(index);
                } else {
                    shelf.select_only(index);
                }
            }
            if response.double_clicked() {
                let _ = open::that_detached(&file.path);
            }
            if response.secondary_clicked() {
                platform::reveal_path(&file.path);
            }
            if response.drag_started() {
                let paths = self.hub.shelf.lock().map(|s| s.drag_paths(index)).unwrap_or_default();
                if !paths.is_empty() {
                    if platform::start_file_drag(frame, paths) {
                        self.hub.play(Tone::Tap);
                    } else {
                        // No drag source on this OS: show the file where it lives instead.
                        platform::reveal_path(&file.path);
                    }
                }
            }
        }

        let selected = self.hub.shelf.lock().map_or(0, |s| s.selected_count());
        if delete && selected > 0 && let Ok(mut shelf) = self.hub.shelf.lock() {
            shelf.remove_selected();
        }
        if select_all && let Ok(mut shelf) = self.hub.shelf.lock() {
            shelf.select_all(true);
        }
        if selected > 0 {
            let button = Rect::from_center_size(pos2(area.right() - 56.0, area.top() + 17.0), vec2(22.0, 22.0));
            let response = kit::icon_button(ui, button, icons::TRASH, 13.0, theme::RED, "remove-selected");
            if response.on_hover_text(t("clipboard.delete")).clicked() && let Ok(mut shelf) = self.hub.shelf.lock() {
                shelf.remove_selected();
            }
        }
    }

    /// Dragging across empty shelf space draws a selection rectangle.
    fn shelf_lasso(&mut self, ui: &mut Ui, area: Rect) -> Option<Rect> {
        let (pressed, down, position) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.interact_pos()));
        if pressed
            && let Some(position) = position
            && area.contains(position)
            && ui.ctx().layer_id_at(position).is_some()
            && !ui.ctx().is_being_dragged(ui.id().with("none"))
        {
            let empty = self.hub.shelf.lock().is_ok_and(|shelf| {
                let inner = area.shrink(10.0);
                let columns = ((inner.width() + 6.0) / 70.0).floor().max(1.0) as usize;
                !(0..shelf.files.len()).any(|index| {
                    let rect = Rect::from_min_size(
                        pos2(inner.left() + (index % columns) as f32 * 70.0, inner.top() + 16.0 + (index / columns) as f32 * 80.0),
                        vec2(64.0, 74.0),
                    );
                    rect.contains(position)
                })
            });
            if empty {
                self.shelf_ui.lasso_start = Some(position);
                if let Ok(mut shelf) = self.hub.shelf.lock() {
                    shelf.select_all(false);
                }
            }
        }
        if !down {
            self.shelf_ui.lasso_start = None;
            return None;
        }
        let start = self.shelf_ui.lasso_start?;
        let rect = Rect::from_two_pos(start, position?);
        ui.painter().rect_filled(rect, rounded(4.0), theme::alpha(theme::ACCENT, 0.2));
        ui.painter().rect_stroke(rect, rounded(4.0), Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
        Some(rect)
    }

    fn clipboard_panel(&mut self, ui: &mut Ui, area: Rect, settings: &Settings, frame: &eframe::Frame) {
        // Header: filters, search, clear
        let header = Rect::from_min_size(area.min, vec2(area.width(), 26.0));
        let mut x = header.left() + 13.0;
        for (index, (filter, icon)) in FILTERS.iter().enumerate() {
            let rect = Rect::from_center_size(pos2(x, header.center().y), vec2(24.0, 24.0));
            let selected = self.shelf_ui.filter == *filter;
            let color = if selected { theme::WHITE } else { theme::FAINT };
            if kit::icon_button(ui, rect, icon, 14.0, color, ("clip-filter", index)).clicked() {
                self.shelf_ui.filter = *filter;
                self.shelf_ui.scroll = 0.0;
            }
            x += 25.0;
        }
        let search_rect = Rect::from_min_max(pos2(x + 2.0, header.top() + 2.0), pos2(header.right() - 30.0, header.bottom() - 2.0));
        ui.painter().rect_filled(search_rect, rounded(11.0), theme::CARD);
        let edit = egui::TextEdit::singleline(&mut self.shelf_ui.search)
            .hint_text(t("clipboard.search"))
            .frame(egui::Frame::NONE)
            .font(theme::regular(12.0))
            .margin(egui::vec2(8.0, 3.0));
        let response = ui.put(search_rect, edit);
        if response.clicked() {
            // The notch normally never takes focus: typing is the one reason to ask for it.
            platform::force_focus(frame);
            response.request_focus();
        }
        if response.changed() {
            self.shelf_ui.scroll = 0.0;
        }
        let clear_rect = Rect::from_center_size(pos2(header.right() - 12.0, header.center().y), vec2(24.0, 24.0));
        let clear_color = if self.shelf_ui.confirm_clear { theme::RED } else { theme::FAINT };
        let clear = kit::icon_button(ui, clear_rect, icons::TRASH, 14.0, clear_color, "clip-clear").on_hover_text(t("clipboard.emptyTray"));
        if clear.clicked() {
            if self.shelf_ui.confirm_clear {
                if let Ok(mut store) = self.hub.clipboard.store.lock() {
                    store.clear();
                }
                self.shelf_ui.confirm_clear = false;
            } else {
                self.shelf_ui.confirm_clear = true;
            }
        }
        if !clear.hovered() && !clear_rect.expand(40.0).contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default()) {
            self.shelf_ui.confirm_clear = false;
        }

        // Entries
        let list = Rect::from_min_max(pos2(area.left(), header.bottom() + 6.0), area.max);
        let entries: Vec<ClipEntry> = self
            .hub
            .clipboard
            .store
            .lock()
            .map(|store| {
                let mut hits: Vec<ClipEntry> = store
                    .search(&self.shelf_ui.search)
                    .into_iter()
                    .filter(|entry| self.shelf_ui.filter.is_none_or(|kind| entry.kind == kind))
                    .cloned()
                    .collect();
                hits.sort_by_key(|entry| !entry.pinned);
                hits
            })
            .unwrap_or_default();

        if entries.is_empty() {
            ui.painter().text(list.center(), Align2::CENTER_CENTER, t("clipboard.empty"), theme::regular(12.0), theme::FAINT);
            return;
        }

        const ROW: f32 = 44.0;
        const GAP: f32 = 4.0;
        let content = entries.len() as f32 * (ROW + GAP);
        let hovered = ui.rect_contains_pointer(list);
        if hovered {
            self.shelf_ui.scroll -= ui.input(|i| i.smooth_scroll_delta.y);
        }
        self.shelf_ui.scroll = self.shelf_ui.scroll.clamp(0.0, (content - list.height()).max(0.0));
        let clip = list.intersect(ui.clip_rect());
        let old_clip = ui.clip_rect();
        ui.set_clip_rect(clip);
        let first = (self.shelf_ui.scroll / (ROW + GAP)).floor() as usize;
        for (index, entry) in entries.iter().enumerate().skip(first) {
            let y = list.top() + index as f32 * (ROW + GAP) - self.shelf_ui.scroll;
            if y > list.bottom() {
                break;
            }
            let rect = Rect::from_min_size(pos2(list.left(), y), vec2(list.width(), ROW));
            self.clip_row(ui, rect, entry, settings);
        }
        ui.set_clip_rect(old_clip);
    }

    fn clip_row(&mut self, ui: &mut Ui, rect: Rect, entry: &ClipEntry, _settings: &Settings) {
        let response = ui.interact(rect, ui.id().with(("clip", entry.id)), Sense::click());
        let fill = if response.hovered() { theme::CARD_HOVER } else { theme::CARD };
        ui.painter().rect_filled(rect, rounded(10.0), fill);

        let icon_center = rect.left_center() + vec2(20.0, 0.0);
        let color = if entry.kind == ClipKind::Color { parse_color(&entry.text) } else { None };
        if let Some(color) = color {
            let swatch = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
            ui.painter().circle_filled(icon_center, 9.0, swatch);
            ui.painter().circle_stroke(icon_center, 9.0, Stroke::new(1.0, theme::LINE));
        } else if entry.kind == ClipKind::Image
            && let Some(file) = &entry.image_file
            && let Some(texture) = self.textures.image_file(ui.ctx(), &Store::image_path(file))
        {
            let thumb = Rect::from_center_size(icon_center, vec2(30.0, 30.0));
            ui.painter().add(
                egui::epaint::RectShape::filled(thumb, rounded(6.0), Color32::WHITE)
                    .with_texture(texture.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
            );
        } else {
            let icon = if entry.sensitive { icons::LOCK_KEY } else { entry.kind.icon() };
            let tint = if entry.sensitive { theme::ORANGE } else { theme::DIM };
            ui.painter().text(icon_center, Align2::CENTER_CENTER, icon, theme::icon(16.0), tint);
        }

        let text_left = rect.left() + 40.0;
        let actions_width = if response.hovered() { 86.0 } else { 8.0 };
        let width = rect.right() - actions_width - text_left;
        text_clipped(ui, kit_pos(text_left, rect.center().y - 7.0), Align2::LEFT_CENTER, &entry.preview(), theme::regular(12.5), theme::WHITE, width);
        let age = format_age(crate::services::clipboard::unix_now().saturating_sub(entry.created_unix));
        let meta = if entry.pinned { format!("{age} · {}", t("clipboard.pin")) } else { age };
        text_clipped(ui, kit_pos(text_left, rect.center().y + 9.0), Align2::LEFT_CENTER, &meta, theme::regular(10.5), theme::FAINT, width);

        if response.hovered() {
            let y = rect.center().y;
            let buttons = [
                (rect.right() - 16.0, icons::TRASH, theme::DIM, "clipboard.delete", 0),
                (rect.right() - 42.0, if entry.pinned { icons::PUSH_PIN_SLASH } else { icons::PUSH_PIN }, theme::DIM, "clipboard.pin", 1),
                (rect.right() - 68.0, icons::COPY, theme::DIM, "clipboard.copy", 2),
            ];
            let mut acted = false;
            for (x, icon, tint, tip, action) in buttons {
                let button = Rect::from_center_size(pos2(x, y), vec2(24.0, 24.0));
                if kit::icon_button(ui, button, icon, 13.0, tint, ("clip-action", entry.id, action)).on_hover_text(t(tip)).clicked() {
                    acted = true;
                    match action {
                        0 => {
                            if let Ok(mut store) = self.hub.clipboard.store.lock() {
                                store.delete(entry.id);
                            }
                        }
                        1 => {
                            if let Ok(mut store) = self.hub.clipboard.store.lock() {
                                store.toggle_pin(entry.id);
                            }
                        }
                        _ => self.hub.clipboard.copy_back(entry),
                    }
                }
            }
            if !acted && response.clicked() {
                self.hub.clipboard.copy_back(entry);
                self.hub.play(Tone::Tap);
            }
        }
    }
}

fn kit_pos(x: f32, y: f32) -> egui::Pos2 {
    pos2(x, y)
}

/// "gerade eben", "5 Min.", "2 Std.", "3 Tage"
pub fn format_age(seconds: u64) -> String {
    if seconds < 60 {
        t("port.age.now")
    } else if seconds < 3600 {
        tf("port.age.minutes", &[&(seconds / 60).to_string()])
    } else if seconds < 86_400 {
        tf("port.age.hours", &[&(seconds / 3600).to_string()])
    } else {
        tf("port.age.days", &[&(seconds / 86_400).to_string()])
    }
}
