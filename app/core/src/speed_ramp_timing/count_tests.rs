use super::*;
use crate::types::{Nest, SpeedRampPoint};

fn ramp(segments: usize, preferred_segments: Option<usize>) -> SpeedRamp {
    SpeedRamp {
        points: vec![
            SpeedRampPoint {
                at_ms: 100,
                factor: 1.0,
            },
            SpeedRampPoint {
                at_ms: 200,
                factor: 1.0,
            },
        ],
        segments,
        preferred_segments,
        timebase_fps: None,
        timebase_audio_rate: None,
    }
}

#[test]
fn persisted_ramp_counts_reject_unsupported_values_in_every_project_copy() {
    for count in [0, 1, 121, usize::MAX] {
        for bad_ramp in [ramp(count, None), ramp(2, Some(count))] {
            let mut project = Project::new("counts", Default::default());
            let mut media = crate::edit::make_media_clip("c1", "a1", 0, 1000);
            media.speed_ramp = Some(bad_ramp);
            project.track_mut("v1").unwrap().clips = vec![Clip::Media(media)];
            assert!(
                serde_json::from_value::<Project>(serde_json::to_value(&project).unwrap()).is_err()
            );

            let tracks = project.tracks.clone();
            project.track_mut("v1").unwrap().clips.clear();
            project.nests.push(Nest {
                id: "nest1".into(),
                name: None,
                tracks: tracks.clone(),
            });
            assert!(
                serde_json::from_value::<Project>(serde_json::to_value(&project).unwrap()).is_err()
            );

            project.nests.clear();
            project.ensure_sequence_bank();
            project.sequences[0].tracks = tracks;
            assert!(
                serde_json::from_value::<Project>(serde_json::to_value(&project).unwrap()).is_err()
            );
        }
    }
}

#[test]
fn programmatic_invalid_counts_refuse_expansion_before_iteration() {
    for count in [0, 1, 121, usize::MAX] {
        for bad_ramp in [ramp(count, None), ramp(2, Some(count))] {
            assert!(speed_ramp_segments(0, 1000, &bad_ramp).is_empty());
            let mut frame_aware = bad_ramp;
            frame_aware.timebase_fps = Some(30.0);
            assert!(speed_ramp_segments(0, 1000, &frame_aware).is_empty());
        }
    }
}

#[test]
fn supported_legacy_and_frame_aware_counts_preserve_duration_and_point_holding() {
    for count in [2, 24, 120] {
        for preferred in [None, Some(120)] {
            let legacy = ramp(count, preferred);
            let encoded = serde_json::to_value(&legacy).unwrap();
            let restored: SpeedRamp = serde_json::from_value(encoded).unwrap();
            assert_eq!(restored, legacy);
            assert_eq!(
                speed_ramp_segments(0, 1000, &restored)
                    .iter()
                    .map(|s| s.dur_ms)
                    .sum::<u64>(),
                1000
            );
            let mut frame_aware = restored;
            frame_aware.timebase_fps = Some(30.0);
            frame_aware.timebase_audio_rate = Some(48_000);
            assert_eq!(
                speed_ramp_segments(0, 1000, &frame_aware)
                    .iter()
                    .map(|s| s.frame_count.unwrap())
                    .sum::<u64>(),
                30
            );
        }
    }
}
