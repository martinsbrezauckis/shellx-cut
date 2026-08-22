use super::*;
use crate::types::{
    AnimState, Asset, ClipAnimation, ClipFreeze, GainWindow, Keyframe, KfInterp, KfParam, KfPoint,
    ProjectSettings, SpeedRamp, SpeedRampPoint,
};
use crate::{rebuild_from_log, Actor, ActorKind, ProjectStore};
use serde_json::json;

fn asset(duration_ms: u64) -> Asset {
    Asset {
        path: "/testdata/source.mp4".into(),
        hash: format!("sha256:{duration_ms}"),
        probe: Some(json!({"duration_ms": duration_ms, "kind": "video", "has_audio": true})),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    }
}

fn actor() -> Actor {
    Actor {
        kind: ActorKind::Agent,
        name: "overwrite-test".into(),
        via: "test".into(),
        request: None,
    }
}

fn media(id: &str, asset: &str, source: [u64; 2]) -> Clip {
    Clip::Media(make_media_clip(id, asset, source[0], source[1]))
}

#[test]
fn consumes_partial_multiple_and_gap_overlaps_without_moving_the_tail() {
    let mut project = Project::new("overwrite", ProjectSettings::default());
    project.assets.insert("a1".into(), asset(8_000));
    project.assets.insert("a2".into(), asset(8_000));
    project.track_mut("v1").unwrap().clips = vec![
        media("c1", "a1", [0, 1_000]),
        Clip::Gap(GapClip::new(500)),
        media("c2", "a1", [1_000, 2_000]),
        media("c3", "a1", [2_000, 3_000]),
    ];
    project.track_mut("a1t").unwrap().clips = vec![media("c4", "a1", [0, 3_500])];
    project.track_mut("a1t").unwrap().gain_windows = vec![GainWindow {
        range_ms: [1_000, 2_000],
        db: -12.0,
        attack_ms: 20,
    }];
    project.markers.push(crate::types::Marker {
        id: "m1".into(),
        at_ms: 2_500,
        label: "keep time".into(),
        note: None,
        color: None,
    });

    let effects = overwrite(
        &mut project,
        "a2",
        &["v1".into(), "a1t".into()],
        750,
        [100, 1_600],
    )
    .expect("linked A/V overwrite");

    assert_eq!(effects.len(), 2, "both targets are one operation's effects");
    assert_eq!(
        effects[0].detail["overlapped_clip_ids"],
        json!(["c1", "c2"])
    );
    assert_eq!(effects[0].detail["overwritten_gap_ms"], 500);
    assert_eq!(effects[0].detail["tail_extended_ms"], 0);
    assert_eq!(effects[1].detail["split_clip"], "c7");

    let video = &project.track("v1").unwrap().clips;
    let Clip::Media(left) = &video[0] else {
        panic!("left c1")
    };
    let Clip::Media(inserted) = &video[1] else {
        panic!("inserted c5")
    };
    let Clip::Media(right) = &video[2] else {
        panic!("right c2")
    };
    let Clip::Media(tail) = &video[3] else {
        panic!("unmoved c3")
    };
    assert_eq!((left.id.as_str(), left.src_out_ms), ("c1", 750));
    assert_eq!(
        (
            inserted.id.as_str(),
            inserted.src_in_ms,
            inserted.src_out_ms
        ),
        ("c5", 100, 1_600)
    );
    assert_eq!(
        (right.id.as_str(), right.src_in_ms, right.src_out_ms),
        ("c2", 1_750, 2_000)
    );
    assert_eq!(tail.id, "c3");
    assert_eq!(
        video[0].timeline_duration_ms()
            + video[1].timeline_duration_ms()
            + video[2].timeline_duration_ms(),
        2_500
    );
    assert_eq!(project.track("v1").unwrap().duration_ms(), 3_500);

    let audio = &project.track("a1t").unwrap().clips;
    let Clip::Media(audio_left) = &audio[0] else {
        panic!("audio left")
    };
    let Clip::Media(audio_inserted) = &audio[1] else {
        panic!("audio insert")
    };
    let Clip::Media(audio_right) = &audio[2] else {
        panic!("audio right")
    };
    assert_eq!((audio_left.id.as_str(), audio_left.src_out_ms), ("c4", 750));
    assert_eq!(
        (
            audio_inserted.id.as_str(),
            audio_inserted.src_in_ms,
            audio_inserted.src_out_ms
        ),
        ("c6", 100, 1_600)
    );
    assert_eq!(
        (audio_right.id.as_str(), audio_right.src_in_ms),
        ("c7", 2_250)
    );
    assert_eq!(project.markers[0].at_ms, 2_500, "markers do not ripple");
    assert_eq!(
        project.track("a1t").unwrap().gain_windows[0].range_ms,
        [1_000, 2_000]
    );
}

