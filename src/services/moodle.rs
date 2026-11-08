//! The school's Moodle, read through its web service REST API (the one the Moodle app uses). The token
//! comes from the user: pasted from Moodle's security keys page, or fetched once from
//! `login/token.php` with name and password, which are then forgotten. It is kept encrypted in the
//! platform's secret store, never in the settings file, the log or a URL: every call sends it in
//! the body of a POST.

use crate::platform;
use crate::settings::SharedSettings;
use chrono::{DateTime, Local, TimeZone};
use eframe::egui;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Name of the token in the secret store.
pub const TOKEN_SECRET: &str = "school.moodle.token";
const REFRESH: Duration = Duration::from_secs(10 * 60);
const RETRY: Duration = Duration::from_secs(90);
const TIMEOUT: Duration = Duration::from_secs(20);
/// How many upcoming events to ask for.
const LIMIT: u32 = 30;

/// Something due in a course: an assignment, a quiz, a forum post…
#[derive(Clone, Debug, PartialEq)]
pub struct Deadline {
    pub id: i64,
    pub name: String,
    pub course: String,
    pub due: DateTime<Local>,
    pub url: String,
    pub module: String,
    pub overdue: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum MoodleStatus {
    /// No Moodle address in the settings.
    #[default]
    NotConfigured,
    /// An address, but no token yet.
    NoToken,
    Loading,
    Ready(Arc<Vec<Deadline>>),
    /// Moodle said no (an expired token, a disabled web service) or could not be reached.
    Failed(String),
}

pub struct MoodleService {
    pub status: Mutex<MoodleStatus>,
    /// The school tab is on screen: only then does the service read Moodle.
    pub wanted: AtomicBool,
    refresh_now: AtomicBool,
}

impl MoodleService {
    pub fn start(ctx: egui::Context, settings: SharedSettings, demo: bool) -> Arc<Self> {
        let service = Arc::new(Self { status: Mutex::new(MoodleStatus::default()), wanted: AtomicBool::new(false), refresh_now: AtomicBool::new(true) });
        if demo {
            service.set(MoodleStatus::Ready(Arc::new(demo_deadlines())));
            return service;
        }
        let worker = service.clone();
        let _ = std::thread::Builder::new().name("moodle".into()).spawn(move || worker.run(&settings, &ctx));
        service
    }

    /// Read again soon, e.g. after a new token was saved.
    pub fn refresh(&self) {
        self.refresh_now.store(true, Ordering::Relaxed);
    }

    fn set(&self, status: MoodleStatus) {
        if let Ok(mut current) = self.status.lock() {
            *current = status;
        }
    }

    fn run(&self, settings: &SharedSettings, ctx: &egui::Context) {
        let mut next: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let base = settings.read(|s| if s.enable_local_only_mode { String::new() } else { normalize_base(&s.school_moodle_url) });
            if base.is_empty() {
                self.set(MoodleStatus::NotConfigured);
                continue;
            }
            let forced = self.refresh_now.swap(false, Ordering::Relaxed);
            let due = next.is_none_or(|at| Instant::now() >= at);
            if !forced && !(due && self.wanted.load(Ordering::Relaxed)) {
                continue;
            }
            let Some(token) = platform::secret_get(TOKEN_SECRET).filter(|t| !t.is_empty()) else {
                self.set(MoodleStatus::NoToken);
                next = None;
                continue;
            };
            if !matches!(self.status.lock().map(|s| s.clone()), Ok(MoodleStatus::Ready(_))) {
                self.set(MoodleStatus::Loading);
                ctx.request_repaint();
            }
            match upcoming(&base, &token) {
                Ok(list) => {
                    self.set(MoodleStatus::Ready(Arc::new(list)));
                    next = Some(Instant::now() + REFRESH);
                }
                Err(error) => {
                    crate::vlog!("MOODLE", "refresh failed: {error}");
                    self.set(MoodleStatus::Failed(error));
                    next = Some(Instant::now() + RETRY);
                }
            }
            ctx.request_repaint();
        }
    }
}

/// "moodle.school.at/" → "https://moodle.school.at"; empty stays empty.
pub fn normalize_base(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    if url.is_empty() {
        String::new()
    } else if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("https://{url}")
    }
}

