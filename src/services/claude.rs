//! Claude Code in the notch: a loopback endpoint for its HTTP hooks, session tracking, permission
//! answers from the notch and the plan limits.
//!
//! The hooks are only added to `~/.claude/settings.json` after an explicit click, with a backup first.

use super::http;
use chrono::{DateTime, Utc};
use eframe::egui;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const PORT: u16 = 47833;
pub const HOOK_PATH: &str = "/vnotch/claude";
const HOOK_TIMEOUT: u64 = 3;
const PERMISSION_TIMEOUT: u64 = 600;
const MAX_BODY: usize = 4 * 1024 * 1024;
const QUESTION_TOOL: &str = "AskUserQuestion";
const PLAN_TOOL: &str = "ExitPlanMode";
const DENY_MESSAGE: &str = "Denied from the notch.";
const NO_DECISION: &str = "{}";

const EVENTS: [&str; 8] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Notification",
    "Stop",
    "SessionEnd",
];

pub fn hook_url() -> String {
    format!("http://127.0.0.1:{PORT}{HOOK_PATH}")
}

// ---------------------------------------------------------------------------------------------------------------------
// Setup: Claude Code's user settings
// ---------------------------------------------------------------------------------------------------------------------

pub fn claude_dir() -> PathBuf {
    crate::paths::home_dir().join(".claude")
}

pub fn is_claude_present() -> bool {
    claude_dir().is_dir()
}

fn is_our_hook(hook: &Value, url: &str) -> bool {
    hook["type"] == "http" && hook["url"] == url
}

fn has_our_hook(groups: Option<&Value>, url: &str) -> bool {
    groups.and_then(Value::as_array).is_some_and(|groups| {
        groups.iter().any(|group| {
            group["hooks"].as_array().is_some_and(|list| list.iter().any(|hook| is_our_hook(hook, url)))
        })
    })
}

pub fn hooks_installed_in(settings: &Value, url: &str) -> bool {
    EVENTS.iter().all(|event| has_our_hook(settings.pointer(&format!("/hooks/{event}")), url))
}

/// Adds our entries and touches nothing else. Fails rather than overwriting an unexpected shape.
pub fn install_hooks_into(settings: &mut Value, url: &str) -> Result<(), String> {
    let root = settings.as_object_mut().ok_or("settings is not an object")?;
    let hooks = root.entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks.as_object_mut().ok_or("\"hooks\" is not an object")?;
    for event in EVENTS {
        let timeout = if event == "PermissionRequest" { PERMISSION_TIMEOUT } else { HOOK_TIMEOUT };
        let groups = hooks.entry(event).or_insert_with(|| Value::Array(Vec::new()));
        let groups = groups.as_array_mut().ok_or_else(|| format!("\"{event}\" is not a list"))?;
        let mut present = false;
        for group in groups.iter_mut() {
            if let Some(list) = group["hooks"].as_array_mut() {
                for hook in list.iter_mut().filter(|hook| is_our_hook(hook, url)) {
                    hook["timeout"] = json!(timeout);
                    present = true;
                }
            }
        }
        if !present {
            groups.push(json!({"hooks": [{"type": "http", "url": url, "timeout": timeout}]}));
        }
    }
    Ok(())
}

pub fn uninstall_hooks_from(settings: &mut Value, url: &str) {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else { return };
    for event in EVENTS {
        let Some(groups) = hooks.get_mut(event).and_then(Value::as_array_mut) else { continue };
        for group in groups.iter_mut() {
            if let Some(list) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                list.retain(|hook| !is_our_hook(hook, url));
            }
        }
        groups.retain(|group| group["hooks"].as_array().is_none_or(|list| !list.is_empty()));
        if groups.is_empty() {
            hooks.remove(event);
        }
    }
    if hooks.is_empty()
        && let Some(root) = settings.as_object_mut()
    {
        root.remove("hooks");
    }
}

fn settings_path() -> PathBuf {
    claude_dir().join("settings.json")
}

fn read_claude_settings() -> Result<Value, String> {
    match std::fs::read_to_string(settings_path()) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("settings.json: {e}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(error) => Err(error.to_string()),
    }
}