#[test]
fn tail_overwrite_pads_only_the_needed_gap_then_extends() {
    let mut project = Project::new("tail", ProjectSettings::default());
    project.assets.insert("a1".into(), asset(2_000));
    project.assets.insert("a2".into(), asset(2_000));
    project.track_mut("v1").unwrap().clips = vec![media("c1", "a1", [0, 1_000])];

    let effects =
        overwrite(&mut project, "a2", &["v1".into()], 1_500, [0, 500]).expect("tail overwrite");
    assert_eq!(effects[0].detail["tail_gap_ms"], 500);
    assert_eq!(effects[0].detail["tail_extended_ms"], 1_000);
    assert_eq!(
        effects[0].detail["overwritten_existing_ms"],
        json!([1_500, 1_500])
    );
    let clips = &project.track("v1").unwrap().clips;
    assert!(matches!(clips[1], Clip::Gap(ref gap) if gap.duration_ms == 500));
    assert_eq!(project.track("v1").unwrap().duration_ms(), 2_000);
}

#[test]
fn video_only_and_audio_only_overwrites_leave_the_other_track_unchanged() {
    let mut project = Project::new("one-sided", ProjectSettings::default());
    project.assets.insert("a1".into(), asset(4_000));
    project.assets.insert("a2".into(), asset(4_000));
    project.track_mut("v1").unwrap().clips = vec![media("c1", "a1", [0, 3_000])];
    project.track_mut("a1t").unwrap().clips = vec![media("c2", "a1", [0, 3_000])];

    let audio_before = project.track("a1t").unwrap().clone();
    let video_effects = overwrite(&mut project, "a2", &["v1".into()], 500, [100, 700])
        .expect("video-only overwrite");
    assert_eq!(video_effects.len(), 1);
    assert_eq!(project.track("a1t").unwrap(), &audio_before);
    assert_eq!(project.track("v1").unwrap().duration_ms(), 3_000);

    let video_before = project.track("v1").unwrap().clone();
    let audio_effects = overwrite(&mut project, "a2", &["a1t".into()], 1_000, [0, 500])
        .expect("audio-only overwrite");
    assert_eq!(audio_effects.len(), 1);
    assert_eq!(project.track("v1").unwrap(), &video_before);
    assert_eq!(project.track("a1t").unwrap().duration_ms(), 3_000);
}

#[test]
fn rejects_a_ramped_boundary_without_mutating_any_selected_track() {
    let mut project = Project::new("ramp", ProjectSettings::default());
    project.assets.insert("a1".into(), asset(2_000));
    project.assets.insert("a2".into(), asset(2_000));
    let mut ramped = make_media_clip("c1", "a1", 0, 2_000);
    ramped.speed_ramp = Some(SpeedRamp {
        points: vec![
            SpeedRampPoint {
                at_ms: 0,
                factor: 1.0,
            },
            SpeedRampPoint {
                at_ms: 2_000,
                factor: 1.0,
            },
        ],
        segments: 2,
        preferred_segments: None,
        timebase_fps: None,
        timebase_audio_rate: None,
    });
    project.track_mut("v1").unwrap().clips = vec![Clip::Media(ramped)];
    let before = project.clone();

    let error = overwrite(&mut project, "a2", &["v1".into()], 500, [0, 500])
        .expect_err("ramped edge is ambiguous");
    assert_eq!(error.code, codes::CONFLICT);
    assert_eq!(project, before, "validation happens before mutation");
}

