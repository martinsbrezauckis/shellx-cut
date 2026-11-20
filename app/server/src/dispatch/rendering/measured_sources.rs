//! The engine's own measurement of the sources a render placed.
//!
//! Which quality checks apply to an export (no sound: loudness does not
//! apply; only stills: frozen frames are the design) is decided from facts
//! this engine measured, never from the probe facts cached in `project.json`:
//! a received project can state those falsely ("silent" for an audible clip,
//! "image" for moving video) and so waive a check that would have failed. Each
//! placed source is probed again here, for this render; a source that cannot
//! be probed is left out, and an unknown source waives nothing.

use cut_core::{Edl, Project, TrackKind};
use cut_perception::{MeasuredSources, SourceFacts};
use std::collections::BTreeSet;
use std::path::Path;

/// Probe every distinct source placed on a picture or program-audio track.
pub(super) fn measure_placed_sources(dir: &Path, project: &Project, edl: &Edl) -> MeasuredSources {
    let placed: BTreeSet<&str> = edl
        .segments
        .iter()
        .filter(|s| s.track_kind == TrackKind::Video || edl.is_audio_bearing_track(&s.track))
        .filter_map(|s| s.asset.as_deref())
        .collect();
    let mut measured = MeasuredSources::new();
    for id in placed {
        let Some(asset) = project.assets.get(id) else {
            continue;
        };
        let path = crate::dispatch::media::asset_source_path(dir, asset);
        if let Ok(probe) = cut_media::probe(&path) {
            measured.insert(
                id.to_string(),
                SourceFacts {
                    has_audio: probe.has_audio,
                    still: is_single_picture(&path, &probe),
                },
            );
        }
    }
    measured
}

/// A still is a source measured to hold exactly ONE picture: an image-kind
/// probe with no duration and no frame rate, whose video packets the engine
/// counted as one. An image sequence, an animated or multi-frame image, or a
/// count that could not be made is moving video for the checks.
fn is_single_picture(path: &Path, probe: &cut_media::MediaProbe) -> bool {
    if probe.kind != cut_media::probe::kinds::IMAGE
        || probe.duration_ms.is_some_and(|ms| ms > 0)
        || probe.fps.is_some_and(|fps| fps > 0.0)
    {
        return false;
    }
    let video_index = probe.raw["streams"].as_array().and_then(|streams| {
        streams
            .iter()
            .find(|s| s["codec_type"] == "video")
            .and_then(|s| s["index"].as_u64())
    });
    let Some(video_index) = video_index else {
        return false;
    };
    cut_media::ffmpeg::ffprobe_packet_counts(path).is_ok_and(|counts| {
        counts
            .iter()
            .find(|(index, _)| u64::from(*index) == video_index)
            .is_some_and(|(_, packets)| *packets == 1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cut_core::{edl_from_project, Clip, ProjectSettings};
    use serde_json::json;

    fn png(path: &Path) {
        let made = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=64x36",
            ])
            .args(["-frames:v", "1", "-update", "1"])
            .arg(path)
            .status()
            .unwrap();
        assert!(made.success());
    }

    fn placed(path: &Path) -> (Project, Edl) {
        let mut p = Project::new("t", ProjectSettings::default());
        p.assets.insert(
            "a1".into(),
            serde_json::from_value(json!({"path": path, "hash": "h"})).unwrap(),
        );
        let clip: Clip = serde_json::from_value(
            json!({"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 2_000}),
        )
        .unwrap();
        p.tracks[0].clips.push(clip);
        let edl = edl_from_project(&p);
        (p, edl)
    }

    /// One picture is a still; a name the image reader expands to a sequence
    /// of pictures is moving video for the checks, though it probes as an image.
    #[test]
    fn only_a_single_measured_picture_counts_as_a_still() {
        let dir = tempfile::tempdir().unwrap();
        let single = dir.path().join("card.png");
        png(&single);
        let (p, edl) = placed(&single);
        assert!(measure_placed_sources(dir.path(), &p, &edl)["a1"].still);

        for n in 1..=3 {
            png(&dir.path().join(format!("seq{n:03}.png")));
        }
        // A missing seq%03d.png beside seq001..seq003.png must not read the neighbors.
        let pattern_path = dir.path().join("seq%03d.png");
        let (p, edl) = placed(&pattern_path);
        assert!(measure_placed_sources(dir.path(), &p, &edl).is_empty());

        // When literal seq%03d.png exists beside seq001..seq003.png, only that one picture
        // is measured, so it counts as a single still rather than a multi-frame sequence.
        png(&pattern_path);
        let measured = measure_placed_sources(dir.path(), &p, &edl);
        assert_eq!(
            measured.get("a1").map(|f| f.still),
            Some(true),
            "{measured:?}"
        );

        // An actual animated/multi-frame picture (animated GIF) is not a single still.
        let anim = dir.path().join("multi.gif");
        let made = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=64x48:r=10:d=0.3",
                "-c:v",
                "gif",
            ])
            .arg(&anim)
            .status()
            .unwrap();
        assert!(made.success(), "animated gif fixture failed");
        let (p, edl) = placed(&anim);
        let measured = measure_placed_sources(dir.path(), &p, &edl);
        assert_eq!(
            measured.get("a1").map(|f| f.still),
            Some(false),
            "{measured:?}"
        );
    }
}
