//! Looks for a newer release on GitHub. It only tells; installing stays with the user.

use super::http;
use eframe::egui;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const RELEASES_API: &str = "https://api.github.com/repos/pichlervalentin4-lab/V-Notch/releases";
pub const RELEASES_PAGE: &str = "https://github.com/pichlervalentin4-lab/V-Notch/releases";

#[derive(Clone, Debug, Default, PartialEq)]
pub enum UpdateStatus {
    #[default]
    Unknown,
    Checking,
    UpToDate,
    Available { version: String, url: String },
    Failed,
}

pub struct UpdateService {
    pub status: Mutex<UpdateStatus>,
    ctx: egui::Context,
}

/// `1.10.0` is newer than `1.9.3`. Anything that does not parse is never "newer".
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    let core = text.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// The newest release the user wants to hear about, from the releases list.
pub fn pick_release(releases: &Value, include_prerelease: bool) -> Option<(String, String)> {
    releases
        .as_array()?
        .iter()
        .filter(|release| release["draft"] != true)
        .filter(|release| include_prerelease || release["prerelease"] != true)
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            let url = release["html_url"].as_str()?;
            parse_version(tag).map(|version| (version, tag.to_string(), url.to_string()))
        })
        .max_by_key(|(version, _, _)| *version)
        .map(|(_, tag, url)| (tag, url))
}

impl UpdateService {
    pub fn new(ctx: egui::Context) -> Arc<Self> {
        Arc::new(Self { status: Mutex::new(UpdateStatus::Unknown), ctx })
    }

    pub fn check(self: &Arc<Self>, include_prerelease: bool) {
        self.set(UpdateStatus::Checking);
        let service = self.clone();
        let _ = std::thread::Builder::new().name("updates".into()).spawn(move || {
            let next = match http::get_json(RELEASES_API, Duration::from_secs(15)) {
                Ok(releases) => match pick_release(&releases, include_prerelease) {
                    Some((version, url)) if is_newer(&version, env!("CARGO_PKG_VERSION")) => {
                        UpdateStatus::Available { version, url }
                    }
                    _ => UpdateStatus::UpToDate,
                },
                Err(error) => {
                    crate::vlog!("UPDATES", "{error}");
                    UpdateStatus::Failed
                }
            };
            service.set(next);
        });
    }

    fn set(&self, next: UpdateStatus) {
        if let Ok(mut status) = self.status.lock() {
            *status = next;
            self.ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("v1.10.0", "1.9.3"));
        assert!(is_newer("2", "1.9.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("nightly", "1.0.0"));
        assert!(is_newer("1.2.0-beta.1", "1.1.0"));
    }

    #[test]
    fn drafts_are_ignored_and_prereleases_optional() {
        let releases = json!([
            {"tag_name": "v3.0.0", "draft": true, "html_url": "d"},
            {"tag_name": "v2.0.0", "prerelease": true, "html_url": "p"},
            {"tag_name": "v1.0.0", "html_url": "s"},
            {"tag_name": "nightly", "html_url": "n"}
        ]);
        assert_eq!(pick_release(&releases, false).unwrap().0, "v1.0.0");
        assert_eq!(pick_release(&releases, true).unwrap().0, "v2.0.0");
        assert!(pick_release(&json!([]), true).is_none());
    }
}