/// An overwrite can split the source window of a plain constant-speed clip.
/// These states instead name time within the original *whole* clip, so cloning
/// them onto shortened remnants would change the result (most visibly, reverse
/// would play the wrong source half). Reject the operation before touching any
/// selected track until each state has a proven exact remap.
#[test]
fn rejects_time_dependent_boundaries_without_mutating_any_selected_track() {
    let cases: [(&str, fn(&mut MediaClip)); 4] = [
        ("reverse playback", |clip| clip.reverse = true),
        ("a freeze-frame", |clip| {
            clip.freeze = Some(ClipFreeze { at_ms: 1_000 });
        }),
        ("a Ken Burns animation", |clip| {
            clip.animation = Some(ClipAnimation {
                from: AnimState::default(),
                to: AnimState {
                    zoom: 1.5,
                    x: 0.6,
                    y: 0.4,
                },
            });
        }),
        ("parameter keyframes", |clip| {
            clip.keyframes = vec![Keyframe {
                param: KfParam::Opacity,
                points: vec![
                    KfPoint {
                        t_ms: 0,
                        value: 0.0,
                    },
                    KfPoint {
                        t_ms: 2_000,
                        value: 1.0,
                    },
                ],
                interp: KfInterp::Linear,
            }];
        }),
    ];

    for (feature, configure) in cases {
        let mut project = Project::new("time-dependent", ProjectSettings::default());
        project.assets.insert("a1".into(), asset(2_000));
        project.assets.insert("a2".into(), asset(2_000));
        // Keep a first, plain target ahead of the blocked one. This proves
        // validation completes for every selected track before an atomic A/V
        // overwrite starts to mutate either target.
        project.track_mut("v1").unwrap().clips = vec![media("c0", "a1", [0, 2_000])];
        let mut original = make_media_clip("c1", "a1", 0, 2_000);
        configure(&mut original);
        project.track_mut("a1t").unwrap().clips = vec![Clip::Media(original)];
        let before = project.clone();

        let error = overwrite(
            &mut project,
            "a2",
            &["v1".into(), "a1t".into()],
            500,
            [0, 500],
        )
        .expect_err("time-dependent clip cannot be boundary-split");
        assert_eq!(error.code, codes::CONFLICT, "{feature}");
        assert_eq!(error.clip_id.as_deref(), Some("c1"), "{feature}");
        assert_eq!(error.at_ms, Some(500), "{feature}");
        assert!(error.message.contains(feature), "{feature}: {error}");
        assert_eq!(
            project, before,
            "{feature}: validation happens before mutation"
        );
    }
}

/// A crossfade is intentionally stored on its right clip and travels with
/// that clip. Replacing its old left neighbour with a new media clip leaves a
/// real media-to-media seam, so preserving the right clip's transition is
/// correct (the EDL proves it is live rather than dangling metadata).
#[test]
fn preserves_right_owned_crossfade_when_overwrite_replaces_left_neighbour() {
    let mut project = Project::new("crossfade", ProjectSettings::default());
    project.assets.insert("a1".into(), asset(2_000));
    project.assets.insert("a2".into(), asset(2_000));
    let mut right = make_media_clip("c2", "a1", 1_000, 2_000);
    right.xfade_in_ms = 250;
    right.xfade_kind = Some("wipeleft".into());
    project.track_mut("v1").unwrap().clips =
        vec![media("c1", "a1", [0, 1_000]), Clip::Media(right)];

    overwrite(&mut project, "a2", &["v1".into()], 0, [0, 1_000])
        .expect("overwrite replaces only the old left neighbour");
    let clips = &project.track("v1").unwrap().clips;
    assert!(matches!(&clips[0], Clip::Media(clip) if clip.asset == "a2"));
    let Clip::Media(right) = &clips[1] else {
        panic!("the untouched right clip remains media")
    };
    assert_eq!(right.id, "c2");
    assert_eq!(right.xfade_in_ms, 250);
    assert_eq!(right.xfade_kind.as_deref(), Some("wipeleft"));

    let edl = crate::edl::edl_from_project(&project);
    let right_segment = edl
        .segments
        .iter()
        .find(|segment| segment.clip_id.as_deref() == Some("c2"))
        .expect("right clip produces an EDL segment");
    assert_eq!(right_segment.xfade_in_ms, 250);
    assert_eq!(right_segment.xfade_kind.as_deref(), Some("wipeleft"));
    assert_eq!(
        right_segment.timeline_in_ms, 750,
        "the EDL realizes the dissolve"
    );
    assert_eq!(edl.duration_ms, 1_750);
}

