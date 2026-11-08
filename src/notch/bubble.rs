//! The round Claude bubble beside the closed notch (the original's ActivityBubble). While a Claude
//! session works its star turns; while one waits the star pulses. Hovering opens the bubble into a
//! small pill with the session's name and what it is doing; a click opens the Claude tab.

use super::{Activity, Notch, View, glyph};
use crate::i18n::t;
use crate::services::claude::{Session, Status};
use crate::services::sounds::Tone;
use crate::settings::Settings;
use crate::theme;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Ui, pos2, vec2};

/// Space between the notch (with its ear) and the bubble.
const GAP: f32 = 10.0;
/// Width of the opened bubble.
const OPEN_W: f32 = 236.0;
const STAR: Color32 = Color32::from_rgb(0xD9, 0x77, 0x57);
const SUBTITLE: Color32 = Color32::from_rgb(0x9A, 0x9A, 0x9F);

/// One word for what a session is at, like Claude Code's own spinner: the tool decides it.
pub fn status_key(session: &Session) -> &'static str {
    let tool = session.tool.as_deref().unwrap_or_default();
    if session.status == Status::Waiting {
        return match tool {
            "AskUserQuestion" => "claude.status.question",
            "ExitPlanMode" => "claude.status.plan",
            _ => "claude.status.waiting",
        };
    }
    match tool {
        "" => "claude.status.thinking",
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => "claude.status.writing",
        "Read" | "Grep" | "Glob" | "LS" | "NotebookRead" => "claude.status.reading",
        "Bash" | "PowerShell" | "BashOutput" | "KillShell" | "KillBash" | "Monitor" => "claude.status.running",
        "WebFetch" | "WebSearch" => "claude.status.browsing",
        "Task" | "Agent" | "SendMessage" => "claude.status.delegating",
        "TodoWrite" | "EnterPlanMode" => "claude.status.planning",
        _ => "claude.status.working",
    }
}

/// Where the closed bubble sits: just right of the notch and its ear, flush with its top.
pub fn bubble_origin(notch: Rect, ear: f32) -> Pos2 {
    pos2((notch.right() + ear + GAP).round(), notch.top())
}

impl Notch {
    pub(super) fn paint_claude_bubble(&mut self, ui: &mut Ui, notch: Rect, settings: &Settings, activity: &Activity) {
        let id = ui.id().with("claude-bubble");
        // Shown with the notch closed (it goes back in as soon as the notch starts to open).
        let wanted = activity.claude_busy && !self.expanded && self.toast.is_none() && settings.tab_visible("Claude");
        let shown = ui.ctx().animate_bool_with_time(id.with("shown"), wanted, if wanted { 0.35 } else { 0.12 });
        if shown <= 0.001 {
            self.bubble_rect = None;
            return;
        }
        let (sessions, _) = self.hub.claude.snapshot();
        let session = sessions
            .iter()
            .filter(|s| matches!(s.status, Status::Waiting | Status::Working))
            .min_by_key(|s| if s.status == Status::Waiting { 0 } else { 1 })
            .cloned();

        let size = self.height.value.clamp(26.0, 40.0).max(26.0);
        let ear = if settings.enable_dynamic_island_mode { 0.0 } else { (self.radius.value * 0.75).clamp(4.0, 10.0) - 1.0 };
        let origin = bubble_origin(notch, ear);
        let hovered_before = self.bubble_rect.is_some_and(|r| ui.rect_contains_pointer(r));
        let open = ui.ctx().animate_bool_with_time(id.with("open"), hovered_before && session.is_some(), 0.3);
        let open_h = size + 10.0;
        let width = egui::lerp(size..=OPEN_W, open);
        let height = egui::lerp(size..=open_h, open);
        // It slides out from under the notch as it appears.
        let slide = (1.0 - shown) * -(size + GAP);
        let rect = Rect::from_min_size(origin + vec2(slide, 0.0), vec2(width, height));
        self.bubble_rect = Some(rect);

        let opacity = settings.opacity.max(0.85);
        ui.painter().rect_filled(rect, egui::CornerRadius::same((height / 2.0).min(127.0) as u8), theme::alpha(Color32::BLACK, opacity * shown));

        let response = ui.interact(rect, id, Sense::click());
        if response.clicked() {
            self.open(View::Claude);
            self.hub.play(Tone::Tap);
        }

        let time = ui.input(|i| i.time) as f32;
        let waiting = session.as_ref().is_some_and(|s| s.status == Status::Waiting) || activity.claude_waiting > 0;
        let face = egui::lerp(16.0..=19.0, open);
        let face_center = pos2(rect.left() + height / 2.0, rect.center().y);
        let (angle, scale) = if waiting {
            // A pulse up to 130 % and back every 1.3 s.
            let phase = (time / 0.65).rem_euclid(2.0);
            let t = if phase < 1.0 { phase } else { 2.0 - phase };
            (0.0, 1.0 + 0.3 * ease_in_out(t))
        } else {
            // One turn every 2.4 s.
            ((time / 2.4).rem_euclid(1.0) * std::f32::consts::TAU, 1.0)
        };
        glyph::paint_rotated(ui.painter(), face_center, face * scale, glyph::CLAUDE, theme::alpha(STAR, shown), angle);

        if open > 0.05
            && let Some(session) = &session
        {
            let alpha = ((open - 0.4) / 0.6).clamp(0.0, 1.0) * shown;
            let left = rect.left() + 40.0;
            let clip = Rect::from_min_max(pos2(left, rect.top()), pos2(rect.right() - 14.0, rect.bottom()));
            let painter = ui.painter().with_clip_rect(clip.intersect(ui.clip_rect()));
            painter.text(pos2(left, rect.center().y - 7.0), Align2::LEFT_CENTER, session.name(), theme::semibold(12.0), theme::alpha(theme::WHITE, alpha));
            let subtitle = match (&session.activity, session.status) {
                (Some(activity), Status::Working) if !activity.is_empty() => activity.clone(),
                _ => t(status_key(session)),
            };
            let color = if session.status == Status::Waiting { theme::ORANGE } else { SUBTITLE };
            painter.text(pos2(left, rect.center().y + 8.0), Align2::LEFT_CENTER, subtitle, theme::semibold(10.5), theme::alpha(color, alpha));
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
}

fn ease_in_out(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bubble_sits_right_of_the_notch() {
        // The original's 34 px pill: 230 px wide, the bubble 20 px to its right (ear and gap).
        let notch = Rect::from_min_size(pos2(423.0, 0.0), vec2(230.0, 34.0));
        assert_eq!(bubble_origin(notch, 10.0), pos2(673.0, 0.0));
    }
}
