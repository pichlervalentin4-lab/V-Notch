//! Microsoft Teams through Microsoft Graph: the assignments of the user's classes and the next
//! meetings in their calendar. The app registration is the user's own (client id and tenant in the
//! settings, neither secret). Signing in uses the device code flow: the user signs in in their own
//! browser, so the password never reaches the app. Only the refresh token is kept, encrypted in the
//! platform's secret store; access tokens live in memory. No token ever goes into a URL or the log.

use crate::platform;
use crate::services::moodle::Deadline;
use crate::settings::SharedSettings;
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use eframe::egui;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Name of the refresh token in the secret store.
pub const REFRESH_SECRET: &str = "school.teams.refresh";
const SCOPES: &str = "offline_access User.Read Calendars.Read EduAssignments.ReadBasic";
const GRAPH: &str = "https://graph.microsoft.com/v1.0";
const REFRESH: Duration = Duration::from_secs(15 * 60);
const RETRY: Duration = Duration::from_secs(120);
const TIMEOUT: Duration = Duration::from_secs(20);
/// Meetings this far ahead.
const MEETING_DAYS: i64 = 7;
const MAX_MEETINGS: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct Meeting {
    pub subject: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    /// The Teams join link, or the calendar entry when there is none.
    pub url: String,
    pub online: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TeamsData {
    pub assignments: Vec<Deadline>,
    pub meetings: Vec<Meeting>,
    /// Why the assignments are missing while the meetings came (no school licence, no consent…).
    pub assignments_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum TeamsStatus {
    /// No client id in the settings.
    #[default]
    NotConfigured,
    SignedOut,
    Loading,
    Ready(Arc<TeamsData>),
    Failed(String),
}

/// The steps of a sign-in, shown in the settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum SignIn {
    #[default]
    Starting,
    /// The user enters `code` on `uri` in their browser.
    Code { code: String, uri: String },
    Done(Result<(), String>),
}

pub struct TeamsService {
    pub status: Mutex<TeamsStatus>,
    /// The school tab is on screen: only then does the service read Graph.
    pub wanted: AtomicBool,
    refresh_now: AtomicBool,
}

impl TeamsService {
    pub fn start(ctx: egui::Context, settings: SharedSettings, demo: bool) -> Arc<Self> {
        let service = Arc::new(Self { status: Mutex::new(TeamsStatus::default()), wanted: AtomicBool::new(false), refresh_now: AtomicBool::new(true) });
        if demo {
            return service;
        }
        let worker = service.clone();
        let _ = std::thread::Builder::new().name("teams".into()).spawn(move || worker.run(&settings, &ctx));
        service
    }

    pub fn refresh(&self) {
        self.refresh_now.store(true, Ordering::Relaxed);
    }

    fn set(&self, status: TeamsStatus) {
        if let Ok(mut current) = self.status.lock() {
            *current = status;
        }
    }

    fn run(&self, settings: &SharedSettings, ctx: &egui::Context) {
        let mut next: Option<Instant> = None;
        let mut access: Option<(String, Instant)> = None;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let (local_only, client, tenant) = settings.read(|s| (s.enable_local_only_mode, s.teams_client_id.trim().to_string(), tenant_of(&s.teams_tenant)));
            if local_only || client.is_empty() {
                self.set(TeamsStatus::NotConfigured);
                access = None;
                continue;
            }
            let forced = self.refresh_now.swap(false, Ordering::Relaxed);
            if forced {
                access = None;
            }
            let due = next.is_none_or(|at| Instant::now() >= at);
            if !forced && !(due && self.wanted.load(Ordering::Relaxed)) {
                continue;
            }
            let token = match access.as_ref().filter(|(_, until)| Instant::now() < *until) {
                Some((token, _)) => Ok(token.clone()),
                None => match platform::secret_get(REFRESH_SECRET).filter(|t| !t.is_empty()) {
                    None => {
                        self.set(TeamsStatus::SignedOut);
                        next = None;
                        continue;
                    }
                    Some(refresh) => redeem_refresh(&tenant, &client, &refresh).map(|(token, lifetime)| {
                        access = Some((token.clone(), Instant::now() + lifetime));
                        token
                    }),
                },
            };
            if !matches!(self.status.lock().map(|s| s.clone()), Ok(TeamsStatus::Ready(_))) {
                self.set(TeamsStatus::Loading);
                ctx.request_repaint();
            }
            match token.and_then(|token| read_all(&token)) {
                Ok(data) => {
                    if forced {
                        // Counts only, never names or tokens.
                        crate::vlog!("TEAMS", "{} assignment(s), {} meeting(s)", data.assignments.len(), data.meetings.len());
                    }
                    self.set(TeamsStatus::Ready(Arc::new(data)));
                    next = Some(Instant::now() + REFRESH);
                }
                Err(error) => {
                    crate::vlog!("TEAMS", "refresh failed: {error}");
                    access = None;
                    self.set(TeamsStatus::Failed(error));
                    next = Some(Instant::now() + RETRY);
                }
            }
            ctx.request_repaint();
        }
    }
}