fn write_claude_settings(settings: &Value) -> Result<(), String> {
    let path = settings_path();
    if path.exists() {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        std::fs::copy(&path, claude_dir().join(format!("settings.json.vnotch-backup-{stamp}"))).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.vnotch-tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn hooks_installed() -> bool {
    read_claude_settings().is_ok_and(|settings| hooks_installed_in(&settings, &hook_url()))
}

pub fn install_hooks() -> Result<(), String> {
    let mut settings = read_claude_settings()?;
    install_hooks_into(&mut settings, &hook_url())?;
    write_claude_settings(&settings)
}

pub fn uninstall_hooks() -> Result<(), String> {
    let mut settings = read_claude_settings()?;
    uninstall_hooks_from(&mut settings, &hook_url());
    write_claude_settings(&settings)
}

// ---------------------------------------------------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Working,
    Waiting,
    Done,
}

/// The last folder of a working directory. Splits on both separators, because the hook
/// can report a Windows path to a Linux build (WSL) and the other way round.
fn project_name(cwd: &str) -> String {
    cwd.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty() && !name.ends_with(':'))
        .map_or_else(|| "Claude".to_string(), str::to_string)
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub cwd: String,
    pub status: Status,
    pub activity: Option<String>,
    pub tool: Option<String>,
    pub prompt: Option<String>,
    pub summary: Option<String>,
    pub title: Option<String>,
    pub last_event: Instant,
    pub turn_started: Option<DateTime<Utc>>,
    pub finished: Option<DateTime<Utc>>,
    transcript: Option<PathBuf>,
    title_read: Option<Instant>,
}

impl Session {
    pub fn project(&self) -> String {
        project_name(&self.cwd)
    }

    pub fn name(&self) -> String {
        self.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| self.project())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub text: String,
    pub header: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Always,
    Deny,
}

/// A held `PermissionRequest` hook: Claude Code waits for our HTTP answer.
#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub id: u64,
    pub session_id: String,
    pub tool: String,
    pub detail: Option<String>,
    pub always_label: Option<String>,
    pub questions: Option<Vec<Question>>,
    pub plan: Option<String>,
    pub since: Instant,
    input: Value,
    suggestions: Value,
}

struct Held {
    request: PermissionRequest,
    answer: Sender<String>,
}

#[derive(Default)]
struct TrackerState {
    sessions: HashMap<String, Session>,
    pending: Vec<Held>,
    next_request: u64,
    /// Bumped on every change so the UI and the attention sound can react.
    revision: u64,
    newly_waiting: bool,
    newly_finished: bool,
}

#[derive(Default)]
pub struct Tracker {
    state: Mutex<TrackerState>,
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn flatten(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max).collect();
    cut.push('…');
    cut
}

fn preview(text: &str) -> String {
    truncate(&flatten(&text.replace("**", "").replace('`', "")), 160)
}

/// The command, file or URL a person decides on.
pub fn tool_detail(tool: &str, input: &Value) -> Option<String> {
    let key = match tool {
        "Bash" | "PowerShell" => "command",
        "Edit" | "MultiEdit" | "Write" | "Read" => "file_path",
        "NotebookEdit" => "notebook_path",
        "WebFetch" => "url",
        "WebSearch" => "query",
        "Glob" | "Grep" => "pattern",
        "Task" | "Agent" => "description",
        _ => "",
    };
    if tool == QUESTION_TOOL {
        return parse_questions(input).map(|questions| questions[0].text.clone());
    }
    let value = input.get(key).and_then(Value::as_str)?;
    (!value.is_empty()).then(|| value.to_string())
}

pub fn describe_tool(tool: &str, input: &Value) -> String {
    if tool.is_empty() {
        return String::new();
    }
    let Some(mut detail) = tool_detail(tool, input) else { return tool.to_string() };
    if matches!(tool, "Edit" | "MultiEdit" | "Write" | "Read" | "NotebookEdit") {
        let trimmed = detail.trim_end_matches(['/', '\\']);
        detail = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed).to_string();
    }
    format!("{tool}: {}", truncate(&flatten(&detail), 80))
}

pub fn parse_questions(input: &Value) -> Option<Vec<Question>> {
    let items = input.get("questions")?.as_array()?;
    let questions: Vec<Question> = items
        .iter()
        .filter_map(|item| {
            let options: Vec<QuestionOption> = item["options"]
                .as_array()?
                .iter()
                .filter_map(|option| {
                    let label = option["label"].as_str()?.to_string();
                    Some(QuestionOption { label, description: text(option, "description") })
                })
                .collect();
            let question = item["question"].as_str()?.to_string();
            (!options.is_empty()).then(|| Question {
                text: question,
                header: text(item, "header"),
                options,
                multi_select: item["multiSelect"].as_bool().unwrap_or(false),
            })
        })
        .collect();
    (!questions.is_empty()).then_some(questions)
}

