//! What Spotlight can find: apps, files, system commands and the calculator.

use super::calc;
use crate::platform::{self, AppEntry, SystemCommand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Launch(AppEntry),
    OpenFile(PathBuf),
    Command(SystemCommand),
    CopyText(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub title: String,
    pub subtitle: String,
    pub icon: &'static str,
    pub icon_path: Option<PathBuf>,
    pub action: Action,
    pub score: f32,
}

impl Hit {
    /// The key usage statistics are stored under.
    pub fn usage_key(&self) -> String {
        match &self.action {
            Action::Launch(app) => format!("app:{}", app.launch),
            Action::OpenFile(path) => format!("file:{}", path.display()),
            Action::Command(command) => format!("cmd:{}", command.id),
            Action::CopyText(_) => String::new(),
        }
    }
}

/// How well `query` matches `text`: exact > prefix > word start > inside > scattered letters. 0 is no match.
pub fn score_text(query: &str, text: &str) -> f32 {
    let query = query.trim().to_lowercase();
    let text = text.to_lowercase();
    if query.is_empty() {
        return 0.0;
    }
    if text == query {
        return 1000.0;
    }
    if text.starts_with(&query) {
        return 800.0 - text.chars().count() as f32 * 0.5;
    }
    let is_word_start = |index: usize| {
        index == 0 || !text[..index].chars().next_back().is_some_and(char::is_alphanumeric)
    };
    if text.match_indices(&query).any(|(index, _)| is_word_start(index)) {
        return 600.0 - text.chars().count() as f32 * 0.5;
    }
    if text.contains(&query) {
        return 400.0 - text.chars().count() as f32 * 0.5;
    }
    // Initials: "vsc" finds "Visual Studio Code".
    let initials: String = text
        .split(|c: char| !c.is_alphanumeric())
        .filter_map(|word| word.chars().next())
        .collect();
    if initials.starts_with(&query) {
        return 350.0;
    }
    if query.chars().count() >= 3 {
        let mut letters = query.chars();
        let mut wanted = letters.next();
        let mut gaps = 0;
        for c in text.chars() {
            match wanted {
                Some(w) if w == c => wanted = letters.next(),
                Some(_) => gaps += 1,
                None => break,
            }
        }
        if wanted.is_none() {
            return (200.0 - gaps as f32 * 4.0).max(10.0);
        }
    }
    0.0
}

const MAX_FILES: usize = 40_000;
const MAX_DEPTH: usize = 6;
const SKIP_DIRS: [&str; 12] = [
    "node_modules", "target", ".git", "__pycache__", ".venv", "venv", "appdata", "library", "dist", "build", ".cache", ".gradle",
];

/// Walks the usual user folders. Hidden folders and build output stay out.
fn collect_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.starts_with('.') || name.starts_with('~') {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                collect_files(&path, depth + 1, out);
            }
        } else if kind.is_file() {
            out.push(path);
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct UsageFile {
    counts: HashMap<String, u32>,
}

#[derive(Default)]
pub struct Index {
    pub apps: Vec<AppEntry>,
    pub files: Vec<PathBuf>,
    pub ready: bool,
}

pub struct Searcher {
    pub index: Arc<Mutex<Index>>,
    usage: Mutex<UsageFile>,
    history_enabled: bool,
}

fn usage_path() -> PathBuf {
    crate::paths::data_dir().join("spotlight-usage.json")
}

impl Searcher {
    pub fn start(history_enabled: bool, demo: bool) -> Arc<Self> {
        let index = Arc::new(Mutex::new(Index::default()));
        let usage = if history_enabled {
            std::fs::read_to_string(usage_path()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
        } else {
            UsageFile::default()
        };
        if demo {
            if let Ok(mut guard) = index.lock() {
                guard.apps = ["Firefox", "Visual Studio Code", "Terminal", "Rechner", "Einstellungen"]
                    .iter()
                    .map(|name| AppEntry { name: (*name).to_string(), launch: (*name).to_string(), icon: None })
                    .collect();
                guard.files = ["Dokumente/Skript.pdf", "Bilder/Urlaub.png"].iter().map(PathBuf::from).collect();
                guard.ready = true;
            }
        } else {
            let worker = index.clone();
            let _ = std::thread::Builder::new().name("spotlight-index".into()).spawn(move || {
                let apps = platform::installed_apps();
                if let Ok(mut guard) = worker.lock() {
                    guard.apps = apps;
                }
                let mut files = Vec::new();
                if let Some(user) = directories::UserDirs::new() {
                    for dir in [user.desktop_dir(), user.document_dir(), user.download_dir(), user.picture_dir(), user.audio_dir(), user.video_dir()]
                        .into_iter()
                        .flatten()
                    {
                        collect_files(dir, 0, &mut files);
                    }
                }
                if let Ok(mut guard) = worker.lock() {
                    guard.files = files;
                    guard.ready = true;
                }
            });
        }
        Arc::new(Self { index, usage: Mutex::new(usage), history_enabled })
    }

    fn boost(&self, key: &str) -> f32 {
        if !self.history_enabled {
            return 0.0;
        }
        self.usage.lock().ok().and_then(|usage| usage.counts.get(key).copied()).map_or(0.0, |count| (count as f32 * 25.0).min(150.0))
    }

    /// Remembers what was opened, so it ranks higher next time. Stored only on this device.
    pub fn record_use(&self, hit: &Hit) {
        let key = hit.usage_key();
        if key.is_empty() || !self.history_enabled {
            return;
        }
        if let Ok(mut usage) = self.usage.lock() {
            *usage.counts.entry(key).or_insert(0) += 1;
            if let Ok(text) = serde_json::to_string(&*usage) {
                let _ = std::fs::write(usage_path(), text);
            }
        }
    }

    pub fn clear_history(&self) {
        if let Ok(mut usage) = self.usage.lock() {
            usage.counts.clear();
        }
        let _ = std::fs::remove_file(usage_path());
    }

    /// Most used apps, for the empty search field.
    pub fn home(&self, limit: usize) -> Vec<Hit> {
        let Ok(index) = self.index.lock() else { return Vec::new() };
        let mut hits: Vec<Hit> = index
            .apps
            .iter()
            .map(|app| {
                let mut hit = app_hit(app, 0.0);
                hit.score = self.boost(&hit.usage_key());
                hit
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.title.cmp(&b.title)));
        hits.truncate(limit);
        hits
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let query = query.trim();
        if query.is_empty() {
            return self.home(limit);
        }
        let mut hits = Vec::new();
        if let Some(value) = calc::evaluate(query) {
            let result = calc::format_result(value);
            hits.push(Hit {
                title: format!("= {result}"),
                subtitle: crate::i18n::t("port.spotlight.calcCopy"),
                icon: crate::icons::CALCULATOR,
                icon_path: None,
                action: Action::CopyText(result),
                score: 2000.0,
            });
        }
        for command in platform::system_commands() {
            let label = crate::i18n::t(command.label_key);
            let score = score_text(query, &label).max(score_keywords(query, command.keywords));
            if score > 0.0 {
                let mut hit = Hit {
                    title: label,
                    subtitle: crate::i18n::t("port.spotlight.system"),
                    icon: command.icon,
                    icon_path: None,
                    action: Action::Command(command),
                    score,
                };
                hit.score += self.boost(&hit.usage_key()) + 40.0;
                hits.push(hit);
            }
        }
        if let Ok(index) = self.index.lock() {
            for app in &index.apps {
                let score = score_text(query, &app.name);
                if score > 0.0 {
                    let mut hit = app_hit(app, score + 100.0);
                    hit.score += self.boost(&hit.usage_key());
                    hits.push(hit);
                }
            }
            if query.chars().count() >= 2 {
                for path in &index.files {
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    let score = score_text(query, &name);
                    if score >= 300.0 {
                        let mut hit = Hit {
                            title: name,
                            subtitle: path.parent().map_or_else(String::new, |p| p.display().to_string()),
                            icon: crate::icons::FILE,
                            icon_path: None,
                            action: Action::OpenFile(path.clone()),
                            score: score - 50.0,
                        };
                        hit.score += self.boost(&hit.usage_key());
                        hits.push(hit);
                    }
                }
            }
        }
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.title.cmp(&b.title)));
        hits.truncate(limit);
        hits
    }
}