/// The tenant for the sign-in URL; without one any work or school account may sign in.
pub fn tenant_of(setting: &str) -> String {
    let tenant = setting.trim();
    let valid = !tenant.is_empty() && tenant.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    if valid { tenant.to_string() } else { "organizations".to_string() }
}

fn authority(tenant: &str) -> String {
    format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0")
}

/// POSTs a form to the sign-in service; its error replies are JSON too.
fn post_form(url: &str, fields: &[(&str, &str)]) -> Result<Value, String> {
    let mut response = crate::services::http::agent(TIMEOUT)
        .post(url)
        .header("User-Agent", concat!("V-Notch-Rust/", env!("CARGO_PKG_VERSION")))
        .send_form(fields.iter().copied())
        // The transport's error text never carries the body, so no token can leak into it.
        .map_err(|e| format!("login.microsoftonline.com: {e}"))?;
    response.body_mut().read_json().map_err(|_| "Microsoft did not answer with JSON".to_string())
}

/// Microsoft's error text without the trace and correlation ids it appends.
fn sign_in_error(reply: &Value) -> String {
    let text = reply["error_description"].as_str().or(reply["error"].as_str()).unwrap_or("Microsoft sign-in failed");
    text.lines().next().unwrap_or(text).trim().to_string()
}

/// Swaps the refresh token for an access token and keeps the new refresh token Microsoft hands out.
fn redeem_refresh(tenant: &str, client: &str, refresh: &str) -> Result<(String, Duration), String> {
    let reply = post_form(
        &format!("{}/token", authority(tenant)),
        &[("client_id", client), ("grant_type", "refresh_token"), ("refresh_token", refresh), ("scope", SCOPES)],
    )?;
    let (access, rotated, lifetime) = parse_tokens(&reply)?;
    if let Some(rotated) = rotated {
        platform::secret_set(REFRESH_SECRET, &rotated);
    }
    Ok((access, lifetime))
}

/// Access token, refresh token and lifetime of a token reply (renewed a minute early).
pub fn parse_tokens(reply: &Value) -> Result<(String, Option<String>, Duration), String> {
    let Some(access) = reply["access_token"].as_str().filter(|t| !t.is_empty()) else {
        return Err(sign_in_error(reply));
    };
    let refresh = reply["refresh_token"].as_str().filter(|t| !t.is_empty()).map(str::to_string);
    let lifetime = reply["expires_in"].as_u64().unwrap_or(3600).saturating_sub(60).max(60);
    Ok((access.to_string(), refresh, Duration::from_secs(lifetime)))
}

