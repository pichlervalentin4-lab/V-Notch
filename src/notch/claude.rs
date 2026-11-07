//! The Claude tab, built like the original's Claude page: a header with the sound switch, the plan
//! limits as two cards, then one card per session with its status, and any permission request,
//! question or plan inside the card of the session that asks.

use super::glyph;
use super::kit::{self, rounded, text_clipped};
use super::Notch;
use crate::i18n::{t, tf};
use crate::icons;
use crate::services::claude::{self, Decision, LimitWindow, Limits, PermissionRequest, Session, Status, format_elapsed};
use crate::services::sounds::Tone;
use crate::settings::Settings;
use crate::theme;
use chrono::Utc;
use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct ClaudeUi {
    pub wanted_height: f32,
    pub limits_share: Arc<Mutex<Option<Limits>>>,
    setup_error: Option<String>,
    choices: HashMap<(u64, usize), Vec<String>>,
}

impl Default for ClaudeUi {
    fn default() -> Self {
        Self { wanted_height: 300.0, limits_share: Arc::new(Mutex::new(None)), setup_error: None, choices: HashMap::new() }
    }
}

// The original's colours (ControlCenterView.xaml.cs).
const WORKING: Color32 = Color32::from_rgb(0xD9, 0x77, 0x57);
const WAITING: Color32 = Color32::from_rgb(0xFF, 0xD6, 0x0A);
const RED: Color32 = Color32::from_rgb(0xFF, 0x45, 0x3A);
const TRACK: Color32 = Color32::from_gray(0x4D);
const MUTED: Color32 = Color32::from_gray(0x8C);
const HEADER_TEXT: Color32 = Color32::from_gray(0x8A);
const CARD: f32 = 0x0F as f32 / 255.0;
const CARD_HOVER: f32 = 0x1C as f32 / 255.0;
/// Space the tab keeps above (nav bar) and below its content, for the notch's height.
const CHROME: f32 = 34.0 + 14.0;

