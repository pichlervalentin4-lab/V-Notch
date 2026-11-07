//! Live translation of the selected text, with currency conversion for prices.
//!
//! The original runs a local Qwen model through llama.cpp. This port asks a local Ollama server (the
//! same model works there) or, if the user chooses so, the configured Spotlight AI provider.

use crate::i18n::{LANGUAGES, t};
use crate::icons;
use crate::notch::kit::{self, rounded, text_clipped};
use crate::platform;
use crate::services::{Hub, http};
use crate::settings::Settings;
use crate::spotlight::ai;
use crate::theme;
use eframe::egui::{self, Align2, Color32, Key, Rect, Ui, ViewportBuilder, ViewportId, pos2, vec2};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const WIDTH: f32 = 460.0;
const HEIGHT: f32 = 300.0;
const MARGIN: f32 = 20.0;

#[derive(Default)]
struct Job {
    source: String,
    result: String,
    busy: bool,
    error: Option<String>,
    conversion: Option<String>,
}

const TITLE: &str = "V-Notch Translation";

pub struct Translator {
    hub: Arc<Hub>,
    pub open: bool,
    job: Arc<Mutex<Job>>,
    viewport_id: ViewportId,
    frame_removed: bool,
}

/// A price found in text.
#[derive(Debug, PartialEq)]
pub struct Price {
    pub amount: f64,
    pub currency: &'static str,
}

const CURRENCIES: [(&str, &str); 12] = [
    ("$", "USD"), ("usd", "USD"), ("€", "EUR"), ("eur", "EUR"), ("£", "GBP"), ("gbp", "GBP"),
    ("¥", "JPY"), ("jpy", "JPY"), ("chf", "CHF"), ("cad", "CAD"), ("aud", "AUD"), ("sek", "SEK"),
];

/// Reads "1,234.56" and "1.234,56" alike: the last separator with one or two digits behind it is the decimal mark.
pub fn parse_amount(text: &str) -> Option<f64> {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | ',')).collect();
    if digits.is_empty() || !digits.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let decimal_at = digits.rfind(['.', ',']).filter(|index| {
        let tail = digits.len() - index - 1;
        (1..=2).contains(&tail)
    });
    let mut clean = String::new();
    for (index, c) in digits.char_indices() {
        if c.is_ascii_digit() {
            clean.push(c);
        } else if Some(index) == decimal_at {
            clean.push('.');
        }
    }
    clean.parse().ok()
}

/// Recognizes "$12.99", "12,99 EUR" and similar. Longer text is not a price.
pub fn parse_price(text: &str) -> Option<Price> {
    let text = text.trim();
    if text.len() > 24 || !text.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let lower = text.to_lowercase();
    let (_, currency) = CURRENCIES.iter().find(|(symbol, _)| {
        lower.starts_with(symbol) || lower.ends_with(symbol)
    })?;
    let without: String = lower.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | ',')).collect();
    let letters = lower.chars().filter(|c| c.is_alphabetic()).count();
    if letters > 3 {
        return None;
    }
    Some(Price { amount: parse_amount(&without)?, currency })
}

fn convert_price(price: &Price, target: &str) -> Option<String> {
    if price.currency == target {
        return None;
    }
    let url = format!("https://api.frankfurter.dev/v1/latest?base={}&symbols={target}", price.currency);
    let value = http::get_json(&url, Duration::from_secs(8)).ok()?;
    let rate = value.pointer(&format!("/rates/{target}")).and_then(Value::as_f64)?;
    Some(format!("{:.2} {} = {:.2} {target}", price.amount, price.currency, price.amount * rate))
}

fn language_name(code: &str) -> &str {
    LANGUAGES.iter().find(|(id, _)| *id == code).map_or(code, |(_, name)| name)
}

pub fn build_prompt(text: &str, target: &str) -> String {
    format!(
        "Translate the text between the markers into {}. Answer with the translation only, without notes, and keep line breaks.\n<text>\n{text}\n</text>",
        language_name(target)
    )
}

impl Translator {
    pub fn new(hub: Arc<Hub>) -> Self {
        Self { hub, open: false, job: Arc::new(Mutex::new(Job::default())), viewport_id: ViewportId::from_hash_of("v-notch-translate"), frame_removed: false }
    }

