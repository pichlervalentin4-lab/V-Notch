//! The Claude tab: sessions, permission requests answered from the notch, plan limits.

use super::kit::{self, rounded, text_clipped};
use super::Notch;
use crate::i18n::{t, tf};
use crate::icons;
use crate::services::claude::{self, Decision, Limits, LimitWindow, PermissionRequest, Session, Status, format_elapsed};
use crate::services::sounds::Tone;
use crate::settings::Settings;
use crate::theme;
use chrono::Utc;
use eframe::egui::{Align2, Color32, Rect, Sense, Ui, pos2, vec2};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct ClaudeUi {
    pub wanted_height: f32,
    pub limits_share: Arc<Mutex<Option<Limits>>>,
    scroll: f32,
    setup_error: Option<String>,
    choices: HashMap<(u64, usize), Vec<String>>,
}

impl Default for ClaudeUi {
    fn default() -> Self {
        Self { wanted_height: 300.0, limits_share: Arc::new(Mutex::new(None)), scroll: 0.0, setup_error: None, choices: HashMap::new() }
    }
}

const SESSION_ROW: f32 = 58.0;
const CLAUDE_COLOR: Color32 = Color32::from_rgb(217, 119, 87);

impl Notch {
    pub(super) fn claude_view(&mut self, ui: &mut Ui, body: Rect, _settings: &Settings) {
        let (sessions, pending) = self.hub.claude.snapshot();
        let installed = self.hub.demo || claude::hooks_installed();

        // Limits line
        let limits = self.limits.clone();
        let mut top = body.top();
        if limits.is_some() {
            let line = Rect::from_min_size(body.min, vec2(body.width(), 22.0));
            self.limits_line(ui, line, limits.as_ref());
            top += 28.0;
        }

        let mut y = top;
        for request in &pending {
            let height = self.permission_card(ui, Rect::from_min_size(pos2(body.left(), y), vec2(body.width(), 0.0)), request, &sessions);
            y += height + 8.0;
        }

        if sessions.is_empty() && pending.is_empty() {
            let area = Rect::from_min_max(pos2(body.left(), y), body.max);
            self.claude_empty(ui, area, installed);
            self.claude_ui.wanted_height = 150.0;
            return;
        }

        let list = Rect::from_min_max(pos2(body.left(), y), body.max);
        let content = sessions.len() as f32 * (SESSION_ROW + 6.0);
        if ui.rect_contains_pointer(list) {
            self.claude_ui.scroll -= ui.input(|i| i.smooth_scroll_delta.y);
        }
        self.claude_ui.scroll = self.claude_ui.scroll.clamp(0.0, (content - list.height()).max(0.0));
        let old_clip = ui.clip_rect();
        ui.set_clip_rect(list.intersect(old_clip));
        for (index, session) in sessions.iter().enumerate() {
            let row_top = list.top() + index as f32 * (SESSION_ROW + 6.0) - self.claude_ui.scroll;
            if row_top > list.bottom() || row_top + SESSION_ROW < list.top() {
                continue;
            }
            self.session_row(ui, Rect::from_min_size(pos2(list.left(), row_top), vec2(list.width(), SESSION_ROW)), session);
        }
        ui.set_clip_rect(old_clip);

        if !installed {
            self.setup_hint(ui, body);
        }

        // The notch grows with its content, up to where the list starts to scroll.
        let used = (y - body.top()) + content;
        self.claude_ui.wanted_height = (used + 58.0).clamp(150.0, 420.0);
    }

    fn claude_empty(&mut self, ui: &mut Ui, area: Rect, installed: bool) {
        let center = area.center();
        ui.painter().text(center - vec2(0.0, 26.0), Align2::CENTER_CENTER, icons::SPARKLE, theme::icon_fill(26.0), CLAUDE_COLOR);
        if !claude::is_claude_present() && !self.hub.demo {
            ui.painter().text(center + vec2(0.0, 6.0), Align2::CENTER_CENTER, t("control.claude.notFound"), theme::regular(12.5), theme::DIM);
            return;
        }
        if !self.hub.claude_listening && !self.hub.demo {
            ui.painter().text(center + vec2(0.0, 6.0), Align2::CENTER_CENTER, tf("control.claude.portBusy", &[&claude::PORT.to_string()]), theme::regular(12.5), theme::ORANGE);
            return;
        }
        if installed {
            ui.painter().text(center + vec2(0.0, 6.0), Align2::CENTER_CENTER, t("control.claude.noSessions"), theme::regular(12.5), theme::DIM);
            let remove = Rect::from_center_size(center + vec2(0.0, 34.0), vec2(170.0, 24.0));
            if kit::pill_button(ui, remove, &t("control.claude.remove"), theme::CARD_HOVER, theme::DIM, "claude-remove").clicked() {
                self.claude_ui.setup_error = claude::uninstall_hooks().err();
            }
        } else {
            self.setup_hint(ui, area);
        }
        if let Some(error) = &self.claude_ui.setup_error {
            text_clipped(ui, area.center_bottom() - vec2(0.0, 8.0), Align2::CENTER_CENTER, error, theme::regular(11.0), theme::RED, area.width());
        }
    }