fn score_keywords(query: &str, keywords: &str) -> f32 {
    if query.chars().count() < 2 {
        return 0.0;
    }
    keywords.split_whitespace().map(|word| score_text(query, word)).fold(0.0, f32::max) * 0.8
}

fn app_hit(app: &AppEntry, score: f32) -> Hit {
    Hit {
        title: app.name.clone(),
        subtitle: crate::i18n::t("port.spotlight.app"),
        icon: crate::icons::APP_WINDOW,
        icon_path: app.icon.clone(),
        action: Action::Launch(app.clone()),
        score,
    }
}

/// Carries out a hit. True when the window can close afterwards.
pub fn run(hit: &Hit) -> bool {
    match &hit.action {
        Action::Launch(app) => platform::launch_app(app),
        Action::OpenFile(path) => open::that_detached(path).is_ok(),
        Action::Command(command) => platform::run_system_command(command.id),
        Action::CopyText(text) => arboard::Clipboard::new().and_then(|mut board| board.set_text(text.clone())).is_ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn better_matches_score_higher() {
        let exact = score_text("code", "code");
        let prefix = score_text("code", "codeblocks");
        let word = score_text("code", "visual studio code");
        let inner = score_text("ode", "code");
        let initials = score_text("vsc", "Visual Studio Code");
        let scattered = score_text("vstudo", "visual studio code");
        assert!(exact > prefix && prefix > word && word > inner, "{exact} {prefix} {word} {inner}");
        assert!(inner > 0.0 && initials > 0.0 && scattered > 0.0);
        assert!(initials > scattered);
    }

    #[test]
    fn non_matches_score_zero() {
        assert_eq!(score_text("xyz", "firefox"), 0.0);
        assert_eq!(score_text("", "firefox"), 0.0);
        // Two letters must be contiguous, or nearly everything would match.
        assert_eq!(score_text("fx", "firefox"), 0.0);
    }

    #[test]
    fn shorter_names_win_among_equals() {
        assert!(score_text("fire", "firefox") > score_text("fire", "firefox developer edition"));
    }

    #[test]
    fn matching_handles_umlauts_and_multibyte_text() {
        assert!(score_text("räu", "Aufräumen") > 0.0);
        assert!(score_text("日本", "日本語") > 0.0);
        assert!(score_text("語", "日本語") > 0.0);
    }

    #[test]
    fn search_combines_sources() {
        let searcher = Searcher::start(false, true);
        let hits = searcher.search("code", 10);
        assert_eq!(hits[0].title, "Visual Studio Code");
        let calc = searcher.search("2+2", 10);
        assert_eq!(calc[0].title, "= 4");
        assert_eq!(calc[0].action, Action::CopyText("4".into()));
    }

    #[test]
    fn system_commands_are_found_by_keyword() {
        let searcher = Searcher::start(false, true);
        let hits = searcher.search("sperren", 10);
        assert!(hits.iter().any(|h| matches!(&h.action, Action::Command(c) if c.id == "lock")));
    }

    #[test]
    fn files_need_a_good_match() {
        let searcher = Searcher::start(false, true);
        assert!(searcher.search("skript", 10).iter().any(|h| matches!(h.action, Action::OpenFile(_))));
        assert!(!searcher.search("spt", 10).iter().any(|h| matches!(h.action, Action::OpenFile(_))));
    }

    #[test]
    fn empty_query_shows_apps() {
        let searcher = Searcher::start(false, true);
        assert_eq!(searcher.search("", 3).len(), 3);
    }

    #[test]
    fn walking_skips_build_output_and_hidden_folders() {
        let root = std::env::temp_dir().join(format!("vnotch-walk-{}", std::process::id()));
        for dir in ["keep", "node_modules", ".hidden"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join("file.txt"), "x").unwrap();
        }
        let mut files = Vec::new();
        collect_files(&root, 0, &mut files);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(files.len(), 1);
        assert!(files[0].to_string_lossy().contains("keep"));
    }
}