/// What the "don't ask again" rule would allow, for the label of the Always button.
pub fn always_label(suggestions: &Value) -> Option<String> {
    let rules = suggestions.as_array()?;
    let first = rules.first()?;
    if let Some(rule) = first.pointer("/rules/0") {
        let tool = text(rule, "toolName");
        let content = text(rule, "ruleContent");
        return Some(if content.is_empty() { tool } else { format!("{tool}({})", truncate(&content, 40)) });
    }
    let mode = text(first, "mode");
    (!mode.is_empty()).then_some(mode)
}

fn permission_body(verdict: Value) -> String {
    json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": verdict}}).to_string()
}

pub fn decision_body(decision: Decision, suggestions: &Value) -> String {
    match decision {
        Decision::Deny => permission_body(json!({"behavior": "deny", "message": DENY_MESSAGE})),
        Decision::Allow => permission_body(json!({"behavior": "allow"})),
        Decision::Always => {
            let mut verdict = json!({"behavior": "allow"});
            if suggestions.as_array().is_some_and(|rules| !rules.is_empty()) {
                verdict["updatedPermissions"] = suggestions.clone();
            }
            permission_body(verdict)
        }
    }
}

/// Allows the question tool with the answers filled in, so it returns them without a dialog.
pub fn answer_body(input: &Value, answers: &HashMap<String, String>) -> String {
    let mut updated = if input.is_object() { input.clone() } else { json!({}) };
    updated["answers"] = json!(answers);
    permission_body(json!({"behavior": "allow", "updatedInput": updated}))
}

impl Tracker {
    /// Applies one hook call. A `PermissionRequest` returns a receiver the HTTP thread waits on.
    pub fn handle(&self, payload: &Value) -> Option<mpsc::Receiver<String>> {
        let session_id = text(payload, "session_id");
        if session_id.is_empty() {
            return None;
        }
        let event = text(payload, "hook_event_name");
        let tool = text(payload, "tool_name");
        let empty = Value::Null;
        let input = payload.get("tool_input").unwrap_or(&empty);
        let mut guard = self.state.lock().ok()?;
        let state = &mut *guard;
        state.revision += 1;

        if event == "SessionEnd" {
            state.sessions.remove(&session_id);
            release(state, |held| held.request.session_id == session_id);
            return None;
        }

        let session = state.sessions.entry(session_id.clone()).or_insert_with(|| Session {
            id: session_id.clone(),
            cwd: String::new(),
            status: Status::Idle,
            activity: None,
            tool: None,
            prompt: None,
            summary: None,
            title: None,
            last_event: Instant::now(),
            turn_started: None,
            finished: None,
            transcript: None,
            title_read: None,
        });
        session.last_event = Instant::now();
        let cwd = text(payload, "cwd");
        if !cwd.is_empty() {
            session.cwd = cwd;
        }
        let transcript = text(payload, "transcript_path");
        if !transcript.is_empty() {
            session.transcript = Some(PathBuf::from(transcript));
        }

        match event.as_str() {
            "SessionStart" => {}
            "UserPromptSubmit" => {
                session.status = Status::Working;
                session.activity = None;
                session.tool = None;
                session.summary = None;
                session.finished = None;
                session.turn_started = Some(Utc::now());
                let prompt = preview(&text(payload, "prompt"));
                if !prompt.is_empty() {
                    session.prompt = Some(prompt);
                }
                session.title_read = None;
                release(state, |held| held.request.session_id == session_id);
            }
            "PreToolUse" => {
                join_turn(session);
                session.status = Status::Working;
                session.tool = Some(tool.clone());
                session.activity = Some(describe_tool(&tool, input));
            }
            "PostToolUse" | "PostToolUseFailure" => {
                join_turn(session);
                session.status = Status::Working;
                session.tool = None;
                session.activity = None;
                // The dialog was answered in the terminal: stop holding its request.
                release(state, |held| {
                    held.request.session_id == session_id && held.request.tool == tool && held.request.input == *input
                });
            }
            "PermissionRequest" => {
                join_turn(session);
                session.status = Status::Waiting;
                session.tool = Some(tool.clone());
                session.activity = Some(describe_tool(&tool, input));
                state.newly_waiting = true;
                let suggestions = payload.get("permission_suggestions").cloned().unwrap_or(Value::Null);
                let plan = if tool == PLAN_TOOL { input["plan"].as_str().map(|p| p.trim().to_string()) } else { None };
                let (answer, receiver) = mpsc::channel();
                state.next_request += 1;
                state.pending.push(Held {
                    request: PermissionRequest {
                        id: state.next_request,
                        session_id: session_id.clone(),
                        detail: tool_detail(&tool, input),
                        always_label: always_label(&suggestions),
                        questions: if tool == QUESTION_TOOL { parse_questions(input) } else { None },
                        plan: plan.filter(|p| !p.is_empty()),
                        since: Instant::now(),
                        tool,
                        input: input.clone(),
                        suggestions,
                    },
                    answer,
                });
                return Some(receiver);
            }
            "Notification" => {
                let kind = text(payload, "notification_type");
                let message = text(payload, "message");
                if kind == "permission_prompt" || message.to_lowercase().contains("permission") {
                    session.status = Status::Waiting;
                    state.newly_waiting = true;
                }
            }
            "Stop" | "StopFailure" => {
                session.status = Status::Done;
                session.tool = None;
                session.activity = None;
                session.finished = Some(Utc::now());
                let reply = text(payload, "last_assistant_message");
                if !reply.is_empty() {
                    session.summary = Some(preview(&reply));
                }
                session.title_read = None;
                state.newly_finished = true;
                release(state, |held| held.request.session_id == session_id);
            }
            _ => {}
        }
        None
    }

