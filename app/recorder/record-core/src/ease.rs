//! ease.rs — easing curves for keyframe interpolation.
//!
//! The whole point of "polished without editing" is that auto-zoom moves with
//! eased acceleration, not the robotic linear ramp Cut's native keyframes are
//! limited to. `Ease::apply` maps normalized progress t∈[0,1] → eased [0,1].
//! `EaseInOut` (smooth accel + decel) is the default for zoom.

use serde::{Deserialize, Serialize};

/// Easing curve applied from a keyframe to the next one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ease {
    /// Constant velocity.
    Linear,
    /// Quadratic ease-in (slow start).
    EaseIn,
    /// Quadratic ease-out (slow stop).
    EaseOut,
    /// Cubic ease-in-out (slow start AND stop) — the natural zoom feel.
    #[default]
    EaseInOut,
}

impl Ease {
    /// Map normalized progress `t` in [0,1] to an eased value in [0,1].
    /// Inputs are clamped, so callers need not pre-clamp.
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            Ease::EaseIn => t * t,
            Ease::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Ease::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_are_fixed() {
        for e in [Ease::Linear, Ease::EaseIn, Ease::EaseOut, Ease::EaseInOut] {
            assert!((e.apply(0.0) - 0.0).abs() < 1e-9, "{e:?} f(0)");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-9, "{e:?} f(1)");
        }
    }

    #[test]
    fn monotonic_nondecreasing() {
        for e in [Ease::Linear, Ease::EaseIn, Ease::EaseOut, Ease::EaseInOut] {
            let mut prev = -1.0;
            for i in 0..=100 {
                let v = e.apply(i as f64 / 100.0);
                assert!(v >= prev - 1e-9, "{e:?} not monotonic at {i}");
                prev = v;
            }
        }
    }

    #[test]
    fn clamps_out_of_range() {
        assert_eq!(Ease::EaseInOut.apply(-5.0), 0.0);
        assert_eq!(Ease::EaseInOut.apply(5.0), 1.0);
    }
}
