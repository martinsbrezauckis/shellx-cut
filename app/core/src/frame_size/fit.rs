//! Bring a project format that came from a journal, a history snapshot or a
//! cached `project.json` inside the limits Cut makes (item 141).
//!
//! The live verbs (`project.create`, `project.format`, first-import adoption)
//! refuse a format outside the limit. A received project's files are not made
//! by those verbs, so every project rebuilt from them passes through
//! [`fit_project_formats`]. Opening never refuses the project for its format:
//! the person keeps every edit; the format is reduced to the largest frame of
//! the same shape that Cut makes, and the open reports what changed. Nothing is
//! written to the journal by opening; the next format change the person makes
//! is an ordinary, checked `project.format`.

use super::{check_frame_size, MAX_FRAME_PIXELS, MAX_FRAME_SIDE, MIN_FRAME_SIDE};
use crate::{Project, ProjectSettings};

/// Fallback when a stored side is zero, so no shape can be kept.
const FALLBACK: (u32, u32) = (1920, 1080);
/// Frame rates outside `(0, MAX_FPS]` are refused by `project.format`.
const MAX_FPS: f64 = 240.0;
const FALLBACK_FPS: f64 = 30.0;

/// One format that was outside the limits and what it became.
#[derive(Debug, Clone, PartialEq)]
pub struct FormatFit {
    /// The sequence whose stored settings changed; `None` for the current one.
    pub sequence: Option<String>,
    pub from: (u32, u32, f64),
    pub to: (u32, u32, f64),
}

impl FormatFit {
    /// One sentence for the person and the agent.
    pub fn message(&self) -> String {
        let (fw, fh, ff) = self.from;
        let (tw, th, tf) = self.to;
        let what = match &self.sequence {
            Some(id) => format!("Sequence '{id}' had"),
            None => "This project had".to_string(),
        };
        let rate = if ff.to_bits() == tf.to_bits() {
            String::new()
        } else {
            format!(" at {ff} fps")
        };
        let to_rate = if ff.to_bits() == tf.to_bits() {
            String::new()
        } else {
            format!(" at {tf} fps")
        };
        format!(
            "{what} the format {fw}x{fh}{rate}, which is larger than Cut makes; \
             it is shown and exported as {tw}x{th}{to_rate}. Choose another format \
             in the project settings if you need one."
        )
    }
}

/// A stored frame within the side and area limits, unchanged (an odd size
/// stays odd: the person is asked to change it, it is never changed for
/// them; owner decision 2026-11-19). A frame beyond the limits becomes the
/// largest even frame of the same shape within them.
pub fn fit_frame(width: u32, height: u32) -> (u32, u32) {
    if check_frame_size(width, height, "project format").is_ok() {
        return (width, height);
    }
    if width == 0 || height == 0 {
        return FALLBACK;
    }
    let (w, h) = (f64::from(width), f64::from(height));
    let max_side = f64::from(MAX_FRAME_SIDE);
    let scale = (max_side / w)
        .min(max_side / h)
        .min((MAX_FRAME_PIXELS as f64 / (w * h)).sqrt())
        .min(1.0);
    let even = |v: f64| {
        let v = (v.floor() as u32).clamp(MIN_FRAME_SIDE, MAX_FRAME_SIDE);
        v - v % 2
    };
    let (mut tw, mut th) = (even(w * scale), even(h * scale));
    // Rounding and the minimum side can still leave the area just over the
    // limit for an extreme shape: shave the longer side until it fits.
    while u64::from(tw) * u64::from(th) > MAX_FRAME_PIXELS {
        if tw >= th {
            tw -= 2;
        } else {
            th -= 2;
        }
    }
    (tw, th)
}

fn fit_settings(settings: &mut ProjectSettings) -> Option<((u32, u32, f64), (u32, u32, f64))> {
    let from = (settings.width, settings.height, settings.fps);
    let (w, h) = fit_frame(settings.width, settings.height);
    let fps = if !settings.fps.is_finite() || settings.fps <= 0.0 {
        FALLBACK_FPS
    } else {
        settings.fps.min(MAX_FPS)
    };
    let to = (w, h, fps);
    if from.0 == to.0 && from.1 == to.1 && from.2.to_bits() == to.2.to_bits() {
        return None;
    }
    settings.width = w;
    settings.height = h;
    settings.fps = fps;
    Some((from, to))
}

/// Fit the current settings and every stored sequence's settings. Returns
/// what changed (empty for every project Cut itself made).
pub fn fit_project_formats(project: &mut Project) -> Vec<FormatFit> {
    let mut fits = Vec::new();
    if let Some((from, to)) = fit_settings(&mut project.settings) {
        fits.push(FormatFit {
            sequence: None,
            from,
            to,
        });
    }
    let active = project.active_sequence.clone();
    for sequence in &mut project.sequences {
        if let Some((from, to)) = fit_settings(&mut sequence.settings) {
            // The active sequence mirrors the current settings: report it once.
            if sequence.id != active {
                fits.push(FormatFit {
                    sequence: Some(sequence.id.clone()),
                    from,
                    to,
                });
            }
        }
    }
    fits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_size::check_project_frame_size;

    #[test]
    fn every_fitted_frame_passes_the_project_check_and_keeps_its_shape() {
        for (w, h) in [
            (8192, 8192),
            (16384, 16384),
            (100_000, 16),
            (16, 100_000),
            (u32::MAX, u32::MAX),
            (8193, 4321),
            (8191, 4321), // odd and just over the area: reduced to even
            (8, 8),
            (3, 100_000),
            (12288, 6480),
        ] {
            let (tw, th) = fit_frame(w, h);
            assert!(
                check_project_frame_size(tw, th).is_ok(),
                "{w}x{h} -> {tw}x{th}"
            );
            let (a, b) = (f64::from(w) / f64::from(h), f64::from(tw) / f64::from(th));
            // The shape is kept unless a side had to be raised to the minimum.
            if tw > MIN_FRAME_SIDE && th > MIN_FRAME_SIDE {
                assert!(
                    (a / b - 1.0).abs() < 0.01,
                    "{w}x{h} -> {tw}x{th} changed shape"
                );
            }
        }
        assert_eq!(fit_frame(8192, 8192), (5948, 5948));
        assert_eq!(fit_frame(0, 1080), FALLBACK);
    }

    #[test]
    fn frames_within_the_limit_are_unchanged() {
        for (w, h) in [
            (1920, 1080),
            (7680, 4320),
            (8192, 4320),
            (4320, 8192),
            (16, 16),
            // Odd sizes within the limits are kept as stored: the person is
            // asked to change them (owner decision 2026-11-19).
            (1281, 721),
            (641, 361),
            (7681, 4321),
            (8191, 4319),
        ] {
            assert_eq!(fit_frame(w, h), (w, h));
        }
    }
}