    pub fn decide(&self, request_id: u64, decision: Decision) {
        self.answer_with(request_id, |held| decision_body(decision, &held.request.suggestions));
    }

    pub fn answer_question(&self, request_id: u64, answers: &HashMap<String, String>) {
        self.answer_with(request_id, |held| answer_body(&held.request.input, answers));
    }

    fn answer_with(&self, request_id: u64, body: impl Fn(&Held) -> String) {
        let Ok(mut state) = self.state.lock() else { return };
        state.revision += 1;
        if let Some(index) = state.pending.iter().position(|held| held.request.id == request_id) {
            let held = state.pending.remove(index);
            let _ = held.answer.send(body(&held));
            let session_id = held.request.session_id;
            let still_waiting = state.pending.iter().any(|h| h.request.session_id == session_id);
            if !still_waiting && let Some(session) = state.sessions.get_mut(&session_id) {
                session.status = Status::Working;
            }
        }
    }

    pub fn snapshot(&self) -> (Vec<Session>, Vec<PermissionRequest>) {
        let Ok(state) = self.state.lock() else { return (Vec::new(), Vec::new()) };
        let mut sessions: Vec<Session> = state.sessions.values().cloned().collect();
        sessions.sort_by_key(|s| (status_rank(s.status), std::cmp::Reverse(s.last_event)));
        (sessions, state.pending.iter().map(|held| held.request.clone()).collect())
    }

    /// True once after a session started waiting, or finished, since the last call.
    pub fn take_attention(&self) -> (bool, bool) {
        let Ok(mut state) = self.state.lock() else { return (false, false) };
        (std::mem::take(&mut state.newly_waiting), std::mem::take(&mut state.newly_finished))
    }

    /// Ages out sessions and reads their titles from the transcript. Called periodically.
    pub fn sweep(&self) {
        const STALE_WORKING: Duration = Duration::from_secs(30 * 60);
        const FORGET: Duration = Duration::from_secs(3 * 3600);
        const TITLE_RETRY: Duration = Duration::from_secs(15);
        let Ok(mut state) = self.state.lock() else { return };
        let now = Instant::now();
        state.sessions.retain(|_, s| now.duration_since(s.last_event) < FORGET);
        let waiting: Vec<String> = state.pending.iter().map(|h| h.request.session_id.clone()).collect();
        for session in state.sessions.values_mut() {
            if session.status == Status::Working
                && !waiting.contains(&session.id)
                && now.duration_since(session.last_event) > STALE_WORKING
            {
                session.status = Status::Idle;
            }
            let due = session.title_read.is_none_or(|read| now.duration_since(read) > TITLE_RETRY);
            if session.title.is_none() && due && let Some(path) = session.transcript.clone() {
                session.title_read = Some(now);
                session.title = read_title(&path);
            }
        }
    }
}

fn status_rank(status: Status) -> u8 {
    match status {
        Status::Waiting => 0,
        Status::Working => 1,
        Status::Done => 2,
        Status::Idle => 3,
    }
}

fn join_turn(session: &mut Session) {
    if session.turn_started.is_none() || session.status == Status::Done || session.status == Status::Idle {
        session.finished = None;
        let from_transcript = session.transcript.as_deref().and_then(read_turn_start);
        session.turn_started = Some(from_transcript.filter(|t| *t <= Utc::now()).unwrap_or_else(Utc::now));
    }
}

