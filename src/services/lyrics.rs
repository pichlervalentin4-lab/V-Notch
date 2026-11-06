//! Synced lyrics: LRC parsing and lookup on LRCLIB and lrc mux.

use super::http;
use serde_json::Value;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, PartialEq)]
pub struct LyricLine {
    pub time: Duration,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    pub provider: String,
}

impl Lyrics {
    /// Index of the line that should be showing at `position`.
    pub fn line_at(&self, position: Duration) -> Option<usize> {
        match self.lines.binary_search_by(|line| line.time.cmp(&position)) {
            Ok(index) => Some(index),
            Err(0) => None,
            Err(index) => Some(index - 1),
        }
    }
}

/// Parses `[mm:ss.xx] text`, including several stamps on one line. Metadata tags are skipped.
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            match parse_stamp(&stripped[..end]) {
                Some(time) => stamps.push(time),
                None => break,
            }
            rest = stripped[end + 1..].trim_start();
        }
        for time in stamps {
            lines.push(LyricLine { time, text: rest.to_string() });
        }
    }
    lines.sort_by_key(|line| line.time);
    lines
}

fn parse_stamp(stamp: &str) -> Option<Duration> {
    let (minutes, seconds) = stamp.split_once(':')?;
    let minutes: u64 = minutes.trim().parse().ok()?;
    let seconds: f64 = seconds.trim().replace(',', ".").parse().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some(Duration::from_secs(minutes * 60) + Duration::from_secs_f64(seconds))
}

/// Strips what players add but lyric databases do not have: "(Remastered 2011)", "- Live", "feat. X".
pub fn clean_title(title: &str) -> String {
    let lower = title.to_lowercase();
    let noise = ["feat", "ft.", "remaster", "live", "official", "lyric", "video", "audio", "version", "mono", "stereo", "edit", "radio"];
    let mut result = title.to_string();
    for (open, close) in [('(', ')'), ('[', ']')] {
        while let (Some(start), Some(end)) = (result.find(open), result.find(close)) {
            if end <= start {
                break;
            }
            let inner = result[start + 1..end].to_lowercase();
            if noise.iter().any(|word| inner.contains(word)) {
                result.replace_range(start..=end, "");
            } else {
                break;
            }
        }
    }
    if let Some(index) = result.find(" - ") {
        let tail = result[index + 3..].to_lowercase();
        if noise.iter().any(|word| tail.contains(word)) {
            result.truncate(index);
        }
    }
    let _ = lower;
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn from_lrclib_item(item: &Value) -> Option<Lyrics> {
    let synced = item.get("syncedLyrics")?.as_str()?;
    let lines = parse_lrc(synced);
    (!lines.is_empty()).then(|| Lyrics { lines, provider: "LRCLIB".into() })
}

/// Picks the search hit with the same title and the closest length.
pub fn best_search_hit<'a>(items: &'a [Value], title: &str, seconds: u64) -> Option<&'a Value> {
    let wanted = normalize(title);
    items
        .iter()
        .filter(|item| item.get("syncedLyrics").and_then(Value::as_str).is_some_and(|s| !s.is_empty()))
        .filter(|item| {
            let name = normalize(item.get("trackName").and_then(Value::as_str).unwrap_or_default());
            name.contains(&wanted) || wanted.contains(&name)
        })
        .min_by_key(|item| {
            let duration = item.get("duration").and_then(Value::as_f64).unwrap_or(0.0) as i64;
            if duration > 0 && seconds > 0 { (duration - seconds as i64).unsigned_abs() } else { u64::MAX / 2 }
        })
        .filter(|item| {
            let duration = item.get("duration").and_then(Value::as_f64).unwrap_or(0.0) as i64;
            seconds == 0 || duration == 0 || (duration - seconds as i64).unsigned_abs() <= 8
        })
}

fn lrclib_exact(title: &str, artist: &str, seconds: u64) -> Option<Lyrics> {
    let url = format!(
        "https://lrclib.net/api/get?track_name={}&artist_name={}&duration={seconds}",
        http::encode(title),
        http::encode(artist)
    );
    from_lrclib_item(&http::get_json(&url, TIMEOUT).ok()?)
}

fn lrclib_search(title: &str, artist: &str, seconds: u64) -> Option<Lyrics> {
    let url = format!(
        "https://lrclib.net/api/search?track_name={}&artist_name={}",
        http::encode(title),
        http::encode(artist)
    );
    let value = http::get_json(&url, TIMEOUT).ok()?;
    from_lrclib_item(best_search_hit(value.as_array()?, title, seconds)?)
}

