//! Keeps a fresh forecast around without ever making the UI wait for the network.

use super::weather::{self, Forecast, Place};
use crate::settings::SharedSettings;
use eframe::egui;
use std::sync::mpsc::{self, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const REFRESH: Duration = Duration::from_secs(15 * 60);
const RETRY: Duration = Duration::from_secs(60);
const START_DELAY: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Default, PartialEq)]
pub enum WeatherStatus {
    #[default]
    Off,
    Loading,
    Ready(Arc<Forecast>),
    Failed,
}

pub struct WeatherService {
    pub status: Mutex<WeatherStatus>,
    refresh: Mutex<Sender<()>>,
    woken: AtomicBool,
}

impl WeatherService {
    pub fn start(ctx: egui::Context, settings: SharedSettings, demo: bool) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel();
        let service = Arc::new(Self {
            status: Mutex::new(WeatherStatus::Loading),
            refresh: Mutex::new(sender),
            woken: AtomicBool::new(false),
        });
        if demo {
            if let Ok(mut status) = service.status.lock() {
                *status = WeatherStatus::Ready(Arc::new(demo_forecast()));
            }
            return service;
        }
        let worker = service.clone();
        let _ = std::thread::Builder::new().name("weather".into()).spawn(move || {
            // The first HTTPS request loads the TLS libraries, which briefly stalls the window on
            // Windows. Waiting until the opening animation is over hides that; opening the weather
            // tab ends the wait early.
            let _ = receiver.recv_timeout(START_DELAY);
            let mut cached_place: Option<(String, Place)> = None;
            loop {
                let (enabled, city, fahrenheit) = settings.read(|s| {
                    (s.enable_weather && s.allow_online_weather && s.is_online_allowed(), s.manual_city.clone(), s.use_fahrenheit)
                });
                let wait = if enabled {
                    let is_first = worker.status.lock().is_ok_and(|s| !matches!(*s, WeatherStatus::Ready(_)));
                    if is_first {
                        worker.set(WeatherStatus::Loading, &ctx);
                    }
                    match refresh(&city, fahrenheit, &mut cached_place) {
                        Ok(forecast) => {
                            worker.set(WeatherStatus::Ready(Arc::new(forecast)), &ctx);
                            REFRESH
                        }
                        Err(error) => {
                            crate::vlog!("WEATHER", "{error}");
                            // A forecast from a few minutes ago beats an error message.
                            if is_first {
                                worker.set(WeatherStatus::Failed, &ctx);
                            }
                            RETRY
                        }
                    }
                } else {
                    worker.set(WeatherStatus::Off, &ctx);
                    REFRESH
                };
                let _ = receiver.recv_timeout(wait);
            }
        });
        service
    }

    fn set(&self, next: WeatherStatus, ctx: &egui::Context) {
        if let Ok(mut status) = self.status.lock() {
            *status = next;
            ctx.request_repaint();
        }
    }

    /// Ends the start-up wait once, when the weather tab is opened before the first fetch.
    pub fn wake(&self) {
        if !self.woken.swap(true, Ordering::Relaxed) {
            self.refresh_now();
        }
    }

    /// Fetches again right away, for instance after the city changed.
    pub fn refresh_now(&self) {
        if let Ok(sender) = self.refresh.lock() {
            let _ = sender.send(());
        }
    }
}

fn refresh(city: &str, fahrenheit: bool, cached: &mut Option<(String, Place)>) -> Result<Forecast, String> {
    let city = city.trim();
    let place = match cached {
        Some((name, place)) if name == city => place.clone(),
        _ => {
            let place = if city.is_empty() { weather::locate_by_ip()? } else { weather::geocode(city)? };
            *cached = Some((city.to_string(), place.clone()));
            place
        }
    };
    weather::fetch(&place, fahrenheit)
}

fn demo_forecast() -> Forecast {
    use super::weather::{DayForecast, HourForecast};
    let hours = (0..24)
        .map(|i| {
            let x = i as f32 / 23.0;
            HourForecast {
                time: format!("2026-11-06T{:02}:00", (14 + i) % 24),
                temperature: 12.0 - 8.0 * (x * 3.1).sin().abs() + 3.0 * x,
                rain_chance: if (6..12).contains(&i) { 55.0 } else { 10.0 },
                rain: if (6..12).contains(&i) { 0.6 } else { 0.0 },
                code: if (6..12).contains(&i) { 61 } else { 2 },
                is_day: !(3..17).contains(&i),
            }
        })
        .collect();
    let days = ["2026-11-06", "2026-11-07", "2026-11-08", "2026-11-09", "2026-11-10", "2026-11-11", "2026-11-12"]
        .iter()
        .enumerate()
        .map(|(i, date)| DayForecast {
            date: (*date).to_string(),
            code: [2, 61, 3, 0, 1, 71, 2][i],
            max: 13.0 - i as f32,
            min: 5.0 - i as f32 * 0.5,
            rain_chance: [10.0, 70.0, 20.0, 0.0, 5.0, 60.0, 15.0][i],
            uv: 2.0,
            sunrise: format!("{date}T07:05"),
            sunset: format!("{date}T16:40"),
        })
        .collect();
    Forecast {
        place: "Traun".into(),
        temperature: 12.0,
        feels_like: 10.0,
        humidity: 72.0,
        wind: 9.0,
        gusts: 21.0,
        code: 2,
        is_day: true,
        hours,
        days,
    }
}
