//! The school's WebUntis: the user's own timetable (lessons that are cancelled or changed) and the
//! tests and exams ahead. Untis has no token for this, so the password is kept encrypted in the
//! platform's secret store and used to sign in for each refresh; the session is closed again right
//! after. Neither the password nor the session ever goes into a URL, the log or an error message.

use crate::platform;
use crate::settings::SharedSettings;
use chrono::{Datelike, Duration as Days, Local, NaiveDate, NaiveTime};
use eframe::egui;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Name of the password in the secret store.
pub const PASSWORD_SECRET: &str = "school.untis.password";
const REFRESH: Duration = Duration::from_secs(20 * 60);
const RETRY: Duration = Duration::from_secs(120);
const TIMEOUT: Duration = Duration::from_secs(20);
/// Exams this far ahead, as the original's tiles (three weeks).
const EXAM_DAYS: i64 = 21;
/// Timetable changes this far ahead.
const CHANGE_DAYS: i64 = 7;
const CLIENT: &str = "V-Notch";

#[derive(Clone, Debug, PartialEq)]
pub struct Exam {
    pub date: NaiveDate,
    pub start: Option<NaiveTime>,
    pub subject: String,
    /// "Test", "Schularbeit", …
    pub kind: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub date: NaiveDate,
    pub start: Option<NaiveTime>,
    pub subject: String,
    pub cancelled: bool,
    /// Untis' note on the change ("Vertretung", "Raumänderung", …), may be empty.
    pub note: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timetable {
    pub exams: Vec<Exam>,
    pub changes: Vec<Change>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum UntisStatus {
    /// Server, school or user missing in the settings.
    #[default]
    NotConfigured,
    NoPassword,
    Loading,
    Ready(Arc<Timetable>),
    Failed(String),
}

pub struct UntisService {
    pub status: Mutex<UntisStatus>,
    pub wanted: AtomicBool,
    refresh_now: AtomicBool,
}

/// Where and as whom to sign in.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub server: String,
    pub school: String,
    pub user: String,
}

impl Account {
    fn from_settings(server: &str, school: &str, user: &str) -> Option<Self> {
        let server = server.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
        let server = server.split('/').next().unwrap_or_default();
        let (school, user) = (school.trim(), user.trim());
        (!server.is_empty() && !school.is_empty() && !user.is_empty()).then(|| Self { server: server.into(), school: school.into(), user: user.into() })
    }

    /// Without a server the school is looked up in Untis' own school search (what the Untis app does).
    fn from_settings_or_search(server: &str, school: &str, user: &str, found: &mut Option<(String, Account)>) -> Result<Option<Self>, String> {
        if let Some(account) = Self::from_settings(server, school, user) {
            return Ok(Some(account));
        }
        let (school, user) = (school.trim(), user.trim());
        if !server.trim().is_empty() || school.is_empty() || user.is_empty() {
            return Ok(None);
        }
        let key = format!("{school}\n{user}");
        if let Some((cached, account)) = found
            && *cached == key
        {
            return Ok(Some(account.clone()));
        }
        let (server, login) = search_school(school)?;
        let account = Self { server, school: login, user: user.into() };
        *found = Some((key, account.clone()));
        Ok(Some(account))
    }

    fn rpc_url(&self) -> String {
        format!("https://{}/WebUntis/jsonrpc.do?school={}", self.server, crate::services::http::encode(&self.school))
    }
}

impl UntisService {
    pub fn start(ctx: egui::Context, settings: SharedSettings, demo: bool) -> Arc<Self> {
        let service = Arc::new(Self { status: Mutex::new(UntisStatus::default()), wanted: AtomicBool::new(false), refresh_now: AtomicBool::new(true) });
        if demo {
            service.set(UntisStatus::Ready(Arc::new(demo_timetable())));
            return service;
        }
        let worker = service.clone();
        let _ = std::thread::Builder::new().name("untis".into()).spawn(move || worker.run(&settings, &ctx));
        service
    }

    pub fn refresh(&self) {
        self.refresh_now.store(true, Ordering::Relaxed);
    }

    fn set(&self, status: UntisStatus) {
        if let Ok(mut current) = self.status.lock() {
            *current = status;
        }
    }

