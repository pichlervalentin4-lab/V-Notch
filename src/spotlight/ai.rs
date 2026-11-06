//! Spotlight AI: sends the conversation straight to the chosen provider and streams the answer.
//! Nothing is sent before the user presses Enter. Keys live in the OS secret store.

use crate::platform;
use crate::services::http;
use crate::settings::Settings;
use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const PROVIDERS: [&str; 5] = ["Claude", "OpenAI", "Gemini", "DeepSeek", "Ollama"];
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
pub const MAX_MESSAGE_CHARS: usize = 16_000;

pub fn default_model(provider: &str) -> &'static str {
    match provider {
        "Claude" => "claude-sonnet-5-5",
        "OpenAI" => "gpt-5",
        "Gemini" => "gemini-2.5-flash",
        "DeepSeek" => "deepseek-chat",
        _ => "qwen3:4b",
    }
}

pub fn secret_name(provider: &str) -> String {
    format!("spotlight-api-key-{}", provider.to_lowercase())
}

pub fn model_for(settings: &Settings, provider: &str) -> String {
    let chosen = match provider {
        "Claude" => &settings.spotlight_claude_model,
        "OpenAI" => &settings.spotlight_openai_model,
        "Gemini" => &settings.spotlight_gemini_model,
        "DeepSeek" => &settings.spotlight_deepseek_model,
        _ => &settings.spotlight_ollama_model,
    };
    let chosen = chosen.trim();
    if chosen.is_empty() { default_model(provider).to_string() } else { chosen.to_string() }
}

/// Ollama runs locally and needs no key.
pub fn needs_key(provider: &str) -> bool {
    provider != "Ollama"
}

pub fn is_local(provider: &str) -> bool {
    provider == "Ollama"
}

pub fn is_configured(provider: &str) -> bool {
    !needs_key(provider) || platform::secret_get(&secret_name(provider)).is_some_and(|key| !key.trim().is_empty())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Chat {
    pub id: u64,
    pub title: String,
    pub messages: Vec<ChatMessage>,
}

pub struct Request {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Value,
}

/// Builds the provider's request. Pure, so every provider's format can be tested.
pub fn build_request(provider: &str, model: &str, key: &str, base_url: &str, history: &[ChatMessage]) -> Request {
    let messages: Vec<Value> = history.iter().map(|m| json!({"role": m.role, "content": m.text})).collect();
    let key = key.trim();
    match provider {
        "Claude" => Request {
            url: "https://api.anthropic.com/v1/messages".into(),
            headers: vec![("x-api-key", key.to_string()), ("anthropic-version", "2023-06-01".to_string())],
            body: json!({"model": model, "max_tokens": 4096, "messages": messages, "stream": true}),
        },
        "Gemini" => {
            let contents: Vec<Value> = history
                .iter()
                .map(|m| {
                    let role = if m.role == "assistant" { "model" } else { "user" };
                    json!({"role": role, "parts": [{"text": m.text}]})
                })
                .collect();
            Request {
                url: format!(
                    "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
                    http::encode(model)
                ),
                headers: vec![("x-goog-api-key", key.to_string())],
                body: json!({"contents": contents}),
            }
        }
        "Ollama" => Request {
            url: format!("{}/v1/chat/completions", base_url.trim_end_matches('/')),
            headers: Vec::new(),
            body: json!({"model": model, "messages": messages, "stream": true}),
        },
        _ => Request {
            url: if provider == "DeepSeek" {
                "https://api.deepseek.com/chat/completions".into()
            } else {
                "https://api.openai.com/v1/chat/completions".into()
            },
            headers: vec![("Authorization", format!("Bearer {key}"))],
            body: json!({"model": model, "messages": messages, "stream": true}),
        },
    }
}

/// The text inside one server-sent event, for each provider's own format.
pub fn read_delta(provider: &str, event: &Value) -> String {
    let text = match provider {
        "Claude" => {
            if event.pointer("/delta/type").and_then(Value::as_str) == Some("text_delta") {
                event.pointer("/delta/text").and_then(Value::as_str)
            } else {
                None
            }
        }
        "Gemini" => event.pointer("/candidates/0/content/parts/0/text").and_then(Value::as_str),
        _ => event.pointer("/choices/0/delta/content").and_then(Value::as_str),
    };
    text.unwrap_or_default().to_string()
}

/// The locale key explaining a failed request. Provider messages are not shown: they may echo private text.
pub fn error_key(status: u16) -> &'static str {
    match status {
        0 => "spotlight.ai.networkError",
        400 => "spotlight.ai.requestError",
        401 => "spotlight.ai.authError",
        402 => "spotlight.ai.billingError",
        403 => "spotlight.ai.permissionError",
        404 => "spotlight.ai.modelError",
        408 => "spotlight.ai.requestTimeout",
        413 => "spotlight.ai.contextError",
        429 => "spotlight.ai.rateError",
        500..=599 => "spotlight.ai.serverError",
        _ => "spotlight.ai.providerError",
    }
}