/// The incoming transition is owned by c2, not its old left neighbour. When
/// overwrite consumes c2's head, c2's surviving piece must retain that
/// transition against the new source; clearing it would lengthen the rendered
/// track by the overlap and move c3 even though overwrite never ripples.
#[test]
fn preserves_truncated_right_owned_crossfade_without_laid_or_replay_drift() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::create(dir.path(), "overwrite-xfade", None).unwrap();
    store
        .record_import(Some("a1".into()), asset(8_000), actor(), None)
        .unwrap();
    store
        .record_import(Some("a2".into()), asset(8_000), actor(), None)
        .unwrap();
    for at_ms in [0, 2_000, 4_000] {
        store
            .apply(
                "edit.insert",
                json!({"asset":"a1", "track":"v1", "at_ms":at_ms, "src_range_ms":[0,2_000], "ripple":false}),
                actor(),
                None,
            )
            .unwrap();
    }
    store
        .apply(
            "edit.crossfade",
            json!({"track":"v1", "at_ms":2_000, "duration_ms":1_000}),
            actor(),
            None,
        )
        .unwrap();
    let before = crate::edl::edl_from_project(&store.project);
    assert_eq!(
        before
            .segments
            .iter()
            .find(|segment| segment.clip_id.as_deref() == Some("c3"))
            .map(|segment| segment.timeline_in_ms),
        Some(3_000),
        "the upstream crossfade pulls the later clip back in laid time"
    );

    store
        .apply(
            "edit.overwrite",
            json!({"asset":"a2", "at_ms":1_500, "video_track":"v1", "src_range_ms":[0,1_000]}),
            actor(),
            None,
        )
        .expect("the replacement and surviving transition both fit");

    let right = store
        .project
        .track("v1")
        .unwrap()
        .clips
        .iter()
        .find_map(|clip| match clip {
            Clip::Media(media) if media.id == "c2" => Some(media),
            _ => None,
        })
        .expect("the surviving right clip remains");
    assert_eq!(
        right.xfade_in_ms, 1_000,
        "c2 retains its right-owned transition"
    );
    let after = crate::edl::edl_from_project(&store.project);
    assert_eq!(
        after
            .segments
            .iter()
            .find(|segment| segment.clip_id.as_deref() == Some("c3"))
            .map(|segment| segment.timeline_in_ms),
        Some(3_000),
        "the later rendered coordinate does not ripple"
    );
    assert_eq!(
        after.duration_ms, before.duration_ms,
        "rendered extent stays fixed"
    );

    let rebuilt = rebuild_from_log(&store.log.read_all().unwrap()).expect("replay overwrite");
    assert_eq!(
        rebuilt.tracks, store.project.tracks,
        "the preserved transition and overwrite replay without timeline drift"
    );
}

#[test]
fn rejects_crossfade_truncation_that_would_shift_laid_timeline() {
    let mut project = Project::new("crossfade-conflict", ProjectSettings::default());
    project.assets.insert("a1".into(), asset(4_000));
    project.assets.insert("a2".into(), asset(4_000));
    let mut right = make_media_clip("c2", "a1", 2_000, 4_000);
    right.xfade_in_ms = 1_000;
    project.track_mut("v1").unwrap().clips = vec![
        media("c1", "a1", [0, 2_000]),
        Clip::Media(right),
        media("c3", "a1", [0, 1_000]),
    ];
    let before = project.clone();

    let error = overwrite(&mut project, "a2", &["v1".into()], 1_500, [0, 500])
        .expect_err("a shorter replacement would shorten the carried crossfade");
    assert_eq!(error.code, codes::CONFLICT);
    assert_eq!(error.clip_id.as_deref(), Some("c2"));
    assert_eq!(error.at_ms, Some(2_000));
    assert!(error.message.contains("right-owned crossfade"));
    assert_eq!(
        project, before,
        "unsafe transition remapping is atomic and refused"
    );
}