/// Lets go of held requests without a decision, so Claude Code falls back to its own dialog.
fn release(state: &mut TrackerState, matches: impl Fn(&Held) -> bool) {
    let mut index = 0;
    while index < state.pending.len() {
        if matches(&state.pending[index]) {
            let held = state.pending.remove(index);
            let _ = held.answer.send(NO_DECISION.to_string());
        } else {
            index += 1;
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Transcript
// ---------------------------------------------------------------------------------------------------------------------

const TAIL_BYTES: u64 = 256 * 1024;

fn read_tail(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    // The first line may be cut in half when the file is longer than the tail.
    Some(if start > 0 { text.split_once('\n').map_or(String::new(), |(_, rest)| rest.to_string()) } else { text })
}

pub fn find_title(tail: &str) -> Option<String> {
    let mut custom = None;
    let mut generated = None;
    for line in tail.lines() {
        if !(line.contains("custom-title") || line.contains("ai-title")) {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        match entry["type"].as_str() {
            Some("custom-title") => custom = entry["customTitle"].as_str().map(str::to_string).or(custom),
            Some("ai-title") => generated = entry["aiTitle"].as_str().map(str::to_string).or(generated),
            _ => {}
        }
    }
    custom.or(generated)
}

fn read_title(path: &Path) -> Option<String> {
    find_title(&read_tail(path)?)
}

/// When the session's last real prompt was sent: tool results and side chains do not count.
pub fn find_turn_start(tail: &str) -> Option<DateTime<Utc>> {
    for line in tail.lines().rev() {
        if !line.contains("\"user\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        if entry["type"] != "user" || entry["isMeta"] == true || entry["isSidechain"] == true {
            continue;
        }
        let content = &entry["message"]["content"];
        let is_tool_result = content
            .as_array()
            .is_some_and(|parts| parts.iter().any(|part| part["type"] == "tool_result"));
        if is_tool_result {
            continue;
        }
        return entry["timestamp"]
            .as_str()
            .and_then(|stamp| DateTime::parse_from_rfc3339(stamp).ok())
            .map(|time| time.with_timezone(&Utc));
    }
    None
}

fn read_turn_start(path: &Path) -> Option<DateTime<Utc>> {
    find_turn_start(&read_tail(path)?)
}

// ---------------------------------------------------------------------------------------------------------------------
// Plan limits
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LimitWindow {
    pub used_percent: f32,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Limits {
    pub five_hour: Option<LimitWindow>,
    pub week: Option<LimitWindow>,
}

pub fn parse_limits(root: &Value) -> Option<Limits> {
    let window = |node: &Value| {
        let used = node.get("utilization")?.as_f64()? as f32;
        let resets_at = node
            .get("resets_at")
            .and_then(Value::as_str)
            .and_then(|stamp| DateTime::parse_from_rfc3339(stamp).ok())
            .map(|time| time.with_timezone(&Utc));
        Some(LimitWindow { used_percent: used.clamp(0.0, 100.0), resets_at })
    };
    let five_hour = window(&root["five_hour"]);
    let week = window(&root["seven_day"]);
    (five_hour.is_some() || week.is_some()).then_some(Limits { five_hour, week })
}

/// The token Claude Code keeps on disk. It is only ever sent to api.anthropic.com.
fn read_access_token() -> Option<String> {
    let text = std::fs::read_to_string(claude_dir().join(".credentials.json")).ok()?;
    let root: Value = serde_json::from_str(&text).ok()?;
    let oauth = root.get("claudeAiOauth")?;
    if let Some(expires) = oauth.get("expiresAt").and_then(Value::as_i64)
        && expires <= Utc::now().timestamp_millis()
    {
        return None;
    }
    oauth.get("accessToken")?.as_str().map(str::to_string)
}

pub fn fetch_limits() -> Option<Limits> {
    let token = read_access_token()?;
    let headers = [("Authorization", format!("Bearer {token}")), ("anthropic-beta", "oauth-2025-04-20".to_string())];
    let value = http::get_json_with("https://api.anthropic.com/api/oauth/usage", &headers, Duration::from_secs(10)).ok()?;
    parse_limits(&value)
}

// ---------------------------------------------------------------------------------------------------------------------
// HTTP endpoint
// ---------------------------------------------------------------------------------------------------------------------

/// A plain socket server, so no URL reservation or admin rights are needed. Only loopback is bound.
pub fn start_server(tracker: Arc<Tracker>, ctx: egui::Context) -> Result<(), String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, PORT)).map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("claude-hooks".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let tracker = tracker.clone();
                let ctx = ctx.clone();
                let _ = std::thread::Builder::new().name("claude-hook".into()).spawn(move || {
                    let _ = serve(stream, &tracker, &ctx);
                });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn serve(mut stream: TcpStream, tracker: &Tracker, ctx: &egui::Context) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let Some(request) = read_request(&mut BufReader::new(stream.try_clone()?))? else {
        return respond(&mut stream, 400, NO_DECISION);
    };
    // A web page can reach loopback too: anything a browser sends carries an Origin and is refused.
    if request.method != "POST" || request.path != HOOK_PATH || request.has_origin {
        return respond(&mut stream, 404, NO_DECISION);
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&request.body) else {
        return respond(&mut stream, 400, NO_DECISION);
    };
    let held = tracker.handle(&payload);
    ctx.request_repaint();
    let Some(receiver) = held else { return respond(&mut stream, 200, NO_DECISION) };
    let body = receiver.recv_timeout(Duration::from_secs(PERMISSION_TIMEOUT - 5)).unwrap_or_else(|_| NO_DECISION.to_string());
    ctx.request_repaint();
    respond(&mut stream, 200, &body)
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Not Found",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

pub struct HookRequest {
    pub method: String,
    pub path: String,
    pub has_origin: bool,
    pub body: Vec<u8>,
}

pub fn read_request(reader: &mut impl BufRead) -> std::io::Result<Option<HookRequest>> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else { return Ok(None) };
    let (method, path) = (method.to_string(), path.to_string());
    let mut length = 0_usize;
    let mut has_origin = false;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse().unwrap_or(0),
                "origin" => has_origin = true,
                _ => {}
            }
        }
    }
    if length > MAX_BODY {
        return Ok(None);
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(HookRequest { method, path, has_origin, body }))
}

