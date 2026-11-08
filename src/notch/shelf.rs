//! The second tab: the clipboard tray of the original (ClipboardTray.xaml). A search field sits in
//! the tab bar, a row of category chips with counts below it, and one strip of 210 px cards for
//! copied items and the files dropped on the notch. With nothing in it, a dashed drop area shows.

use super::Notch;
use super::kit::{self, rounded, text_clipped};
use crate::i18n::{t, tf};
use crate::icons;
use crate::platform;
use crate::services::clipboard::{ClipEntry, ClipKind, Store, parse_color};
use crate::services::shelf::ShelfFile;
use crate::services::sounds::Tone;
use crate::settings::Settings;
use crate::theme;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use std::collections::HashSet;
use std::time::{Duration, Instant};

// Rows of the tray grid (34 / 42 / rest) and the margins around them, from ClipboardTray.xaml.
const ROW_SEARCH: f32 = 34.0;
const ROW_CHIPS: f32 = 42.0;
const SIDE: f32 = 12.0;
const CHIP_H: f32 = 26.0;
const CHIP_GAP: f32 = 6.0;
const CARD_W: f32 = 210.0;
const CARD_GAP: f32 = 10.0;
const FOOTER_H: f32 = 48.0;
const SEARCH_MAX_W: f32 = 360.0;
const HEADER_BUTTON: f32 = 28.0;
/// Distance between header buttons: 28 px plus a 2 px margin on each side.
const HEADER_STEP: f32 = 32.0;

/// Chip text: the original asks for #96FFFFFF, but its bold SF Pro reads as about 85 % white.
const MUTED_TEXT: Color32 = Color32::from_rgba_premultiplied(0xDC, 0xDC, 0xDC, 0xDC);
const DESTRUCTIVE: Color32 = Color32::from_rgb(0xFF, 0x45, 0x3A);
const CARD_TOP: Color32 = Color32::from_rgb(40, 40, 40);
const CARD_BOTTOM: Color32 = Color32::from_rgb(20, 20, 20);

/// The chips, in the original's order. Label groups match items tagged with that label.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
enum Category {
    #[default]
    All,
    Pinned,
    Links,
    Text,
    Code,
    Images,
    Colors,
    Files,
    Group(&'static str),
}

const CATEGORIES: [Category; 14] = [
    Category::All,
    Category::Pinned,
    Category::Links,
    Category::Text,
    Category::Code,
    Category::Images,
    Category::Colors,
    Category::Files,
    Category::Group("Work"),
    Category::Group("Ideas"),
    Category::Group("Email"),
    Category::Group("Shopping"),
    Category::Group("Receipts"),
    Category::Group("Important"),
];

impl Category {
    fn label(self) -> String {
        t(match self {
            Category::All => "clipboard.category.all",
            Category::Pinned => "clipboard.category.pin",
            Category::Links => "clipboard.category.links",
            Category::Text => "clipboard.category.text",
            Category::Code => "clipboard.category.code",
            Category::Images => "clipboard.category.images",
            Category::Colors => "clipboard.category.colors",
            Category::Files => "clipboard.category.files",
            Category::Group("Work") => "clipboard.category.work",
            Category::Group("Ideas") => "clipboard.category.ideas",
            Category::Group("Email") => "clipboard.category.email",
            Category::Group("Shopping") => "clipboard.category.shopping",
            Category::Group("Receipts") => "clipboard.category.receipts",
            Category::Group(_) => "clipboard.category.important",
        })
    }

    fn icon(self) -> &'static str {
        match self {
            Category::All => icons::SQUARES_FOUR,
            Category::Pinned => icons::PUSH_PIN,
            Category::Links => icons::LINK,
            Category::Text => icons::LIST,
            Category::Code => icons::CODE,
            Category::Images => icons::IMAGE,
            Category::Colors => icons::PALETTE,
            Category::Files => icons::FILE,
            Category::Group("Work") => icons::BRIEFCASE,
            Category::Group("Ideas") => icons::LIGHTBULB,
            Category::Group("Email") => icons::ENVELOPE,
            Category::Group("Shopping") => icons::SHOPPING_CART,
            Category::Group("Receipts") => icons::FILE_TEXT,
            Category::Group(_) => icons::STAR,
        }
    }

    fn matches_clip(self, entry: &ClipEntry) -> bool {
        match self {
            Category::All => true,
            Category::Pinned => entry.pinned,
            Category::Links => entry.kind == ClipKind::Link,
            Category::Text => matches!(entry.kind, ClipKind::Text | ClipKind::Email),
            Category::Code => entry.kind == ClipKind::Code,
            Category::Images => entry.kind == ClipKind::Image,
            Category::Colors => entry.kind == ClipKind::Color,
            Category::Files => entry.kind == ClipKind::Files,
            Category::Group(name) => {
                entry.labels.iter().any(|label| label.eq_ignore_ascii_case(name)) || (name == "Email" && entry.kind == ClipKind::Email)
            }
        }
    }