#[test]
fn rejects_overwrite_start_inside_right_owned_crossfade_head_without_mutation() {
    let mut project = Project::new("crossfade-head-conflict", ProjectSettings::default());
    project.assets.insert("a1".into(), asset(5_000));
    project.assets.insert("a2".into(), asset(5_000));
    let mut right = make_media_clip("c2", "a1", 2_000, 4_000);
    right.xfade_in_ms = 1_000;
    project.track_mut("v1").unwrap().clips = vec![
        media("c1", "a1", [0, 2_000]),
        Clip::Media(right),
        media("c3", "a1", [0, 1_000]),
    ];
    project.track_mut("a1t").unwrap().clips = vec![media("c4", "a1", [0, 5_000])];
    let before = project.clone();

    // c2 starts at editorial 2000. Starting at 2500 splits its 1000ms
    // transition head: left_piece would retain only 500ms of c2 while the EDL
    // clamps its formerly-1000ms right-owned dissolve, shifting c3. This is
    // unsafe whether the overwrite ends inside c2 or consumes through its tail.
    for (source_range, shape) in [
        ([0, 500], "ends inside c2"),
        ([0, 2_000], "consumes c2's tail"),
    ] {
        let error = overwrite(
            &mut project,
            "a2",
            &["v1".into(), "a1t".into()],
            2_500,
            source_range,
        )
        .expect_err("a partial right-owned crossfade head has no no-ripple remap");
        assert_eq!(error.code, codes::CONFLICT, "{shape}");
        assert_eq!(error.clip_id.as_deref(), Some("c2"), "{shape}");
        assert_eq!(error.at_ms, Some(2_500), "{shape}");
        assert!(error.message.contains("right-owned crossfade"), "{shape}");
        assert!(
            error
                .suggested_action
                .as_deref()
                .unwrap_or_default()
                .contains("editorial 3000ms"),
            "{shape}: recovery identifies the first no-ripple-safe editorial boundary"
        );
        assert_eq!(
            project, before,
            "{shape}: unsafe crossfade-head overwrite is atomic"
        );
    }
}

#[test]
fn rejects_overwrite_that_fully_consumes_right_owned_crossfade_without_mutation() {
    let mut project = Project::new("crossfade-owner-conflict", ProjectSettings::default());
    project.assets.insert("a1".into(), asset(5_000));
    project.assets.insert("a2".into(), asset(5_000));
    let mut right = make_media_clip("c2", "a1", 2_000, 4_000);
    right.xfade_in_ms = 1_000;
    project.track_mut("v1").unwrap().clips = vec![
        media("c1", "a1", [0, 2_000]),
        Clip::Media(right),
        media("c3", "a1", [0, 1_000]),
    ];
    project.track_mut("a1t").unwrap().clips = vec![media("c4", "a1", [0, 5_000])];
    let before = project.clone();

    // Removing c2 removes its owned overlap from the EDL. This drifts c3 by
    // 1000ms whether the overwrite begins at c2 or crosses its left boundary.
    for (at_ms, source_range, shape) in [
        (2_000, [0, 2_000], "starts at the transition owner"),
        (
            1_500,
            [0, 2_500],
            "crosses the transition owner's left boundary",
        ),
    ] {
        let error = overwrite(
            &mut project,
            "a2",
            &["v1".into(), "a1t".into()],
            at_ms,
            source_range,
        )
        .expect_err("removing a live transition owner would ripple rendered time");
        assert_eq!(error.code, codes::CONFLICT, "{shape}");
        assert_eq!(error.clip_id.as_deref(), Some("c2"), "{shape}");
        assert_eq!(error.at_ms, Some(at_ms), "{shape}");
        assert!(
            error
                .message
                .contains("fully consumes right-owned crossfade"),
            "{shape}"
        );
        assert_eq!(
            project, before,
            "{shape}: unsafe transition-owner removal is atomic"
        );
    }
}