/// Runs the device code sign-in on its own thread: asks for a code, opens Microsoft's page, waits
/// until the user is through, then keeps the refresh token.
pub fn sign_in(tenant: String, client: String, state: Arc<Mutex<SignIn>>, ctx: egui::Context) {
    let finish = move |state: &Mutex<SignIn>, step: SignIn| {
        if let Ok(mut current) = state.lock() {
            *current = step;
        }
        ctx.request_repaint();
    };
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let reply = post_form(&format!("{}/devicecode", authority(&tenant)), &[("client_id", &client), ("scope", SCOPES)])?;
            let (Some(device), Some(code)) = (reply["device_code"].as_str(), reply["user_code"].as_str()) else {
                return Err(sign_in_error(&reply));
            };
            let uri = reply["verification_uri"].as_str().unwrap_or("https://microsoft.com/devicelogin").to_string();
            let mut interval = reply["interval"].as_u64().unwrap_or(5).max(1);
            let deadline = Instant::now() + Duration::from_secs(reply["expires_in"].as_u64().unwrap_or(900));
            finish(&state, SignIn::Code { code: code.to_string(), uri: uri.clone() });
            let _ = open::that_detached(&uri);
            while Instant::now() < deadline {
                std::thread::sleep(Duration::from_secs(interval));
                let reply = post_form(
                    &format!("{}/token", authority(&tenant)),
                    &[("client_id", &client), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("device_code", device)],
                )?;
                match reply["error"].as_str() {
                    Some("authorization_pending") => continue,
                    Some("slow_down") => interval += 5,
                    Some(_) => return Err(sign_in_error(&reply)),
                    None => {
                        let (_, refresh, _) = parse_tokens(&reply)?;
                        let refresh = refresh.ok_or("Microsoft gave no refresh token")?;
                        return if platform::secret_set(REFRESH_SECRET, &refresh) { Ok(()) } else { Err("could not encrypt the sign-in".into()) };
                    }
                }
            }
            Err("the code expired – start the sign-in again".into())
        })();
        finish(&state, SignIn::Done(result));
    });
}

/// GET on Graph; a failure carries Graph's own message.
fn graph_get(token: &str, path: &str) -> Result<Value, String> {
    let mut response = crate::services::http::agent(TIMEOUT)
        .get(&format!("{GRAPH}{path}"))
        .header("User-Agent", concat!("V-Notch-Rust/", env!("CARGO_PKG_VERSION")))
        .header("Authorization", &format!("Bearer {token}"))
        .header("Prefer", "outlook.timezone=\"UTC\"")
        .call()
        .map_err(|e| format!("graph.microsoft.com: {e}"))?;
    let status = response.status().as_u16();
    let body: Value = response.body_mut().read_json().unwrap_or_default();
    if !(200..300).contains(&status) {
        let message = body["error"]["message"].as_str().or(body["error"]["code"].as_str()).unwrap_or("Graph error");
        return Err(format!("HTTP {status}: {message}"));
    }
    Ok(body)
}

fn read_all(token: &str) -> Result<TeamsData, String> {
    let now = Utc::now();
    let end = now + chrono::Duration::days(MEETING_DAYS);
    let fmt = "%Y-%m-%dT%H:%M:%SZ";
    let calendar = graph_get(
        token,
        &format!(
            "/me/calendarView?startDateTime={}&endDateTime={}&$orderby=start/dateTime&$top=20&$select=subject,start,end,isCancelled,isOnlineMeeting,onlineMeeting,webLink",
            now.format(fmt),
            end.format(fmt)
        ),
    )?;
    let meetings = parse_meetings(&calendar, Local::now());
    let (assignments, assignments_error) = match read_assignments(token, Local::now()) {
        Ok(list) => (list, None),
        Err(error) => {
            crate::vlog!("TEAMS", "assignments: {error}");
            (Vec::new(), Some(error))
        }
    };
    Ok(TeamsData { assignments, meetings, assignments_error })
}