    /// Files dropped on the notch count as pinned files.
    fn matches_file(self, file: &ShelfFile) -> bool {
        match self {
            Category::All | Category::Pinned | Category::Files => true,
            Category::Images => file.is_image(),
            _ => false,
        }
    }
}

/// One card in the strip: a file on the shelf or a clipboard entry.
enum Card {
    File(usize, ShelfFile),
    Clip(ClipEntry),
}

#[derive(Default)]
pub struct ShelfUi {
    search: String,
    category: Category,
    chips_scroll: f32,
    cards_scroll: f32,
    selected: HashSet<u64>,
    confirm_clear: bool,
    copied: Option<(u64, Instant)>,
}

impl Notch {
    pub(super) fn shelf_view(&mut self, ui: &mut Ui, body: Rect, settings: &Settings, frame: &eframe::Frame) {
        let (ctrl, delete, select_all, hovering_files) =
            ui.input(|i| (i.modifiers.command, i.key_pressed(egui::Key::Delete), i.modifiers.command && i.key_pressed(egui::Key::A), !i.raw.hovered_files.is_empty()));
        let files = self.hub.shelf.lock().map(|s| s.files.clone()).unwrap_or_default();
        let entries: Vec<ClipEntry> = self
            .hub
            .clipboard
            .store
            .lock()
            .map(|store| {
                let mut hits: Vec<ClipEntry> = store.search(&self.shelf_ui.search).into_iter().cloned().collect();
                hits.sort_by_key(|entry| !entry.pinned);
                hits
            })
            .unwrap_or_default();
        let query = self.shelf_ui.search.to_lowercase();
        let files: Vec<(usize, ShelfFile)> = files.into_iter().enumerate().filter(|(_, f)| query.is_empty() || f.name().to_lowercase().contains(&query)).collect();

        let tabs = self.visible_tabs(settings).len();
        let row = Rect::from_min_max(pos2(body.left() + SIDE, body.top()), pos2(body.right() - SIDE, body.top() + ROW_SEARCH));
        let nav_end = body.left() - 2.0 + super::NAV_SLOT * tabs as f32;
        let visible: Vec<Card> = files
            .iter()
            .filter(|(_, f)| self.shelf_ui.category.matches_file(f))
            .map(|(i, f)| Card::File(*i, f.clone()))
            .chain(entries.iter().filter(|e| self.shelf_ui.category.matches_clip(e)).cloned().map(Card::Clip))
            .collect();
        self.shelf_header(ui, row, nav_end, &visible, frame);

        let chips = Rect::from_min_max(pos2(body.left() + SIDE, row.bottom() + 3.0), pos2(body.right() - SIDE, row.bottom() + ROW_CHIPS - 5.0));
        self.category_chips(ui, chips, &entries, &files);

        let viewport = Rect::from_min_max(pos2(body.left(), body.top() + ROW_SEARCH + ROW_CHIPS), body.max);
        if visible.is_empty() || hovering_files {
            let drop = Rect::from_min_max(viewport.min + vec2(2.0, 2.0), viewport.max - vec2(2.0, 4.0));
            let color = if hovering_files { Color32::WHITE } else { Color32::from_white_alpha(0x75) };
            dashed_rounded_rect(ui, drop, 14.0, Stroke::new(2.0, color), 5.0, 4.0);
        }
        if visible.is_empty() {
            empty_state(ui, viewport);
            self.shelf_ui.cards_scroll = 0.0;
        } else {
            self.cards(ui, viewport, &visible, ctrl, frame);
        }

        if delete && ui.memory(|m| m.focused().is_none()) {
            self.delete_selected();
        }
        if select_all && ui.memory(|m| m.focused().is_none()) {
            self.select_visible(&visible);
        }
        if let Some((_, at)) = self.shelf_ui.copied
            && at.elapsed() > Duration::from_millis(1100)
        {
            self.shelf_ui.copied = None;
        }
    }

    fn selection_count(&self) -> usize {
        self.shelf_ui.selected.len() + self.hub.shelf.lock().map_or(0, |s| s.selected_count())
    }

    fn delete_selected(&mut self) {
        if let Ok(mut store) = self.hub.clipboard.store.lock() {
            for id in self.shelf_ui.selected.drain() {
                store.delete(id);
            }
        }
        if let Ok(mut shelf) = self.hub.shelf.lock() {
            shelf.remove_selected();
        }
    }