    fn run(&self, settings: &SharedSettings, ctx: &egui::Context) {
        let mut next: Option<Instant> = None;
        let mut found: Option<(String, Account)> = None;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let fields = settings.read(|s| {
                (!s.enable_local_only_mode).then(|| (s.school_untis_server.clone(), s.school_untis_school.clone(), s.school_untis_user.clone()))
            });
            let Some((server, school, user)) = fields else {
                self.set(UntisStatus::NotConfigured);
                continue;
            };
            if Account::from_settings(&server, &school, &user).is_none() && (school.trim().is_empty() || user.trim().is_empty()) {
                self.set(UntisStatus::NotConfigured);
                continue;
            }
            let forced = self.refresh_now.swap(false, Ordering::Relaxed);
            let due = next.is_none_or(|at| Instant::now() >= at);
            if !forced && !(due && self.wanted.load(Ordering::Relaxed)) {
                continue;
            }
            let account = match Account::from_settings_or_search(&server, &school, &user, &mut found) {
                Ok(Some(account)) => account,
                Ok(None) => {
                    self.set(UntisStatus::NotConfigured);
                    continue;
                }
                Err(error) => {
                    crate::vlog!("UNTIS", "school search failed: {error}");
                    self.set(UntisStatus::Failed(error));
                    next = Some(Instant::now() + RETRY);
                    ctx.request_repaint();
                    continue;
                }
            };
            let Some(password) = platform::secret_get(PASSWORD_SECRET).filter(|p| !p.is_empty()) else {
                self.set(UntisStatus::NoPassword);
                next = None;
                continue;
            };
            if !matches!(self.status.lock().map(|s| s.clone()), Ok(UntisStatus::Ready(_))) {
                self.set(UntisStatus::Loading);
                ctx.request_repaint();
            }
            match read(&account, &password) {
                Ok(timetable) => {
                    self.set(UntisStatus::Ready(Arc::new(timetable)));
                    next = Some(Instant::now() + REFRESH);
                }
                Err(error) => {
                    crate::vlog!("UNTIS", "refresh failed: {error}");
                    self.set(UntisStatus::Failed(error));
                    next = Some(Instant::now() + RETRY);
                }
            }
            ctx.request_repaint();
        }
    }
}

/// Finds a school by name in Untis' school search: its server and its login name.
fn search_school(query: &str) -> Result<(String, String), String> {
    let body = json!({ "id": "1", "method": "searchSchool", "params": [{ "search": query }], "jsonrpc": "2.0" });
    let mut response = crate::services::http::agent(TIMEOUT)
        .post("https://mobile.webuntis.com/ms/schoolquery2")
        .header("User-Agent", CLIENT)
        .send_json(&body)
        .map_err(|_| "Untis school search not reachable".to_string())?;
    let reply: Value = response.body_mut().read_json().map_err(|_| "Untis school search did not answer".to_string())?;
    pick_school(&rpc_result(reply)?, query)
}

/// The school whose login name or display name matches `query`, else the only hit.
pub fn pick_school(result: &Value, query: &str) -> Result<(String, String), String> {
    let schools = result["schools"].as_array().cloned().unwrap_or_default();
    let query = query.trim().to_lowercase();
    let exact = schools.iter().find(|s| {
        [s["loginName"].as_str(), s["displayName"].as_str()].into_iter().flatten().any(|name| name.trim().to_lowercase() == query)
    });
    let school = exact.or(if schools.len() == 1 { schools.first() } else { None }).ok_or_else(|| {
        if schools.is_empty() { "school not found – check the school name or enter the Untis server".to_string() } else { "several schools match – enter the exact login name or the Untis server".to_string() }
    })?;
    let server = school["server"].as_str().unwrap_or_default().trim_end_matches('/').to_string();
    let login = school["loginName"].as_str().unwrap_or_default().to_string();
    if server.is_empty() || login.is_empty() {
        return Err("Untis gave no server for this school".into());
    }
    Ok((server, login))
}

/// One signed-in session: the JSESSIONID cookie Untis sets.
struct Session {
    cookie: String,
    person_type: i64,
    person_id: i64,
    class_id: i64,
}

