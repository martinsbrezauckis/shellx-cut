//! SRT export tests — exact-output (no examples/ file for SRT; the format is
//! simple enough that the expected document is asserted verbatim).

mod common;

use common::scenario;
use cut_export::{export_srt, ExportError};

#[test]
fn srt_exact_output_for_scenario() {
    let srt = export_srt(&scenario()).expect("render srt");
    let expected = "1\n\
                    00:00:00,000 --> 00:00:01,200\n\
                    Hello world\n\
                    \n\
                    2\n\
                    00:00:01,500 --> 00:00:02,600\n\
                    Second line\n\
                    \n";
    assert_eq!(srt, expected);
}

#[test]
fn srt_hour_rollover_and_skip_rules() {
    let tl = serde_json::json!({
        "settings": {"fps": 30},
        "assets": {},
        "tracks": [{"id": "cap1", "kind": "caption", "clips": [
            {"id": "s1", "text": "Late cue", "range_ms": [3_661_001, 3_662_500]},
            {"id": "bad1", "text": "", "range_ms": [0, 100]},          // empty text: skipped
            {"id": "bad2", "text": "No range"},                        // missing range: skipped
            {"id": "bad3", "text": "Inverted", "range_ms": [500, 500]} // zero-length: skipped
        ]}]
    });
    let srt = export_srt(&tl).unwrap();
    assert_eq!(srt, "1\n01:01:01,001 --> 01:01:02,500\nLate cue\n\n");
}

#[test]
fn srt_without_caption_track_is_actionable_error() {
    let tl = serde_json::json!({"settings": {"fps": 30}, "assets": {}, "tracks": []});
    let err = export_srt(&tl).unwrap_err();
    assert!(matches!(err, ExportError::NoCaptions));
    assert!(
        err.to_string().contains("captions.generate"),
        "must suggest the fix"
    );
}
