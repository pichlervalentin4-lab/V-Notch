//! Follows the current player: track, position, artwork, synced lyrics.

use super::artwork::{self, Artwork};
use super::lyrics::{self, Lyrics};
use crate::platform::{MediaBackend, MediaCommand, MediaInfo};
use crate::settings::SharedSettings;
use eframe::egui;
use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(700);
const KEEP_AFTER_STOP: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, Default, PartialEq)]
pub enum LyricsState {
    #[default]
    None,
    Searching,
    Found(Arc<Lyrics>),
    NotFound,
}

#[derive(Default)]
pub struct MediaState {
    pub info: Option<MediaInfo>,
    pub art: Option<Artwork>,
    pub lyrics: LyricsState,
}

pub struct MediaService {
    pub state: Mutex<MediaState>,
    commands: Mutex<Sender<MediaCommand>>,
}

impl MediaService {
    pub fn start(ctx: egui::Context, settings: SharedSettings, demo: bool) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel();
        let service = Arc::new(Self { state: Mutex::new(MediaState::default()), commands: Mutex::new(sender) });
        if demo {
            service.demo_fill();
            return service;
        }
        let worker = service.clone();
        let _ = std::thread::Builder::new().name("media".into()).spawn(move || {
            let mut backend = MediaBackend::new();
            let mut last_seen = Instant::now();
            let mut cache: HashMap<String, Arc<Lyrics>> = HashMap::new();
            loop {
                // Commands wake the loop right away, so the UI answers at once.
                let command = receiver.recv_timeout(POLL);
                if let Ok(command) = command {
                    backend.send(command);
                    while let Ok(next) = receiver.try_recv() {
                        backend.send(next);
                    }
                    std::thread::sleep(Duration::from_millis(120));
                }
                let info = backend.poll();
                if info.is_some() {
                    last_seen = Instant::now();
                }
                worker.apply(info, last_seen, &ctx, &settings, &mut cache);
            }
        });
        service
    }

    fn apply(
        self: &Arc<Self>,
        info: Option<MediaInfo>,
        last_seen: Instant,
        ctx: &egui::Context,
        settings: &SharedSettings,
        cache: &mut HashMap<String, Arc<Lyrics>>,
    ) {
        let Ok(mut state) = self.state.lock() else { return };
        let Some(info) = info else {
            // Players vanish for a moment between tracks: hold on to the old state a little.
            if state.info.is_some() && last_seen.elapsed() > KEEP_AFTER_STOP {
                *state = MediaState::default();
                ctx.request_repaint();
            }
            return;
        };
        let new_track = state.info.as_ref().is_none_or(|old| !old.same_track(&info));
        let changed_visibly = new_track
            || state.info.as_ref().is_none_or(|old| {
                old.playing != info.playing
                    || old.art_key != info.art_key
                    || old.duration != info.duration
                    || old.current_position().abs_diff(info.current_position()) > Duration::from_secs(2)
            });

        if new_track {
            state.lyrics = LyricsState::None;
            // Like the original, a new track shows up in the collapsed notch without a pop-up:
            // browser media (TikTok, YouTube Shorts) changes "track" every few seconds.
            let wanted = settings.read(|s| s.enable_online_lyrics && s.is_online_allowed());
            if wanted && !info.duration.is_zero() {
                let key = format!("{}|{}", info.title, info.artist);
                if let Some(found) = cache.get(&key) {
                    state.lyrics = LyricsState::Found(found.clone());
                } else {
                    state.lyrics = LyricsState::Searching;
                    self.fetch_lyrics(&info, ctx);
                }
            }
        }
        if let LyricsState::Found(found) = &state.lyrics {
            cache.insert(format!("{}|{}", info.title, info.artist), found.clone());
            if cache.len() > 64 {
                cache.clear();
            }
        }

        let art_is_current = state.art.as_ref().map(|art| art.key) == Some(info.art_key);
        if !art_is_current {
            match &info.art {
                Some(bytes) => {
                    let (bytes, key) = (bytes.clone(), info.art_key);
                    let worker = self.clone();
                    let ctx = ctx.clone();
                    // Decoding can take a while for big covers; never hold up the poll loop for it.
                    let _ = std::thread::Builder::new().name("artwork".into()).spawn(move || {
                        let art = artwork::process(&bytes, key);
                        if let Ok(mut state) = worker.state.lock()
                            && state.info.as_ref().map(|i| i.art_key) == Some(key)
                        {
                            state.art = art;
                            ctx.request_repaint();
                        }
                    });
                }
                None => state.art = None,
            }
        }

        state.info = Some(info);
        if changed_visibly {
            ctx.request_repaint();
        }
    }

    fn fetch_lyrics(self: &Arc<Self>, info: &MediaInfo, ctx: &egui::Context) {
        let (title, artist, seconds) = (info.title.clone(), info.artist.clone(), info.duration.as_secs());
        let worker = self.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("lyrics".into()).spawn(move || {
            let result = lyrics::fetch(&title, &artist, seconds);
            if let Ok(mut state) = worker.state.lock()
                && state.info.as_ref().is_some_and(|i| i.title == title && i.artist == artist)
            {
                state.lyrics = result.map_or(LyricsState::NotFound, |found| LyricsState::Found(Arc::new(found)));
                ctx.request_repaint();
            }
        });
    }

    pub fn send(&self, command: MediaCommand) {
        if let Ok(sender) = self.commands.lock() {
            let _ = sender.send(command);
        }
    }

    fn demo_fill(&self) {
        let mut cover = image::RgbaImage::new(256, 256);
        for (x, y, pixel) in cover.enumerate_pixels_mut() {
            let (u, v) = (x as f32 / 255.0, y as f32 / 255.0);
            *pixel = image::Rgba([(40.0 + 200.0 * u) as u8, (30.0 + 80.0 * v) as u8, (190.0 - 120.0 * u) as u8, 255]);
        }
        let accent = artwork::accent_color(&cover);
        let mut blur = image::imageops::resize(&cover, 64, 64, image::imageops::FilterType::Triangle);
        artwork::box_blur(&mut blur, 6);
        let lines = lyrics::parse_lrc(
            "[00:00.00] Erste Zeile im Licht\n[00:50.00] Der Notch singt jetzt mit\n[01:10.00] Rust unter der Haube\n[01:30.00] Und alles bleibt flüssig",
        );
        if let Ok(mut state) = self.state.lock() {
            state.info = Some(MediaInfo {
                title: "Neon Horizon".into(),
                artist: "Demo Artist".into(),
                album: "Demo Album".into(),
                source_app: "Spotify".into(),
                playing: true,
                position: Duration::from_secs(62),
                duration: Duration::from_secs(215),
                position_read_at: Instant::now(),
                art: None,
                art_key: 1,
                can_seek: true,
            });
            state.art = Some(Artwork { key: 1, cover: Arc::new(cover), blur: Arc::new(blur), accent });
            state.lyrics = LyricsState::Found(Arc::new(Lyrics { lines, provider: "Demo".into() }));
        }
    }
}
