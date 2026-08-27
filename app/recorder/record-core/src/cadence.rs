//! Versioned capture-cadence evidence kept separate from legacy render settings.
//!
//! `Settings.fps` remains the historical f32 presentation/render timebase under
//! ARCH-TIME-01. Capture intent and measured media facts belong here instead so
//! a decimal request is never reconstructed from that lossy legacy field.

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

pub const CAPTURE_CADENCE_SCHEMA: &str = "shellx-record/capture-cadence/1";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CadenceError {
    #[error("frame-rate rational must have non-zero numerator and denominator")]
    ZeroPart,
    #[error("frame-rate decimal must be finite and positive")]
    InvalidDecimal,
    #[error("frame-rate decimal cannot be represented as a bounded rational")]
    Overflow,
}

/// A positive, reduced exact rate. It serializes as `{num, den}` so API clients
/// can keep NTSC and decimal intent without an IEEE float round-trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FrameRate {
    pub num: u64,
    pub den: u64,
}

impl FrameRate {
    pub fn new(num: u64, den: u64) -> Result<Self, CadenceError> {
        if num == 0 || den == 0 {
            return Err(CadenceError::ZeroPart);
        }
        let divisor = gcd(num, den);
        Ok(Self {
            num: num / divisor,
            den: den / divisor,
        })
    }

    /// Canonically preserve the finite decimal spelling selected by the server
    /// JSON parser. `f64::to_string` supplies its shortest round-trippable
    /// decimal (for example `29.97`), never the binary expansion and never a
    /// narrowed legacy f32.
    pub fn from_server_decimal(value: f64) -> Result<Self, CadenceError> {
        if !value.is_finite() || value <= 0.0 {
            return Err(CadenceError::InvalidDecimal);
        }
        from_decimal_spelling(&value.to_string())
    }

    /// Parse FFprobe's exact `numerator/denominator` form. Invalid, zero, and
    /// non-rational values are deliberately absent rather than guessed.
    pub fn from_ffprobe(value: &str) -> Option<Self> {
        let (num, den) = value.trim().split_once('/')?;
        if den.contains('/') {
            return None;
        }
        let num = num.parse::<u64>().ok()?;
        let den = den.parse::<u64>().ok()?;
        Self::new(num, den).ok()
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

impl<'de> Deserialize<'de> for FrameRate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawFrameRate {
            num: u64,
            den: u64,
        }
        let raw = RawFrameRate::deserialize(deserializer)?;
        Self::new(raw.num, raw.den).map_err(D::Error::custom)
    }
}

/// The current native request policy is intentionally modest and shared by all
/// backends: nearest whole FPS, bounded to their v1 1..240 request range.
pub fn backend_requested_v1(requested: FrameRate) -> FrameRate {
    let rounded =
        (u128::from(requested.num) + u128::from(requested.den) / 2) / u128::from(requested.den);
    let bounded =
        u64::try_from(rounded.clamp(1, 240)).expect("bounded backend FPS always fits in u64");
    FrameRate::new(bounded, 1).expect("bounded backend FPS is always valid")
}

/// Compatibility adapter for direct native-backend callers. Normal server
/// starts construct [`CaptureCadence`] first; this keeps every backend on the
/// same policy if a focused capture test builds `CaptureConfig` directly.
pub fn backend_fps_v1(value: f64) -> u32 {
    if !value.is_finite() {
        return 30;
    }
    if value <= 1.0 {
        return 1;
    }
    if value >= 240.0 {
        return 240;
    }
    FrameRate::from_server_decimal(value)
        .map(backend_requested_v1)
        .map(|rate| rate.num as u32)
        .unwrap_or(30)
}

/// Final-source facts from one successful FFprobe read. Each fact remains
/// optional because FFprobe may only expose a subset; an all-empty object is
/// never persisted as measurement evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProbedMediaCadence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_frame_rate: Option<FrameRate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r_frame_rate: Option<FrameRate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_video_frames: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl ProbedMediaCadence {
    pub fn is_empty(&self) -> bool {
        self.avg_frame_rate.is_none()
            && self.r_frame_rate.is_none()
            && self.decoded_video_frames.is_none()
            && self.duration_ms.is_none()
    }
}