    fn select_visible(&mut self, visible: &[Card]) {
        let mut shelf = self.hub.shelf.lock().ok();
        for card in visible {
            match card {
                Card::Clip(entry) => {
                    self.shelf_ui.selected.insert(entry.id);
                }
                Card::File(index, _) => {
                    if let Some(entry) = shelf.as_mut().and_then(|s| s.files.get_mut(*index)) {
                        entry.selected = true;
                    }
                }
            }
        }
    }

    /// Search field next to the tab icons, and select-all / delete on the right.
    fn shelf_header(&mut self, ui: &mut Ui, row: Rect, nav_end: f32, visible: &[Card], frame: &eframe::Frame) {
        // The settings gear sits in the last header slot (see `paint_nav`); the buttons come before it.
        let trash_x = row.right() - 14.0 - HEADER_STEP - 16.0 + 2.0;
        let grid_x = trash_x - HEADER_STEP;
        let y = row.top() + 14.0;

        let column = (nav_end + 8.0, grid_x - HEADER_BUTTON / 2.0 - 10.0);
        let width = (column.1 - column.0).clamp(120.0, SEARCH_MAX_W);
        let center = (column.0 + column.1) / 2.0;
        let search = Rect::from_center_size(pos2(center, y + 1.0), vec2(width, 30.0));
        let response = ui.interact(search, ui.id().with("shelf-search-surface"), Sense::click());
        let edit_id = ui.id().with("shelf-search");
        let focused = ui.memory(|m| m.has_focus(edit_id));
        let (fill, border) = if focused {
            (0x2A, 0x88)
        } else if response.hovered() {
            (0x26, 0x54)
        } else {
            (0x1E, 0x38)
        };
        ui.painter().rect(search, rounded(15.0), Color32::from_white_alpha(fill), Stroke::new(1.0, Color32::from_white_alpha(border)), egui::StrokeKind::Inside);
        ui.painter().text(pos2(search.left() + 19.5, search.center().y), Align2::CENTER_CENTER, icons::MAGNIFYING_GLASS, theme::icon_fill(13.5), Color32::from_white_alpha(0xC8));
        let text_rect = Rect::from_min_max(pos2(search.left() + 33.0, search.top()), pos2(search.right() - 32.0, search.bottom()));
        if self.shelf_ui.search.is_empty() {
            ui.painter().text(pos2(text_rect.left(), search.center().y), Align2::LEFT_CENTER, t("clipboard.search"), theme::bold(12.5), Color32::from_white_alpha(0xB4));
        }
        let edit = egui::TextEdit::singleline(&mut self.shelf_ui.search)
            .id(edit_id)
            .frame(egui::Frame::NONE)
            .font(theme::bold(12.5))
            .text_color(Color32::WHITE)
            .vertical_align(egui::Align::Center)
            .margin(egui::vec2(0.0, 0.0));
        let edit_response = ui.put(text_rect, edit);
        if response.clicked() || edit_response.clicked() {
            // The notch normally never takes focus: typing is the one reason to ask for it.
            platform::force_focus(frame);
            edit_response.request_focus();
        }
        if edit_response.changed() {
            self.shelf_ui.cards_scroll = 0.0;
        }
        if !self.shelf_ui.search.is_empty() {
            let clear = Rect::from_center_size(pos2(search.right() - 18.0, search.center().y), vec2(22.0, 22.0));
            if kit::icon_button(ui, clear, icons::X, 12.0, Color32::from_white_alpha(0xB4), "shelf-search-clear").clicked() {
                self.shelf_ui.search.clear();
            }
        }

        let grid = Rect::from_center_size(pos2(grid_x, y), vec2(HEADER_BUTTON, HEADER_BUTTON));
        if header_button(ui, grid, icons::SQUARES_FOUR, None, "shelf-select-all").on_hover_text(t("clipboard.selectAll")).clicked() {
            self.select_visible(visible);
            self.hub.play(Tone::Tap);
        }
        let trash = Rect::from_center_size(pos2(trash_x, y), vec2(HEADER_BUTTON, HEADER_BUTTON));
        let selected = self.selection_count();
        let tip = if selected > 0 { tf("clipboard.deleteSelected", &[&selected.to_string()]) } else { t("clipboard.emptyTray") };
        if self.shelf_ui.confirm_clear {
            ui.painter().rect_filled(trash, rounded(14.0), theme::alpha(DESTRUCTIVE, 0.22));
        }
        let response = header_button(ui, trash, icons::TRASH, Some(DESTRUCTIVE), "shelf-trash").on_hover_text(tip);
        if response.clicked() {
            if selected > 0 {
                self.delete_selected();
            } else if self.shelf_ui.confirm_clear {
                // Second click: empty the tray (pinned items stay, as in the original).
                if let Ok(mut store) = self.hub.clipboard.store.lock() {
                    let unpinned: Vec<u64> = store.entries.iter().filter(|e| !e.pinned).map(|e| e.id).collect();
                    for id in unpinned {
                        store.delete(id);
                    }
                }
                self.shelf_ui.confirm_clear = false;
            } else {
                self.shelf_ui.confirm_clear = true;
            }
        }
        if !response.hovered() {
            self.shelf_ui.confirm_clear = false;
        }
    }

