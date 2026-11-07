//! Spotlight: the launcher window with search and AI chat, opened with Alt+Space.

pub mod ai;
pub mod calc;
pub mod search;

use crate::i18n::{t, tf};
use crate::icons;
use crate::notch::Textures;
use crate::notch::kit::{self, rounded, text_clipped};
use crate::services::Hub;
use crate::theme;
use ai::AiService;
use eframe::egui::{self, Align2, Color32, Id, Key, Modifiers, Rect, Sense, Ui, ViewportBuilder, ViewportId, pos2, vec2};
use search::{Action, Hit, Searcher};
use std::sync::Arc;
use std::time::Instant;

pub const WIDTH: f32 = 640.0;
const SEARCH_HEIGHT: f32 = 58.0;
const ROW_HEIGHT: f32 = 48.0;
const MAX_RESULTS: usize = 7;
const AI_HEIGHT: f32 = 460.0;
const MARGIN: f32 = 24.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Search,
    Ai,
}

pub struct Spotlight {
    hub: Arc<Hub>,
    pub open: bool,
    mode: Mode,
    query: String,
    results: Vec<Hit>,
    last_query: Option<String>,
    selected: usize,
    searcher: Arc<Searcher>,
    ai: Arc<AiService>,
    ai_input: String,
    show_history: bool,
    opened_at: Instant,
    was_focused: bool,
    confirm: Option<Hit>,
    textures: Textures,
    pub viewport_id: ViewportId,
    /// Test runs keep the window open even without focus.
    pub keep_open: bool,
}