fn rpc(account: &Account, cookie: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
    let body = json!({ "id": "1", "method": method, "params": params, "jsonrpc": "2.0" });
    let mut request = crate::services::http::agent(TIMEOUT).post(&account.rpc_url()).header("User-Agent", CLIENT);
    if let Some(cookie) = cookie {
        request = request.header("Cookie", cookie);
    }
    let mut response = request.send_json(&body).map_err(|_| format!("{} not reachable", account.server))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status} from {}", account.server));
    }
    let reply: Value = response.body_mut().read_json().map_err(|_| format!("{} did not answer with JSON", account.server))?;
    rpc_result(reply)
}

/// The `result` of a JSON-RPC reply, or Untis' own error message.
pub fn rpc_result(reply: Value) -> Result<Value, String> {
    if let Some(error) = reply.get("error").filter(|e| !e.is_null()) {
        let message = error["message"].as_str().unwrap_or("Untis error");
        return Err(match error["code"].as_i64() {
            Some(-8504) => "wrong user name or password".to_string(),
            _ => message.to_string(),
        });
    }
    reply.get("result").cloned().ok_or_else(|| "Untis sent no result".to_string())
}

fn sign_in(account: &Account, password: &str) -> Result<Session, String> {
    let result = rpc(account, None, "authenticate", json!({ "user": account.user, "password": password, "client": CLIENT }))?;
    let session = result["sessionId"].as_str().ok_or("Untis sent no session")?;
    Ok(Session {
        cookie: format!("JSESSIONID={session}; schoolname=\"_{}\"", account.school),
        person_type: result["personType"].as_i64().unwrap_or(5),
        person_id: result["personId"].as_i64().unwrap_or_default(),
        class_id: result["klasseId"].as_i64().unwrap_or_default(),
    })
}

fn untis_date(date: NaiveDate) -> i64 {
    i64::from(date.year()) * 10_000 + i64::from(date.month()) * 100 + i64::from(date.day())
}

/// Signs in, reads timetable changes and exams, and signs out again.
fn read(account: &Account, password: &str) -> Result<Timetable, String> {
    let session = sign_in(account, password)?;
    let today = Local::now().date_naive();
    let result = (|| {
        let (element_id, element_type) = if session.person_id > 0 { (session.person_id, session.person_type) } else { (session.class_id, 1) };
        let periods = rpc(
            account,
            Some(&session.cookie),
            "getTimetable",
            json!({ "options": {
                "element": { "id": element_id, "type": element_type },
                "startDate": untis_date(today),
                "endDate": untis_date(today + Days::days(EXAM_DAYS)),
                "showLsText": true, "showSubstText": true, "showInfo": true,
                "subjectFields": ["name", "longname"]
            }}),
        )?;
        let mut timetable = Timetable { changes: parse_changes(&periods, today, CHANGE_DAYS), exams: Vec::new() };
        // The exams list of the web view; some schools hide it, then tests come from the lessons' texts.
        timetable.exams = exams_from_api(account, &session, today).unwrap_or_else(|_| exams_from_periods(&periods, today));
        Ok(timetable)
    })();
    let _ = rpc(account, Some(&session.cookie), "logout", json!({}));
    result
}

fn exams_from_api(account: &Account, session: &Session, today: NaiveDate) -> Result<Vec<Exam>, String> {
    let agent = crate::services::http::agent(TIMEOUT);
    let base = format!("https://{}/WebUntis/api", account.server);
    let mut token_reply = agent
        .get(&format!("{base}/token/new"))
        .header("User-Agent", CLIENT)
        .header("Cookie", &session.cookie)
        .call()
        .map_err(|_| "no token".to_string())?;
    let bearer = token_reply.body_mut().read_to_string().map_err(|_| "no token".to_string())?;
    let url = format!(
        "{base}/exams?startDate={}&endDate={}&klasseId={}&withGrades=false",
        untis_date(today),
        untis_date(today + Days::days(EXAM_DAYS)),
        session.class_id
    );
    let mut reply = agent
        .get(&url)
        .header("User-Agent", CLIENT)
        .header("Cookie", &session.cookie)
        .header("Authorization", &format!("Bearer {}", bearer.trim()))
        .call()
        .map_err(|_| "no exams".to_string())?;
    if !(200..300).contains(&reply.status().as_u16()) {
        return Err("no exams".into());
    }
    let value: Value = reply.body_mut().read_json().map_err(|_| "no exams".to_string())?;
    parse_exams(&value, today)
}