    /// Copies the selection of the active app, then translates it.
    pub fn trigger(&mut self, settings: &Settings) {
        if !settings.enable_live_translation {
            return;
        }
        if settings.translation_ignore_fullscreen && self.hub.system.state.lock().is_ok_and(|s| s.fullscreen != platform::FullscreenKind::None) {
            return;
        }
        if let Some(app) = platform::foreground_app()
            && settings.translation_excluded_apps.split(';').any(|name| name.trim().eq_ignore_ascii_case(&app))
        {
            return;
        }
        let previous = arboard::Clipboard::new().ok().and_then(|mut board| board.get_text().ok());
        platform::copy_selection();
        self.open = true;
        if let Ok(mut job) = self.job.lock() {
            *job = Job { busy: true, ..Default::default() };
        }
        let job = self.job.clone();
        let settings = settings.clone();
        let ctx = self.hub.ctx.clone();
        let _ = std::thread::Builder::new().name("translate".into()).spawn(move || {
            // The target app needs a moment to answer the copy shortcut.
            std::thread::sleep(Duration::from_millis(180));
            let text = arboard::Clipboard::new().ok().and_then(|mut board| board.get_text().ok()).unwrap_or_default();
            if text.trim().is_empty() || previous.as_deref() == Some(text.as_str()) && text.len() > 4000 {
                if let Ok(mut job) = job.lock() {
                    job.busy = false;
                    job.error = Some(t("port.translate.nothing"));
                }
                ctx.request_repaint();
                return;
            }
            if let Ok(mut job) = job.lock() {
                job.source.clone_from(&text);
            }
            ctx.request_repaint();
            run_translation(&job, &text, &settings, &ctx);
        });
    }

    pub fn show_viewport(&mut self, ctx: &egui::Context, center_x: f32, top: f32) {
        if !self.open {
            self.frame_removed = false;
            return;
        }
        let width = WIDTH + MARGIN * 2.0;
        let builder = ViewportBuilder::default()
            .with_title(TITLE)
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_inner_size([width, HEIGHT + MARGIN * 2.0])
            .with_position([center_x - width / 2.0, top]);
        let id = self.viewport_id;
        ctx.show_viewport_immediate(id, builder, |ui, _| self.window(ui));
        // Windows 11 would outline the whole transparent window, shadow margin included.
        #[cfg(windows)]
        if !self.frame_removed {
            self.frame_removed = crate::platform::prepare_floating_window_by_title(TITLE);
        }
    }

    fn window(&mut self, ui: &mut Ui) {
        let full = ui.ctx().content_rect();
        let panel = full.shrink(MARGIN);
        let shadow = egui::Shadow { offset: [0, 8], blur: 20, spread: 0, color: theme::alpha(Color32::BLACK, 0.5) };
        ui.painter().add(shadow.as_shape(panel, rounded(16.0)));
        ui.painter().rect_filled(panel, rounded(16.0), Color32::from_rgb(12, 12, 14));

        let (source, result, busy, error, conversion) = match self.job.lock() {
            Ok(job) => (job.source.clone(), job.result.clone(), job.busy, job.error.clone(), job.conversion.clone()),
            Err(_) => return,
        };

        ui.painter().text(panel.left_top() + vec2(22.0, 22.0), Align2::CENTER_CENTER, icons::TRANSLATE, theme::icon(16.0), theme::ACCENT);
        text_clipped(ui, panel.left_top() + vec2(38.0, 22.0), Align2::LEFT_CENTER, &t("translation.title"), theme::semibold(13.0), theme::WHITE, 240.0);
        let close = Rect::from_center_size(panel.right_top() + vec2(-22.0, 22.0), vec2(26.0, 26.0));
        if kit::icon_button(ui, close, icons::X, 14.0, theme::DIM, "translate-close").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
            self.open = false;
        }
        let copy = Rect::from_center_size(panel.right_top() + vec2(-52.0, 22.0), vec2(26.0, 26.0));
        if kit::icon_button(ui, copy, icons::COPY, 14.0, theme::DIM, "translate-copy").on_hover_text(t("translation.copy")).clicked()
            && !result.is_empty()
            && let Ok(mut board) = arboard::Clipboard::new()
        {
            let _ = board.set_text(result.clone());
        }

        let inner = Rect::from_min_max(panel.min + vec2(16.0, 42.0), panel.max - vec2(16.0, 14.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).id_salt("translate-body"));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut child, |ui| {
            ui.set_width(inner.width() - 10.0);
            if let Some(conversion) = &conversion {
                ui.label(egui::RichText::new(conversion).font(theme::semibold(16.0)).color(theme::GREEN));
                ui.add_space(6.0);
            }
            if let Some(error) = &error {
                ui.label(egui::RichText::new(error).font(theme::regular(13.0)).color(theme::ORANGE));
            }
            if !result.is_empty() {
                ui.label(egui::RichText::new(&result).font(theme::regular(14.5)));
            } else if busy {
                ui.label(egui::RichText::new(t("spotlight.ai.thinking")).font(theme::regular(13.0)).color(theme::DIM));
            }
            if !source.is_empty() {
                ui.add_space(10.0);
                ui.label(egui::RichText::new(t("translation.original")).font(theme::medium(11.0)).color(theme::FAINT));
                ui.label(egui::RichText::new(&source).font(theme::regular(12.0)).color(theme::DIM));
            }
        });
        if busy {
            ui.ctx().request_repaint_after(Duration::from_millis(60));
        }
        let _ = pos2(0.0, 0.0);
    }
}