impl Notch {
    pub(super) fn claude_view(&mut self, ui: &mut Ui, body: Rect, settings: &Settings) {
        let (sessions, pending) = self.hub.claude.snapshot();
        let installed = self.hub.demo || claude::hooks_installed();
        let limits = self.limits.clone();

        let mut child = ui.new_child(UiBuilder::new().max_rect(body).id_salt("claude-scroll"));
        let scroll = &mut child.style_mut().spacing.scroll;
        *scroll = egui::style::ScrollStyle { bar_width: 4.0, bar_inner_margin: 0.0, bar_outer_margin: 0.0, floating: false, ..egui::style::ScrollStyle::solid() };
        child.visuals_mut().widgets.inactive.bg_fill = theme::alpha(Color32::WHITE, 0.45);
        child.visuals_mut().extreme_bg_color = Color32::TRANSPARENT;
        let output = egui::ScrollArea::vertical().auto_shrink([false, true]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let width = ui.available_width();
            self.claude_header(ui, width, installed, settings);
            if !installed {
                self.setup_card(ui, width);
            }
            if let Some(limits) = &limits
                && (limits.five_hour.is_some() || limits.week.is_some())
            {
                limit_cards(ui, width, limits);
            }
            if installed || !sessions.is_empty() {
                if !self.hub.claude_listening && !self.hub.demo {
                    hint_card(ui, width, &tf("control.claude.portBusy", &[&claude::PORT.to_string()]));
                }
                caption(ui, &t("control.claude.sessions"), sessions.len());
                if sessions.is_empty() {
                    hint_card(ui, width, &t("control.claude.noSessions"));
                }
                for session in &sessions {
                    let requests: Vec<&PermissionRequest> = pending.iter().filter(|r| r.session_id == session.id).collect();
                    self.session_card(ui, width, session, &requests);
                }
            }
            // Requests of sessions the notch has not seen yet still need an answer.
            for request in pending.iter().filter(|r| !sessions.iter().any(|s| s.id == r.session_id)) {
                self.request_card(ui, width, request);
            }
            if let Some(error) = &self.claude_ui.setup_error {
                let (rect, _) = ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
                text_clipped(ui, rect.left_center(), Align2::LEFT_CENTER, error, theme::bold(11.0), RED, width);
            }
        });
        // The notch fits its content, between the original's 120 and 420 points.
        self.claude_ui.wanted_height = (output.content_size.y + CHROME).clamp(120.0, 420.0);
    }

    /// The orange Claude mark and "Claude Code", the speaker that switches the sounds, and "Remove".
    fn claude_header(&mut self, ui: &mut Ui, width: f32, installed: bool, settings: &Settings) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 20.0 + 10.0), Sense::hover());
        let row = Rect::from_min_size(rect.min, vec2(width, 20.0));
        glyph::paint(ui.painter(), pos2(row.left() + 2.0 + 7.5, row.center().y), 15.0, glyph::CLAUDE, WORKING);
        ui.painter().text(pos2(row.left() + 26.0, row.center().y), Align2::LEFT_CENTER, "Claude Code", theme::bold(15.0), Color32::WHITE);

        let mut right = row.right();
        if installed {
            let label = t("control.claude.remove");
            let label_width = kit::text_width(ui, &label, theme::bold(11.5));
            let hit = Rect::from_min_max(pos2(right - label_width, row.top()), pos2(right, row.bottom()));
            let response = ui.interact(hit, ui.id().with("claude-remove"), Sense::click());
            let color = if response.hovered() { Color32::WHITE } else { HEADER_TEXT };
            ui.painter().text(pos2(right, row.center().y), Align2::RIGHT_CENTER, &label, theme::bold(11.5), color);
            if response.clicked() && !self.hub.demo {
                self.claude_ui.setup_error = claude::uninstall_hooks().err();
            }
            right -= label_width + 12.0;
        }
        let on = settings.play_claude_sounds;
        let button = Rect::from_center_size(pos2(right - 12.0, row.center().y), vec2(24.0, 24.0));
        let response = ui.interact(button, ui.id().with("claude-sounds"), Sense::click());
        // The original's icon button: a small dark box with a hairline border.
        let fill = if response.hovered() { theme::alpha(Color32::WHITE, 0x1F as f32 / 255.0) } else { theme::alpha(Color32::BLACK, 0x33 as f32 / 255.0) };
        ui.painter().rect_filled(button, rounded(6.0), fill);
        ui.painter().rect_stroke(button, rounded(6.0), Stroke::new(1.0, theme::alpha(Color32::WHITE, 0x26 as f32 / 255.0)), egui::StrokeKind::Inside);
        let ink = theme::alpha(Color32::WHITE, if on { 1.0 } else { 0.6 });
        if on {
            glyph::paint(ui.painter(), button.center(), 12.0, glyph::SPEAKER, ink);
        } else {
            ui.painter().text(button.center(), Align2::CENTER_CENTER, icons::SPEAKER_X, theme::icon(13.0), ink);
        }
        let tip = t(if on { "control.claude.soundsOn" } else { "control.claude.soundsOff" });
        if response.on_hover_text(tip).clicked() {
            self.hub.settings.update(|s| s.play_claude_sounds = !on);
            self.hub.play(Tone::Tap);
        }
    }

    fn setup_card(&mut self, ui: &mut Ui, width: f32) {
        let present = claude::is_claude_present() || self.hub.demo;
        let listening = self.hub.claude_listening || self.hub.demo;
        let hint = if !present {
            t("control.claude.notFound")
        } else if !listening {
            tf("control.claude.portBusy", &[&claude::PORT.to_string()])
        } else {
            t("control.claude.setupHint")
        };
        let (rect, _) = ui.allocate_exact_size(vec2(width, 50.0 + 10.0), Sense::hover());
        let card = Rect::from_min_size(rect.min, vec2(width, 50.0));
        ui.painter().rect_filled(card, rounded(12.0), theme::alpha(Color32::WHITE, CARD));
        text_clipped(ui, pos2(card.left() + 14.0, card.center().y), Align2::LEFT_CENTER, &hint, theme::bold(12.0), MUTED, width - 140.0);
        if present && listening {
            let button = Rect::from_min_size(pos2(card.right() - 14.0 - 90.0, card.center().y - 14.0), vec2(90.0, 28.0));
            if text_button(ui, button, &t("control.claude.setup"), true, "claude-setup").clicked() {
                self.claude_ui.setup_error = claude::install_hooks().err();
                self.hub.play(Tone::Click);
            }
        }
    }

    /// One session: a status dot (rippling while it works or waits), its name, project and status,
    /// the last prompt or summary, and how long the turn has run.
    fn session_card(&mut self, ui: &mut Ui, width: f32, session: &Session, requests: &[&PermissionRequest]) {
        let color = status_color(session.status);
        let preview = if session.status == Status::Done { session.summary.clone() } else { session.prompt.clone() };
        let preview = preview.filter(|text| !text.is_empty());
        let text_height = 18.0 + 16.0 + if preview.is_some() { 17.0 } else { 0.0 };
        let request_heights: Vec<f32> = requests.iter().map(|request| self.request_height(request, width - 24.0 - 28.0)).collect();
        let requests_height: f32 = if request_heights.is_empty() { 0.0 } else { 10.0 + request_heights.iter().map(|h| h + 6.0).sum::<f32>() - 6.0 };
        let height = 11.0 + text_height + requests_height + 11.0;
        let (rect, _) = ui.allocate_exact_size(vec2(width, height + 6.0), Sense::hover());
        let card = Rect::from_min_size(rect.min, vec2(width, height));
        let response = ui.interact(card, ui.id().with(("session", &session.id)), Sense::hover());
        let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.15);
        ui.painter().rect_filled(card, rounded(12.0), theme::alpha(Color32::WHITE, CARD + (CARD_HOVER - CARD) * hover));

        let left = card.left() + 14.0;
        let dot = pos2(left + 7.0, card.top() + 11.0 + 2.0 + 7.0);
        if matches!(session.status, Status::Working | Status::Waiting) {
            // The original's ripple: a ring grows from 0.6 to 1.7 times the dot's box and fades out.
            let phase = ((ui.input(|i| i.time) % 1.6) / 1.6) as f32;
            let eased = 1.0 - (1.0 - phase).powi(3);
            let radius = 7.0 * (0.6 + 1.1 * eased);
            ui.painter().circle_filled(dot, radius, theme::alpha(color, 0.22 * (1.0 - eased) * 2.0));
        }
        ui.painter().circle_filled(dot, 4.0, color);

        let text_left = left + 24.0;
        let time = elapsed_label(session);
        let time_width = time.as_ref().map_or(0.0, |text| kit::text_width(ui, text, theme::bold(12.0)) + 12.0);
        let text_width = card.right() - 14.0 - text_left - time_width;
        let mut y = card.top() + 11.0;
        text_clipped(ui, pos2(text_left, y), Align2::LEFT_TOP, &session.name(), theme::bold(13.5), Color32::WHITE, text_width);
        y += 18.0;
        let status = status_text(session);
        let mut job = egui::text::LayoutJob::default();
        let semibold = theme::semibold(11.5);
        if session.title.as_ref().is_some_and(|title| !title.is_empty()) {
            job.append(&format!("{}  ·  ", session.project()), 0.0, egui::TextFormat::simple(semibold.clone(), MUTED));
        }
        let status_ink = if matches!(session.status, Status::Waiting | Status::Done) { color } else { MUTED };
        job.append(&status, 0.0, egui::TextFormat::simple(semibold, status_ink));
        job.wrap = egui::text::TextWrapping::truncate_at_width(text_width);
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(pos2(text_left, y + 2.0), galley, MUTED);
        y += 16.0;
        if let Some(preview) = &preview {
            let line = preview.lines().next().unwrap_or_default();
            text_clipped(ui, pos2(text_left, y + 3.0), Align2::LEFT_TOP, line, theme::medium(11.0), HEADER_TEXT, text_width);
            y += 17.0;
        }
        if let Some(time) = &time {
            ui.painter().text(pos2(card.right() - 14.0, card.top() + 12.0), Align2::RIGHT_TOP, time, theme::bold(12.0), MUTED);
        }
        if hover > 0.01 {
            ui.painter().text(pos2(card.right() - 14.0, card.top() + 46.0), Align2::RIGHT_CENTER, icons::ARROW_SQUARE_OUT, theme::icon(12.0), theme::alpha(Color32::WHITE, hover));
        }

        let mut top = y + 10.0;
        for (request, height) in requests.iter().zip(request_heights) {
            let area = Rect::from_min_size(pos2(text_left, top), vec2(card.right() - 14.0 - text_left, height));
            self.paint_request(ui, area, request);
            top += height + 6.0;
        }
    }

    /// A request whose session the notch does not know: drawn on its own, full width.
    fn request_card(&mut self, ui: &mut Ui, width: f32, request: &PermissionRequest) {
        let height = self.request_height(request, width);
        let (rect, _) = ui.allocate_exact_size(vec2(width, height + 6.0), Sense::hover());
        self.paint_request(ui, Rect::from_min_size(rect.min, vec2(width, height)), request);
    }

    fn request_height(&self, request: &PermissionRequest, width: f32) -> f32 {
        if let Some(questions) = &request.questions {
            return 9.0 + questions.iter().map(|q| 22.0 + q.options.len() as f32 * 30.0).sum::<f32>() + 36.0 + 9.0;
        }
        if let Some(plan) = &request.plan {
            let lines = (plan.len() as f32 * 6.5 / width.max(100.0)).ceil().clamp(2.0, 8.0);
            return 9.0 + 18.0 + lines * 15.0 + 38.0 + 9.0;
        }
        9.0 + 16.0 + if request.detail.is_some() { 17.0 } else { 0.0 } + 9.0
    }

    /// The original's request card: a dark box with a hairline border; the tool and what it wants on
    /// the left, Deny, Always and Allow on the right (Allow in Claude's orange).
    fn paint_request(&mut self, ui: &mut Ui, card: Rect, request: &PermissionRequest) {
        ui.painter().rect_filled(card, rounded(10.0), theme::alpha(Color32::BLACK, 0x33 as f32 / 255.0));
        ui.painter().rect_stroke(card, rounded(10.0), Stroke::new(1.0, theme::alpha(Color32::WHITE, 0x26 as f32 / 255.0)), egui::StrokeKind::Inside);
        if request.questions.is_some() {
            self.question_card(ui, card.shrink2(vec2(12.0, 9.0)), request);
            return;
        }
        let inner = Rect::from_min_max(card.min + vec2(12.0, 9.0), card.max - vec2(10.0, 9.0));
        let buttons_width = 3.0 * 76.0 + 2.0 * 6.0;
        if let Some(plan) = &request.plan {
            text_clipped(ui, inner.min, Align2::LEFT_TOP, &t("claude.status.plan"), theme::bold(12.5), Color32::WHITE, inner.width());
            let lines = ((inner.height() - 18.0 - 38.0) / 15.0).max(1.0) as usize;
            kit::text_wrapped(ui, inner.min + vec2(0.0, 18.0), plan, theme::medium(11.5), MUTED, inner.width(), lines);
            let y = inner.bottom() - 28.0;
            let approve = Rect::from_min_size(pos2(inner.right() - 110.0, y), vec2(110.0, 28.0));
            if text_button(ui, approve, &t("control.claude.approvePlan"), true, ("plan-approve", request.id)).clicked() {
                self.hub.claude.decide(request.id, Decision::Allow);
                self.hub.play(Tone::Tap);
            }
            let keep = Rect::from_min_size(pos2(approve.left() - 6.0 - 120.0, y), vec2(120.0, 28.0));
            if text_button(ui, keep, &t("control.claude.keepPlanning"), false, ("plan-keep", request.id)).clicked() {
                self.hub.claude.decide(request.id, Decision::Deny);
                self.hub.play(Tone::Tap);
            }
            return;
        }
        let text_width = inner.width() - buttons_width - 12.0;
        // File tools: "Edit  main.rs" with the folder under it in a monospaced font.
        let file_tool = matches!(request.tool.as_str(), "Edit" | "MultiEdit" | "Write" | "Read" | "NotebookEdit");
        let detail = request.detail.clone().unwrap_or_default();
        let center_y = inner.center().y;
        let has_detail = !detail.is_empty();
        let title_y = if has_detail { center_y - 8.5 } else { center_y };
        if file_tool && let Some(name) = std::path::Path::new(&detail).file_name().map(|n| n.to_string_lossy().into_owned()) {
            let mut job = egui::text::LayoutJob::default();
            job.append(&format!("{}  ", request.tool), 0.0, egui::TextFormat::simple(theme::bold(12.5), MUTED));
            job.append(&name, 0.0, egui::TextFormat::simple(theme::bold(12.5), Color32::WHITE));
            job.wrap = egui::text::TextWrapping::truncate_at_width(text_width);
            let galley = ui.painter().layout_job(job);
            let size = galley.size();
            ui.painter().galley(pos2(inner.left(), title_y - size.y / 2.0), galley, Color32::WHITE);
            let folder = std::path::Path::new(&detail).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            text_clipped(ui, pos2(inner.left(), center_y + 8.5), Align2::LEFT_CENTER, &folder, egui::FontId::monospace(11.0), MUTED, text_width);
        } else {
            text_clipped(ui, pos2(inner.left(), title_y), Align2::LEFT_CENTER, &request.tool, theme::bold(12.5), Color32::WHITE, text_width);
            if has_detail {
                let line = detail.lines().next().unwrap_or_default();
                text_clipped(ui, pos2(inner.left(), center_y + 8.5), Align2::LEFT_CENTER, line, egui::FontId::monospace(11.0), MUTED, text_width);
            }
        }
        let mut x = inner.right();
        let buttons = [
            ("control.claude.allow", true, Decision::Allow),
            ("control.claude.always", false, Decision::Always),
            ("control.claude.deny", false, Decision::Deny),
        ];
        for (key, accent, decision) in buttons {
            if decision == Decision::Always && request.always_label.is_none() {
                continue;
            }
            let rect = Rect::from_min_size(pos2(x - 76.0, center_y - 14.0), vec2(76.0, 28.0));
            x -= 76.0 + 6.0;
            let response = text_button(ui, rect, &t(key), accent, (key, request.id));
            let response = match (&request.always_label, decision) {
                (Some(rule), Decision::Always) => response.on_hover_text(rule),
                _ => response,
            };
            if response.clicked() {
                self.hub.claude.decide(request.id, decision);
                self.hub.play(Tone::Tap);
            }
        }
    }

    fn question_card(&mut self, ui: &mut Ui, area: Rect, request: &PermissionRequest) {
        let Some(questions) = &request.questions else { return };
        let mut y = area.top();
        for (question_index, question) in questions.iter().enumerate() {
            text_clipped(ui, pos2(area.left(), y + 8.0), Align2::LEFT_CENTER, &question.text, theme::bold(12.0), Color32::WHITE, area.width());
            y += 22.0;
            for (option_index, option) in question.options.iter().enumerate() {
                let row = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), 26.0));
                let key = (request.id, question_index);
                let chosen = self.claude_ui.choices.get(&key).is_some_and(|c| c.contains(&option.label));
                let response = ui.interact(row, ui.id().with(("option", request.id, question_index, option_index)), Sense::click());
                let fill = if chosen {
                    theme::alpha(WORKING, 0.85)
                } else if response.hovered() {
                    theme::alpha(Color32::WHITE, 0x1F as f32 / 255.0)
                } else {
                    theme::alpha(Color32::WHITE, CARD)
                };
                ui.painter().rect_filled(row, rounded(6.0), fill);
                let label = if option.description.is_empty() { option.label.clone() } else { format!("{} — {}", option.label, option.description) };
                let ink = if chosen { Color32::from_gray(0x0B) } else { Color32::WHITE };
                text_clipped(ui, row.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, &label, theme::bold(11.5), ink, row.width() - 20.0);
                if response.clicked() {
                    let entry = self.claude_ui.choices.entry(key).or_default();
                    if question.multi_select {
                        if let Some(position) = entry.iter().position(|l| *l == option.label) {
                            entry.remove(position);
                        } else {
                            entry.push(option.label.clone());
                        }
                    } else {
                        *entry = vec![option.label.clone()];
                    }
                }
                y += 30.0;
            }
        }
        let complete = (0..questions.len()).all(|i| self.claude_ui.choices.get(&(request.id, i)).is_some_and(|c| !c.is_empty()));
        let button = Rect::from_min_size(pos2(area.right() - 90.0, area.bottom() - 28.0), vec2(90.0, 28.0));
        if text_button(ui, button, &t("control.claude.submit"), complete, ("submit", request.id)).clicked() && complete {
            let mut answers = HashMap::new();
            for (index, question) in questions.iter().enumerate() {
                let chosen = self.claude_ui.choices.remove(&(request.id, index)).unwrap_or_default();
                answers.insert(question.text.clone(), chosen.join(", "));
            }
            self.hub.claude.answer_question(request.id, &answers);
            self.hub.play(Tone::Tap);
        }
    }
}