fn post_form(url: &str, fields: &[(&str, &str)]) -> Result<Value, String> {
    let host = url.split('/').nth(2).unwrap_or("?").to_string();
    let mut response = crate::services::http::agent(TIMEOUT)
        .post(url)
        .header("User-Agent", concat!("V-Notch-Rust/", env!("CARGO_PKG_VERSION")))
        .send_form(fields.iter().copied())
        // The error text of the transport never carries the body, so the token cannot leak into it.
        .map_err(|e| format!("{host}: {e}"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status} from {host}"));
    }
    response.body_mut().read_json().map_err(|_| format!("{host} did not answer with JSON"))
}

/// Swaps name and password for a token of the Moodle app's web service. Neither is kept.
pub fn fetch_token(base: &str, username: &str, password: &str) -> Result<String, String> {
    let reply = post_form(&format!("{base}/login/token.php"), &[("username", username), ("password", password), ("service", "moodle_mobile_app")])?;
    parse_token(&reply)
}

pub fn parse_token(reply: &Value) -> Result<String, String> {
    if let Some(token) = reply["token"].as_str().filter(|t| !t.is_empty()) {
        return Ok(token.to_string());
    }
    Err(reply["error"].as_str().unwrap_or("Moodle gave no token").to_string())
}

/// The page where the Moodle app signs in, also through the school's Microsoft login. At the end the
/// browser is sent to a `moodlemobile://token=…` link; the user copies that link into the settings.
pub fn browser_login_url(base: &str) -> String {
    let passport = (chrono::Utc::now().timestamp_subsec_nanos() % 1_000_000).to_string();
    format!("{base}/admin/tool/mobile/launch.php?service=moodle_mobile_app&passport={passport}&urlscheme=moodlemobile")
}

/// What the user pasted: a plain token, or the `moodlemobile://token=…` link of the browser sign-in
/// (base64 of "signature:::token" or "signature:::token:::privatetoken").
pub fn token_from_input(input: &str) -> Option<String> {
    let input = input.trim();
    let encoded = input.split_once("token=").map(|(_, rest)| rest).unwrap_or(input);
    let encoded = encoded.trim_end_matches('/');
    if input.contains("token=") || encoded.len() > 40 {
        let decoded = base64_decode(encoded)?;
        let text = String::from_utf8(decoded).ok()?;
        let token = text.split(":::").nth(1)?.trim().to_string();
        return (!token.is_empty() && token.chars().all(|c| c.is_ascii_alphanumeric())).then_some(token);
    }
    (!input.is_empty() && input.chars().all(|c| c.is_ascii_alphanumeric())).then(|| input.to_string())
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = text.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut buffer = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            buffer |= value(*c)? << (18 - 6 * i);
        }
        let bytes = [(buffer >> 16) as u8, (buffer >> 8) as u8, buffer as u8];
        out.extend_from_slice(&bytes[..chunk.len().saturating_sub(1)]);
    }
    Some(out)
}

/// What is due from now on, soonest first.
pub fn upcoming(base: &str, token: &str) -> Result<Vec<Deadline>, String> {
    let from = chrono::Utc::now().timestamp().to_string();
    let limit = LIMIT.to_string();
    let reply = post_form(
        &format!("{base}/webservice/rest/server.php"),
        &[
            ("wstoken", token),
            ("wsfunction", "core_calendar_get_action_events_by_timesort"),
            ("moodlewsrestformat", "json"),
            ("timesortfrom", &from),
            ("limitnum", &limit),
        ],
    )?;
    parse_events(&reply)
}

/// The events of `core_calendar_get_action_events_by_timesort`, or Moodle's own error message.
pub fn parse_events(reply: &Value) -> Result<Vec<Deadline>, String> {
    if reply.get("exception").is_some() || reply.get("errorcode").is_some() {
        return Err(reply["message"].as_str().or(reply["errorcode"].as_str()).unwrap_or("Moodle error").to_string());
    }
    let events = reply["events"].as_array().ok_or("Moodle sent no events")?;
    let mut list: Vec<Deadline> = events
        .iter()
        .filter_map(|event| {
            let due = Local.timestamp_opt(event["timesort"].as_i64()?, 0).single()?;
            let course = &event["course"];
            let course = course["shortname"].as_str().filter(|s| !s.is_empty()).or(course["fullname"].as_str()).unwrap_or_default();
            Some(Deadline {
                id: event["id"].as_i64().unwrap_or_default(),
                name: clean_name(event["activityname"].as_str().or(event["name"].as_str()).unwrap_or_default()),
                course: course.to_string(),
                due,
                url: event["url"].as_str().unwrap_or_default().to_string(),
                module: event["modulename"].as_str().unwrap_or_default().to_string(),
                overdue: event["overdue"].as_bool().unwrap_or(false),
            })
        })
        .collect();
    list.sort_by_key(|d| d.due);
    Ok(list)
}