fn run_translation(job: &Arc<Mutex<Job>>, text: &str, settings: &Settings, ctx: &egui::Context) {
    let set_error = |message: String| {
        if let Ok(mut job) = job.lock() {
            job.error = Some(message);
            job.busy = false;
        }
        ctx.request_repaint();
    };

    if settings.enable_currency_conversion
        && settings.is_online_allowed()
        && let Some(price) = parse_price(text)
        && let Some(conversion) = convert_price(&price, &settings.translation_target_currency)
    {
        if let Ok(mut job) = job.lock() {
            job.conversion = Some(conversion);
            job.busy = false;
        }
        ctx.request_repaint();
        return;
    }

    let (provider, model) = if settings.translation_engine == "ollama" {
        ("Ollama".to_string(), settings.translation_ollama_model.clone())
    } else {
        let provider = settings.spotlight_ai_provider.clone();
        let model = ai::model_for(settings, &provider);
        (provider, model)
    };
    if !ai::is_local(&provider) && (!settings.is_online_allowed() || !settings.allow_online_ai) {
        set_error(t("spotlight.ai.privacyBlocked"));
        return;
    }
    let key = platform::secret_get(&ai::secret_name(&provider)).unwrap_or_default();
    if ai::needs_key(&provider) && key.trim().is_empty() {
        set_error(t("spotlight.ai.configure"));
        return;
    }
    let history = vec![ai::ChatMessage { role: "user".into(), text: build_prompt(text, &settings.translation_target_language) }];
    let request = ai::build_request(&provider, &model, &key, &settings.spotlight_ollama_url, &history);
    let headers: Vec<(&str, String)> = request.headers.iter().map(|(n, v)| (*n, v.clone())).collect();
    let reader = match http::post_stream(&request.url, &headers, &request.body, Duration::from_secs(90)) {
        Ok(reader) => reader,
        Err((status, _)) => {
            let message = if ai::is_local(&provider) && status == 0 { t("port.translate.noOllama") } else { t(ai::error_key(status)) };
            set_error(message);
            return;
        }
    };
    for line in BufReader::new(reader).lines().map_while(Result::ok) {
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let Ok(event) = serde_json::from_str::<Value>(data) else { continue };
        let delta = ai::read_delta(&provider, &event);
        if !delta.is_empty() {
            if let Ok(mut job) = job.lock() {
                job.result.push_str(&delta);
            }
            ctx.request_repaint();
        }
    }
    if let Ok(mut job) = job.lock() {
        job.busy = false;
    }
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_read_both_decimal_styles() {
        assert_eq!(parse_amount("12.99"), Some(12.99));
        assert_eq!(parse_amount("12,99"), Some(12.99));
        assert_eq!(parse_amount("1,234.56"), Some(1234.56));
        assert_eq!(parse_amount("1.234,56"), Some(1234.56));
        assert_eq!(parse_amount("1.234"), Some(1234.0));
        assert_eq!(parse_amount("1,000,000"), Some(1_000_000.0));
        assert_eq!(parse_amount("none"), None);
    }

    #[test]
    fn prices_are_recognized() {
        assert_eq!(parse_price("$12.99"), Some(Price { amount: 12.99, currency: "USD" }));
        assert_eq!(parse_price("12,99 EUR"), Some(Price { amount: 12.99, currency: "EUR" }));
        assert_eq!(parse_price("  £5 "), Some(Price { amount: 5.0, currency: "GBP" }));
    }

    #[test]
    fn ordinary_text_is_not_a_price() {
        assert_eq!(parse_price("Hallo Welt"), None);
        assert_eq!(parse_price("42"), None);
        assert_eq!(parse_price("Wir treffen uns um 12 Uhr im Cafe in Traun eur"), None);
        assert_eq!(parse_price("12 Eurobonds"), None);
    }

    #[test]
    fn the_prompt_names_the_language_and_keeps_the_text() {
        let prompt = build_prompt("Hello", "de");
        assert!(prompt.contains("Deutsch"));
        assert!(prompt.contains("Hello"));
    }
}
