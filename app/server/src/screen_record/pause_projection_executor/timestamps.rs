//! Streaming admission of source presentation timing before CFR normalization.
use std::{
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
};

use cut_core::{error_codes, CutError};
use cut_media::ffmpeg::{
    run_owned_command_with_stdout_lines, OwnedProcessControl, LOCAL_INPUT_FORMATS,
    LOCAL_INPUT_PROTOCOLS,
};

use super::types::VerifiedPauseProjectionSource;

pub(super) fn verify(
    ffprobe: &str,
    path: &Path,
    source: &VerifiedPauseProjectionSource,
    control: &OwnedProcessControl,
) -> Result<(), CutError> {
    let mut command = Command::new(ffprobe);
    command
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            LOCAL_INPUT_PROTOCOLS,
            "-format_whitelist",
            LOCAL_INPUT_FORMATS,
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=time_base:frame=best_effort_timestamp",
            "-of",
            "compact=p=1:nk=0",
        ])
        .arg(path);
    let state = Arc::new(Mutex::new(SourceTiming::default()));
    let observed = state.clone();
    let output = run_owned_command_with_stdout_lines(
        &mut command,
        control,
        "verify pause source timestamps",
        Arc::new(move |line| {
            if let Ok(mut state) = observed.lock() {
                state.observe(line);
            }
        }),
    )?;
    if !output.status.success() || !output.stderr.is_empty() {
        return Err(invalid("source presentation timestamp probe failed"));
    }
    let state = state
        .lock()
        .map_err(|_| invalid("source timestamp observer failed"))?;
    state.finish(source.decoded_video_frames(), source.duration_ms())
}

/// Constant memory regardless of recording length. Owned stdout diagnostics
/// may be truncated; admission consumes every bounded protocol row through EOF.
#[derive(Default)]
struct SourceTiming {
    count: u64,
    first: Option<i64>,
    previous: Option<i64>,
    time_base: Option<(u64, u64)>,
    error: Option<&'static str>,
}

impl SourceTiming {
    fn observe(&mut self, line: &str) {
        if self.error.is_some() {
            return;
        }
        // The bounded reader splits on LF; Windows ffprobe emits CRLF. Remove
        // only its terminal CR so malformed field whitespace remains invalid.
        let line = line.strip_suffix('\r').unwrap_or(line);
        // Scalar rows are far smaller than the owned reader's 16-KiB line cap.
        // Reject long rows so truncated lines cannot masquerade as valid facts.
        if line.len() > 96 {
            self.error = Some("source timestamp record is oversized");
            return;
        }
        if let Some(value) = line.strip_prefix("frame|best_effort_timestamp=") {
            // ffprobe adds one empty field for an unrequested side-data entry.
            let value = value.strip_suffix('|').unwrap_or(value);
            let Some(pts) = value.parse::<i64>().ok().filter(|pts| *pts >= 0) else {
                self.error = Some("source frame has missing or negative presentation time");
                return;
            };
            if self.previous.is_some_and(|previous| pts <= previous) {
                self.error = Some("source presentation times are duplicate or nonmonotonic");
                return;
            }
            let Some(count) = self.count.checked_add(1) else {
                self.error = Some("source timestamp count overflowed");
                return;
            };
            self.count = count;
            self.first.get_or_insert(pts);
            self.previous = Some(pts);
        } else if let Some(value) = line.strip_prefix("stream|time_base=") {
            if self.time_base.is_some() {
                self.error = Some("source has multiple presentation time bases");
                return;
            }
            self.time_base = value
                .split_once('/')
                .and_then(|(num, den)| Some((num.parse::<u64>().ok()?, den.parse::<u64>().ok()?)))
                .filter(|(num, den)| *num > 0 && *den > 0);
            if self.time_base.is_none() {
                self.error = Some("source has no positive rational time base");
            }
        } else {
            self.error =
                Some("source timestamp evidence is malformed or missing a presentation time");
        }
    }

    fn finish(&self, frames: u64, duration_ms: u64) -> Result<(), CutError> {
        if let Some(error) = self.error {
            return Err(invalid(error));
        }
        if self.count != frames || frames == 0 {
            return Err(invalid(
                "source timestamp count differs from verified decoded frames",
            ));
        }
        let (num, den) = self
            .time_base
            .ok_or_else(|| invalid("source has no positive rational time base"))?;
        let (Some(first), Some(last)) = (self.first, self.previous) else {
            return Err(invalid("source presentation timestamps are missing"));
        };
        // The journal rounds duration to ms. Admit only that inherent half-ms
        // rounding interval; the requested output rate never enters this check.
        let elapsed = u128::from((last - first) as u64)
            .checked_mul(u128::from(num))
            .and_then(|ticks| ticks.checked_mul(2000))
            .ok_or_else(|| invalid("source timestamp arithmetic overflowed"))?;
        let bound = (u128::from(duration_ms) * 2 + 1)
            .checked_mul(u128::from(den))
            .ok_or_else(|| invalid("source timestamp arithmetic overflowed"))?;
        if elapsed >= bound {
            return Err(invalid(
                "source presentation time exceeds its sealed media duration",
            ));
        }
        Ok(())
    }
}