/// Immutable capture intent plus optional final-media evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureCadence {
    pub schema: String,
    pub requested: FrameRate,
    pub backend_requested: FrameRate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probed_media: Option<ProbedMediaCadence>,
}

impl CaptureCadence {
    pub fn from_server_fps(value: f64) -> Result<Self, CadenceError> {
        let requested = FrameRate::from_server_decimal(value)?;
        Ok(Self {
            schema: CAPTURE_CADENCE_SCHEMA.into(),
            backend_requested: backend_requested_v1(requested),
            requested,
            probed_media: None,
        })
    }

    pub fn with_probed_media(mut self, probed_media: ProbedMediaCadence) -> Self {
        self.probed_media = (!probed_media.is_empty()).then_some(probed_media);
        self
    }
}

fn from_decimal_spelling(value: &str) -> Result<FrameRate, CadenceError> {
    let (mantissa, exponent) = match value.find(['e', 'E']) {
        Some(index) => {
            let (mantissa, exponent) = value.split_at(index);
            let exponent = exponent[1..]
                .parse::<i32>()
                .map_err(|_| CadenceError::InvalidDecimal)?;
            (mantissa, exponent)
        }
        None => (value, 0),
    };
    let (whole, fraction) = mantissa
        .split_once('.')
        .map_or((mantissa, ""), |(whole, fraction)| (whole, fraction));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(CadenceError::InvalidDecimal);
    }
    let digits = format!("{whole}{fraction}");
    let numerator = digits.parse::<u128>().map_err(|_| CadenceError::Overflow)?;
    if numerator == 0 {
        return Err(CadenceError::InvalidDecimal);
    }
    let scale =
        i64::try_from(fraction.len()).map_err(|_| CadenceError::Overflow)? - i64::from(exponent);
    let (numerator, denominator) = if scale >= 0 {
        (
            numerator,
            pow10(u32::try_from(scale).map_err(|_| CadenceError::Overflow)?)?,
        )
    } else {
        (
            numerator
                .checked_mul(pow10(
                    u32::try_from(scale.unsigned_abs()).map_err(|_| CadenceError::Overflow)?,
                )?)
                .ok_or(CadenceError::Overflow)?,
            1,
        )
    };
    let divisor = gcd_u128(numerator, denominator);
    let numerator = u64::try_from(numerator / divisor).map_err(|_| CadenceError::Overflow)?;
    let denominator = u64::try_from(denominator / divisor).map_err(|_| CadenceError::Overflow)?;
    FrameRate::new(numerator, denominator)
}

fn pow10(power: u32) -> Result<u128, CadenceError> {
    (0..power).try_fold(1_u128, |value, _| {
        value.checked_mul(10).ok_or(CadenceError::Overflow)
    })
}

fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn gcd_u128(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_decimals_reduce_without_a_legacy_float_round_trip() {
        assert_eq!(
            FrameRate::from_server_decimal(29.97).unwrap(),
            FrameRate::new(2_997, 100).unwrap()
        );
        assert_eq!(
            FrameRate::from_server_decimal(47.5).unwrap(),
            FrameRate::new(95, 2).unwrap()
        );
        assert_eq!(
            FrameRate::from_server_decimal(1.0).unwrap(),
            FrameRate::new(1, 1).unwrap()
        );
    }

    #[test]
    fn decimal_overflow_is_rejected() {
        assert_eq!(
            FrameRate::from_server_decimal(1e-20),
            Err(CadenceError::Overflow)
        );
    }

    #[test]
    fn ffprobe_rates_are_exact_and_zero_zero_is_absent() {
        assert_eq!(
            FrameRate::from_ffprobe("30000/1001"),
            Some(FrameRate::new(30_000, 1_001).unwrap())
        );
        assert_eq!(FrameRate::from_ffprobe("0/0"), None);
        assert_eq!(FrameRate::from_ffprobe("30"), None);
    }

    #[test]
    fn v1_backend_policy_rounds_every_platform_request_the_same_way() {
        let requested = FrameRate::from_server_decimal(29.97).unwrap();
        assert_eq!(
            backend_requested_v1(requested),
            FrameRate::new(30, 1).unwrap()
        );
        assert_eq!(backend_fps_v1(29.97), 30);
    }
}
