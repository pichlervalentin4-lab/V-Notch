//! Interaction sounds and ambient noise, all synthesized. No audio files ship with the app.

use std::f32::consts::TAU;

pub const SAMPLE_RATE: u32 = 44_100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tone {
    Click,
    SoftClick,
    Tap,
    Bong,
    Finish,
}

/// Renders a short mono sound into samples.
pub fn render_tone(tone: Tone) -> Vec<f32> {
    let voice = |frequency: f32, length: f32, decay: f32, gain: f32, out: &mut Vec<f32>, offset: f32| {
        let start = (offset * SAMPLE_RATE as f32) as usize;
        let count = (length * SAMPLE_RATE as f32) as usize;
        if out.len() < start + count {
            out.resize(start + count, 0.0);
        }
        for i in 0..count {
            let t = i as f32 / SAMPLE_RATE as f32;
            out[start + i] += (TAU * frequency * t).sin() * (-t * decay).exp() * gain;
        }
    };
    let mut samples = Vec::new();
    match tone {
        Tone::Click => {
            voice(1650.0, 0.04, 130.0, 0.5, &mut samples, 0.0);
            voice(820.0, 0.05, 90.0, 0.25, &mut samples, 0.0);
        }
        Tone::SoftClick => voice(1100.0, 0.05, 110.0, 0.35, &mut samples, 0.0),
        Tone::Tap => {
            voice(2300.0, 0.03, 180.0, 0.4, &mut samples, 0.0);
            voice(1200.0, 0.04, 120.0, 0.2, &mut samples, 0.0);
        }
        Tone::Bong => {
            voice(660.0, 0.7, 6.0, 0.4, &mut samples, 0.0);
            voice(1320.0, 0.5, 9.0, 0.15, &mut samples, 0.0);
        }
        Tone::Finish => {
            voice(880.0, 0.35, 9.0, 0.35, &mut samples, 0.0);
            voice(1174.7, 0.5, 7.0, 0.3, &mut samples, 0.12);
        }
    }
    samples
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseKind {
    Rain,
    Ocean,
    White,
    Wind,
    Brook,
    Pink,
    Brown,
    Fan,
}

impl NoiseKind {
    pub const ALL: [NoiseKind; 8] = [
        NoiseKind::Rain,
        NoiseKind::Ocean,
        NoiseKind::White,
        NoiseKind::Wind,
        NoiseKind::Brook,
        NoiseKind::Pink,
        NoiseKind::Brown,
        NoiseKind::Fan,
    ];

    pub fn label_key(self) -> &'static str {
        match self {
            NoiseKind::Rain => "whitenoise.rain",
            NoiseKind::Ocean => "whitenoise.ocean",
            NoiseKind::White => "whitenoise.white",
            NoiseKind::Wind => "whitenoise.wind",
            NoiseKind::Brook => "whitenoise.brook",
            NoiseKind::Pink => "whitenoise.pink",
            NoiseKind::Brown => "whitenoise.brown",
            NoiseKind::Fan => "whitenoise.fan",
        }
    }

    pub fn icon(self) -> &'static str {
        use crate::icons::*;
        match self {
            NoiseKind::Rain => CLOUD_RAIN,
            NoiseKind::Ocean => WAVES,
            NoiseKind::White => SPARKLE,
            NoiseKind::Wind => WIND,
            NoiseKind::Brook => DROP,
            NoiseKind::Pink => LEAF,
            NoiseKind::Brown => COFFEE,
            NoiseKind::Fan => FAN,
        }
    }
}

/// An endless procedural noise generator. Each kind is shaped noise with its own slow modulation.
pub struct NoiseGenerator {
    kind: NoiseKind,
    state: u32,
    time: f32,
    pink: [f32; 3],
    brown: f32,
    low: f32,
    low2: f32,
}

impl NoiseGenerator {
    pub fn new(kind: NoiseKind) -> Self {
        Self { kind, state: 0x1234_5678, time: 0.0, pink: [0.0; 3], brown: 0.0, low: 0.0, low2: 0.0 }
    }