/// Moodle calls an assignment's event "Hausübung 3 is due" / "… ist fällig"; the tab wants the name.
fn clean_name(name: &str) -> String {
    let name = name.trim();
    for suffix in [" is due", " ist fällig", " ist zu erledigen", " closes", " schließt"] {
        if let Some(stripped) = name.strip_suffix(suffix) {
            return stripped.trim().to_string();
        }
    }
    name.to_string()
}

/// "DBI 3AHIF Aistleitner 2627" → "DBI", as the original shortens courses.
pub fn short_course(course: &str) -> &str {
    course.split(' ').next().filter(|word| !word.is_empty()).unwrap_or(course)
}

fn demo_deadlines() -> Vec<Deadline> {
    let now = Local::now();
    let item = |id: i64, name: &str, course: &str, hours: i64| Deadline {
        id,
        name: name.into(),
        course: course.into(),
        due: now + chrono::Duration::hours(hours),
        url: String::new(),
        module: "assign".into(),
        overdue: false,
    };
    vec![
        item(1, "Hausübung 4 – Normalformen", "DBI 3AHIT", 52),
        item(2, "Übung 06: Rekursion", "POS 3AHIT", 20),
        item(3, "Protokoll Messung", "NW 3AHIT", 120),
        item(4, "Präsentation Netzwerke", "SYT 3AHIT", 200),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_become_deadlines_soonest_first() {
        let reply = json!({
            "events": [
                { "id": 7, "name": "HÜ 2 is due", "activityname": "HÜ 2", "timesort": 2_000_000_100_i64, "modulename": "assign",
                  "url": "https://m.example/mod/assign/view.php?id=7", "course": { "shortname": "DBI", "fullname": "DBI 3AHIT" } },
                { "id": 3, "name": "Quiz 1 closes", "timesort": 2_000_000_000_i64, "modulename": "quiz", "overdue": true,
                  "course": { "fullname": "Mathe 3AHIT" } }
            ],
            "firstid": 3, "lastid": 7
        });
        let list = parse_events(&reply).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Quiz 1");
        assert_eq!(list[0].course, "Mathe 3AHIT");
        assert!(list[0].overdue);
        assert_eq!(list[1].name, "HÜ 2");
        assert_eq!(list[1].course, "DBI");
        assert_eq!(list[1].due.timestamp(), 2_000_000_100);
    }

    #[test]
    fn moodle_errors_come_through() {
        let reply = json!({ "exception": "moodle_exception", "errorcode": "invalidtoken", "message": "Invalid token - token not found" });
        assert_eq!(parse_events(&reply), Err("Invalid token - token not found".to_string()));
        assert!(parse_events(&json!({ "nothing": 1 })).is_err());
    }

    #[test]
    fn token_replies() {
        assert_eq!(parse_token(&json!({ "token": "abc123", "privatetoken": null })), Ok("abc123".into()));
        assert_eq!(parse_token(&json!({ "error": "Invalid login, please try again", "errorcode": "invalidlogin" })), Err("Invalid login, please try again".into()));
    }

    #[test]
    fn tokens_from_the_browser_link() {
        // base64 of "a1b2c3:::0123456789abcdef0123456789abcdef:::private" (made up)
        let link = "moodlemobile://token=YTFiMmMzOjo6MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY6Ojpwcml2YXRl";
        assert_eq!(token_from_input(link).as_deref(), Some("0123456789abcdef0123456789abcdef"));
        assert_eq!(token_from_input(" 0123456789abcdef0123456789abcdef ").as_deref(), Some("0123456789abcdef0123456789abcdef"));
        assert_eq!(token_from_input("not a token!"), None);
        assert!(browser_login_url("https://moodle.example-school.at").starts_with("https://moodle.example-school.at/admin/tool/mobile/launch.php?service=moodle_mobile_app&passport="));
    }

    #[test]
    fn addresses_and_courses() {
        assert_eq!(normalize_base(" moodle.example-school.at/ "), "https://moodle.example-school.at");
        assert_eq!(normalize_base("http://10.0.0.2/moodle"), "http://10.0.0.2/moodle");
        assert_eq!(normalize_base(""), "");
        assert_eq!(short_course("DBI 3AHIF Aistleitner 2627"), "DBI");
        assert_eq!(clean_name("Hausübung 3 ist fällig"), "Hausübung 3");
    }
}