fn read_assignments(token: &str, now: DateTime<Local>) -> Result<Vec<Deadline>, String> {
    let classes = graph_get(token, "/education/me/classes?$select=id,displayName")?;
    let names: Vec<(String, String)> = classes["value"]
        .as_array()
        .map(|list| list.iter().filter_map(|c| Some((c["id"].as_str()?.to_string(), c["displayName"].as_str().unwrap_or_default().to_string()))).collect())
        .unwrap_or_default();
    let reply = graph_get(token, "/education/me/assignments")?;
    let mut list = parse_assignments(&reply, &names, now);
    // Leave out what is already handed in; a failed lookup keeps the assignment.
    list.retain(|(_, class, id)| {
        let path = format!("/education/classes/{class}/assignments/{id}/submissions?$select=status");
        !graph_get(token, &path).ok().is_some_and(|reply| submitted(&reply))
    });
    Ok(list.into_iter().map(|(deadline, _, _)| deadline).collect())
}

/// Whether the user's submission is handed in (or already back from the teacher).
pub fn submitted(reply: &Value) -> bool {
    reply["value"].as_array().is_some_and(|list| list.iter().any(|s| matches!(s["status"].as_str(), Some("submitted" | "returned"))))
}

/// Graph's "2026-11-09T07:50:00.0000000" (UTC, as asked for) or "2026-11-09T07:50:00Z".
fn graph_time(value: &Value) -> Option<DateTime<Local>> {
    let text = value.as_str()?.trim_end_matches('Z');
    let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
    Some(Utc.from_utc_datetime(&naive).with_timezone(&Local))
}

/// Published assignments still to come, soonest first, with their class and assignment ids.
pub fn parse_assignments(reply: &Value, classes: &[(String, String)], now: DateTime<Local>) -> Vec<(Deadline, String, String)> {
    let mut list: Vec<(Deadline, String, String)> = reply["value"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|a| a["status"].as_str().is_none_or(|s| s == "assigned"))
                .filter_map(|a| {
                    let due = graph_time(&a["dueDateTime"])?;
                    if due < now {
                        return None;
                    }
                    let id = a["id"].as_str()?.to_string();
                    let class = a["classId"].as_str().unwrap_or_default().to_string();
                    let course = classes.iter().find(|(cid, _)| *cid == class).map(|(_, name)| name.clone()).unwrap_or_else(|| "Teams".into());
                    let deadline = Deadline {
                        id: stable_id(&id),
                        name: a["displayName"].as_str().unwrap_or_default().trim().to_string(),
                        course,
                        due,
                        url: a["webUrl"].as_str().unwrap_or_default().to_string(),
                        module: "teams".into(),
                        overdue: false,
                    };
                    Some((deadline, class, id))
                })
                .collect()
        })
        .unwrap_or_default();
    list.sort_by_key(|(d, _, _)| d.due);
    list
}

