//! Clipboard history: watching, classifying, masking secrets and keeping entries on disk.

use crate::settings::SharedSettings;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClipKind {
    #[default]
    Text,
    Link,
    Code,
    Color,
    Email,
    Image,
    Files,
}

impl ClipKind {
    pub fn icon(self) -> &'static str {
        use crate::icons::*;
        match self {
            ClipKind::Text => FILE_TEXT,
            ClipKind::Link => LINK,
            ClipKind::Code => CODE,
            ClipKind::Color => PALETTE,
            ClipKind::Email => ENVELOPE,
            ClipKind::Image => IMAGE,
            ClipKind::Files => FOLDER,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipEntry {
    pub id: u64,
    pub kind: ClipKind,
    pub text: String,
    pub image_file: Option<String>,
    pub files: Vec<String>,
    pub created_unix: u64,
    pub pinned: bool,
    pub labels: Vec<String>,
    /// Masked in the list and never written to disk.
    #[serde(skip)]
    pub sensitive: bool,
    pub hash: u64,
}

fn is_hex_color(text: &str) -> bool {
    let body = text.strip_prefix('#').unwrap_or_default();
    matches!(body.len(), 3 | 4 | 6 | 8) && body.chars().all(|c| c.is_ascii_hexdigit())
}

/// `#rrggbb` and `#rrggbbaa` as RGBA, for the colour swatch.
pub fn parse_color(text: &str) -> Option<[u8; 4]> {
    let text = text.trim();
    if !is_hex_color(text) {
        return None;
    }
    let body = &text[1..];
    let digit = |i: usize, n: usize| u8::from_str_radix(body.get(i..i + n)?, 16).ok();
    match body.len() {
        3 | 4 => {
            let d = |i| digit(i, 1).map(|v| v * 17);
            Some([d(0)?, d(1)?, d(2)?, if body.len() == 4 { d(3)? } else { 255 }])
        }
        6 | 8 => {
            let d = |i| digit(i, 2);
            Some([d(0)?, d(2)?, d(4)?, if body.len() == 8 { d(6)? } else { 255 }])
        }
        _ => None,
    }
}

fn is_url(text: &str) -> bool {
    let lower = text.to_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www."))
        && !text.chars().any(char::is_whitespace)
}

fn is_email(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else { return false };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !text.chars().any(char::is_whitespace)
}

fn looks_like_code(text: &str) -> bool {
    const MARKERS: [&str; 22] = [
        "fn ", "def ", "class ", "public ", "private ", "import ", "#include", "function ", "const ", "let ",
        "var ", "return ", "if (", "for (", "while (", "=> ", "</", "/>", "println!", "System.out", "console.log", "SELECT ",
    ];
    let lines = text.lines().count();
    let hits = MARKERS.iter().filter(|m| text.contains(**m)).count();
    let symbols = text.chars().filter(|c| matches!(c, '{' | '}' | ';' | '(' | ')' | '=')).count();
    hits >= 2 || (hits >= 1 && lines > 1) || (lines > 2 && symbols * 12 > text.len())
}

pub fn classify_text(text: &str) -> ClipKind {
    let trimmed = text.trim();
    if is_hex_color(trimmed) {
        ClipKind::Color
    } else if is_url(trimmed) {
        ClipKind::Link
    } else if is_email(trimmed) {
        ClipKind::Email
    } else if looks_like_code(trimmed) {
        ClipKind::Code
    } else {
        ClipKind::Text
    }
}

/// Evidence-based, as in the original: known token formats and labelled values, not "looks random".
pub fn is_sensitive(text: &str) -> bool {
    let trimmed = text.trim();
    let lower = trimmed.to_lowercase();

    // API tokens and keys with a recognizable prefix.
    const PREFIXES: [&str; 14] = [
        "sk-", "pk_live_", "sk_live_", "ghp_", "gho_", "ghs_", "github_pat_", "glpat-", "xoxb-", "xoxp-", "akia",
        "aiza", "ya29.", "-----begin",
    ];
    let single_word = !trimmed.chars().any(char::is_whitespace);
    if PREFIXES.iter().any(|p| lower.starts_with(p)) && (single_word || lower.starts_with("-----begin")) && trimmed.len() >= 16 {
        return true;
    }

    // JSON Web Tokens.
    if single_word && trimmed.starts_with("eyJ") && trimmed.split('.').count() == 3 && trimmed.len() > 30 {
        return true;
    }

    // Credentials inside a URL: scheme://user:password@host
    if let Some(after) = lower.split("://").nth(1)
        && let Some(at) = after.find('@')
    {
        let credentials = &after[..at];
        if credentials.contains(':') && !credentials.contains('/') {
            return true;
        }
    }

    // user@host:password as in leaked credential dumps.
    if single_word && let Some((left, right)) = trimmed.split_once(':') && is_email(left) && !right.is_empty() {
        return true;
    }

    if lower.starts_with("wifi:") && lower.contains(";p:") {
        return true;
    }

    // "password: hunter2" and friends.
    const LABELS: [&str; 10] =
        ["password", "passwort", "passwd", "pwd", "api_key", "apikey", "api-key", "secret", "token", "kennwort"];
    for line in lower.lines() {
        for label in LABELS {
            if let Some(index) = line.find(label) {
                let rest = line[index + label.len()..].trim_start_matches(['"', '\'']).trim_start();
                let has_value = rest
                    .strip_prefix(':')
                    .or_else(|| rest.strip_prefix('='))
                    .or_else(|| rest.strip_prefix("=>"))
                    .is_some_and(|value| !value.trim().is_empty());
                if has_value {
                    return true;
                }
            }
        }
    }

    // One-time codes next to the word that announces them.
    const CODE_WORDS: [&str; 6] = ["otp", "verification code", "security code", "bestätigungscode", "sicherheitscode", "einmalcode"];
    if CODE_WORDS.iter().any(|word| lower.contains(word)) {
        let digits = lower.split(|c: char| !c.is_ascii_digit()).any(|run| (4..=8).contains(&run.len()));
        if digits {
            return true;
        }
    }

    // Card numbers: 13 to 19 digits that pass the Luhn check.
    let compact: String = trimmed.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
    if (13..=19).contains(&compact.len()) && compact.chars().all(|c| c.is_ascii_digit()) && luhn(&compact) {
        return true;
    }
    false
}

fn luhn(digits: &str) -> bool {
    let mut sum = 0;
    for (index, c) in digits.chars().rev().enumerate() {
        let Some(mut value) = c.to_digit(10) else { return false };
        if index % 2 == 1 {
            value *= 2;
            if value > 9 {
                value -= 9;
            }
        }
        sum += value;
    }
    sum % 10 == 0
}

const MASK: &str = "••••••••";

impl ClipEntry {
    /// One line for the list, with secrets hidden.
    pub fn preview(&self) -> String {
        if self.sensitive {
            return MASK.to_string();
        }
        match self.kind {
            ClipKind::Image => crate::i18n::t("clipboard.image"),
            ClipKind::Files => {
                if let [only] = self.files.as_slice() {
                    PathBuf::from(only).file_name().map_or_else(|| only.clone(), |n| n.to_string_lossy().into_owned())
                } else {
                    crate::i18n::tf("clipboard.fileCount", &[&self.files.len().to_string()])
                }
            }
            _ => self.text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(160).collect(),
        }
    }
}

#[derive(Default)]
pub struct Store {
    pub entries: Vec<ClipEntry>,
    next_id: u64,
    /// Set while the app itself writes to the clipboard, so that write is not captured again.
    ignore_hash: Option<u64>,
    pub revision: u64,
}

impl Store {
    fn file() -> PathBuf {
        crate::paths::data_dir().join("clipboard.json")
    }

    fn image_dir() -> PathBuf {
        let dir = crate::paths::data_dir().join("clipboard");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    pub fn image_path(file: &str) -> PathBuf {
        Self::image_dir().join(file)
    }

    fn load() -> Self {
        let entries: Vec<ClipEntry> =
            std::fs::read_to_string(Self::file()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        let next_id = entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        Self { entries, next_id, ignore_hash: None, revision: 1 }
    }

    fn save(&self) {
        let keep: Vec<&ClipEntry> = self.entries.iter().filter(|e| !e.sensitive).collect();
        if let Ok(text) = serde_json::to_string(&keep) {
            let tmp = Self::file().with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, Self::file());
            }
        }
    }

    /// Adds an entry, or moves an identical one back to the top. Returns the new entry's id.
    pub fn add(&mut self, mut entry: ClipEntry, max_items: usize) -> Option<u64> {
        if self.ignore_hash == Some(entry.hash) {
            self.ignore_hash = None;
            return None;
        }
        if let Some(index) = self.entries.iter().position(|e| e.hash == entry.hash) {
            let mut existing = self.entries.remove(index);
            existing.created_unix = entry.created_unix;
            let id = existing.id;
            self.entries.insert(0, existing);
            self.touch();
            return Some(id);
        }
        entry.id = self.next_id;
        self.next_id += 1;
        let id = entry.id;
        self.entries.insert(0, entry);
        self.trim(max_items);
        self.touch();
        Some(id)
    }

    fn touch(&mut self) {
        self.revision += 1;
        self.save();
    }

    /// The oldest unpinned entries go first; pinned ones are never dropped.
    fn trim(&mut self, max_items: usize) {
        while self.entries.len() > max_items {
            let Some(index) = self.entries.iter().rposition(|e| !e.pinned) else { break };
            let removed = self.entries.remove(index);
            Self::delete_files(&removed);
        }
    }

    fn delete_files(entry: &ClipEntry) {
        if let Some(file) = &entry.image_file {
            let _ = std::fs::remove_file(Self::image_path(file));
        }
    }

    pub fn delete(&mut self, id: u64) {
        if let Some(index) = self.entries.iter().position(|e| e.id == id) {
            let removed = self.entries.remove(index);
            Self::delete_files(&removed);
            self.touch();
        }
    }

    pub fn toggle_pin(&mut self, id: u64) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.pinned = !entry.pinned;
            self.touch();
        }
    }

