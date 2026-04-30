//! RealtyLens motion tokens.
//!
//! From the spec: "Standard easing: `cubic-bezier(0.16, 1, 0.3, 1)`" with
//! durations between 0.18 s and 0.7 s. Honor `prefers-reduced-motion`.

use std::time::Duration;

/// A cubic Bezier easing represented by its two control points
/// `(x1, y1, x2, y2)` in 0..=1 space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Easing {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl Easing {
    pub const fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// Format as a CSS-style string for any consumer that wants to hand
    /// the curve to a CSS pipeline.
    pub fn to_css(self) -> String {
        format!(
            "cubic-bezier({:.3},{:.3},{:.3},{:.3})",
            self.x1, self.y1, self.x2, self.y2
        )
    }
}

/// The signature RealtyLens easing — same value as the CSS `--ease-rl`.
pub const MOTION_EASE_RL: Easing = Easing::cubic_bezier(0.16, 1.0, 0.30, 1.0);

/// 180 ms — used for hover, focus, small layout shifts.
pub const MOTION_DURATION_SHORT: Duration = Duration::from_millis(180);

/// 700 ms — used for hero entrances and `fadeUp`.
pub const MOTION_DURATION_LONG: Duration = Duration::from_millis(700);

/// 360 ms — palette open, panel slide, tab switch.
pub const MOTION_DURATION_DEFAULT: Duration = Duration::from_millis(360);

/// Returns whether motion should be suppressed. Linear Taco honors
/// `prefers-reduced-motion: reduce` globally — when that's set, transitions
/// flip to instant. The Warp-side wire-in feeds the system signal in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReduceMotion(pub bool);

impl ReduceMotion {
    pub const fn enabled(self) -> bool {
        self.0
    }

    /// Pick `dur` if motion is allowed, otherwise `Duration::ZERO`.
    pub fn duration(self, dur: Duration) -> Duration {
        if self.0 { Duration::ZERO } else { dur }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_serialization_matches_realtylens_spec() {
        assert_eq!(
            MOTION_EASE_RL.to_css(),
            "cubic-bezier(0.160,1.000,0.300,1.000)"
        );
    }

    #[test]
    fn reduced_motion_zeroes_durations() {
        assert_eq!(
            ReduceMotion(true).duration(MOTION_DURATION_LONG),
            Duration::ZERO
        );
        assert_eq!(
            ReduceMotion(false).duration(MOTION_DURATION_LONG),
            MOTION_DURATION_LONG
        );
    }
}