/// The two plan limits side by side: label and percentage, an orange bar (red from 90 %) and
/// when the window resets.
fn limit_cards(ui: &mut Ui, width: f32, limits: &Limits) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 70.0 + 14.0), Sense::hover());
    let cells = kit::columns(Rect::from_min_size(rect.min, vec2(width, 70.0)), 2, 8.0);
    let windows = [(t("control.claude.limit5h"), &limits.five_hour), (t("control.claude.limitWeek"), &limits.week)];
    for (card, (label, window)) in cells.into_iter().zip(windows) {
        let Some(window) = window else { continue };
        limit_card(ui, card, &label, window);
    }
}

fn limit_card(ui: &Ui, card: Rect, label: &str, window: &LimitWindow) {
    ui.painter().rect_filled(card, rounded(12.0), theme::alpha(Color32::WHITE, CARD));
    let inner = card.shrink2(vec2(14.0, 11.0));
    let high = window.used_percent >= 90.0;
    ui.painter().text(inner.left_top(), Align2::LEFT_TOP, label, theme::bold(12.0), Color32::WHITE);
    let percent = format!("{:.0} %", window.used_percent);
    ui.painter().text(inner.right_top(), Align2::RIGHT_TOP, percent, theme::bold(12.5), if high { WORKING } else { Color32::WHITE });
    let bar = Rect::from_min_size(pos2(inner.left(), inner.top() + 15.0 + 8.0), vec2(inner.width(), 4.0));
    ui.painter().rect_filled(bar, rounded(2.0), TRACK);
    let used = (window.used_percent / 100.0).clamp(0.0, 1.0);
    if used > 0.0 {
        let fill = Rect::from_min_size(bar.min, vec2((bar.width() * used).max(4.0), 4.0));
        ui.painter().rect_filled(fill, rounded(2.0), if high { RED } else { WORKING });
    }
    if let Some(reset) = window.resets_at {
        let text = tf("control.claude.resetsIn", &[&format_elapsed(reset - Utc::now())]);
        ui.painter().text(pos2(inner.left(), bar.bottom() + 7.0), Align2::LEFT_TOP, text, theme::bold(11.0), MUTED);
    }
}