fn date_of(value: &Value) -> Option<NaiveDate> {
    let n = value.as_i64()?;
    NaiveDate::from_ymd_opt((n / 10_000) as i32, ((n / 100) % 100) as u32, (n % 100) as u32)
}

fn time_of(value: &Value) -> Option<NaiveTime> {
    let n = value.as_i64()?;
    NaiveTime::from_hms_opt((n / 100) as u32, (n % 100) as u32, 0)
}

/// `/api/exams`: `{"data":{"exams":[{"examDate":20261112,"startTime":800,"subject":"D","examType":"SA","name":"…"}]}}`.
pub fn parse_exams(value: &Value, today: NaiveDate) -> Result<Vec<Exam>, String> {
    let list = value["data"]["exams"].as_array().ok_or("no exams")?;
    let mut exams: Vec<Exam> = list
        .iter()
        .filter_map(|exam| {
            let date = date_of(&exam["examDate"])?;
            (date >= today).then(|| Exam {
                date,
                start: time_of(&exam["startTime"]),
                subject: exam["subject"].as_str().unwrap_or_default().to_string(),
                kind: exam["examType"].as_str().unwrap_or_default().to_string(),
                title: exam["name"].as_str().or(exam["text"].as_str()).unwrap_or_default().to_string(),
            })
        })
        .collect();
    exams.sort_by_key(|e| (e.date, e.start));
    Ok(exams)
}

fn subject_of(period: &Value) -> String {
    period["su"].as_array().and_then(|s| s.first()).and_then(|s| s["name"].as_str()).unwrap_or_default().to_string()
}

/// Cancelled and changed lessons of the next `days` days, soonest first.
pub fn parse_changes(periods: &Value, today: NaiveDate, days: i64) -> Vec<Change> {
    let until = today + Days::days(days);
    let mut changes: Vec<Change> = periods
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|period| {
                    let code = period["code"].as_str().unwrap_or_default();
                    if code != "cancelled" && code != "irregular" {
                        return None;
                    }
                    let date = date_of(&period["date"])?;
                    if date < today || date > until {
                        return None;
                    }
                    let note = period["substText"].as_str().filter(|s| !s.is_empty()).or(period["info"].as_str()).unwrap_or_default();
                    Some(Change { date, start: time_of(&period["startTime"]), subject: subject_of(period), cancelled: code == "cancelled", note: note.to_string() })
                })
                .collect()
        })
        .unwrap_or_default();
    changes.sort_by_key(|c| (c.date, c.start));
    changes.dedup();
    changes
}