    /// The scrollable row of category chips: icon, name and count.
    fn category_chips(&mut self, ui: &mut Ui, area: Rect, entries: &[ClipEntry], files: &[(usize, ShelfFile)]) {
        let chip_top = area.top() + 3.0;
        let mut widths = Vec::with_capacity(CATEGORIES.len());
        for category in CATEGORIES {
            let count = entries.iter().filter(|e| category.matches_clip(e)).count() + files.iter().filter(|(_, f)| category.matches_file(f)).count();
            let label = category.label();
            let count_text = count.to_string();
            let width = 12.0 + 13.0 + 6.0 + kit::text_width(ui, &label, theme::bold(12.0)) + 5.0 + kit::text_width(ui, &count_text, theme::bold(11.0)) + 12.0;
            widths.push((category, label, count_text, width));
        }
        let total: f32 = widths.iter().map(|w| w.3 + CHIP_GAP).sum::<f32>() - CHIP_GAP;
        let overflow = (total - area.width()).max(0.0);
        if overflow > 0.0 && ui.rect_contains_pointer(area) {
            let delta = ui.input(|i| i.smooth_scroll_delta);
            self.shelf_ui.chips_scroll -= delta.x + delta.y;
        }
        self.shelf_ui.chips_scroll = self.shelf_ui.chips_scroll.clamp(0.0, overflow);

        let clip = area.expand2(vec2(0.0, 3.0)).intersect(ui.clip_rect());
        let painter = ui.painter().with_clip_rect(clip);
        let mut x = area.left() - self.shelf_ui.chips_scroll;
        for (category, label, count, width) in widths {
            let chip = Rect::from_min_size(pos2(x, chip_top), vec2(width, CHIP_H));
            x += width + CHIP_GAP;
            if chip.right() < area.left() || chip.left() > area.right() {
                continue;
            }
            let response = ui.interact(chip.intersect(clip), ui.id().with(("shelf-chip", category)), Sense::click());
            let selected = self.shelf_ui.category == category;
            let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered() && !selected, 0.15);
            let fill = if selected { Color32::from_white_alpha(0x2E) } else { Color32::from_white_alpha(0x12) };
            painter.rect_filled(chip, rounded(13.0), fill);
            if hover > 0.0 {
                painter.rect_filled(chip, rounded(13.0), Color32::from_white_alpha((0x18 as f32 * hover) as u8));
            }
            let color = if selected {
                Color32::WHITE
            } else {
                kit::lerp_color(MUTED_TEXT, Color32::from_white_alpha(0xE6), hover)
            };
            let y = chip.center().y;
            // Links, text and code are line glyphs in the original; the rest are filled.
            let font = if matches!(category, Category::Links | Category::Text | Category::Code) { theme::icon(13.0) } else { theme::icon_fill(13.0) };
            painter.text(pos2(chip.left() + 12.0 + 6.5, y), Align2::CENTER_CENTER, category.icon(), font, color);
            let label_rect = painter.text(pos2(chip.left() + 12.0 + 13.0 + 6.0, y), Align2::LEFT_CENTER, label, theme::bold(12.0), color);
            painter.text(pos2(label_rect.right() + 5.0, y + 0.5), Align2::LEFT_CENTER, count, theme::bold(11.0), color);
            if response.clicked() && !selected {
                self.shelf_ui.category = category;
                self.shelf_ui.cards_scroll = 0.0;
                self.hub.play(Tone::Tap);
            }
        }
        // Chips run out under a fade at whichever edge has more of them.
        let black = Color32::BLACK;
        if self.shelf_ui.chips_scroll < overflow - 0.5 {
            kit::horizontal_fade(ui, Rect::from_min_max(pos2(area.right() - 26.0, clip.top()), pos2(area.right(), clip.bottom())), Color32::TRANSPARENT, black);
        }
        if self.shelf_ui.chips_scroll > 0.5 {
            kit::horizontal_fade(ui, Rect::from_min_max(pos2(area.left(), clip.top()), pos2(area.left() + 26.0, clip.bottom())), black, Color32::TRANSPARENT);
        }
    }

    /// The horizontal strip of cards. The mouse wheel scrolls it sideways.
    fn cards(&mut self, ui: &mut Ui, viewport: Rect, cards: &[Card], ctrl: bool, frame: &eframe::Frame) {
        let strip = Rect::from_min_max(viewport.min + vec2(8.0, 7.0), viewport.max - vec2(8.0, 12.0));
        let total = cards.len() as f32 * (CARD_W + CARD_GAP) - CARD_GAP;
        let overflow = (total - strip.width()).max(0.0);
        if ui.rect_contains_pointer(viewport) {
            let delta = ui.input(|i| i.smooth_scroll_delta);
            self.shelf_ui.cards_scroll -= delta.x + delta.y;
        }
        self.shelf_ui.cards_scroll = self.shelf_ui.cards_scroll.clamp(0.0, overflow);
        let clip = strip.expand2(vec2(8.0, 4.0)).intersect(ui.clip_rect());
        let old_clip = ui.clip_rect();
        ui.set_clip_rect(clip);
        for (index, card) in cards.iter().enumerate() {
            let left = strip.left() + index as f32 * (CARD_W + CARD_GAP) - self.shelf_ui.cards_scroll;
            let rect = Rect::from_min_size(pos2(left, strip.top()), vec2(CARD_W, strip.height()));
            if rect.right() < clip.left() || rect.left() > clip.right() {
                continue;
            }
            match card {
                Card::Clip(entry) => self.clip_card(ui, rect, entry, ctrl),
                Card::File(index, file) => self.file_card(ui, rect, *index, file, ctrl, frame),
            }
        }
        ui.set_clip_rect(old_clip);
        let black = Color32::BLACK;
        if self.shelf_ui.cards_scroll > 0.5 {
            kit::horizontal_fade(ui, Rect::from_min_max(pos2(viewport.left(), clip.top()), pos2(viewport.left() + 26.0, clip.bottom())), black, Color32::TRANSPARENT);
        }
        if self.shelf_ui.cards_scroll < overflow - 0.5 {
            kit::horizontal_fade(ui, Rect::from_min_max(pos2(viewport.right() - 26.0, clip.top()), pos2(viewport.right(), clip.bottom())), Color32::TRANSPARENT, black);
        }
    }

    fn clip_card(&mut self, ui: &mut Ui, rect: Rect, entry: &ClipEntry, ctrl: bool) {
        let response = ui.interact(rect, ui.id().with(("clip", entry.id)), Sense::click());
        let selected = self.shelf_ui.selected.contains(&entry.id);
        card_surface(ui, rect, response.hovered(), selected);

        let content = Rect::from_min_max(rect.min + vec2(14.0, 13.0), pos2(rect.right() - 14.0, rect.bottom() - FOOTER_H));
        let color = if entry.kind == ClipKind::Color { parse_color(&entry.text) } else { None };
        if let Some(color) = color {
            let swatch = Rect::from_min_max(content.min, pos2(content.right(), content.bottom() - 24.0));
            ui.painter().rect_filled(swatch, rounded(10.0), Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]));
            ui.painter().rect_stroke(swatch, rounded(10.0), Stroke::new(1.0, Color32::from_white_alpha(0x20)), egui::StrokeKind::Inside);
            text_clipped(ui, pos2(content.left(), content.bottom() - 10.0), Align2::LEFT_CENTER, &entry.text, theme::semibold(14.0), theme::WHITE, content.width());
        } else if entry.kind == ClipKind::Image
            && let Some(file) = &entry.image_file
            && let Some(texture) = self.textures.image_file(ui.ctx(), &Store::image_path(file))
        {
            let area = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - FOOTER_H));
            paint_cover(ui, area, texture.id(), texture.size_vec2());
        } else {
            let text = entry.preview();
            let tint = if entry.sensitive { theme::ORANGE } else { theme::WHITE };
            let font = if entry.kind == ClipKind::Code { egui::FontId::monospace(12.5) } else { theme::semibold(14.0) };
            kit::text_wrapped(ui, content.min, &text, font, tint, content.width(), 5);
        }

        let icon = if entry.sensitive { icons::LOCK_KEY } else { entry.kind.icon() };
        let age = format_age(crate::services::clipboard::unix_now().saturating_sub(entry.created_unix));
        let size = match entry.kind {
            ClipKind::Image => String::new(),
            ClipKind::Files => tf("clipboard.fileCount", &[&entry.files.len().to_string()]),
            _ => format!("{} chars", entry.text.chars().count()),
        };
        card_footer(ui, rect, icon, &age, &size);

        let mut acted = false;
        if response.hovered() || ui.rect_contains_pointer(rect) {
            let pin_icon = if entry.pinned { icons::PUSH_PIN_SLASH } else { icons::PUSH_PIN };
            let pin_tip = if entry.pinned { "clipboard.unpin" } else { "clipboard.pin" };
            match card_actions(ui, rect, [(pin_icon, pin_tip), (icons::TRASH, "clipboard.delete")], entry.id) {
                Some(0) => {
                    acted = true;
                    if let Ok(mut store) = self.hub.clipboard.store.lock() {
                        store.toggle_pin(entry.id);
                    }
                }
                Some(_) => {
                    acted = true;
                    if let Ok(mut store) = self.hub.clipboard.store.lock() {
                        store.delete(entry.id);
                    }
                    self.shelf_ui.selected.remove(&entry.id);
                }
                None => {}
            }
        }
        if !acted && response.clicked() {
            if ctrl {
                if !self.shelf_ui.selected.remove(&entry.id) {
                    self.shelf_ui.selected.insert(entry.id);
                }
            } else {
                self.shelf_ui.selected.clear();
                self.hub.clipboard.copy_back(entry);
                self.shelf_ui.copied = Some((entry.id, Instant::now()));
                self.hub.play(Tone::Tap);
            }
        }
        if self.shelf_ui.copied.is_some_and(|(id, _)| id == entry.id) {
            copied_badge(ui, rect);
        }
    }

    fn file_card(&mut self, ui: &mut Ui, rect: Rect, index: usize, file: &ShelfFile, ctrl: bool, frame: &eframe::Frame) {
        let response = ui.interact(rect, ui.id().with(("file", index)), Sense::click_and_drag());
        card_surface(ui, rect, response.hovered(), file.selected);

        if file.is_image()
            && let Some(texture) = self.textures.image_file(ui.ctx(), &file.path)
        {
            let area = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - FOOTER_H));
            paint_cover(ui, area, texture.id(), texture.size_vec2());
        } else {
            // The original's file badge: icon and type tag, the name below.
            let panel = Rect::from_min_max(rect.min + vec2(20.0, 30.0), pos2(rect.right() - 20.0, rect.bottom() - 40.0));
            let tag = file.path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_else(|| "FILE".into());
            let tag_w = kit::text_width(ui, &tag, theme::bold(11.0)) + 12.0;
            let row_w = 24.0 + 7.0 + tag_w;
            let x = panel.center().x - row_w / 2.0;
            let y = panel.top() + 14.0;
            match self.textures.app_icon(ui.ctx(), &file.path) {
                Some(texture) => {
                    let icon = Rect::from_center_size(pos2(x + 12.0, y), vec2(24.0, 24.0));
                    ui.painter().image(texture.id(), icon, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    ui.painter().text(pos2(x + 12.0, y), Align2::CENTER_CENTER, icons::FILE, theme::icon_fill(22.0), theme::DIM);
                }
            }
            let badge = Rect::from_min_size(pos2(x + 31.0, y - 9.0), vec2(tag_w, 18.0));
            ui.painter().rect_filled(badge, rounded(4.0), Color32::from_white_alpha(0x26));
            ui.painter().text(badge.center(), Align2::CENTER_CENTER, &tag, theme::bold(11.0), theme::WHITE);
            let mut job = egui::text::LayoutJob::simple(file.name(), theme::regular(11.0), theme::WHITE, panel.width());
            job.wrap.max_rows = 2;
            job.halign = egui::Align::Center;
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(pos2(panel.center().x, y + 20.0), galley, theme::WHITE);
        }

        let size = std::fs::metadata(&file.path).map(|m| format_size(m.len())).unwrap_or_default();
        card_footer(ui, rect, icons::PUSH_PIN, &t("clipboard.category.files"), &size);

        let mut acted = false;
        if ui.rect_contains_pointer(rect)
            && let Some(_) = card_actions(ui, rect, [(icons::FOLDER_OPEN, "spotlight.reveal"), (icons::TRASH, "clipboard.delete")], index as u64 + (1 << 40)).inspect(|action| {
                if *action == 0 {
                    platform::reveal_path(&file.path);
                } else if let Ok(mut shelf) = self.hub.shelf.lock() {
                    shelf.select_only(index);
                    shelf.remove_selected();
                }
            })
        {
            acted = true;
        }
        if !acted && response.clicked()
            && let Ok(mut shelf) = self.hub.shelf.lock()
        {
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
}

/// A 28 px round header button: grey glyph, white with a light circle on hover.
fn header_button(ui: &mut Ui, rect: Rect, icon: &str, tint: Option<Color32>, id: &str) -> egui::Response {
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, rounded(14.0), Color32::from_white_alpha(0x22));
    }
    let color = tint.unwrap_or(if response.hovered() { Color32::WHITE } else { Color32::from_white_alpha(0x73) });
    let alpha = if response.is_pointer_button_down_on() { 0.6 } else { 1.0 };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, theme::icon_fill(16.0), theme::alpha(color, alpha));
    response
}