fn invalid(cause: &str) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "cannot admit pause source timing",
        cause,
    )
}

#[cfg(test)]
mod tests {
    use super::SourceTiming;
    fn timing(lines: &[&str]) -> SourceTiming {
        let mut state = SourceTiming::default();
        for line in lines {
            state.observe(line);
        }
        state
    }
    #[test]
    fn accepts_retained_mac_vfr_timing_and_rejects_invalid_evidence() {
        let lines = [
            "frame|best_effort_timestamp=0|",
            "frame|best_effort_timestamp=30",
            "frame|best_effort_timestamp=50",
            "frame|best_effort_timestamp=80",
            "frame|best_effort_timestamp=110",
            "stream|time_base=1/600",
        ];
        let valid = timing(&lines);
        assert!(valid.finish(5, 200).is_ok());
        assert!(valid.finish(4, 200).is_err());
        assert!(valid.finish(5, 100).is_err());
        for bad in [
            "stream|time_base=0/600",
            "stream|time_base=1/0",
            "stream|time_base=unknown",
        ] {
            let mut rows = lines;
            rows[5] = bad;
            assert!(timing(&rows).finish(5, 200).is_err());
        }
        for bad in [
            "frame|best_effort_timestamp=30",
            "frame|best_effort_timestamp=-1",
            "frame|best_effort_timestamp=N/A",
            "frame|",
            "garbage",
        ] {
            let mut rows = lines;
            rows[2] = bad;
            assert!(timing(&rows).finish(5, 200).is_err());
        }
        assert!(timing(&lines[..5]).finish(5, 200).is_err());
        let mut oversized = timing(&lines);
        oversized.observe(&"x".repeat(20000));
        assert!(oversized.finish(5, 200).is_err());
    }
    #[test]
    fn accepts_lf_and_crlf_scalar_side_data_and_time_base_rows_equally() {
        for ending in ["", "\r"] {
            let mut state = SourceTiming::default();
            for row in [
                "frame|best_effort_timestamp=0|",
                "frame|best_effort_timestamp=30",
                "frame|best_effort_timestamp=50",
                "stream|time_base=1/600",
            ] {
                state.observe(&format!("{row}{ending}"));
            }
            assert!(state.finish(3, 100).is_ok());
            assert!(state.finish(2, 100).is_err());
            assert!(state.finish(3, 50).is_err());
        }
    }

    #[test]
    fn crlf_normalization_keeps_invalid_timestamp_evidence_rejected() {
        for ending in ["", "\r"] {
            for bad in [
                "frame|best_effort_timestamp=0",
                "frame|best_effort_timestamp=-1",
                "frame|best_effort_timestamp=N/A",
                "frame|best_effort_timestamp=30 ",
                "frame|best_effort_timestamp=30||",
                "frame|best_effort_timestamp=30\r\r",
                "frame|best_effort_timestamp=3\r0",
                "frame|",
                "garbage",
            ] {
                let mut state = SourceTiming::default();
                state.observe(&format!("frame|best_effort_timestamp=0|{ending}"));
                state.observe(&format!("{bad}{ending}"));
                state.observe(&format!("stream|time_base=1/600{ending}"));
                // A terminal CR is one supported line ending. A second CR or
                // an embedded CR must never turn malformed fields into facts.
                assert!(state.finish(2, 100).is_err(), "accepted {bad:?}{ending:?}");
            }
            for bad in [
                "stream|time_base=0/600",
                "stream|time_base=1/0",
                "stream|time_base=unknown",
                "stream|time_base=1/600 ",
                "stream|time_base=1/6\r00",
            ] {
                let mut state = SourceTiming::default();
                state.observe(&format!("frame|best_effort_timestamp=0{ending}"));
                state.observe(&format!("{bad}{ending}"));
                assert!(state.finish(1, 100).is_err(), "accepted {bad:?}{ending:?}");
            }
            let mut state = SourceTiming::default();
            state.observe(&format!("{}{ending}", "x".repeat(97)));
            assert!(state.finish(1, 100).is_err());
            let mut state = SourceTiming::default();
            state.observe(&format!("frame|best_effort_timestamp=0{ending}"));
            state.observe(&format!("stream|time_base=1/600{ending}"));
            state.observe(&format!("stream|time_base=1/600{ending}"));
            assert!(state.finish(1, 100).is_err());
        }
    }

    #[test]
    fn validates_a_long_uninterrupted_run_without_retaining_rows() {
        let mut state = SourceTiming::default();
        // Over 30 MiB of probe rows, beyond the former 8-MiB JSON admission cap.
        for pts in 0..1_000_000 {
            state.observe(&format!("frame|best_effort_timestamp={pts}"));
        }
        state.observe("stream|time_base=1/24");
        assert!(state.finish(1_000_000, 41_666_667).is_ok());
        assert_eq!(state.count, 1_000_000);
    }
}