    fn setup_hint(&mut self, ui: &mut Ui, area: Rect) {
        let center = pos2(area.center().x, area.bottom() - 46.0);
        text_clipped(ui, center, Align2::CENTER_CENTER, &t("control.claude.setupHint"), theme::regular(12.0), theme::DIM, area.width() - 20.0);
        let button = Rect::from_center_size(center + vec2(0.0, 28.0), vec2(170.0, 26.0));
        if kit::pill_button(ui, button, &t("control.claude.setup"), CLAUDE_COLOR, Color32::WHITE, "claude-setup").clicked() {
            self.claude_ui.setup_error = claude::install_hooks().err();
            self.hub.play(Tone::Click);
        }
    }

    fn limits_line(&mut self, ui: &mut Ui, line: Rect, limits: Option<&Limits>) {
        let Some(limits) = limits else { return };
        let parts = kit::columns(line, 2, 16.0);
        let draw = |ui: &Ui, rect: Rect, label: String, window: &LimitWindow| {
            let ratio = window.used_percent / 100.0;
            let color = if ratio > 0.9 {
                theme::RED
            } else if ratio > 0.7 {
                theme::ORANGE
            } else {
                CLAUDE_COLOR
            };
            let mut text = format!("{label} {:.0} %", window.used_percent);
            if let Some(reset) = window.resets_at {
                let left = reset - Utc::now();
                text = format!("{text} · {}", tf("control.claude.resetsIn", &[&format_elapsed(left)]));
            }
            ui.painter().text(rect.left_center() - vec2(0.0, 3.0), Align2::LEFT_CENTER, text, theme::regular(10.5), theme::DIM);
            let track = Rect::from_min_size(pos2(rect.left(), rect.bottom() - 4.0), vec2(rect.width(), 3.0));
            ui.painter().rect_filled(track, rounded(1.5), theme::alpha(Color32::WHITE, 0.12));
            let used = Rect::from_min_size(track.min, vec2(track.width() * ratio.clamp(0.0, 1.0), 3.0));
            ui.painter().rect_filled(used, rounded(1.5), color);
        };
        if let Some(window) = &limits.five_hour {
            draw(ui, parts[0], t("control.claude.limit5h"), window);
        }
        if let Some(window) = &limits.week {
            draw(ui, parts[1], t("control.claude.limitWeek"), window);
        }
    }

    fn session_row(&mut self, ui: &mut Ui, row: Rect, session: &Session) {
        kit::card(ui, row, 12.0, theme::CARD);
        let (color, status) = status_of(session);
        let time = ui.input(|i| i.time) as f32;
        let dot = row.left_center() + vec2(18.0, 0.0);
        let breathing = matches!(session.status, Status::Working | Status::Waiting);
        let alpha = if breathing { 0.55 + 0.45 * (time * 3.0).sin() } else { 1.0 };
        ui.painter().circle_filled(dot, 4.5, theme::alpha(color, alpha));

        let left = row.left() + 34.0;
        let right = row.right() - 12.0;
        text_clipped(ui, pos2(left, row.top() + 15.0), Align2::LEFT_CENTER, &session.name(), theme::semibold(12.5), theme::WHITE, right - left - 110.0);
        let elapsed = match (session.turn_started, session.finished) {
            (Some(start), Some(end)) => Some(end - start),
            (Some(start), None) if session.status != Status::Idle => Some(Utc::now() - start),
            _ => None,
        };
        let trailing = elapsed.map_or(status.clone(), |value| format!("{status} · {}", format_elapsed(value)));
        ui.painter().text(pos2(right, row.top() + 15.0), Align2::RIGHT_CENTER, trailing, theme::regular(11.0), color);

        let detail = session
            .activity
            .clone()
            .filter(|text| !text.is_empty())
            .or_else(|| if session.status == Status::Done { session.summary.clone() } else { session.prompt.clone() })
            .unwrap_or_default();
        text_clipped(ui, pos2(left, row.top() + 37.0), Align2::LEFT_CENTER, &detail, theme::regular(11.5), theme::DIM, right - left);
    }