/// The card body: a dark vertical gradient with a thin rim, brighter when hovered or selected.
fn card_surface(ui: &Ui, rect: Rect, hovered: bool, selected: bool) {
    kit::vertical_gradient_rect(ui, rect, 16.0, CARD_TOP, CARD_BOTTOM);
    let rim = if selected {
        Stroke::new(2.0, Color32::from_white_alpha(0xE6))
    } else if hovered {
        Stroke::new(2.0, Color32::from_white_alpha(0x40))
    } else {
        Stroke::new(1.2, Color32::from_white_alpha(0x18))
    };
    ui.painter().rect_stroke(rect, rounded(16.0), rim, egui::StrokeKind::Inside);
}

/// The 48 px footer: a 20 px icon, the source line and the size on the right.
fn card_footer(ui: &Ui, rect: Rect, icon: &str, source: &str, size: &str) {
    let y = rect.bottom() - FOOTER_H + 14.0 + 13.0;
    ui.painter().text(pos2(rect.left() + 13.0 + 10.0, y), Align2::CENTER_CENTER, icon, theme::icon_fill(16.0), theme::DIM);
    let size_w = if size.is_empty() { 0.0 } else { kit::text_width(ui, size, theme::medium(11.0)) + 6.0 };
    if !size.is_empty() {
        ui.painter().text(pos2(rect.right() - 13.0, y), Align2::RIGHT_CENTER, size, theme::medium(11.0), theme::DIM);
    }
    let left = rect.left() + 13.0 + 22.0 + 5.0;
    text_clipped(ui, pos2(left, y), Align2::LEFT_CENTER, source, theme::semibold(11.5), theme::WHITE, rect.right() - 13.0 - size_w - left);
}