#[test]
fn rejects_overwrite_that_fully_consumes_multiple_right_owned_crossfades() {
    let mut project = Project::new(
        "multiple-crossfade-owner-conflict",
        ProjectSettings::default(),
    );
    project.assets.insert("a1".into(), asset(8_000));
    project.assets.insert("a2".into(), asset(8_000));
    let mut middle = make_media_clip("c2", "a1", 2_000, 4_000);
    middle.xfade_in_ms = 1_000;
    let mut right = make_media_clip("c3", "a1", 4_000, 6_000);
    right.xfade_in_ms = 500;
    project.track_mut("v1").unwrap().clips = vec![
        media("c1", "a1", [0, 2_000]),
        Clip::Media(middle),
        Clip::Media(right),
        media("c4", "a1", [6_000, 7_000]),
    ];
    project.track_mut("a1t").unwrap().clips = vec![media("c5", "a1", [0, 7_000])];
    let before = project.clone();

    // Both c2 and c3 own live incoming overlaps. Rejecting the first owner is
    // enough to keep the linked edit atomic; allowing it would lose both EDL
    // pullbacks and shift c4 by 1500ms.
    let error = overwrite(
        &mut project,
        "a2",
        &["v1".into(), "a1t".into()],
        2_000,
        [0, 4_000],
    )
    .expect_err("one no-ripple overwrite cannot delete multiple live transition owners");
    assert_eq!(error.code, codes::CONFLICT);
    assert_eq!(error.clip_id.as_deref(), Some("c2"));
    assert_eq!(error.at_ms, Some(2_000));
    assert_eq!(
        project, before,
        "the first detected transition owner prevents all linked-track mutation"
    );
}

#[test]
fn allows_overwrite_at_the_end_of_the_right_owned_crossfade_head() {
    let mut project = Project::new("crossfade-head-boundary", ProjectSettings::default());
    project.assets.insert("a1".into(), asset(5_000));
    project.assets.insert("a2".into(), asset(5_000));
    let mut right = make_media_clip("c2", "a1", 2_000, 4_000);
    right.xfade_in_ms = 1_000;
    project.track_mut("v1").unwrap().clips = vec![
        media("c1", "a1", [0, 2_000]),
        Clip::Media(right),
        media("c3", "a1", [0, 1_000]),
    ];
    let before = crate::edl::edl_from_project(&project);

    overwrite(&mut project, "a2", &["v1".into()], 3_000, [0, 500])
        .expect("the full transition head remains on c2's left piece");
    let after = crate::edl::edl_from_project(&project);
    let c3_start = |edl: &crate::edl::Edl| {
        edl.segments
            .iter()
            .find(|segment| segment.clip_id.as_deref() == Some("c3"))
            .map(|segment| segment.timeline_in_ms)
    };
    assert_eq!(
        c3_start(&after),
        c3_start(&before),
        "later laid time stays fixed"
    );
}

#[test]
fn durable_linked_overwrite_replays_with_stable_clip_ids() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::create(dir.path(), "replay", None).unwrap();
    store
        .record_import(None, asset(6_000), actor(), None)
        .unwrap();
    store
        .record_import(None, asset(6_000), actor(), None)
        .unwrap();
    for track in ["v1", "a1t"] {
        store
            .apply(
                "edit.insert",
                json!({"asset":"a1", "track":track, "at_ms":0, "src_range_ms":[0,4_000], "ripple":false}),
                actor(),
                None,
            )
            .unwrap();
    }
    let before_overwrite = store.project.tracks.clone();
    let operation = store
        .apply(
            "edit.overwrite",
            json!({"asset":"a2", "at_ms":1_000, "video_track":"v1", "audio_track":"a1t", "src_range_ms":[500,2_000]}),
            actor(),
            Some("replace the middle without ripple".into()),
        )
        .expect("one durable linked overwrite");
    assert_eq!(operation.effects.len(), 2);
    assert_eq!(
        operation
            .effects
            .iter()
            .filter(|effect| effect.detail.contains_key("added_clip"))
            .count(),
        2
    );
    assert!(operation
        .effects
        .iter()
        .all(|effect| effect.detail["overwrite"] == true));

    let rebuilt = rebuild_from_log(&store.log.read_all().unwrap()).expect("replay overwrite");
    assert_eq!(
        serde_json::to_value(&rebuilt.tracks).unwrap(),
        serde_json::to_value(&store.project.tracks).unwrap(),
        "recorded added/split ids replay byte-identically"
    );
    let mut reopened = ProjectStore::open(&store.dir).expect("cold reopen overwrite");
    assert_eq!(
        reopened.project.tracks, store.project.tracks,
        "cold reopen has no timeline drift"
    );
    reopened
        .undo(actor())
        .expect("one-step undo of atomic overwrite");
    assert_eq!(
        reopened.project.tracks, before_overwrite,
        "one undo restores both linked overwrite targets"
    );
}