#[derive(Default)]
pub struct AiState {
    pub chat: Chat,
    pub streaming: bool,
    pub error: Option<String>,
    pub history: Vec<Chat>,
}

pub struct AiService {
    pub state: Mutex<AiState>,
    stop: Arc<AtomicBool>,
    ctx: egui::Context,
}

fn store_path() -> PathBuf {
    crate::paths::data_dir().join("spotlight-chats.json")
}

fn load_history() -> Vec<Chat> {
    std::fs::read_to_string(store_path()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

impl AiService {
    pub fn new(ctx: egui::Context) -> Arc<Self> {
        let state = AiState { history: load_history(), ..Default::default() };
        Arc::new(Self { state: Mutex::new(state), stop: Arc::new(AtomicBool::new(false)), ctx })
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn new_chat(&self) {
        self.stop();
        if let Ok(mut state) = self.state.lock() {
            state.chat = Chat::default();
            state.error = None;
        }
    }

    pub fn open_chat(&self, id: u64) {
        self.stop();
        if let Ok(mut state) = self.state.lock()
            && let Some(chat) = state.history.iter().find(|c| c.id == id).cloned()
        {
            state.chat = chat;
            state.error = None;
        }
    }

    pub fn delete_chat(&self, id: u64) {
        if let Ok(mut state) = self.state.lock() {
            state.history.retain(|c| c.id != id);
            if state.chat.id == id {
                state.chat = Chat::default();
            }
            save_history(&state.history);
        }
    }

    pub fn clear_history(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.history.clear();
        }
        let _ = std::fs::remove_file(store_path());
    }

    /// Sends `text` with the chat so far. The reply arrives in `state.chat` piece by piece.
    pub fn send(self: &Arc<Self>, text: String, settings: &Settings) {
        let provider = settings.spotlight_ai_provider.clone();
        if !is_local(&provider) && (!settings.is_online_allowed() || !settings.allow_online_ai) {
            self.fail(crate::i18n::t("spotlight.ai.privacyBlocked"));
            return;
        }
        if text.chars().count() > MAX_MESSAGE_CHARS {
            self.fail(crate::i18n::t("spotlight.ai.tooLong"));
            return;
        }
        let key = platform::secret_get(&secret_name(&provider)).unwrap_or_default();
        if needs_key(&provider) && key.trim().is_empty() {
            self.fail(crate::i18n::t("spotlight.ai.configure"));
            return;
        }
        let model = model_for(settings, &provider);
        let base_url = settings.spotlight_ollama_url.clone();
        let save = settings.save_ai_chat_history;

        let history = {
            let Ok(mut state) = self.state.lock() else { return };
            if state.streaming {
                return;
            }
            if state.chat.id == 0 {
                state.chat.id = SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |d| d.as_millis() as u64);
                state.chat.title = text.chars().take(60).collect();
            }
            state.chat.messages.push(ChatMessage { role: "user".into(), text });
            state.streaming = true;
            state.error = None;
            let history = state.chat.messages.clone();
            state.chat.messages.push(ChatMessage { role: "assistant".into(), text: String::new() });
            history
        };
        self.stop.store(false, Ordering::Relaxed);
        self.ctx.request_repaint();

        let service = self.clone();
        let _ = std::thread::Builder::new().name("spotlight-ai".into()).spawn(move || {
            let request = build_request(&provider, &model, &key, &base_url, &history);
            let result = service.stream(&provider, &request);
            let Ok(mut state) = service.state.lock() else { return };
            state.streaming = false;
            if let Err(status) = result {
                state.error = Some(crate::i18n::t(error_key(status)));
                let empty = state.chat.messages.last().is_some_and(|m| m.role == "assistant" && m.text.is_empty());
                if empty {
                    state.chat.messages.pop();
                }
            }
            if save {
                let chat = state.chat.clone();
                state.history.retain(|c| c.id != chat.id);
                state.history.insert(0, chat);
                state.history.truncate(100);
                save_history(&state.history);
            }
            drop(state);
            service.ctx.request_repaint();
        });
    }

    fn stream(&self, provider: &str, request: &Request) -> Result<(), u16> {
        let headers: Vec<(&str, String)> = request.headers.iter().map(|(n, v)| (*n, v.clone())).collect();
        let reader = http::post_stream(&request.url, &headers, &request.body, REQUEST_TIMEOUT).map_err(|(status, _)| status)?;
        for line in BufReader::new(reader).lines() {
            if self.stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let line = line.map_err(|_| 0_u16)?;
            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim();
            if data == "[DONE]" {
                break;
            }
            let Ok(event) = serde_json::from_str::<Value>(data) else { continue };
            let delta = read_delta(provider, &event);
            if !delta.is_empty() {
                if let Ok(mut state) = self.state.lock()
                    && let Some(last) = state.chat.messages.last_mut()
                {
                    last.text.push_str(&delta);
                }
                self.ctx.request_repaint();
            }
        }
        Ok(())
    }

    fn fail(&self, message: String) {
        if let Ok(mut state) = self.state.lock() {
            state.error = Some(message);
        }
        self.ctx.request_repaint();
    }
}

fn save_history(history: &[Chat]) {
    if let Ok(text) = serde_json::to_string(history) {
        let _ = std::fs::write(store_path(), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> Vec<ChatMessage> {
        vec![
            ChatMessage { role: "user".into(), text: "Hallo".into() },
            ChatMessage { role: "assistant".into(), text: "Hi".into() },
            ChatMessage { role: "user".into(), text: "Wie geht's?".into() },
        ]
    }

    #[test]
    fn claude_request_uses_its_headers_and_max_tokens() {
        let request = build_request("Claude", "m", " key ", "", &history());
        assert_eq!(request.url, "https://api.anthropic.com/v1/messages");
        assert!(request.headers.iter().any(|(n, v)| *n == "x-api-key" && v == "key"));
        assert!(request.headers.iter().any(|(n, _)| *n == "anthropic-version"));
        assert_eq!(request.body["max_tokens"], 4096);
        assert_eq!(request.body["stream"], true);
        assert_eq!(request.body["messages"][2]["content"], "Wie geht's?");
    }

    #[test]
    fn gemini_keeps_the_key_out_of_the_url_and_renames_the_assistant() {
        let request = build_request("Gemini", "gemini x", "secret", "", &history());
        assert!(!request.url.contains("secret"));
        assert!(request.url.contains("gemini%20x:streamGenerateContent"));
        assert_eq!(request.body["contents"][1]["role"], "model");
        assert_eq!(request.body["contents"][0]["parts"][0]["text"], "Hallo");
    }

    #[test]
    fn openai_style_providers_use_bearer_tokens() {
        for (provider, host) in [("OpenAI", "api.openai.com"), ("DeepSeek", "api.deepseek.com")] {
            let request = build_request(provider, "m", "k", "", &history());
            assert!(request.url.contains(host));
            assert!(request.headers.iter().any(|(n, v)| *n == "Authorization" && v == "Bearer k"));
        }
    }

    #[test]
    fn ollama_stays_local_and_sends_no_credentials() {
        let request = build_request("Ollama", "m", "", "http://127.0.0.1:11434/", &history());
        assert_eq!(request.url, "http://127.0.0.1:11434/v1/chat/completions");
        assert!(request.headers.is_empty());
    }

    #[test]
    fn deltas_are_read_per_provider() {
        assert_eq!(read_delta("Claude", &json!({"delta": {"type": "text_delta", "text": "a"}})), "a");
        assert_eq!(read_delta("Claude", &json!({"delta": {"type": "input_json_delta", "text": "a"}})), "");
        assert_eq!(read_delta("OpenAI", &json!({"choices": [{"delta": {"content": "b"}}]})), "b");
        assert_eq!(read_delta("DeepSeek", &json!({"choices": [{"delta": {}}]})), "");
        assert_eq!(read_delta("Gemini", &json!({"candidates": [{"content": {"parts": [{"text": "c"}]}}]})), "c");
    }

    #[test]
    fn status_codes_map_to_known_messages() {
        assert_eq!(error_key(401), "spotlight.ai.authError");
        assert_eq!(error_key(429), "spotlight.ai.rateError");
        assert_eq!(error_key(503), "spotlight.ai.serverError");
        assert_eq!(error_key(0), "spotlight.ai.networkError");
        assert_eq!(error_key(418), "spotlight.ai.providerError");
    }

    #[test]
    fn every_provider_has_a_default_model() {
        for provider in PROVIDERS {
            assert!(!default_model(provider).is_empty());
        }
        let settings = Settings { spotlight_openai_model: " my-model ".into(), ..Default::default() };
        assert_eq!(model_for(&settings, "OpenAI"), "my-model");
        assert_eq!(model_for(&settings, "Claude"), default_model("Claude"));
    }

    #[test]
    fn only_ollama_works_without_a_key() {
        assert!(is_configured("Ollama"));
        assert!(needs_key("Claude") && !needs_key("Ollama"));
    }
}