/// The little box of actions in a hovered card's top right corner. Returns the clicked one.
fn card_actions(ui: &mut Ui, rect: Rect, actions: [(&str, &str); 2], id: u64) -> Option<usize> {
    let size = vec2(2.0 + 26.0 * 2.0 + 2.0, 30.0);
    let frame = Rect::from_min_size(pos2(rect.right() - 7.0 - size.x, rect.top() + 7.0), size);
    ui.painter().rect(frame, rounded(7.0), Color32::from_rgba_unmultiplied(0x18, 0x18, 0x18, 0xE6), Stroke::new(1.0, Color32::from_white_alpha(0x26)), egui::StrokeKind::Inside);
    let mut clicked = None;
    for (index, (icon, tip)) in actions.iter().enumerate() {
        let button = Rect::from_min_size(pos2(frame.left() + 2.0 + 26.0 * index as f32, frame.top() + 2.0), vec2(26.0, 26.0));
        let response = ui.interact(button, ui.id().with(("card-action", id, index)), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(button, rounded(6.0), Color32::from_white_alpha(0x22));
        }
        ui.painter().text(button.center(), Align2::CENTER_CENTER, *icon, theme::icon_fill(14.0), Color32::WHITE);
        if response.on_hover_text(t(tip)).clicked() {
            clicked = Some(index);
        }
    }
    clicked
}