/// Meetings that have not ended yet, soonest first; cancelled ones are left out.
pub fn parse_meetings(reply: &Value, now: DateTime<Local>) -> Vec<Meeting> {
    let mut list: Vec<Meeting> = reply["value"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|e| !e["isCancelled"].as_bool().unwrap_or(false))
                .filter_map(|e| {
                    let start = graph_time(&e["start"]["dateTime"])?;
                    let end = graph_time(&e["end"]["dateTime"]).unwrap_or(start);
                    if end < now {
                        return None;
                    }
                    let join = e["onlineMeeting"]["joinUrl"].as_str().filter(|u| !u.is_empty());
                    Some(Meeting {
                        subject: e["subject"].as_str().unwrap_or_default().trim().to_string(),
                        start,
                        end,
                        url: join.or(e["webLink"].as_str()).unwrap_or_default().to_string(),
                        online: join.is_some() || e["isOnlineMeeting"].as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    list.sort_by_key(|m| m.start);
    list.truncate(MAX_MEETINGS);
    list
}

/// Graph ids are long strings; the tab wants a number that stays the same between refreshes.
fn stable_id(id: &str) -> i64 {
    // FNV-1a, so the number does not change between runs or Rust versions.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    (hash >> 1) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at(h: u32) -> DateTime<Local> {
        Utc.with_ymd_and_hms(2026, 11, 8, h, 0, 0).unwrap().with_timezone(&Local)
    }

    #[test]
    fn assignments_from_graph() {
        let reply = json!({ "value": [
            { "id": "a2", "classId": "c1", "displayName": " Protokoll 3 ", "status": "assigned", "dueDateTime": "2026-11-12T22:59:00Z", "webUrl": "https://teams.example/a2" },
            { "id": "a1", "classId": "c9", "displayName": "Übung 1", "status": "assigned", "dueDateTime": "2026-11-09T08:00:00Z" },
            { "id": "a0", "classId": "c1", "displayName": "Alt", "status": "assigned", "dueDateTime": "2026-11-01T08:00:00Z" },
            { "id": "d1", "classId": "c1", "displayName": "Entwurf", "status": "draft", "dueDateTime": "2026-11-20T08:00:00Z" },
            { "id": "n1", "classId": "c1", "displayName": "Ohne Frist", "status": "assigned", "dueDateTime": null }
        ]});
        let classes = vec![("c1".to_string(), "NW 3AHIT".to_string())];
        let list = parse_assignments(&reply, &classes, at(20));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0.name, "Übung 1");
        assert_eq!(list[0].0.course, "Teams");
        assert_eq!(list[1].0.name, "Protokoll 3");
        assert_eq!(list[1].0.course, "NW 3AHIT");
        assert_eq!(list[1].0.url, "https://teams.example/a2");
        assert_eq!((list[1].1.as_str(), list[1].2.as_str()), ("c1", "a2"));
        assert_eq!(list[1].0.id, stable_id("a2"));
        assert!(submitted(&json!({ "value": [{ "status": "submitted" }] })));
        assert!(!submitted(&json!({ "value": [{ "status": "working" }] })));
    }

    #[test]
    fn meetings_from_the_calendar() {
        let reply = json!({ "value": [
            { "subject": "Klassenrat", "start": { "dateTime": "2026-11-09T07:50:00.0000000" }, "end": { "dateTime": "2026-11-09T08:40:00.0000000" },
              "isOnlineMeeting": true, "onlineMeeting": { "joinUrl": "https://teams.example/join" }, "webLink": "https://outlook.example/1" },
            { "subject": "Abgesagt", "isCancelled": true, "start": { "dateTime": "2026-11-09T09:00:00.0000000" }, "end": { "dateTime": "2026-11-09T10:00:00.0000000" } },
            { "subject": "Läuft gerade", "start": { "dateTime": "2026-11-08T19:30:00.0000000" }, "end": { "dateTime": "2026-11-08T20:30:00.0000000" }, "webLink": "https://outlook.example/2" },
            { "subject": "Vorbei", "start": { "dateTime": "2026-11-08T10:00:00.0000000" }, "end": { "dateTime": "2026-11-08T11:00:00.0000000" } }
        ]});
        let list = parse_meetings(&reply, at(20));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].subject, "Läuft gerade");
        assert!(!list[0].online);
        assert_eq!(list[0].url, "https://outlook.example/2");
        assert_eq!(list[1].subject, "Klassenrat");
        assert!(list[1].online);
        assert_eq!(list[1].url, "https://teams.example/join");
    }

    #[test]
    fn tokens_and_tenants() {
        let (access, refresh, lifetime) = parse_tokens(&json!({ "access_token": "acc", "refresh_token": "ref", "expires_in": 3599 })).unwrap();
        assert_eq!((access.as_str(), refresh.as_deref(), lifetime.as_secs()), ("acc", Some("ref"), 3539));
        let error = json!({ "error": "invalid_grant", "error_description": "AADSTS65001: The user or administrator has not consented.\r\nTrace ID: 123" });
        assert_eq!(parse_tokens(&error), Err("AADSTS65001: The user or administrator has not consented.".to_string()));
        assert_eq!(tenant_of(""), "organizations");
        assert_eq!(tenant_of(" example.onmicrosoft.com "), "example.onmicrosoft.com");
        assert_eq!(tenant_of("0c9e1b2a-1111-2222-3333-444455556666"), "0c9e1b2a-1111-2222-3333-444455556666");
        assert_eq!(tenant_of("evil/../x"), "organizations");
    }
}