impl Spotlight {
    pub fn new(hub: Arc<Hub>) -> Self {
        let history = hub.settings.read(|s| s.enable_spotlight_history);
        let searcher = Searcher::start(history, hub.demo);
        let ai = AiService::new(hub.ctx.clone());
        Self {
            hub,
            open: false,
            mode: Mode::Search,
            query: String::new(),
            results: Vec::new(),
            last_query: None,
            selected: 0,
            searcher,
            ai,
            ai_input: String::new(),
            show_history: false,
            opened_at: Instant::now(),
            was_focused: false,
            confirm: None,
            textures: Textures::default(),
            viewport_id: ViewportId::from_hash_of("v-notch-spotlight"),
            keep_open: false,
        }
    }

    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.show();
        }
    }

    pub fn show(&mut self) {
        self.open = true;
        self.opened_at = Instant::now();
        self.was_focused = false;
        self.query.clear();
        self.selected = 0;
        self.last_query = None;
        self.confirm = None;
        self.show_history = false;
        self.mode = if self.hub.settings.read(|s| s.spotlight_default_ai) { Mode::Ai } else { Mode::Search };
    }

    pub fn show_ai_demo(&mut self) {
        self.show();
        self.mode = Mode::Ai;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.ai.stop();
    }

    pub fn clear_history(&self) {
        self.searcher.clear_history();
        self.ai.clear_history();
    }

    fn height(&self) -> f32 {
        match self.mode {
            Mode::Ai => AI_HEIGHT,
            Mode::Search => {
                let rows = self.results.len().min(MAX_RESULTS);
                let footer = if rows > 0 { 30.0 } else { 0.0 };
                SEARCH_HEIGHT + rows as f32 * ROW_HEIGHT + footer + if rows > 0 { 10.0 } else { 0.0 }
            }
        }
    }

    /// Puts text into the search box, as if it had been typed.
    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
    }

    fn refresh_results(&mut self) {
        if self.mode == Mode::Search && self.last_query.as_deref() != Some(self.query.as_str()) {
            self.results = self.searcher.search(&self.query, MAX_RESULTS);
            self.last_query = Some(self.query.clone());
            self.selected = 0;
        }
    }

    /// Size of the window including its shadow margin.
    pub fn outer_size(&self) -> egui::Vec2 {
        vec2(WIDTH + MARGIN * 2.0, self.height() + MARGIN * 2.0)
    }

    /// Draws the window into a ui of the caller. Picture runs use this, because egui
    /// cannot capture an immediate viewport.
    pub fn draw_embedded(&mut self, ui: &mut Ui) {
        self.refresh_results();
        self.window(ui);
    }

    /// Call every frame from the main viewport. Draws the window while it is open.
    pub fn show_viewport(&mut self, ctx: &egui::Context, screen_center_x: f32, top: f32) {
        if !self.open {
            return;
        }
        self.refresh_results();
        let height = self.height() + MARGIN * 2.0;
        let width = WIDTH + MARGIN * 2.0;
        let builder = ViewportBuilder::default()
            .with_title("V-Notch Spotlight")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            .with_inner_size([width, height])
            .with_position([screen_center_x - width / 2.0, top]);
        let id = self.viewport_id;
        ctx.show_viewport_immediate(id, builder, |ui, _class| self.window(ui));
    }

    fn window(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let first_moments = self.opened_at.elapsed().as_secs_f32() < 0.4;
        if first_moments {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.request_repaint();
        }
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
        if focused {
            self.was_focused = true;
        } else if self.was_focused && !self.keep_open {
            self.close();
            return;
        }

        let full = ctx.content_rect();
        let panel = full.shrink(MARGIN);
        let shadow = egui::Shadow { offset: [0, 10], blur: 24, spread: 0, color: theme::alpha(Color32::BLACK, 0.5) };
        ui.painter().add(shadow.as_shape(panel, rounded(18.0)));
        ui.painter().rect_filled(panel, rounded(18.0), Color32::from_rgb(12, 12, 14));

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(panel).id_salt("spotlight-panel"));
        match self.mode {
            Mode::Search => self.search_mode(&mut child, panel),
            Mode::Ai => self.ai_mode(&mut child, panel),
        }

        let (tab, escape) = ctx.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Tab), i.consume_key(Modifiers::NONE, Key::Escape)));
        if tab {
            self.mode = if self.mode == Mode::Search { Mode::Ai } else { Mode::Search };
            self.last_query = None;
            self.opened_at = Instant::now() - std::time::Duration::from_secs(1);
        }
        if escape {
            self.on_escape();
        }
    }

    fn on_escape(&mut self) {
        match self.mode {
            Mode::Search => self.close(),
            Mode::Ai => {
                let streaming = self.ai.state.lock().is_ok_and(|s| s.streaming);
                if streaming {
                    self.ai.stop();
                } else if self.show_history {
                    self.show_history = false;
                } else {
                    self.mode = Mode::Search;
                    self.last_query = None;
                }
            }
        }
    }

    fn search_mode(&mut self, ui: &mut Ui, panel: Rect) {
        let row = Rect::from_min_size(panel.min, vec2(panel.width(), SEARCH_HEIGHT));
        ui.painter().text(row.left_center() + vec2(24.0, 0.0), Align2::CENTER_CENTER, icons::MAGNIFYING_GLASS, theme::icon(18.0), theme::DIM);
        let input_rect = Rect::from_min_max(row.min + vec2(44.0, 12.0), row.max - vec2(90.0, 12.0));
        let response = ui.put(
            input_rect,
            egui::TextEdit::singleline(&mut self.query)
                .hint_text(t("spotlight.placeholder"))
                .frame(egui::Frame::NONE)
                .font(theme::regular(17.0))
                .id(Id::new("spotlight-input")),
        );
        if !response.has_focus() {
            response.request_focus();
        }
        // Mode switch, on the right of the search field
        let chip = Rect::from_min_size(row.right_center() - vec2(78.0, 12.0), vec2(62.0, 24.0));
        if kit::pill_button(ui, chip, "AI", theme::CARD_HOVER, theme::DIM, "to-ai").on_hover_text("Tab").clicked() {
            self.mode = Mode::Ai;
        }

        let rows = self.results.len().min(MAX_RESULTS);
        if rows == 0 {
            return;
        }
        let (down, up, enter) = ui.ctx().input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        if down {
            self.selected = (self.selected + 1) % rows;
        }
        if up {
            self.selected = (self.selected + rows - 1) % rows;
        }
        if self.selected >= rows {
            self.selected = 0;
        }

        ui.painter().line_segment(
            [pos2(panel.left(), row.bottom()), pos2(panel.right(), row.bottom())],
            egui::Stroke::new(1.0, theme::LINE),
        );

        if let Some(hit) = self.confirm.clone() {
            self.confirm_row(ui, panel, row.bottom() + 5.0, &hit);
            return;
        }

        let results = self.results.clone();
        let mut activated = enter.then_some(self.selected);
        for (index, hit) in results.iter().take(rows).enumerate() {
            let rect = Rect::from_min_size(
                pos2(panel.left() + 6.0, row.bottom() + 5.0 + index as f32 * ROW_HEIGHT),
                vec2(panel.width() - 12.0, ROW_HEIGHT),
            );
            let response = ui.interact(rect, ui.id().with(("hit", index)), Sense::click());
            if response.hovered() && ui.input(|i| i.pointer.delta().length() > 0.0) {
                self.selected = index;
            }
            if index == self.selected {
                ui.painter().rect_filled(rect, rounded(10.0), theme::CARD_HOVER);
            }
            let icon_center = rect.left_center() + vec2(22.0, 0.0);
            let texture = hit.icon_path.as_ref().and_then(|path| self.textures.app_icon(ui.ctx(), path));
            match texture {
                Some(texture) => {
                    let area = Rect::from_center_size(icon_center, vec2(28.0, 28.0));
                    ui.painter().image(texture.id(), area, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    ui.painter().text(icon_center, Align2::CENTER_CENTER, hit.icon, theme::icon(20.0), theme::DIM);
                }
            }
            let left = rect.left() + 46.0;
            let width = rect.width() - 60.0;
            text_clipped(ui, pos2(left, rect.center().y - 9.0), Align2::LEFT_CENTER, &hit.title, theme::medium(14.0), theme::WHITE, width);
            text_clipped(ui, pos2(left, rect.center().y + 10.0), Align2::LEFT_CENTER, &hit.subtitle, theme::regular(11.0), theme::FAINT, width);
            if response.clicked() {
                activated = Some(index);
            }
        }
        let footer_y = panel.bottom() - 15.0;
        let hints = format!("↑↓  {}     ⏎  {}     Tab  AI     Esc  {}", t("spotlight.navigate"), t("spotlight.open"), t("spotlight.close"));
        text_clipped(ui, pos2(panel.left() + 16.0, footer_y), Align2::LEFT_CENTER, &hints, theme::regular(10.5), theme::FAINT, panel.width() - 32.0);

        if let Some(index) = activated
            && let Some(hit) = results.get(index)
        {
            self.activate(hit.clone());
        }
    }

    fn activate(&mut self, hit: Hit) {
        if matches!(&hit.action, Action::Command(command) if command.confirm) {
            self.confirm = Some(hit);
            return;
        }
        self.run_hit(&hit);
    }

    fn run_hit(&mut self, hit: &Hit) {
        self.searcher.record_use(hit);
        if self.hub.demo {
            return;
        }
        if search::run(hit) {
            self.close();
        } else {
            self.hub.toast(crate::services::Toast::Info { icon: icons::WARNING, text: t("spotlight.launchFailed") });
        }
    }

    fn confirm_row(&mut self, ui: &mut Ui, panel: Rect, y: f32, hit: &Hit) {
        let question = tf("spotlight.system.confirmMessage", &[&hit.title]);
        text_clipped(ui, pos2(panel.left() + 20.0, y + 18.0), Align2::LEFT_CENTER, &question, theme::medium(13.0), theme::WHITE, panel.width() - 40.0);
        let yes = Rect::from_min_size(pos2(panel.right() - 196.0, y + 36.0), vec2(90.0, 26.0));
        let no = Rect::from_min_size(pos2(panel.right() - 100.0, y + 36.0), vec2(84.0, 26.0));
        if kit::pill_button(ui, yes, &t("dialog.confirm"), theme::RED.gamma_multiply(0.8), Color32::WHITE, "confirm-yes").clicked() {
            let hit = hit.clone();
            self.confirm = None;
            self.run_hit(&hit);
        }
        if kit::pill_button(ui, no, &t("dialog.cancel"), theme::CARD_HOVER, theme::WHITE, "confirm-no").clicked() {
            self.confirm = None;
        }
    }

    fn ai_mode(&mut self, ui: &mut Ui, panel: Rect) {
        let settings = self.hub.settings.get();
        let (chat, streaming, error, history) = match self.ai.state.lock() {
            Ok(state) => (state.chat.clone(), state.streaming, state.error.clone(), state.history.clone()),
            Err(_) => return,
        };

        // Header
        let header = Rect::from_min_size(panel.min, vec2(panel.width(), 44.0));
        ui.painter().text(header.left_center() + vec2(24.0, 0.0), Align2::CENTER_CENTER, icons::SPARKLE, theme::icon_fill(16.0), theme::ACCENT);
        let provider = &settings.spotlight_ai_provider;
        let title = tf("spotlight.ai.agent", &[provider]);
        text_clipped(ui, header.left_center() + vec2(42.0, 0.0), Align2::LEFT_CENTER, &title, theme::semibold(13.0), theme::WHITE, 300.0);
        let mut x = header.right() - 22.0;
        let back = Rect::from_center_size(pos2(x, header.center().y), vec2(26.0, 26.0));
        if kit::icon_button(ui, back, icons::MAGNIFYING_GLASS, 14.0, theme::DIM, "ai-back").on_hover_text(t("spotlight.ai.back")).clicked() {
            self.mode = Mode::Search;
            self.last_query = None;
        }
        x -= 30.0;
        let new_chat = Rect::from_center_size(pos2(x, header.center().y), vec2(26.0, 26.0));
        if kit::icon_button(ui, new_chat, icons::PLUS, 14.0, theme::DIM, "ai-new").on_hover_text(t("spotlight.ai.newChat")).clicked() {
            self.ai.new_chat();
            self.show_history = false;
        }
        x -= 30.0;
        let toggle = Rect::from_center_size(pos2(x, header.center().y), vec2(26.0, 26.0));
        let history_color = if self.show_history { theme::WHITE } else { theme::DIM };
        if kit::icon_button(ui, toggle, icons::CLOCK_COUNTER_CLOCKWISE, 14.0, history_color, "ai-history").on_hover_text(t("spotlight.ai.history")).clicked() {
            self.show_history = !self.show_history;
        }
        ui.painter().line_segment([header.left_bottom(), header.right_bottom()], egui::Stroke::new(1.0, theme::LINE));

        let input_height = 54.0;
        let middle = Rect::from_min_max(header.left_bottom() + vec2(14.0, 8.0), panel.right_bottom() - vec2(14.0, input_height + 4.0));

        if self.show_history {
            self.history_list(ui, middle, &history);
        } else {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(middle).id_salt("ai-messages"));
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(&mut child, |ui| {
                ui.set_width(middle.width() - 12.0);
                if chat.messages.is_empty() {
                    ui.add_space(70.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new(t("spotlight.ai.title")).font(theme::semibold(18.0)));
                        ui.add_space(6.0);
                        let hint = if ai::is_configured(provider) { t("port.spotlight.aiLocal") } else { t("spotlight.ai.configure") };
                        ui.label(egui::RichText::new(hint).font(theme::regular(12.5)).color(theme::DIM));
                        ui.add_space(14.0);
                        for key in ["spotlight.ai.promptOne", "spotlight.ai.promptTwo"] {
                            let text = t(key);
                            if ui.button(egui::RichText::new(&text).font(theme::regular(12.5))).clicked() {
                                self.ai_input = text;
                            }
                        }
                    });
                }
                for message in &chat.messages {
                    let is_user = message.role == "user";
                    let (fill, align) = if is_user {
                        (theme::alpha(theme::ACCENT, 0.4), egui::Align::Max)
                    } else {
                        (theme::CARD, egui::Align::Min)
                    };
                    ui.with_layout(egui::Layout::top_down(align), |ui| {
                        egui::Frame::new().fill(fill).corner_radius(rounded(12.0)).inner_margin(egui::Margin::symmetric(11, 8)).show(ui, |ui| {
                            ui.set_max_width(middle.width() * 0.84);
                            let text = if message.text.is_empty() && streaming { t("spotlight.ai.thinking") } else { message.text.clone() };
                            ui.add(egui::Label::new(egui::RichText::new(text).font(theme::regular(13.0))).wrap().selectable(true));
                        });
                    });
                    ui.add_space(8.0);
                }
                if let Some(error) = &error {
                    ui.label(egui::RichText::new(error).font(theme::regular(12.0)).color(theme::ORANGE));
                }
            });
        }

        // Input
        let input_box = Rect::from_min_max(panel.left_bottom() + vec2(12.0, -input_height + 4.0), panel.right_bottom() - vec2(12.0, 10.0));
        ui.painter().rect_filled(input_box, rounded(14.0), theme::CARD);
        let field = Rect::from_min_max(input_box.min + vec2(12.0, 10.0), input_box.max - vec2(44.0, 8.0));
        let response = ui.put(
            field,
            egui::TextEdit::singleline(&mut self.ai_input)
                .hint_text(t("spotlight.ai.placeholder"))
                .frame(egui::Frame::NONE)
                .font(theme::regular(14.0))
                .id(Id::new("spotlight-ai-input")),
        );
        if !response.has_focus() && !self.show_history {
            response.request_focus();
        }
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let button = Rect::from_center_size(input_box.right_center() - vec2(22.0, 0.0), vec2(30.0, 30.0));
        let clicked_stop = streaming && kit::icon_button_fill(ui, button, icons::STOP, 14.0, theme::WHITE, "ai-stop").on_hover_text(t("spotlight.ai.stop")).clicked();
        let clicked_send = !streaming && kit::icon_button_fill(ui, button, icons::PAPER_PLANE_RIGHT, 15.0, theme::WHITE, "ai-send").on_hover_text(t("spotlight.ai.send")).clicked();
        if clicked_stop {
            self.ai.stop();
        }
        if (clicked_send || enter) && !streaming && !self.ai_input.trim().is_empty() && !self.hub.demo {
            let text = std::mem::take(&mut self.ai_input);
            self.ai.send(text.trim().to_string(), &settings);
        }
        if streaming {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    fn history_list(&mut self, ui: &mut Ui, area: Rect, history: &[ai::Chat]) {
        if history.is_empty() {
            ui.painter().text(area.center(), Align2::CENTER_CENTER, t("spotlight.ai.noHistory"), theme::regular(13.0), theme::DIM);
            return;
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).id_salt("ai-history-list"));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut child, |ui| {
            for chat in history {
                let (rect, response) = ui.allocate_exact_size(vec2(area.width() - 12.0, 42.0), Sense::click());
                let fill = if response.hovered() { theme::CARD_HOVER } else { theme::CARD };
                ui.painter().rect_filled(rect, rounded(10.0), fill);
                text_clipped(ui, rect.left_center() + vec2(12.0, -7.0), Align2::LEFT_CENTER, &chat.title, theme::medium(12.5), theme::WHITE, rect.width() - 60.0);
                let characters: usize = chat.messages.iter().map(|m| m.text.chars().count()).sum();
                let meta = t("spotlight.ai.metadata").replace("{0}", &chat.messages.len().to_string()).replace("{1:N0}", &characters.to_string());
                text_clipped(ui, rect.left_center() + vec2(12.0, 9.0), Align2::LEFT_CENTER, &meta, theme::regular(10.5), theme::FAINT, rect.width() - 60.0);
                let delete = Rect::from_center_size(rect.right_center() - vec2(20.0, 0.0), vec2(26.0, 26.0));
                let removed = kit::icon_button(ui, delete, icons::TRASH, 13.0, theme::DIM, ("ai-delete", chat.id)).clicked();
                if removed {
                    self.ai.delete_chat(chat.id);
                } else if response.clicked() {
                    self.ai.open_chat(chat.id);
                    self.show_history = false;
                }
                ui.add_space(4.0);
            }
        });
    }
}