    pub fn toggle_label(&mut self, id: u64, label: &str) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            if let Some(index) = entry.labels.iter().position(|l| l == label) {
                entry.labels.remove(index);
            } else {
                entry.labels.push(label.to_string());
            }
            self.touch();
        }
    }

    /// Deletes everything except pinned entries.
    pub fn clear(&mut self) {
        let (keep, drop): (Vec<_>, Vec<_>) = std::mem::take(&mut self.entries).into_iter().partition(|e| e.pinned);
        for entry in &drop {
            Self::delete_files(entry);
        }
        self.entries = keep;
        self.touch();
    }

    pub fn search(&self, query: &str) -> Vec<&ClipEntry> {
        let query = query.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                query.is_empty()
                    || (!e.sensitive && e.text.to_lowercase().contains(&query))
                    || e.files.iter().any(|f| f.to_lowercase().contains(&query))
                    || e.labels.iter().any(|l| l.to_lowercase().contains(&query))
            })
            .collect()
    }

    fn remove_expired_sensitive(&mut self, now: u64) {
        let before = self.entries.len();
        self.entries.retain(|e| !e.sensitive || now.saturating_sub(e.created_unix) < SENSITIVE_LIFETIME_SECS);
        if self.entries.len() != before {
            self.revision += 1;
        }
    }
}

