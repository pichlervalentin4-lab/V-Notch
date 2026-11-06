//! The countdown shown in the controls tab and as a live activity in the closed notch.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TimerState {
    #[default]
    Idle,
    Running {
        ends_at: Instant,
    },
    Paused {
        left: Duration,
    },
    Finished {
        at: Instant,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Countdown {
    pub duration: Duration,
    pub state: TimerState,
}

impl Default for Countdown {
    fn default() -> Self {
        Self { duration: Duration::from_secs(5 * 60), state: TimerState::Idle }
    }
}

const STEP: Duration = Duration::from_secs(60);
const MAX: Duration = Duration::from_secs(24 * 3600);

impl Countdown {
    pub fn start(&mut self, now: Instant) {
        self.state = match self.state {
            TimerState::Paused { left } => TimerState::Running { ends_at: now + left },
            _ => TimerState::Running { ends_at: now + self.duration },
        };
    }

    pub fn pause(&mut self, now: Instant) {
        if let TimerState::Running { ends_at } = self.state {
            self.state = TimerState::Paused { left: ends_at.saturating_duration_since(now) };
        }
    }

    pub fn reset(&mut self) {
        self.state = TimerState::Idle;
    }

    pub fn increase(&mut self, now: Instant) {
        self.adjust(now, true);
    }

    pub fn decrease(&mut self, now: Instant) {
        self.adjust(now, false);
    }

    fn adjust(&mut self, now: Instant, up: bool) {
        let change = |value: Duration| {
            if up { (value + STEP).min(MAX) } else { value.saturating_sub(STEP).max(Duration::from_secs(1)) }
        };
        match self.state {
            TimerState::Idle | TimerState::Finished { .. } => self.duration = change(self.duration),
            TimerState::Running { ends_at } => {
                let left = change(ends_at.saturating_duration_since(now));
                self.state = TimerState::Running { ends_at: now + left };
            }
            TimerState::Paused { left } => self.state = TimerState::Paused { left: change(left) },
        }
    }

    pub fn left(&self, now: Instant) -> Duration {
        match self.state {
            TimerState::Idle => self.duration,
            TimerState::Running { ends_at } => ends_at.saturating_duration_since(now),
            TimerState::Paused { left } => left,
            TimerState::Finished { .. } => Duration::ZERO,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, TimerState::Running { .. })
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.state, TimerState::Idle)
    }

    /// Call once a frame. True on the frame the countdown reaches zero.
    pub fn tick(&mut self, now: Instant) -> bool {
        if let TimerState::Running { ends_at } = self.state
            && now >= ends_at
        {
            self.state = TimerState::Finished { at: now };
            return true;
        }
        false
    }
}

/// "1:05:09" or "5:09".
pub fn format_left(left: Duration) -> String {
    // Rounded up, so a fresh 5:00 shows 5:00 and it only reads 0:00 at the very end.
    let total = (left.as_millis().div_ceil(1000)) as u64;
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_pauses_and_resumes() {
        let t0 = Instant::now();
        let mut timer = Countdown { duration: Duration::from_secs(100), state: TimerState::Idle };
        timer.start(t0);
        assert_eq!(timer.left(t0 + Duration::from_secs(10)), Duration::from_secs(90));
        timer.pause(t0 + Duration::from_secs(10));
        assert_eq!(timer.left(t0 + Duration::from_secs(50)), Duration::from_secs(90));
        timer.start(t0 + Duration::from_secs(50));
        assert_eq!(timer.left(t0 + Duration::from_secs(60)), Duration::from_secs(80));
    }

    #[test]
    fn finishes_exactly_once() {
        let t0 = Instant::now();
        let mut timer = Countdown { duration: Duration::from_secs(5), state: TimerState::Idle };
        timer.start(t0);
        assert!(!timer.tick(t0 + Duration::from_secs(4)));
        assert!(timer.tick(t0 + Duration::from_secs(5)));
        assert!(!timer.tick(t0 + Duration::from_secs(6)));
        assert_eq!(timer.left(t0 + Duration::from_secs(6)), Duration::ZERO);
    }

    #[test]
    fn adjusting_stays_within_limits() {
        let now = Instant::now();
        let mut timer = Countdown { duration: Duration::from_secs(30), state: TimerState::Idle };
        timer.decrease(now);
        assert_eq!(timer.duration, Duration::from_secs(1));
        timer.duration = MAX;
        timer.increase(now);
        assert_eq!(timer.duration, MAX);
    }

    #[test]
    fn adjusting_a_running_timer_moves_its_end() {
        let now = Instant::now();
        let mut timer = Countdown { duration: Duration::from_secs(60), state: TimerState::Idle };
        timer.start(now);
        timer.increase(now);
        assert_eq!(timer.left(now), Duration::from_secs(120));
    }

    #[test]
    fn formatting() {
        assert_eq!(format_left(Duration::from_secs(309)), "5:09");
        assert_eq!(format_left(Duration::from_secs(3909)), "1:05:09");
        assert_eq!(format_left(Duration::from_millis(1)), "0:01");
        assert_eq!(format_left(Duration::ZERO), "0:00");
    }
}
