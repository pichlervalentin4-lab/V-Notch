//! Spring and easing helpers behind every moving part of the notch.

/// A damped spring. Advance it once per frame with [`Spring::step`].
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SpringConfig {
    pub stiffness: f32,
    pub damping: f32,
}

impl SpringConfig {
    /// `speed` scales the whole motion, `bounce` picks a slightly underdamped curve.
    pub fn new(speed: f32, bounce: bool) -> Self {
        let stiffness = 170.0 * speed.clamp(0.25, 4.0);
        let ratio = if bounce { 0.72 } else { 1.0 };
        Self { stiffness, damping: 2.0 * stiffness.sqrt() * ratio }
    }

    pub const INSTANT: Self = Self { stiffness: 0.0, damping: 0.0 };
}

impl Spring {
    pub fn new(value: f32) -> Self {
        Self { value, velocity: 0.0, target: value }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn snap(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }

    /// Semi-implicit Euler with fixed substeps, so long frames stay stable.
    pub fn step(&mut self, dt: f32, config: SpringConfig) {
        if config.stiffness <= 0.0 {
            self.snap(self.target);
            return;
        }
        let mut left = dt.clamp(0.0, 0.1);
        while left > 0.0 {
            let h = left.min(1.0 / 240.0);
            let accel = config.stiffness * (self.target - self.value) - config.damping * self.velocity;
            self.velocity += accel * h;
            self.value += self.velocity * h;
            left -= h;
        }
        if self.is_settled() {
            self.value = self.target;
            self.velocity = 0.0;
        }
    }

    pub fn is_settled(&self) -> bool {
        (self.value - self.target).abs() < 0.05 && self.velocity.abs() < 0.5
    }
}

pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Exponential approach that does not depend on the frame rate.
pub fn approach(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    let next = target + (current - target) * (-rate * dt).exp();
    if (next - target).abs() < 0.001 { target } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_reaches_target_and_settles() {
        let mut spring = Spring::new(0.0);
        spring.set_target(100.0);
        let config = SpringConfig::new(1.0, true);
        for _ in 0..600 {
            spring.step(1.0 / 60.0, config);
        }
        assert!(spring.is_settled());
        assert_eq!(spring.value, 100.0);
    }

    #[test]
    fn bouncy_spring_overshoots_but_stiff_one_does_not() {
        let run = |bounce| {
            let mut spring = Spring::new(0.0);
            spring.set_target(100.0);
            let config = SpringConfig::new(1.0, bounce);
            let mut peak = 0.0_f32;
            for _ in 0..240 {
                spring.step(1.0 / 60.0, config);
                peak = peak.max(spring.value);
            }
            peak
        };
        assert!(run(true) > 100.5);
        assert!(run(false) <= 100.05);
    }

    #[test]
    fn long_frames_do_not_explode() {
        let mut spring = Spring::new(0.0);
        spring.set_target(1.0);
        spring.step(5.0, SpringConfig::new(4.0, true));
        assert!(spring.value.is_finite() && spring.value.abs() < 10.0);
    }

    #[test]
    fn approach_is_monotonic() {
        let a = approach(0.0, 1.0, 10.0, 0.016);
        let b = approach(a, 1.0, 10.0, 0.016);
        assert!(a > 0.0 && b > a && b < 1.0);
    }
}