const SENSITIVE_LIFETIME_SECS: u64 = 60;

pub struct ClipboardService {
    pub store: Mutex<Store>,
    /// The newest capture, for the "copied" notification in the pill.
    pub latest: Mutex<Option<(u64, String, ClipKind)>>,
}

impl ClipboardService {
    pub fn start(settings: SharedSettings, ctx: egui::Context, demo: bool) -> Arc<Self> {
        let store = if demo { Store::default() } else { Store::load() };
        let service = Arc::new(Self { store: Mutex::new(store), latest: Mutex::new(None) });
        if demo {
            return service;
        }
        let worker = service.clone();
        let _ = std::thread::Builder::new().name("clipboard".into()).spawn(move || worker.run(&settings, &ctx));
        service
    }

    fn run(&self, settings: &SharedSettings, ctx: &egui::Context) {
        let mut board = None;
        let mut last_counter = crate::platform::clipboard_change_counter();
        let mut last_hash = 0_u64;
        let mut tick = 0_u32;
        loop {
            std::thread::sleep(Duration::from_millis(400));
            tick = tick.wrapping_add(1);
            if board.is_none() {
                board = arboard::Clipboard::new().ok();
            }
            let Some(clipboard) = board.as_mut() else { continue };

            let now = unix_now();
            if settings.read(|s| s.clipboard_auto_delete_sensitive)
                && let Ok(mut store) = self.store.lock()
            {
                store.remove_expired_sensitive(now);
            }

            // Where the OS has a change counter nothing is read until it moves.
            let counter = crate::platform::clipboard_change_counter();
            if counter.is_some() {
                if counter == last_counter {
                    continue;
                }
                last_counter = counter;
            }

            let (text_on, images_on, files_on, max_items) = settings.read(|s| {
                (s.clipboard_capture_text, s.clipboard_capture_images, s.clipboard_capture_files, s.clipboard_max_items)
            });
            let probe_image = images_on && (counter.is_some() || tick % 5 == 0);
            let Some(mut entry) = read_clipboard(clipboard, text_on, probe_image, files_on, last_hash) else { continue };
            if entry.hash == last_hash {
                continue;
            }
            last_hash = entry.hash;
            entry.created_unix = now;
            if entry.kind != ClipKind::Image && entry.kind != ClipKind::Files {
                entry.sensitive = is_sensitive(&entry.text);
            }
            let preview = entry.preview();
            let kind = entry.kind;
            if let Ok(mut store) = self.store.lock()
                && let Some(id) = store.add(entry, max_items)
                && let Ok(mut latest) = self.latest.lock()
            {
                *latest = Some((id, preview, kind));
            }
            ctx.request_repaint();
        }
    }