    fn white(&mut self) -> f32 {
        // xorshift32: cheap, and more than random enough for noise.
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    pub fn next_sample(&mut self) -> f32 {
        let dt = 1.0 / SAMPLE_RATE as f32;
        self.time += dt;
        if self.time > 3600.0 {
            self.time -= 3600.0;
        }
        let t = self.time;
        let white = self.white();
        self.pink[0] = 0.997 * self.pink[0] + 0.029 * white;
        self.pink[1] = 0.985 * self.pink[1] + 0.033 * white;
        self.pink[2] = 0.950 * self.pink[2] + 0.097 * white;
        let pink = (self.pink[0] + self.pink[1] + self.pink[2] + white * 0.19) * 0.5;
        self.brown = (self.brown + white * 0.02).clamp(-1.0, 1.0) * 0.998;
        let brown = self.brown * 2.6;
        let sample = match self.kind {
            NoiseKind::White => white * 0.28,
            NoiseKind::Pink => pink * 0.55,
            NoiseKind::Brown => brown * 0.65,
            NoiseKind::Rain => {
                self.low += (white - self.low) * 0.45;
                let hiss = white - self.low;
                // Sparse droplets on top of a steady hiss.
                let drop = if self.white() > 0.9992 { self.white() * 0.8 } else { 0.0 };
                hiss * 0.28 + pink * 0.18 + drop
            }
            NoiseKind::Ocean => {
                let swell = 0.55 + 0.45 * (t * TAU / 9.0).sin() * (t * TAU / 23.0).cos();
                self.low += (white - self.low) * 0.06;
                self.low * swell * 1.1 + brown * 0.18
            }
            NoiseKind::Wind => {
                let gust = 0.5 + 0.5 * (t * TAU / 7.0).sin() * (t * TAU / 17.0 + 1.3).sin();
                let cut = 0.012 + 0.05 * gust;
                self.low += (white - self.low) * cut;
                self.low * (0.9 + gust) * 1.8
            }
            NoiseKind::Brook => {
                let ripple = 0.6 + 0.4 * (t * TAU * 3.1).sin() * (t * TAU * 0.7).sin();
                self.low += (white - self.low) * 0.2;
                self.low2 += (self.low - self.low2) * 0.5;
                (self.low - self.low2) * ripple * 2.2 + pink * 0.1
            }
            NoiseKind::Fan => {
                let hum = (t * TAU * 96.0).sin() * 0.05 + (t * TAU * 192.0).sin() * 0.02;
                self.low += (white - self.low) * 0.15;
                self.low * 0.6 + hum
            }
        };
        sample.clamp(-1.0, 1.0)
    }
}

#[cfg(feature = "audio")]
mod backend {
    use super::*;
    use rodio::source::Source;
    use std::collections::HashMap;
    use std::num::NonZero;
    use std::sync::Mutex;
    use std::sync::mpsc::{self, Sender};
    use std::time::Duration;

    enum Message {
        Tone(Tone, f32),
        NoiseStart(NoiseKind, f32),
        NoiseVolume(f32),
        NoiseStop,
    }

    struct NoiseSource(NoiseGenerator);

    impl Iterator for NoiseSource {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            Some(self.0.next_sample())
        }
    }