/// "Sessions" and how many, above the session cards.
fn caption(ui: &mut Ui, text: &str, count: usize) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0 + 8.0), Sense::hover());
    let left = rect.left() + 2.0;
    ui.painter().text(pos2(left, rect.top() + 8.0), Align2::LEFT_CENTER, text, theme::bold(12.0), HEADER_TEXT);
    if count > 0 {
        let width = kit::text_width(ui, text, theme::bold(12.0));
        ui.painter().text(pos2(left + width + 6.0, rect.top() + 8.0), Align2::LEFT_CENTER, count.to_string(), theme::bold(12.0), MUTED);
    }
}

fn hint_card(ui: &mut Ui, width: f32, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 40.0 + 6.0), Sense::hover());
    let card = Rect::from_min_size(rect.min, vec2(width, 40.0));
    ui.painter().rect_filled(card, rounded(12.0), theme::alpha(Color32::WHITE, CARD));
    text_clipped(ui, pos2(card.left() + 14.0, card.center().y), Align2::LEFT_CENTER, text, theme::bold(12.0), MUTED, width - 28.0);
}

/// The original's text button: 28 high, 6-point corners; the accent one in Claude's orange with
/// dark text, the others dark with a hairline border.
fn text_button(ui: &mut Ui, rect: Rect, text: &str, accent: bool, id: impl std::hash::Hash + std::fmt::Debug) -> egui::Response {
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    let hovered = response.hovered();
    if accent {
        ui.painter().rect_filled(rect, rounded(6.0), theme::alpha(WORKING, if hovered { 0.85 } else { 1.0 }));
    } else {
        let fill = if hovered { 0x1F as f32 / 255.0 } else { 0.0 };
        ui.painter().rect_filled(rect, rounded(6.0), theme::alpha(Color32::WHITE, fill));
        ui.painter().rect_filled(rect, rounded(6.0), theme::alpha(Color32::BLACK, if hovered { 0.0 } else { 0x33 as f32 / 255.0 }));
        ui.painter().rect_stroke(rect, rounded(6.0), Stroke::new(1.0, theme::alpha(Color32::WHITE, 0x26 as f32 / 255.0)), egui::StrokeKind::Inside);
    }
    let ink = if accent { Color32::from_gray(0x0B) } else { Color32::WHITE };
    text_clipped(ui, rect.center(), Align2::CENTER_CENTER, text, theme::bold(12.0), ink, rect.width() - 12.0);
    response
}