/// "✓ Copied" over a dimmed card.
fn copied_badge(ui: &Ui, rect: Rect) {
    ui.painter().rect_filled(rect, rounded(16.0), Color32::from_black_alpha(0x35));
    let text = t("clipboard.copied");
    let width = kit::text_width(ui, &text, theme::bold(12.0)) + 28.0;
    let badge = Rect::from_center_size(rect.center(), vec2(width, 32.0));
    ui.painter().rect(badge, rounded(12.0), Color32::from_black_alpha(0xE6), Stroke::new(1.0, Color32::from_white_alpha(0x40)), egui::StrokeKind::Inside);
    ui.painter().text(badge.center(), Align2::CENTER_CENTER, text, theme::bold(12.0), Color32::WHITE);
}

/// An image filling `area` like CSS `object-fit: cover`, with the card's rounded top corners.
fn paint_cover(ui: &Ui, area: Rect, texture: egui::TextureId, size: egui::Vec2) {
    let (iw, ih) = (size.x.max(1.0), size.y.max(1.0));
    let scale = (area.width() / iw).max(area.height() / ih);
    let (uw, uh) = (area.width() / (iw * scale), area.height() / (ih * scale));
    let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(uw, uh));
    let corners = egui::CornerRadius { nw: 16, ne: 16, sw: 0, se: 0 };
    ui.painter().add(egui::epaint::RectShape::filled(area, corners, Color32::WHITE).with_texture(texture, uv));
}