    impl Source for NoiseSource {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> NonZero<u16> {
            NonZero::new(1).expect("one channel")
        }
        fn sample_rate(&self) -> NonZero<u32> {
            NonZero::new(SAMPLE_RATE).expect("a sample rate")
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    pub struct Sounds {
        sender: Mutex<Sender<Message>>,
    }

    impl Sounds {
        /// The output device belongs to one thread; everyone else just sends it messages.
        pub fn start() -> Self {
            let (sender, receiver) = mpsc::channel();
            let spawned = std::thread::Builder::new().name("sounds".into()).spawn(move || run(receiver));
            if let Err(error) = spawned {
                crate::vlog!("SOUNDS", "thread failed: {error}");
            }
            Self { sender: Mutex::new(sender) }
        }

        fn send(&self, message: Message) {
            if let Ok(sender) = self.sender.lock() {
                let _ = sender.send(message);
            }
        }

        pub fn play(&self, tone: Tone, volume: f32) {
            self.send(Message::Tone(tone, volume));
        }

        pub fn noise_start(&self, kind: NoiseKind, volume: f32) {
            self.send(Message::NoiseStart(kind, volume));
        }

        pub fn noise_volume(&self, volume: f32) {
            self.send(Message::NoiseVolume(volume));
        }

        pub fn noise_stop(&self) {
            self.send(Message::NoiseStop);
        }
    }

    fn run(receiver: mpsc::Receiver<Message>) {
        let mut sink: Option<rodio::stream::MixerDeviceSink> = None;
        let mut noise: Option<rodio::Player> = None;
        let mut cache: HashMap<Tone, Vec<f32>> = HashMap::new();
        for message in receiver {
            // Opened lazily, so an app that never makes a sound never holds the device.
            if sink.is_none() {
                match rodio::DeviceSinkBuilder::open_default_sink() {
                    Ok(mut opened) => {
                        opened.log_on_drop(false);
                        sink = Some(opened);
                    }
                    Err(error) => {
                        crate::vlog!("SOUNDS", "no output device: {error}");
                        continue;
                    }
                }
            }
            let Some(output) = sink.as_ref() else { continue };
            match message {
                Message::Tone(tone, volume) => {
                    let samples = cache.entry(tone).or_insert_with(|| render_tone(tone)).clone();
                    let player = rodio::Player::connect_new(output.mixer());
                    player.set_volume(volume);
                    let channels = NonZero::new(1).expect("one channel");
                    let rate = NonZero::new(SAMPLE_RATE).expect("a sample rate");
                    player.append(rodio::buffer::SamplesBuffer::new(channels, rate, samples));
                    player.detach();
                }
                Message::NoiseStart(kind, volume) => {
                    let player = rodio::Player::connect_new(output.mixer());
                    player.set_volume(volume);
                    player.append(NoiseSource(NoiseGenerator::new(kind)));
                    noise = Some(player);
                }
                Message::NoiseVolume(volume) => {
                    if let Some(player) = noise.as_ref() {
                        player.set_volume(volume);
                    }
                }
                Message::NoiseStop => {
                    noise = None;
                }
            }
        }
    }
}

#[cfg(feature = "audio")]
pub use backend::Sounds;

/// Without the `audio` feature the app runs silently.
#[cfg(not(feature = "audio"))]
pub struct Sounds;

#[cfg(not(feature = "audio"))]
impl Sounds {
    pub fn start() -> Self {
        Self
    }
    pub fn play(&self, _tone: Tone, _volume: f32) {}
    pub fn noise_start(&self, _kind: NoiseKind, _volume: f32) {}
    pub fn noise_volume(&self, _volume: f32) {}
    pub fn noise_stop(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tones_are_short_audible_and_in_range() {
        for tone in [Tone::Click, Tone::SoftClick, Tone::Tap, Tone::Bong, Tone::Finish] {
            let samples = render_tone(tone);
            assert!(!samples.is_empty() && samples.len() < SAMPLE_RATE as usize * 2);
            let peak = samples.iter().fold(0.0_f32, |p, s| p.max(s.abs()));
            assert!(peak > 0.05 && peak <= 1.2, "{tone:?} peak {peak}");
        }
    }

    #[test]
    fn noise_stays_in_range_and_is_not_silent() {
        for kind in NoiseKind::ALL {
            let mut generator = NoiseGenerator::new(kind);
            let samples: Vec<f32> = (0..SAMPLE_RATE).map(|_| generator.next_sample()).collect();
            assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "{kind:?} clipped");
            let energy = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
            assert!(energy > 1e-5, "{kind:?} is silent");
        }
    }

    #[test]
    fn noise_kinds_sound_different() {
        let energy = |kind| {
            let mut generator = NoiseGenerator::new(kind);
            (0..SAMPLE_RATE).map(|_| generator.next_sample().powi(2)).sum::<f32>()
        };
        assert!((energy(NoiseKind::White) - energy(NoiseKind::Brown)).abs() > 1.0);
    }
}