/// "12 s", "3 min", "1 h 05 min": how long a turn has been running.
pub fn format_elapsed(elapsed: chrono::TimeDelta) -> String {
    let seconds = elapsed.num_seconds().max(0);
    if seconds < 60 {
        format!("{seconds} s")
    } else if seconds < 3600 {
        format!("{} min", seconds / 60)
    } else {
        format!("{} h {:02} min", seconds / 3600, (seconds % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://127.0.0.1:47833/vnotch/claude";

    fn event(name: &str, extra: Value) -> Value {
        let mut payload = json!({"session_id": "s1", "hook_event_name": name, "cwd": "C:\\work\\proj"});
        if let (Some(target), Some(source)) = (payload.as_object_mut(), extra.as_object()) {
            target.extend(source.clone());
        }
        payload
    }

    #[test]
    fn install_adds_every_event_and_keeps_foreign_settings() {
        let mut settings = json!({"model": "opus", "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x"}]}]}});
        install_hooks_into(&mut settings, URL).unwrap();
        assert!(hooks_installed_in(&settings, URL));
        assert_eq!(settings["model"], "opus");
        assert_eq!(settings["hooks"]["Stop"][0]["hooks"][0]["command"], "x");
        assert_eq!(settings["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(settings["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"], PERMISSION_TIMEOUT);
    }

    #[test]
    fn install_twice_changes_nothing() {
        let mut once = json!({});
        install_hooks_into(&mut once, URL).unwrap();
        let mut twice = once.clone();
        install_hooks_into(&mut twice, URL).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn uninstall_restores_the_original() {
        let original = json!({"model": "opus", "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x"}]}]}});
        let mut settings = original.clone();
        install_hooks_into(&mut settings, URL).unwrap();
        uninstall_hooks_from(&mut settings, URL);
        assert_eq!(settings, original);
        let mut clean = json!({});
        install_hooks_into(&mut clean, URL).unwrap();
        uninstall_hooks_from(&mut clean, URL);
        assert_eq!(clean, json!({}));
    }

    #[test]
    fn install_refuses_unexpected_shapes() {
        assert!(install_hooks_into(&mut json!({"hooks": 5}), URL).is_err());
        assert!(install_hooks_into(&mut json!({"hooks": {"Stop": 5}}), URL).is_err());
    }

    #[test]
    fn session_moves_through_a_turn() {
        let tracker = Tracker::default();
        tracker.handle(&event("SessionStart", json!({})));
        tracker.handle(&event("UserPromptSubmit", json!({"prompt": "Mach **das**"})));
        let (sessions, _) = tracker.snapshot();
        assert_eq!(sessions[0].status, Status::Working);
        assert_eq!(sessions[0].prompt.as_deref(), Some("Mach das"));
        assert_eq!(sessions[0].project(), "proj");

        tracker.handle(&event("PreToolUse", json!({"tool_name": "Bash", "tool_input": {"command": "npm test"}})));
        assert_eq!(tracker.snapshot().0[0].activity.as_deref(), Some("Bash: npm test"));

        tracker.handle(&event("Stop", json!({"last_assistant_message": "Fertig."})));
        let (sessions, _) = tracker.snapshot();
        assert_eq!(sessions[0].status, Status::Done);
        assert_eq!(sessions[0].summary.as_deref(), Some("Fertig."));
        assert_eq!(tracker.take_attention(), (false, true));
        assert_eq!(tracker.take_attention(), (false, false));
    }

    #[test]
    fn permission_request_is_held_until_decided() {
        let tracker = Tracker::default();
        let receiver = tracker
            .handle(&event("PermissionRequest", json!({
                "tool_name": "Bash",
                "tool_input": {"command": "rm x"},
                "permission_suggestions": [{"rules": [{"toolName": "Bash", "ruleContent": "rm:*"}]}]
            })))
            .unwrap();
        let (sessions, pending) = tracker.snapshot();
        assert_eq!(sessions[0].status, Status::Waiting);
        assert_eq!(pending[0].detail.as_deref(), Some("rm x"));
        assert_eq!(pending[0].always_label.as_deref(), Some("Bash(rm:*)"));
        assert!(receiver.try_recv().is_err());

        tracker.decide(pending[0].id, Decision::Always);
        let answer: Value = serde_json::from_str(&receiver.try_recv().unwrap()).unwrap();
        assert_eq!(answer["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert_eq!(answer["hookSpecificOutput"]["decision"]["updatedPermissions"][0]["rules"][0]["toolName"], "Bash");
        assert!(tracker.snapshot().1.is_empty());
        assert_eq!(tracker.snapshot().0[0].status, Status::Working);
    }

    #[test]
    fn denying_sends_a_message() {
        let body: Value = serde_json::from_str(&decision_body(Decision::Deny, &Value::Null)).unwrap();
        assert_eq!(body["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(body["hookSpecificOutput"]["hookEventName"], "PermissionRequest");
    }

    #[test]
    fn answered_in_the_terminal_releases_the_hold_without_decision() {
        let tracker = Tracker::default();
        let input = json!({"command": "ls"});
        let receiver = tracker
            .handle(&event("PermissionRequest", json!({"tool_name": "Bash", "tool_input": input})))
            .unwrap();
        tracker.handle(&event("PostToolUse", json!({"tool_name": "Bash", "tool_input": input})));
        assert_eq!(receiver.try_recv().unwrap(), NO_DECISION);
        assert!(tracker.snapshot().1.is_empty());
    }

    #[test]
    fn new_prompt_and_stop_release_holds() {
        for next in ["UserPromptSubmit", "Stop"] {
            let tracker = Tracker::default();
            let receiver = tracker.handle(&event("PermissionRequest", json!({"tool_name": "Bash"}))).unwrap();
            tracker.handle(&event(next, json!({})));
            assert_eq!(receiver.try_recv().unwrap(), NO_DECISION, "{next}");
        }
    }

    #[test]
    fn session_end_forgets_the_session() {
        let tracker = Tracker::default();
        tracker.handle(&event("SessionStart", json!({})));
        tracker.handle(&event("SessionEnd", json!({})));
        assert!(tracker.snapshot().0.is_empty());
    }

    #[test]
    fn questions_are_parsed_and_answered() {
        let input = json!({"questions": [{
            "question": "Welche?", "header": "Wahl", "multiSelect": false,
            "options": [{"label": "A", "description": "erste"}, {"label": "B"}]
        }]});
        let questions = parse_questions(&input).unwrap();
        assert_eq!(questions[0].options.len(), 2);
        assert_eq!(questions[0].options[0].description, "erste");
        assert!(parse_questions(&json!({"questions": [{"question": "x", "options": []}]})).is_none());

        let mut answers = HashMap::new();
        answers.insert("Welche?".to_string(), "A".to_string());
        let body: Value = serde_json::from_str(&answer_body(&input, &answers)).unwrap();
        let updated = &body["hookSpecificOutput"]["decision"]["updatedInput"];
        assert_eq!(updated["answers"]["Welche?"], "A");
        assert_eq!(updated["questions"][0]["question"], "Welche?");
    }

    #[test]
    fn tools_are_described_briefly() {
        assert_eq!(describe_tool("Read", &json!({"file_path": "C:\\a\\b.rs"})), "Read: b.rs");
        assert_eq!(describe_tool("Edit", &json!({"file_path": "/a/b.rs"})), "Edit: b.rs");
        assert_eq!(describe_tool("Unknown", &json!({})), "Unknown");
        assert_eq!(describe_tool("", &json!({})), "");
        let long = "x".repeat(200);
        assert!(describe_tool("Bash", &json!({"command": long})).chars().count() < 95);
    }

    #[test]
    fn project_name_works_for_every_path_style() {
        assert_eq!(project_name(r"C:\work\proj"), "proj");
        assert_eq!(project_name("/home/me/proj/"), "proj");
        assert_eq!(project_name(r"C:\"), "Claude");
        assert_eq!(project_name("/"), "Claude");
        assert_eq!(project_name(""), "Claude");
    }

    #[test]
    fn titles_prefer_the_custom_name() {
        let tail = concat!(
            r#"{"type":"ai-title","aiTitle":"Generated"}"#, "\n",
            r#"{"type":"custom-title","customTitle":"Mine"}"#, "\n",
            r#"{"type":"ai-title","aiTitle":"Newer"}"#
        );
        assert_eq!(find_title(tail).as_deref(), Some("Mine"));
        assert_eq!(find_title(r#"{"type":"ai-title","aiTitle":"Only"}"#).as_deref(), Some("Only"));
        assert_eq!(find_title("not json"), None);
    }

    #[test]
    fn turn_start_skips_tool_results_and_side_chains() {
        let tail = concat!(
            r#"{"type":"user","timestamp":"2026-01-01T10:00:00Z","message":{"content":"Prompt"}}"#, "\n",
            r#"{"type":"user","timestamp":"2026-01-01T10:05:00Z","isSidechain":true,"message":{"content":"side"}}"#, "\n",
            r#"{"type":"user","timestamp":"2026-01-01T10:06:00Z","message":{"content":[{"type":"tool_result"}]}}"#, "\n",
            r#"{"type":"assistant","timestamp":"2026-01-01T10:07:00Z"}"#
        );
        let start = find_turn_start(tail).unwrap();
        assert_eq!(start.to_rfc3339(), "2026-01-01T10:00:00+00:00");
    }

    #[test]
    fn limits_are_read_and_clamped() {
        let limits = parse_limits(&json!({
            "five_hour": {"utilization": 42.5, "resets_at": "2026-01-01T12:00:00Z"},
            "seven_day": {"utilization": 150.0}
        }))
        .unwrap();
        assert_eq!(limits.five_hour.unwrap().used_percent, 42.5);
        assert_eq!(limits.week.unwrap().used_percent, 100.0);
        assert!(parse_limits(&json!({})).is_none());
    }

    #[test]
    fn http_requests_are_parsed() {
        let raw = "POST /vnotch/claude HTTP/1.1\r\nContent-Length: 2\r\nHost: x\r\n\r\n{}";
        let request = read_request(&mut raw.as_bytes()).unwrap().unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, HOOK_PATH);
        assert!(!request.has_origin);
        assert_eq!(request.body, b"{}");
    }

    #[test]
    fn browser_requests_are_flagged_and_huge_bodies_refused() {
        let raw = "POST /x HTTP/1.1\r\nOrigin: https://evil.example\r\nContent-Length: 0\r\n\r\n";
        assert!(read_request(&mut raw.as_bytes()).unwrap().unwrap().has_origin);
        let huge = format!("POST /x HTTP/1.1\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1);
        assert!(read_request(&mut huge.as_bytes()).unwrap().is_none());
    }

    #[test]
    fn server_answers_over_a_real_socket() {
        let tracker = Tracker::default();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let ctx = egui::Context::default();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            let body = r#"{"session_id":"s","hook_event_name":"SessionStart"}"#;
            write!(stream, "POST {HOOK_PATH} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            let mut answer = String::new();
            stream.read_to_string(&mut answer).unwrap();
            answer
        });
        let (stream, _) = listener.accept().unwrap();
        serve(stream, &tracker, &ctx).unwrap();
        let answer = client.join().unwrap();
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert_eq!(tracker.snapshot().0.len(), 1);
    }

    #[test]
    fn elapsed_is_readable() {
        assert_eq!(format_elapsed(chrono::TimeDelta::seconds(12)), "12 s");
        assert_eq!(format_elapsed(chrono::TimeDelta::seconds(180)), "3 min");
        assert_eq!(format_elapsed(chrono::TimeDelta::seconds(3900)), "1 h 05 min");
        assert_eq!(format_elapsed(chrono::TimeDelta::seconds(-5)), "0 s");
    }
}