    /// Draws the request and returns how tall it turned out.
    fn permission_card(&mut self, ui: &mut Ui, top: Rect, request: &PermissionRequest, sessions: &[Session]) -> f32 {
        let name = sessions.iter().find(|s| s.id == request.session_id).map(Session::name).unwrap_or_default();
        let width = top.width();
        if let Some(questions) = &request.questions {
            let height = 40.0 + questions.iter().map(|q| 26.0 + q.options.len() as f32 * 30.0).sum::<f32>() + 34.0;
            let card = Rect::from_min_size(top.min, vec2(width, height.min(330.0)));
            kit::card(ui, card, 12.0, theme::alpha(CLAUDE_COLOR, 0.2));
            self.question_card(ui, card, request, &name);
            return card.height();
        }

        let plan = request.plan.as_deref();
        let height = if plan.is_some() { 190.0 } else { 92.0 };
        let card = Rect::from_min_size(top.min, vec2(width, height));
        kit::card(ui, card, 12.0, theme::alpha(CLAUDE_COLOR, 0.2));
        let left = card.left() + 14.0;
        let right = card.right() - 14.0;
        let title = if plan.is_some() { t("claude.status.plan") } else { t("claude.status.waiting") };
        text_clipped(ui, pos2(left, card.top() + 16.0), Align2::LEFT_CENTER, &format!("{name} · {title}"), theme::semibold(12.5), theme::WHITE, right - left);
        let body_text = plan.map(str::to_string).or_else(|| {
            request.detail.as_ref().map(|detail| format!("{}: {detail}", request.tool))
        });
        let body_text = body_text.unwrap_or_else(|| request.tool.clone());
        let lines = if plan.is_some() { 7 } else { 2 };
        kit::text_wrapped(ui, pos2(left, card.top() + 30.0), &body_text, theme::regular(11.5), theme::DIM, right - left, lines);

        let y = card.bottom() - 19.0;
        let mut x = right;
        let mut button = |ui: &mut Ui, label: &str, width: f32, fill: Color32, id: &str| {
            let rect = Rect::from_min_size(pos2(x - width, y - 12.0), vec2(width, 24.0));
            x -= width + 6.0;
            kit::pill_button(ui, rect, label, fill, Color32::WHITE, (id, request.id)).clicked()
        };
        if button(ui, &t("control.claude.allow"), 84.0, theme::GREEN.gamma_multiply(0.75), "allow") {
            self.hub.claude.decide(request.id, Decision::Allow);
            self.hub.play(Tone::Tap);
        }
        if button(ui, &t("control.claude.deny"), 84.0, theme::RED.gamma_multiply(0.75), "deny") {
            self.hub.claude.decide(request.id, Decision::Deny);
            self.hub.play(Tone::Tap);
        }
        if let Some(label) = &request.always_label {
            let text = format!("{} {label}", t("control.claude.always"));
            if button(ui, &text, 210.0, theme::CARD_HOVER, "always") {
                self.hub.claude.decide(request.id, Decision::Always);
                self.hub.play(Tone::Tap);
            }
        }
        card.height()
    }

    fn question_card(&mut self, ui: &mut Ui, card: Rect, request: &PermissionRequest, name: &str) {
        let Some(questions) = &request.questions else { return };
        let left = card.left() + 14.0;
        let right = card.right() - 14.0;
        text_clipped(ui, pos2(left, card.top() + 16.0), Align2::LEFT_CENTER, &format!("{name} · {}", t("claude.status.question")), theme::semibold(12.5), theme::WHITE, right - left);
        let mut y = card.top() + 32.0;
        for (question_index, question) in questions.iter().enumerate() {
            text_clipped(ui, pos2(left, y + 8.0), Align2::LEFT_CENTER, &question.text, theme::medium(11.5), theme::WHITE, right - left);
            y += 22.0;
            for (option_index, option) in question.options.iter().enumerate() {
                let row = Rect::from_min_size(pos2(left, y), vec2(right - left, 26.0));
                let key = (request.id, question_index);
                let chosen = self.claude_ui.choices.get(&key).is_some_and(|c| c.contains(&option.label));
                let response = ui.interact(row, ui.id().with(("option", request.id, question_index, option_index)), Sense::click());
                let fill = if chosen {
                    theme::alpha(CLAUDE_COLOR, 0.7)
                } else if response.hovered() {
                    theme::CARD_HOVER
                } else {
                    theme::CARD
                };
                ui.painter().rect_filled(row, rounded(8.0), fill);
                let label = if option.description.is_empty() { option.label.clone() } else { format!("{} — {}", option.label, option.description) };
                text_clipped(ui, row.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, &label, theme::regular(11.5), theme::WHITE, row.width() - 20.0);
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
        let button = Rect::from_min_size(pos2(right - 110.0, card.bottom() - 30.0), vec2(110.0, 24.0));
        let fill = if complete { theme::GREEN.gamma_multiply(0.75) } else { theme::CARD_HOVER };
        if kit::pill_button(ui, button, &t("control.claude.submit"), fill, Color32::WHITE, ("submit", request.id)).clicked() && complete {
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

fn status_of(session: &Session) -> (Color32, String) {
    match session.status {
        Status::Waiting => (theme::ORANGE, t("claude.status.waiting")),
        Status::Working => {
            let key = match session.tool.as_deref() {
                Some("Edit" | "MultiEdit" | "Write" | "NotebookEdit") => "claude.status.writing",
                Some("Read" | "Glob" | "Grep") => "claude.status.reading",
                Some("Bash" | "PowerShell") => "claude.status.running",
                Some("WebFetch" | "WebSearch") => "claude.status.browsing",
                Some("Task" | "Agent") => "claude.status.delegating",
                Some("ExitPlanMode" | "EnterPlanMode") => "claude.status.planning",
                Some(_) => "claude.status.working",
                None => "claude.status.thinking",
            };
            (CLAUDE_COLOR, t(key))
        }
        Status::Done => (theme::GREEN, t("control.claude.done")),
        Status::Idle => (theme::FAINT, t("control.claude.idle")),
    }
}