/// lrc mux aggregates more sources than LRCLIB alone; it is the fallback.
fn lrc_mux(title: &str, artist: &str, seconds: u64) -> Option<Lyrics> {
    let url = format!(
        "https://api.lrcmux.dev/get?title={}&artist={}&duration={seconds}&level=line&format=json&sources=%21lrclib",
        http::encode(title),
        http::encode(artist)
    );
    parse_lrc_mux(&http::get_json(&url, TIMEOUT).ok()?)
}

pub fn parse_lrc_mux(value: &Value) -> Option<Lyrics> {
    let mut lines: Vec<LyricLine> = value
        .get("lines")?
        .as_array()?
        .iter()
        .filter_map(|item| {
            let text = item.get("text")?.as_str()?.trim().to_string();
            let start = item.get("start")?.as_f64()?;
            (start.is_finite() && start >= 0.0).then(|| LyricLine { time: Duration::from_secs_f64(start), text })
        })
        .collect();
    lines.sort_by_key(|line| line.time);
    let source = value.pointer("/meta/source/name").and_then(Value::as_str);
    (!lines.is_empty()).then(|| Lyrics {
        lines,
        provider: source.map_or_else(|| "lrc mux".to_string(), |name| format!("{name} via lrc mux")),
    })
}

pub fn fetch(title: &str, artist: &str, seconds: u64) -> Option<Lyrics> {
    let cleaned = clean_title(title);
    lrclib_exact(&cleaned, artist, seconds)
        .or_else(|| lrclib_search(&cleaned, artist, seconds))
        .or_else(|| lrc_mux(&cleaned, artist, seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_plain_lines_and_fractions() {
        let lines = parse_lrc("[00:12.50] Hello\n[01:02.00] World");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].time, Duration::from_millis(12_500));
        assert_eq!(lines[1].time, Duration::from_secs(62));
        assert_eq!(lines[1].text, "World");
    }

    #[test]
    fn repeated_stamps_create_sorted_lines() {
        let lines = parse_lrc("[00:30.00][00:10.00] Chorus");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].time, Duration::from_secs(10));
        assert_eq!(lines[1].text, "Chorus");
    }

    #[test]
    fn metadata_and_garbage_are_ignored() {
        let lines = parse_lrc("[ar:Someone]\n[ti:Song]\nno stamp\n[00:01.00] ok\n[xx:yy] bad");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "ok");
    }

    #[test]
    fn line_lookup_follows_position() {
        let lyrics = Lyrics { lines: parse_lrc("[00:10.00] a\n[00:20.00] b"), provider: String::new() };
        assert_eq!(lyrics.line_at(Duration::from_secs(5)), None);
        assert_eq!(lyrics.line_at(Duration::from_secs(10)), Some(0));
        assert_eq!(lyrics.line_at(Duration::from_secs(15)), Some(0));
        assert_eq!(lyrics.line_at(Duration::from_secs(99)), Some(1));
    }

    #[test]
    fn clean_title_removes_player_noise() {
        assert_eq!(clean_title("Song (Remastered 2011)"), "Song");
        assert_eq!(clean_title("Song [Official Video]"), "Song");
        assert_eq!(clean_title("Song - Live at Wembley"), "Song");
        assert_eq!(clean_title("Song (feat. Someone)"), "Song");
        assert_eq!(clean_title("Stay (I Miss You)"), "Stay (I Miss You)");
    }

    #[test]
    fn search_prefers_matching_title_and_length() {
        let items = vec![
            json!({"trackName":"Song","duration":300.0,"syncedLyrics":"[00:01.00] far"}),
            json!({"trackName":"Song","duration":181.0,"syncedLyrics":"[00:01.00] near"}),
            json!({"trackName":"Other","duration":180.0,"syncedLyrics":"[00:01.00] other"}),
            json!({"trackName":"Song","duration":180.0,"syncedLyrics":""}),
        ];
        let hit = best_search_hit(&items, "Song", 180).unwrap();
        assert_eq!(hit["duration"], 181.0);
    }

    #[test]
    fn search_rejects_hits_of_the_wrong_length() {
        let items = vec![json!({"trackName":"Song","duration":300.0,"syncedLyrics":"[00:01.00] far"})];
        assert!(best_search_hit(&items, "Song", 180).is_none());
    }

    #[test]
    fn lrc_mux_lines_are_read_and_sorted() {
        let value = json!({
            "meta": {"source": {"name": "Musixmatch"}},
            "lines": [{"text":"b","start":2.0},{"text":"a","start":1.0},{"bad":1}]
        });
        let lyrics = parse_lrc_mux(&value).unwrap();
        assert_eq!(lyrics.lines[0].text, "a");
        assert_eq!(lyrics.provider, "Musixmatch via lrc mux");
    }
}