fn status_color(status: Status) -> Color32 {
    match status {
        Status::Waiting => WAITING,
        Status::Working => WORKING,
        Status::Done => Color32::WHITE,
        Status::Idle => MUTED,
    }
}

fn status_text(session: &Session) -> String {
    match session.status {
        Status::Waiting => t("control.claude.waiting"),
        Status::Working => session.activity.clone().filter(|a| !a.is_empty()).unwrap_or_else(|| t("control.claude.thinking")),
        Status::Done => t("control.claude.done"),
        Status::Idle => t("control.claude.idle"),
    }
}

/// How long the turn has been running (or ran, once done), as "m:ss" or "h:mm:ss".
fn elapsed_label(session: &Session) -> Option<String> {
    let started = session.turn_started?;
    match session.status {
        Status::Working | Status::Waiting => Some(format_turn(Utc::now() - started)),
        Status::Done => session.finished.map(|finished| format_turn(finished - started)),
        Status::Idle => None,
    }
}

fn format_turn(span: chrono::TimeDelta) -> String {
    let seconds = span.num_seconds().max(0);
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_times_read_like_the_original() {
        assert_eq!(format_turn(chrono::TimeDelta::seconds(5)), "0:05");
        assert_eq!(format_turn(chrono::TimeDelta::seconds(495)), "8:15");
        assert_eq!(format_turn(chrono::TimeDelta::seconds(3_725)), "1:02:05");
        assert_eq!(format_turn(chrono::TimeDelta::seconds(-3)), "0:00");
    }
}
