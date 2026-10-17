//! Time-based tweens and the easing curves the flyout's motion uses.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    Linear,
    CubicIn,
    CubicOut,
    /// XAML's ExponentialEase with EasingMode="EaseIn" and the given exponent.
    ExponentialIn(f32),
    /// XAML's ExponentialEase with EasingMode="EaseOut" and the given exponent.
    ExponentialOut(f32),
}

impl Ease {
    /// Maps linear progress in 0..=1 to eased progress, also in 0..=1.
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            Ease::CubicIn => t * t * t,
            Ease::CubicOut => 1.0 - (1.0 - t).powi(3),
            Ease::ExponentialIn(k) => ((k * t).exp() - 1.0) / (k.exp() - 1.0),
            Ease::ExponentialOut(k) => 1.0 - Ease::ExponentialIn(k).apply(1.0 - t),
        }
    }
}

/// A value moving from `from` to `to` over a duration, starting after an optional delay.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    start: Instant,
    delay: Duration,
    duration: Duration,
    from: f32,
    to: f32,
    ease: Ease,
}

impl Tween {
    pub fn new(from: f32, to: f32, millis: u64, ease: Ease) -> Self {
        Self::starting_at(Instant::now(), from, to, millis, ease)
    }

    pub fn starting_at(start: Instant, from: f32, to: f32, millis: u64, ease: Ease) -> Self {
        Self { start, delay: Duration::ZERO, duration: Duration::from_millis(millis), from, to, ease }
    }

    pub fn delayed(self, millis: u64) -> Self {
        Self { delay: Duration::from_millis(millis), ..self }
    }

    pub fn value(&self, now: Instant) -> f32 {
        self.from + (self.to - self.from) * self.ease.apply(self.progress(now))
    }

    pub fn is_done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    fn progress(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.start).saturating_sub(self.delay);
        if self.duration.is_zero() {
            return 1.0;
        }
        (elapsed.as_secs_f32() / self.duration.as_secs_f32()).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EASES: [Ease; 5] = [Ease::Linear, Ease::CubicIn, Ease::CubicOut, Ease::ExponentialIn(7.0), Ease::ExponentialOut(4.0)];

    #[test]
    fn apply_endpoints_are_fixed() {
        for ease in EASES {
            assert!(ease.apply(0.0).abs() < 1e-6, "{ease:?}");
            assert!((ease.apply(1.0) - 1.0).abs() < 1e-6, "{ease:?}");
        }
    }

    #[test]
    fn apply_is_monotonic() {
        for ease in EASES {
            let samples: Vec<f32> = (0..=20).map(|i| ease.apply(i as f32 / 20.0)).collect();
            assert!(samples.windows(2).all(|w| w[1] >= w[0]), "{ease:?}");
        }
    }

    #[test]
    fn apply_out_of_range_clamps() {
        assert_eq!(Ease::Linear.apply(-1.0), 0.0);
        assert_eq!(Ease::Linear.apply(2.0), 1.0);
    }

    #[test]
    fn value_follows_time() {
        let start = Instant::now();
        let tween = Tween::starting_at(start, 10.0, 20.0, 100, Ease::Linear);

        assert_eq!(tween.value(start), 10.0);
        assert!((tween.value(start + Duration::from_millis(50)) - 15.0).abs() < 1e-3);
        assert_eq!(tween.value(start + Duration::from_millis(500)), 20.0);
        assert!(tween.is_done(start + Duration::from_millis(100)));
    }

    #[test]
    fn value_waits_for_delay() {
        let start = Instant::now();
        let tween = Tween::starting_at(start, 0.0, 1.0, 100, Ease::Linear).delayed(60);

        assert_eq!(tween.value(start + Duration::from_millis(60)), 0.0);
        assert!(!tween.is_done(start + Duration::from_millis(100)));
        assert!(tween.is_done(start + Duration::from_millis(160)));
    }
}
