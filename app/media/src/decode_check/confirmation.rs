//! Confirm only significant proxy/source count mismatches, in either direction.
use super::*;
use std::time::Duration;

pub(super) fn check(
    probe: &Value,
    streams: &[StreamExtent],
    count: impl FnOnce(Duration) -> Result<Value, cut_core::CutError>,
) -> Value {
    let mut cheap = super::decode_check(probe, streams);
    // Newer encoders coalesce capture-clock timestamps; older ones duplicate
    // them, even across damage. A significant surplus is also a rate conversion
    // cue, not proof that all source pictures decoded. Matching counts stay cheap.
    let surplus = match (
        cheap["claimed_frames"].as_u64(),
        cheap["decodable_frames"].as_u64(),
    ) {
        (Some(claimed), Some(decoded)) => {
            let extra = decoded.saturating_sub(claimed);
            extra > 2 && extra.saturating_mul(200) > claimed
        }
        _ => false,
    };
    // A short picture/sound remains a duration failure without another decode.
    if cheap["code"] != "decode_gap" && !(cheap["code"].is_null() && surplus) {
        return cheap;
    }
    let timeout = timeout(probe.get("duration_ms").and_then(Value::as_u64));
    let confirmation = match count(timeout) {
        Ok(counts) => {
            cheap = super::check_with_source_counts(probe, streams, &counts);
            json!({"confirmed": true, "timeout_ms": timeout.as_millis() as u64})
        }
        Err(error) => json!({
            "confirmed": false, "timeout_ms": timeout.as_millis() as u64,
            "code": error.code, "reason": error.message,
        }),
    };
    cheap["source_count_confirmation"] = confirmation;
    cheap
}

// 30 seconds plus 20% of declared duration, never more than five minutes.
// An inherited operation deadline can only shorten this budget.
fn timeout(duration_ms: Option<u64>) -> Duration {
    Duration::from_millis(
        30_000u64
            .saturating_add(duration_ms.unwrap_or(0) / 5)
            .min(300_000),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn fixture(decoded: u64, duration: u64) -> (Value, Vec<StreamExtent>) {
        (
            json!({"duration_ms": 8000, "fps": 25,
            "raw": {"streams": [{"index": 0, "codec_type": "video",
                "duration": "8", "nb_frames": "200"}]}}),
            vec![StreamExtent {
                kind: "video".into(),
                index: 0,
                duration_ms: Some(duration),
                frames: Some(decoded),
            }],
        )
    }

    #[test]
    fn healthy_constant_rate_source_never_starts_the_counting_helper() {
        let (probe, streams) = fixture(200, 8000);
        let invocations = Cell::new(0);
        let check = check(&probe, &streams, |_| {
            invocations.set(invocations.get() + 1);
            Ok(json!({"streams": [{"index": 0, "nb_read_frames": "200"}]}))
        });
        assert_eq!(
            invocations.get(),
            0,
            "healthy CFR must not decode the source again"
        );
        assert_eq!(check, super::super::decode_check(&probe, &streams));
    }

    #[test]
    fn duration_failure_never_starts_the_counting_helper() {
        let (probe, streams) = fixture(50, 2000);
        let check = check(&probe, &streams, |_| {
            panic!("duration failure needs no count")
        });
        assert_eq!(check, super::super::decode_check(&probe, &streams));
        assert_eq!(check["code"], "decode_short");
    }

    #[test]
    fn failed_confirmation_keeps_the_exact_cheap_warning_and_records_uncertainty() {
        let (probe, streams) = fixture(100, 8000);
        for (code, reason) in [
            (cut_core::error_codes::FFMPEG, "operation timed out"),
            (cut_core::error_codes::RENDER_CANCELLED, "render cancelled"),
        ] {
            let invocations = Cell::new(0);
            let mut checked = check(&probe, &streams, |budget| {
                invocations.set(invocations.get() + 1);
                assert_eq!(budget, Duration::from_millis(31_600));
                Err(cut_core::CutError::new(code, reason, "fixture"))
            });
            assert_eq!(invocations.get(), 1);
            assert_eq!(checked["source_count_confirmation"]["confirmed"], false);
            assert_eq!(checked["source_count_confirmation"]["reason"], reason);
            checked
                .as_object_mut()
                .unwrap()
                .remove("source_count_confirmation");
            assert_eq!(checked, super::super::decode_check(&probe, &streams));
        }
    }

    #[test]
    fn duplicated_proxy_pictures_cannot_hide_source_damage() {
        let (probe, streams) = fixture(240, 8000);
        let invocations = Cell::new(0);
        let checked = check(&probe, &streams, |_| {
            invocations.set(invocations.get() + 1);
            Ok(json!({"streams": [{"index": 0, "nb_read_frames": "170",
                "nb_read_packets": "200"}]}))
        });
        assert_eq!(
            invocations.get(),
            1,
            "a rate-converted surplus must be confirmed"
        );
        assert_eq!(checked["code"], "decode_gap", "{checked}");
        assert_eq!(checked["decodable_frames"], 170);
    }

    #[test]
    fn confirmation_budget_scales_but_is_capped() {
        assert_eq!(timeout(None), Duration::from_secs(30));
        assert_eq!(timeout(Some(60_000)), Duration::from_secs(42));
        assert_eq!(timeout(Some(3_600_000)), Duration::from_secs(300));
        assert_eq!(timeout(Some(u64::MAX)), Duration::from_secs(300));
    }
}