/// Tests named in the lessons' texts ("Test", "Schularbeit", "SA", "Prüfung") when the exam list is hidden.
pub fn exams_from_periods(periods: &Value, today: NaiveDate) -> Vec<Exam> {
    let words = ["test", "schularbeit", "prüfung", "klausur", "exam"];
    let mut exams: Vec<Exam> = periods
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|period| {
                    let text = [period["lstext"].as_str(), period["info"].as_str(), period["activityType"].as_str()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" ");
                    let lower = text.to_lowercase();
                    let sa = lower.split(|c: char| !c.is_alphanumeric()).any(|w| w == "sa");
                    let kind = words.iter().find(|w| lower.contains(**w)).map(|w| (*w).to_string()).or(sa.then(|| "SA".into()))?;
                    if period["code"].as_str() == Some("cancelled") {
                        return None;
                    }
                    let date = date_of(&period["date"])?;
                    (date >= today).then(|| Exam {
                        date,
                        start: time_of(&period["startTime"]),
                        subject: subject_of(period),
                        kind: capitalize(&kind),
                        title: text.trim().to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    exams.sort_by_key(|e| (e.date, e.start));
    exams.dedup_by(|a, b| a.date == b.date && a.subject == b.subject);
    exams
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn demo_timetable() -> Timetable {
    let today = Local::now().date_naive();
    let at = |h, m| NaiveTime::from_hms_opt(h, m, 0);
    Timetable {
        exams: vec![
            Exam { date: today + Days::days(2), start: at(8, 0), subject: "AM".into(), kind: "Schularbeit".into(), title: "2. SA".into() },
            Exam { date: today + Days::days(6), start: at(10, 0), subject: "POS".into(), kind: "Test".into(), title: "Rekursion".into() },
            Exam { date: today + Days::days(13), start: at(8, 50), subject: "E".into(), kind: "Test".into(), title: "Vocabulary".into() },
        ],
        changes: vec![Change { date: today + Days::days(1), start: at(11, 45), subject: "NW".into(), cancelled: true, note: String::new() }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(n: i64) -> NaiveDate {
        NaiveDate::from_ymd_opt(2030, 3, 4).unwrap() + Days::days(n)
    }

    #[test]
    fn accounts_need_all_three_fields() {
        let account = Account::from_settings(" https://example.webuntis.com/WebUntis ", "Example School", "student1").unwrap();
        assert_eq!(account.server, "example.webuntis.com");
        assert_eq!(account.rpc_url(), "https://example.webuntis.com/WebUntis/jsonrpc.do?school=Example%20School");
        assert!(Account::from_settings("example.webuntis.com", "", "student1").is_none());
    }

    #[test]
    fn schools_are_found_by_name() {
        let result = json!({ "size": 2, "schools": [
            { "server": "alpha.webuntis.com", "loginName": "Example School", "displayName": "Example School, Sample Town" },
            { "server": "beta.webuntis.com", "loginName": "example-other", "displayName": "Other School" }
        ] });
        assert_eq!(pick_school(&result, "example school").unwrap(), ("alpha.webuntis.com".into(), "Example School".into()));
        assert!(pick_school(&result, "example").is_err());
        let one = json!({ "schools": [{ "server": "gamma.webuntis.com", "loginName": "only" }] });
        assert_eq!(pick_school(&one, "whatever").unwrap().0, "gamma.webuntis.com");
        assert!(pick_school(&json!({ "schools": [] }), "x").is_err());
    }

    #[test]
    fn rpc_errors_are_readable() {
        assert_eq!(rpc_result(json!({ "jsonrpc": "2.0", "id": "1", "result": { "sessionId": "X" } })).unwrap()["sessionId"], "X");
        assert_eq!(rpc_result(json!({ "error": { "code": -8504, "message": "bad credentials" } })), Err("wrong user name or password".into()));
    }

    #[test]
    fn exams_from_the_exam_list() {
        let reply = json!({ "data": { "exams": [
            { "examDate": 20300310, "startTime": 1000, "subject": "POS", "examType": "Test", "name": "Rekursion" },
            { "examDate": 20300305, "startTime": 800, "subject": "AM", "examType": "SA", "name": "2. SA" },
            { "examDate": 20300301, "startTime": 800, "subject": "D", "examType": "SA", "name": "past" }
        ] } });
        let exams = parse_exams(&reply, day(0)).unwrap();
        assert_eq!(exams.len(), 2);
        assert_eq!((exams[0].subject.as_str(), exams[0].date), ("AM", day(1)));
        assert_eq!(exams[0].start, NaiveTime::from_hms_opt(8, 0, 0));
    }

    #[test]
    fn changes_and_tests_from_the_timetable() {
        let periods = json!([
            { "date": 20300305, "startTime": 1145, "code": "cancelled", "su": [{ "name": "NW" }] },
            { "date": 20300306, "startTime": 800, "code": "irregular", "substText": "Vertretung", "su": [{ "name": "D" }] },
            { "date": 20300306, "startTime": 850, "su": [{ "name": "AM" }], "lstext": "2. Schularbeit" },
            { "date": 20300320, "startTime": 800, "code": "cancelled", "su": [{ "name": "E" }] },
            { "date": 20300307, "startTime": 800, "su": [{ "name": "POS" }], "info": "SA Teil 2" }
        ]);
        let changes = parse_changes(&periods, day(0), 7);
        assert_eq!(changes.len(), 2);
        assert!(changes[0].cancelled && changes[0].subject == "NW");
        assert_eq!(changes[1].note, "Vertretung");
        let exams = exams_from_periods(&periods, day(0));
        assert_eq!(exams.iter().map(|e| e.subject.as_str()).collect::<Vec<_>>(), ["AM", "POS"]);
        assert_eq!(exams[0].kind, "Schularbeit");
        assert_eq!(exams[1].kind, "SA");
    }
}