    /// Puts an entry back on the clipboard without capturing it a second time.
    pub fn copy_back(&self, entry: &ClipEntry) {
        if let Ok(mut store) = self.store.lock() {
            store.ignore_hash = Some(entry.hash);
        }
        let Ok(mut clipboard) = arboard::Clipboard::new() else { return };
        match entry.kind {
            ClipKind::Files => {
                let paths: Vec<PathBuf> = entry.files.iter().map(PathBuf::from).collect();
                let _ = clipboard.set().file_list(&paths);
            }
            ClipKind::Image => {
                let Some(file) = &entry.image_file else { return };
                if let Ok(image) = image::open(Store::image_path(file)) {
                    let rgba = image.to_rgba8();
                    let _ = clipboard.set_image(arboard::ImageData {
                        width: rgba.width() as usize,
                        height: rgba.height() as usize,
                        bytes: std::borrow::Cow::Owned(rgba.into_raw()),
                    });
                }
            }
            _ => {
                let _ = clipboard.set_text(entry.text.clone());
            }
        }
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

fn read_clipboard(
    clipboard: &mut arboard::Clipboard,
    text_on: bool,
    probe_image: bool,
    files_on: bool,
    last_hash: u64,
) -> Option<ClipEntry> {
    use crate::platform::hash_bytes;
    if files_on
        && let Ok(files) = clipboard.get().file_list()
        && !files.is_empty()
    {
        let files: Vec<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        return Some(ClipEntry { kind: ClipKind::Files, hash: hash_bytes(files.join("\n").as_bytes()), files, ..Default::default() });
    }
    if let Ok(text) = clipboard.get_text()
        && !text.trim().is_empty()
    {
        if !text_on {
            return None;
        }
        return Some(ClipEntry { kind: classify_text(&text), hash: hash_bytes(text.as_bytes()), text, ..Default::default() });
    }
    if probe_image
        && let Ok(image) = clipboard.get_image()
        && image.bytes.len() <= MAX_IMAGE_BYTES
    {
        let hash = hash_bytes(&image.bytes);
        if hash == last_hash {
            return None;
        }
        let file = format!("{hash:016x}.png");
        let path = Store::image_path(&file);
        if !path.exists() {
            let buffer = image::RgbaImage::from_raw(image.width as u32, image.height as u32, image.bytes.into_owned())?;
            buffer.save(&path).ok()?;
        }
        return Some(ClipEntry { kind: ClipKind::Image, hash, image_file: Some(file), ..Default::default() });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_entry(text: &str, hash: u64) -> ClipEntry {
        ClipEntry { text: text.into(), hash, kind: classify_text(text), ..Default::default() }
    }

    #[test]
    fn classifies_common_shapes() {
        assert_eq!(classify_text("#ff00aa"), ClipKind::Color);
        assert_eq!(classify_text("https://example.com/a?b=1"), ClipKind::Link);
        assert_eq!(classify_text("max@example.com"), ClipKind::Email);
        assert_eq!(classify_text("fn main() {\n    println!(\"hi\");\n}"), ClipKind::Code);
        assert_eq!(classify_text("Einfach nur ein Satz."), ClipKind::Text);
        assert_eq!(classify_text("https://a.com und mehr Text"), ClipKind::Text);
    }

    #[test]
    fn colors_parse() {
        assert_eq!(parse_color("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(parse_color("#10203040"), Some([16, 32, 48, 64]));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("123456"), None);
    }

    #[test]
    fn secrets_are_recognized() {
        for secret in [
            "sk-abcdefghijklmnopqrstuvwxyz123456",
            "ghp_0123456789abcdefghijklmnopqrstuvwxyz",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abcdefghijklmnop",
            "postgres://user:hunter2@db.example.com/app",
            "max@example.com:hunter2",
            "WIFI:S:Home;T:WPA;P:geheim;;",
            "password: hunter2",
            "API_KEY=abc123",
            "Dein Bestätigungscode lautet 482913",
            "4111 1111 1111 1111",
            "-----BEGIN PRIVATE KEY-----\nabcdefghijklmnop",
        ] {
            assert!(is_sensitive(secret), "should be sensitive: {secret}");
        }
    }

    #[test]
    fn ordinary_text_is_not_a_secret() {
        for text in [
            "Hallo, wie geht es dir?",
            "https://example.com/page",
            "max@example.com",
            "Das Passwort habe ich vergessen",
            "1234567890123",
            "sk-short",
            "Rechnung 2024-11-06 über 482913 Euro",
        ] {
            assert!(!is_sensitive(text), "should not be sensitive: {text}");
        }
    }

    #[test]
    fn sensitive_entries_are_masked_and_not_serialized() {
        let entry = ClipEntry { text: "password: x".into(), sensitive: true, ..Default::default() };
        assert_eq!(entry.preview(), MASK);
    }

    #[test]
    fn duplicates_move_to_the_top_instead_of_piling_up() {
        let mut store = Store::default();
        store.add(text_entry("a", 1), 10);
        store.add(text_entry("b", 2), 10);
        store.add(text_entry("a", 1), 10);
        assert_eq!(store.entries.len(), 2);
        assert_eq!(store.entries[0].text, "a");
    }

    #[test]
    fn trimming_spares_pinned_entries() {
        let mut store = Store::default();
        let id = store.add(text_entry("pinned", 1), 2).unwrap();
        store.toggle_pin(id);
        store.add(text_entry("b", 2), 2);
        store.add(text_entry("c", 3), 2);
        assert_eq!(store.entries.len(), 2);
        assert!(store.entries.iter().any(|e| e.text == "pinned"));
        assert!(!store.entries.iter().any(|e| e.text == "b"));
    }

    #[test]
    fn writing_back_is_not_captured_again() {
        let mut store = Store::default();
        store.ignore_hash = Some(7);
        assert_eq!(store.add(text_entry("x", 7), 10), None);
        assert!(store.entries.is_empty());
        assert!(store.add(text_entry("x", 7), 10).is_some());
    }

    #[test]
    fn search_never_matches_inside_secrets() {
        let mut store = Store::default();
        store.add(ClipEntry { text: "password: geheim".into(), sensitive: true, hash: 1, ..Default::default() }, 10);
        store.add(text_entry("geheimnis", 2), 10);
        let hits = store.search("geheim");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "geheimnis");
    }

    #[test]
    fn expired_secrets_disappear() {
        let mut store = Store::default();
        store.add(ClipEntry { sensitive: true, created_unix: 100, hash: 1, ..Default::default() }, 10);
        store.remove_expired_sensitive(100 + SENSITIVE_LIFETIME_SECS - 1);
        assert_eq!(store.entries.len(), 1);
        store.remove_expired_sensitive(100 + SENSITIVE_LIFETIME_SECS);
        assert!(store.entries.is_empty());
    }

    #[test]
    fn luhn_works() {
        assert!(luhn("4111111111111111"));
        assert!(!luhn("4111111111111112"));
    }
}