/// Download arrow, "Copied items appear here." and the drop hint, centred in the drop area.
fn empty_state(ui: &Ui, viewport: Rect) {
    let center = viewport.center() + vec2(0.0, 3.0);
    paint_download_icon(ui, center - vec2(0.0, 30.0), Color32::from_white_alpha(0xE0));
    ui.painter().text(center + vec2(0.0, 4.0), Align2::CENTER_CENTER, t("clipboard.empty"), theme::bold(14.0), Color32::from_white_alpha(0xF2));
    ui.painter().text(center + vec2(0.0, 28.0), Align2::CENTER_CENTER, t("clipboard.dropHint"), theme::medium(12.0), Color32::from_white_alpha(0xA4));
}

/// The original's import glyph: an arrow down onto a bar, 24 px.
fn paint_download_icon(ui: &Ui, center: Pos2, color: Color32) {
    let stroke = Stroke::new(2.5, color);
    let painter = ui.painter();
    painter.line_segment([center + vec2(0.0, -8.5), center + vec2(0.0, 4.5)], stroke);
    painter.line(vec![center + vec2(-5.0, -0.5), center + vec2(0.0, 4.5), center + vec2(5.0, -0.5)], stroke);
    painter.line_segment([center + vec2(-7.0, 8.0), center + vec2(7.0, 8.0)], stroke);
}

/// A rounded rectangle outlined with dashes, like WPF's `StrokeDashArray="5 4"` (in stroke widths).
fn dashed_rounded_rect(ui: &Ui, rect: Rect, radius: f32, stroke: Stroke, dash: f32, gap: f32) {
    let points = rounded_rect_outline(rect, radius);
    let mut closed = points.clone();
    closed.push(points[0]);
    let shapes = Shape::dashed_line(&closed, stroke, dash * stroke.width, gap * stroke.width);
    ui.painter().extend(shapes);
}

/// Points along the outline of a rounded rectangle, clockwise from the top left corner's end.
fn rounded_rect_outline(rect: Rect, radius: f32) -> Vec<Pos2> {
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (pos2(rect.right() - r, rect.top() + r), -90.0_f32),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), 90.0),
        (pos2(rect.left() + r, rect.top() + r), 180.0),
    ];
    let mut points = Vec::with_capacity(4 * 9);
    for (center, start) in corners {
        for step in 0..=8 {
            let angle = (start + step as f32 * 90.0 / 8.0).to_radians();
            points.push(center + vec2(angle.cos(), angle.sin()) * r);
        }
    }
    points
}

fn format_size(bytes: u64) -> String {
    match bytes {
        0..1_024 => format!("{bytes} B"),
        1_024..1_048_576 => format!("{:.0} KB", bytes as f64 / 1_024.0),
        1_048_576..1_073_741_824 => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
        _ => format!("{:.1} GB", bytes as f64 / 1_073_741_824.0),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: ClipKind, labels: &[&str], pinned: bool) -> ClipEntry {
        ClipEntry { kind, labels: labels.iter().map(|l| l.to_string()).collect(), pinned, ..ClipEntry::default() }
    }

    #[test]
    fn categories_filter_like_the_original() {
        let link = entry(ClipKind::Link, &[], false);
        let work = entry(ClipKind::Text, &["work"], true);
        let mail = entry(ClipKind::Email, &[], false);
        assert!(Category::All.matches_clip(&link));
        assert!(Category::Links.matches_clip(&link) && !Category::Text.matches_clip(&link));
        assert!(Category::Pinned.matches_clip(&work) && !Category::Pinned.matches_clip(&link));
        assert!(Category::Group("Work").matches_clip(&work), "labels match without regard to case");
        assert!(Category::Group("Email").matches_clip(&mail));
        let file = ShelfFile { path: "C:/x/photo.PNG".into(), selected: false };
        assert!(Category::Files.matches_file(&file) && Category::Images.matches_file(&file) && !Category::Links.matches_file(&file));
    }

    #[test]
    fn outline_is_closed_and_inside_the_rect() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 150.0));
        let points = rounded_rect_outline(rect, 14.0);
        assert!(points.iter().all(|p| rect.expand(0.01).contains(*p)));
        assert!((points[0] - pos2(rect.right() - 14.0, rect.top())).length() < 0.01);
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2_048), "2 KB");
        assert_eq!(format_size(5 * 1_048_576), "5.0 MB");
    }
}
