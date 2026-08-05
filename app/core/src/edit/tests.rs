use super::*;
use crate::types::{CaptionClip, ProjectSettings};

/// Project with v1 video clips c1[0..5000) c2[src 5000..8000), audio
/// mirror, a caption track, and two markers — the ripple test fixture.
fn fixture() -> Project {
    let mut p = Project::new("t", ProjectSettings::default());
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("c1", "a1", 0, 5000)));
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("c2", "a1", 5000, 8000)));
    p.track_mut("a1t")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("c3", "a1", 0, 8000)));
    p.tracks.push(Track {
        id: "cap1".into(),
        kind: TrackKind::Caption,
        clips: vec![
            Clip::Caption(CaptionClip {
                id: "s1".into(),
                text: "one".into(),
                style_ref: None,
                range_ms: [0, 1000],
            }),
            Clip::Caption(CaptionClip {
                id: "s2".into(),
                text: "two".into(),
                style_ref: None,
                range_ms: [1500, 2500],
            }),
            Clip::Caption(CaptionClip {
                id: "s3".into(),
                text: "three".into(),
                style_ref: None,
                range_ms: [4000, 6000],
            }),
        ],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    p.markers.push(Marker {
        id: "m1".into(),
        at_ms: 1800,
        label: "x".into(),
        note: None,
        color: None,
    });
    p.markers.push(Marker {
        id: "m2".into(),
        at_ms: 7000,
        label: "y".into(),
        note: None,
        color: None,
    });
    p
}

fn misplaced_caption() -> Clip {
    Clip::Caption(CaptionClip {
        id: "bad_cap".into(),
        text: "caption on media track".into(),
        style_ref: None,
        range_ms: [0, 1000],
    })
}

#[test]
fn malformed_caption_on_media_track_errors_instead_of_panicking() {
    let corrupt = || {
        let mut p = fixture();
        p.track_mut("v1").unwrap().clips = vec![misplaced_caption()];
        p
    };

    let err = split(&mut corrupt(), "v1", 500).expect_err("split rejects malformed track");
    assert_eq!(err.code, codes::INVALID_ARGS);
    assert!(err.message.contains("malformed timeline"));

    let err = ripple_delete(&mut corrupt(), Some("v1"), [100, 500], true)
        .expect_err("ripple_delete rejects malformed track");
    assert_eq!(err.code, codes::INVALID_ARGS);
    assert!(err.message.contains("malformed timeline"));

    let mut p = corrupt();
    p.assets.insert(
        "a1".into(),
        crate::types::Asset {
            path: "/tmp/a1.mp4".into(),
            hash: "sha256:test".into(),
            probe: Some(json!({"duration_ms": 10_000})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let err = insert(&mut p, "a1", "v1", 500, Some([0, 1000]), false)
        .expect_err("insert/splice rejects malformed track");
    assert_eq!(err.code, codes::INVALID_ARGS);
    assert!(err.message.contains("malformed timeline"));
}

#[test]
fn set_timeline_rejects_inverted_media_source_range() {
    let mut p = fixture();
    let mut bad = make_media_clip("bad", "a1", 5000, 1000);
    bad.id = "bad".into();
    let track = Track {
        id: "v_bad".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(bad)],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    };
    let err = apply_set_timeline(&mut p, &json!({"tracks":[track],"markers":[]}))
        .expect_err("inverted source ranges must not enter project state");
    assert_eq!(err.code, codes::INVALID_ARGS);
    assert!(err.message.contains("malformed timeline"));
}

#[test]
fn track_visibility_and_lock_flags_are_persisted() {
    let mut p = Project::new("t", ProjectSettings::default());

    let hidden_fx = set_track_visible(&mut p, "v1", false).unwrap();
    assert!(!p.track("v1").unwrap().visible);
    let js = serde_json::to_string(&hidden_fx).unwrap();
    let back: Vec<OpEffect> = serde_json::from_str(&js).unwrap();
    assert_eq!(back[0].track.as_deref(), Some("v1"));
    assert_eq!(
        back[0].detail.get("visible").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        back[0].detail.get("old_visible").and_then(|v| v.as_bool()),
        Some(true)
    );

    set_track_visible(&mut p, "v1", true).unwrap();
    assert!(p.track("v1").unwrap().visible);

    let lock_fx = set_track_locked(&mut p, "v1", true).unwrap();
    assert!(p.track("v1").unwrap().locked);
    let js = serde_json::to_string(&lock_fx).unwrap();
    let back: Vec<OpEffect> = serde_json::from_str(&js).unwrap();
    assert_eq!(back[0].track.as_deref(), Some("v1"));
    assert_eq!(
        back[0].detail.get("locked").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        back[0].detail.get("old_locked").and_then(|v| v.as_bool()),
        Some(false)
    );

    assert_eq!(
        set_track_visible(&mut p, "a1t", false).unwrap_err().code,
        codes::INVALID_ARGS
    );
    assert_eq!(
        set_track_locked(&mut p, "nope", true).unwrap_err().code,
        codes::NOT_FOUND
    );
}

/// edit.color_space tags a video clip's input space; the tag clears with None and
/// the field is the source of truth the renderer reads.
#[test]
fn color_space_tags_and_clears() {
    let mut p = fixture();
    set_color_space(&mut p, "c1", Some(crate::types::ColorSpace::Srgb)).unwrap();
    let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
        unreachable!("c1 is media")
    };
    assert_eq!(c.input_color_space, Some(crate::types::ColorSpace::Srgb));
    // Clearing (None) removes the tag → renders byte-identical to untagged.
    set_color_space(&mut p, "c1", None).unwrap();
    let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
        unreachable!()
    };
    assert_eq!(c.input_color_space, None);
}

/// edit.grade_stack stores a layered stack, CLEARS the single grade (the stack is
/// the authority), drops identity layers, and an empty / all-identity stack clears
/// the clip's grade entirely (byte-identical to ungraded).
#[test]
fn grade_stack_stores_layers_and_supersedes_single_grade() {
    use crate::types::ClipGrade;
    let ident = || ClipGrade {
        contrast: 1.0,
        brightness: 0.0,
        saturation: 1.0,
        gamma: 1.0,
        temperature_k: None,
        lut: None,
    };
    let mut p = fixture();
    // Start from a single grade on c1 (the legacy edit.grade path).
    grade(
        &mut p,
        "c1",
        ClipGrade {
            saturation: 1.4,
            ..ident()
        },
    )
    .unwrap();
    let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
        unreachable!()
    };
    assert!(c.grade.is_some());
    assert!(c.grade_stack.is_empty());

    // A 2-layer stack supersedes: single grade cleared, stack stores both layers.
    let l1 = ClipGrade {
        contrast: 1.2,
        ..ident()
    };
    let l2 = ClipGrade {
        saturation: 0.8,
        temperature_k: Some(5000),
        ..ident()
    };
    grade_stack(&mut p, "c1", vec![l1.clone(), l2.clone()]).unwrap();
    let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
        unreachable!()
    };
    assert_eq!(c.grade, None, "stack supersedes the single grade");
    assert_eq!(c.grade_stack, vec![l1.clone(), l2.clone()]);

    // Identity layers are dropped (only the non-identity one survives).
    grade_stack(&mut p, "c1", vec![ident(), l2.clone(), ident()]).unwrap();
    let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
        unreachable!()
    };
    assert_eq!(c.grade_stack, vec![l2.clone()]);

    // An empty / all-identity stack clears the grade entirely (ungraded).
    grade_stack(&mut p, "c1", vec![ident(), ident()]).unwrap();
    let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
        unreachable!()
    };
    assert!(c.grade_stack.is_empty());
    assert_eq!(c.grade, None);
}

/// A SINGLE-element grade stack stores exactly that one layer — the basis for the
/// render byte-identity to a plain edit.grade (proven at the filter level in
/// render.rs grade_stack_filter tests).
#[test]
fn grade_stack_single_layer_round_trips() {
    use crate::types::ClipGrade;
    let mut p = fixture();
    let g = ClipGrade {
        contrast: 1.15,
        saturation: 0.0,
        brightness: 0.0,
        gamma: 1.0,
        temperature_k: None,
        lut: None,
    };
    grade_stack(&mut p, "c1", vec![g.clone()]).unwrap();
    let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
        unreachable!()
    };
    assert_eq!(c.grade, None);
    assert_eq!(c.grade_stack, vec![g]);
}

/// edit.grade_stack refuses an AUDIO clip (no pixels to grade).
#[test]
fn grade_stack_refuses_audio_clip() {
    use crate::types::ClipGrade;
    let mut p = fixture();
    let err = grade_stack(
        &mut p,
        "c3",
        vec![ClipGrade {
            saturation: 1.5,
            contrast: 1.0,
            brightness: 0.0,
            gamma: 1.0,
            temperature_k: None,
            lut: None,
        }],
    )
    .expect_err("audio clip must be refused");
    assert_eq!(err.code.as_str(), codes::INVALID_ARGS);
}

/// edit.grade_window: power windows STACK (each call appends), remove_index removes one,
/// enabled:false (None) CLEARS them, an IDENTITY grade + bad geometry are refused BEFORE
/// mutating, the no-window default is empty + serde-skipped (byte-identical), and
/// overlay/audio clips are refused (base-track-only v1, like edit.add_mask).
#[test]
fn grade_window_stacks_clears_and_validates() {
    use crate::types::{ClipGrade, MaskShape, WindowShape};
    let ident = || ClipGrade {
        contrast: 1.0,
        brightness: 0.0,
        saturation: 1.0,
        gamma: 1.0,
        temperature_k: None,
        lut: None,
    };
    let rect = |pts: Vec<[f64; 2]>| WindowShape {
        shape: MaskShape::Rect,
        points: pts,
        feather: 0.0,
        invert: false,
    };
    let mut p = Project::new("t", ProjectSettings::default());
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("c1", "a1", 0, 2000)));
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(make_media_clip("ov", "a2", 0, 2000))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    p.track_mut("a1t")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("ac", "a3", 0, 2000)));

    // No window by default → empty (serde-skipped, byte-identical to pre-feature).
    let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
        unreachable!()
    };
    assert!(c.grade_windows.is_empty());

    // Append one window (left half, brighten), then a second (right half, desaturate)
    // → windows STACK in order.
    let g1 = ClipGrade {
        brightness: 0.5,
        ..ident()
    };
    grade_window(
        &mut p,
        "c1",
        Some(rect(vec![[0.0, 0.0], [0.5, 1.0]])),
        g1.clone(),
        None,
    )
    .unwrap();
    let right = rect(vec![[0.5, 0.0], [1.0, 1.0]]);
    let g2 = ClipGrade {
        saturation: 0.0,
        ..ident()
    };
    grade_window(&mut p, "c1", Some(right.clone()), g2.clone(), None).unwrap();
    let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
        unreachable!()
    };
    assert_eq!(c.grade_windows.len(), 2, "windows stack");
    assert_eq!(c.grade_windows[0].grade, g1);
    assert_eq!(c.grade_windows[1].window, right);
    assert_eq!(c.grade_windows[1].grade, g2);

    // An IDENTITY grade is refused (a window grading nothing) — state untouched.
    assert!(grade_window(
        &mut p,
        "c1",
        Some(rect(vec![[0.0, 0.0], [0.5, 1.0]])),
        ident(),
        None,
    )
    .is_err());
    // A polygon with 2 points is refused (needs ≥3) BEFORE mutating.
    let badpoly = WindowShape {
        shape: MaskShape::Polygon,
        points: vec![[0.0, 0.0], [1.0, 1.0]],
        feather: 0.0,
        invert: false,
    };
    assert!(grade_window(
        &mut p,
        "c1",
        Some(badpoly),
        ClipGrade {
            contrast: 1.3,
            ..ident()
        },
        None,
    )
    .is_err());
    let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
        unreachable!()
    };
    assert_eq!(c.grade_windows.len(), 2, "refused ops did not mutate");

    // Overlay-track video clip → refused (base-track-only v1 scope).
    assert!(grade_window(
        &mut p,
        "ov",
        Some(rect(vec![[0.0, 0.0], [0.5, 1.0]])),
        ClipGrade {
            brightness: 0.3,
            ..ident()
        },
        None,
    )
    .is_err());
    // Audio clip → refused (no pixels).
    assert!(grade_window(
        &mut p,
        "ac",
        Some(rect(vec![[0.0, 0.0], [0.5, 1.0]])),
        ClipGrade {
            brightness: 0.3,
            ..ident()
        },
        None,
    )
    .is_err());

    // Remove exactly one window through the real verb parser, preserving order and the
    // other window in one effect/commit.
    let effects = crate::store::apply_edit_verb(
        &mut p,
        "edit.grade_window",
        &json!({"clip": "c1", "remove_index": 0}),
    )
    .unwrap();
    assert_eq!(effects.len(), 1);
    assert_eq!(
        effects[0].detail["old_grade_windows"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        effects[0].detail["new_grade_windows"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
        unreachable!()
    };
    assert_eq!(c.grade_windows.len(), 1);
    assert_eq!(c.grade_windows[0].window, right);
    assert_eq!(c.grade_windows[0].grade, g2);

    // Removal is an exclusive operation. Ambiguous append/clear fields are refused by
    // the verb boundary before core mutation.
    assert!(crate::store::apply_edit_verb(
        &mut p,
        "edit.grade_window",
        &json!({"clip": "c1", "remove_index": 0, "enabled": false}),
    )
    .is_err());

    // A stale/out-of-range index is refused before mutation.
    assert!(grade_window(&mut p, "c1", None, ident(), Some(4)).is_err());
    let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
        unreachable!()
    };
    assert_eq!(c.grade_windows.len(), 1, "invalid remove did not mutate");

    // enabled:false (window = None) CLEARS all windows → byte-identical to never-windowed.
    grade_window(&mut p, "c1", None, ident(), None).unwrap();
    let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
        unreachable!()
    };
    assert!(c.grade_windows.is_empty());
    // serde-skip-empty: the cleared clip serializes WITHOUT a grade_windows key (so a
    // never-windowed project round-trips byte-identical).
    let v = serde_json::to_value(c).unwrap();
    assert!(
        v.get("grade_windows").is_none(),
        "empty windows are serde-skipped"
    );
}

/// edit.color_space refuses an AUDIO clip (no pixels to color-manage).
#[test]
fn color_space_refuses_audio_clip() {
    let mut p = fixture();
    // c3 is on the audio track a1t.
    let err = set_color_space(&mut p, "c3", Some(crate::types::ColorSpace::Rec2020))
        .expect_err("audio clip must be refused");
    assert_eq!(err.code.as_str(), codes::INVALID_ARGS);
}

/// A split carries the parent's input color space onto BOTH halves (same source
/// pixels → same color tag).
#[test]
fn split_inherits_input_color_space() {
    let mut p = fixture();
    set_color_space(&mut p, "c1", Some(crate::types::ColorSpace::Rec2020)).unwrap();
    // Split c1 at 2000 → v1 clips[0]=left half, clips[1]=right half (both from c1).
    split(&mut p, "v1", 2000).unwrap();
    for i in 0..2 {
        let Clip::Media(c) = &p.track("v1").unwrap().clips[i] else {
            unreachable!("half {i} is media")
        };
        assert_eq!(
            c.input_color_space,
            Some(crate::types::ColorSpace::Rec2020),
            "half {i} must inherit the input color space"
        );
    }
}

/// ColorSpace name parsing round-trips and rejects unknowns.
#[test]
fn color_space_parse_round_trip() {
    for s in ["rec709", "rec2020", "srgb", "linear"] {
        assert_eq!(crate::types::ColorSpace::parse(s).unwrap().as_str(), s);
    }
    assert!(crate::types::ColorSpace::parse("prophoto").is_none());
    assert!(crate::types::ColorSpace::parse("REC709").is_some()); // case-insensitive
}

/// split divides the clip under the point; src ranges stay contiguous.
#[test]
fn split_divides_media_clip() {
    let mut p = fixture();
    let eff = split(&mut p, "v1", 2000).unwrap();
    assert_eq!(eff[0].detail["left"], "c1");
    let right_id = eff[0].detail["right"].as_str().unwrap().to_string();
    let v1 = p.track("v1").unwrap();
    assert_eq!(v1.clips.len(), 3);
    match (&v1.clips[0], &v1.clips[1]) {
        (Clip::Media(l), Clip::Media(r)) => {
            assert_eq!((l.src_in_ms, l.src_out_ms), (0, 2000));
            assert_eq!((r.src_in_ms, r.src_out_ms), (2000, 5000));
            assert_eq!(r.id, right_id);
        }
        _ => unreachable!("expected media clips"),
    }
    // Boundary and past-end positions are errors, not silent no-ops.
    assert!(split(&mut p, "v1", 0).is_err());
    assert!(split(&mut p, "v1", 99999).is_err());
}

/// Timeline-wide ripple: AV stays in sync, captions trim/shift, markers move.
#[test]
fn ripple_delete_all_tracks() {
    let mut p = fixture();
    // Remove [1000, 2000): c1 spans it → split into [0,1000)+[2000,5000).
    ripple_delete(&mut p, None, [1000, 2000], true).unwrap();
    let v1 = p.track("v1").unwrap();
    assert_eq!(v1.duration_ms(), 7000); // 8000 - 1000
    match (&v1.clips[0], &v1.clips[1]) {
        (Clip::Media(l), Clip::Media(r)) => {
            assert_eq!((l.src_in_ms, l.src_out_ms), (0, 1000));
            assert_eq!((r.src_in_ms, r.src_out_ms), (2000, 5000));
            assert_ne!(r.id, l.id); // right half got a fresh id
        }
        _ => unreachable!("expected media clips"),
    }
    // Audio rippled identically (sync invariant).
    assert_eq!(p.track("a1t").unwrap().duration_ms(), 7000);
    // Captions: s1 untouched, s2 trimmed to start at the cut, s3 shifted.
    let cap = p.track("cap1").unwrap();
    let ranges: Vec<[u64; 2]> = cap
        .clips
        .iter()
        .map(|c| match c {
            Clip::Caption(cc) => cc.range_ms,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(ranges, vec![[0, 1000], [1000, 1500], [3000, 5000]]);
    // Markers: m1 (1800) clamps into the cut at 1000; m2 shifts left 1000.
    assert_eq!(p.markers[0].at_ms, 1000);
    assert_eq!(p.markers[1].at_ms, 6000);
}

/// A ripple range is expressed in TIMELINE time. On a constant-speed retimed
/// clip, the surviving source edges must scale through tl_off_to_src; using
/// raw timeline offsets corrupts the media window and desyncs A/V.
#[test]
fn ripple_delete_remaps_retimed_media_edges_through_speed() {
    let mut p = fixture();
    match &mut p.track_mut("v1").unwrap().clips[0] {
        Clip::Media(c) => c.speed = 2.0,
        _ => unreachable!("c1 is media"),
    }

    ripple_delete(&mut p, Some("v1"), [1000, 1500], true).unwrap();

    let v1 = p.track("v1").unwrap();
    match (&v1.clips[0], &v1.clips[1]) {
        (Clip::Media(left), Clip::Media(right)) => {
            assert_eq!((left.src_in_ms, left.src_out_ms), (0, 2000));
            assert_eq!((right.src_in_ms, right.src_out_ms), (3000, 5000));
            assert_eq!(left.speed, 2.0);
            assert_eq!(right.speed, 2.0);
        }
        _ => unreachable!("expected two media remnants"),
    }
}

/// Variable speed ramps are non-linear, so ripple_delete must refuse an
/// overlapped ramped clip before rebuilding the track.
#[test]
fn ripple_delete_refuses_speed_ramped_clip_before_mutation() {
    let mut p = fixture();
    speed_ramp(&mut p, "c1", ramp_pts(), 24).expect("set ramp");
    let before = p.track("v1").unwrap().clips.clone();

    let err = ripple_delete(&mut p, Some("v1"), [100, 300], true).unwrap_err();

    assert_eq!(err.code, codes::INVALID_ARGS);
    assert_eq!(p.track("v1").unwrap().clips, before);
}

/// Single-track ripple leaves other tracks and markers in place.
#[test]
fn ripple_delete_single_track() {
    let mut p = fixture();
    ripple_delete(&mut p, Some("v1"), [0, 5000], true).unwrap();
    assert_eq!(p.track("v1").unwrap().duration_ms(), 3000);
    assert_eq!(p.track("a1t").unwrap().duration_ms(), 8000); // untouched
    assert_eq!(p.markers[0].at_ms, 1800); // untouched
                                          // c1 fully covered → reported removed.
    let mut p2 = fixture();
    let eff = ripple_delete(&mut p2, Some("v1"), [0, 5000], true).unwrap();
    assert_eq!(eff[0].detail["clips_removed"], json!(["c1"]));
}

/// trim ripples the timeline and validates ranges.
#[test]
fn trim_validates_and_ripples() {
    let mut p = fixture();
    trim(&mut p, "c1", Some(1000), Some(3000)).unwrap();
    assert_eq!(p.track("v1").unwrap().duration_ms(), 2000 + 3000);
    assert!(trim(&mut p, "c1", Some(3000), Some(3000)).is_err()); // empty
    assert!(trim(&mut p, "nope", None, None).is_err()); // unknown clip
}

/// move gap-fills the source slot and splices into the destination.
#[test]
fn move_clip_gap_fills_and_splices() {
    let mut p = fixture();
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    move_clip(&mut p, "c2", "v2", 1000, false).unwrap();
    // Source: c2 was the LAST clip, so vacating its slot leaves a dangling TRAILING
    // gap — pure black tail — which is trimmed. v1 is just c1 now, length 5000
    // (NOT 8000 with a trailing gap, which would render/play black past the content).
    let v1 = p.track("v1").unwrap();
    assert_eq!(v1.duration_ms(), 5000);
    assert_eq!(v1.clips.len(), 1);
    assert_eq!(v1.clips[0].id(), Some("c1"));
    // Dest: 1000ms gap pad then the clip.
    let v2 = p.track("v2").unwrap();
    assert!(matches!(v2.clips[0], Clip::Gap(ref g) if g.duration_ms == 1000));
    assert_eq!(v2.clips[1].id(), Some("c2"));
    // Kind mismatch is rejected.
    assert!(move_clip(&mut p, "c1", "cap1", 0, false).is_err());
}

/// Moving a clip far right then back to the start must NOT
/// leave a BLACK TAIL. Moving right pads/extends the track; moving back vacates the
/// far slot → a trailing gap. Untrimmed, the track length stays at the far extent and
/// playback/render run past the content into black. The trailing gap must be trimmed
/// so the length returns to the real content extent.
#[test]
fn move_forward_then_back_leaves_no_black_tail() {
    let mut p = fixture();
    assert_eq!(p.track("v1").unwrap().duration_ms(), 8000); // c1[0,5000)+c2[5000,8000)
    move_clip(&mut p, "c1", "v1", 20000, false).unwrap(); // shove c1 far right
    move_clip(&mut p, "c1", "v1", 0, false).unwrap(); // ...then drag it back
    let v1 = p.track("v1").unwrap();
    // The far move padded the track out to 25000; dragging back must NOT leave that
    // far extent as a black tail. The trailing pad-gap is trimmed → the track ends on
    // real CONTENT, well short of the far extent. (An internal gap between clips may
    // remain — that's an ordinary hole, not the reported end-of-timeline black.)
    assert!(
        v1.duration_ms() < 20000,
        "trailing black tail must be trimmed (far extent was 25000), got {}",
        v1.duration_ms()
    );
    assert!(
        !matches!(v1.clips.last(), Some(Clip::Gap(_))),
        "track must end on content, never a dangling trailing gap"
    );
    assert_eq!(
        v1.clips.last().unwrap().id(),
        Some("c2"),
        "composition ends on the real last clip, not black"
    );
    assert!(v1.clips.iter().any(|c| c.id() == Some("c1")));
}

/// insert splits the clip under the point and uses probe for full length.
#[test]
fn insert_splices_mid_clip() {
    let mut p = fixture();
    p.assets.insert(
        "a1".into(),
        crate::types::Asset {
            path: "/x.mp4".into(),
            hash: "sha256:x".into(),
            probe: Some(json!({"duration_ms": 8000})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let eff = insert(&mut p, "a1", "v1", 2500, Some([0, 1000]), false).unwrap();
    let new_id = eff[0].detail["added_clip"].as_str().unwrap();
    let v1 = p.track("v1").unwrap();
    assert_eq!(v1.duration_ms(), 9000);
    assert_eq!(v1.clips[1].id(), Some(new_id));
    // No src_range + probe present → full asset length.
    let eff2 = insert(&mut p, "a1", "v1", 0, None, false).unwrap();
    assert_eq!(eff2[0].detail["src_range_ms"], json!([0, 8000]));
    // Unknown asset is actionable.
    assert!(insert(&mut p, "zz", "v1", 0, None, false).is_err());
}

#[test]
fn insert_explicit_src_range_respects_probe_duration() {
    let mut p = fixture();
    p.assets.insert(
        "a1".into(),
        crate::types::Asset {
            path: "/x.mp4".into(),
            hash: "h".into(),
            probe: Some(json!({"duration_ms":8000})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let err = insert(&mut p, "a1", "v1", 0, Some([0, 9000]), false)
        .expect_err("explicit src_range_ms must not exceed probed source duration");
    assert_eq!(err.code, codes::INVALID_ARGS);
    assert!(
        err.message.contains("src_out_ms") && err.message.contains("duration"),
        "error names source range and duration: {err:?}"
    );
}

/// the ripple-sync contract regression: ripple=true gap-inserts into sibling tracks so AV
/// sync (and overlay/caption alignment) survives the insert; the content-relative-marker contract: the
/// markers move with the content they annotate.
#[test]
fn insert_ripple_shifts_siblings_captions_markers() {
    let mut p = fixture(); // v1 8000ms (c1+c2), a1t 8000ms (c3), cap1, m1@1800 m2@7000
    p.assets.insert(
        "card".into(),
        crate::types::Asset {
            path: "/card.png".into(),
            hash: "sha256:c".into(),
            probe: Some(json!({"kind": "image"})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    // Duck windows on the audio sibling — they must move with the audio.
    p.track_mut("a1t").unwrap().gain_windows = vec![GainWindow {
        range_ms: [1000, 2000],
        db: -18.0,
        attack_ms: 250,
    }];
    // The A/V-offset regression case: 2.5s intro card at v1:0 with ripple.
    let eff = insert(&mut p, "card", "v1", 0, Some([0, 2500]), true).unwrap();
    assert_eq!(eff[0].detail["ripple"], json!(true));
    // Video and audio track ends stay equal — AV offset is the ripple-sync contract bug.
    assert_eq!(p.track("v1").unwrap().duration_ms(), 10500);
    assert_eq!(
        p.track("a1t").unwrap().duration_ms(),
        10500,
        "audio rippled with video"
    );
    // The audio shift is a leading GAP (renders as silence), content intact.
    match &p.track("a1t").unwrap().clips[..] {
        [Clip::Gap(g), Clip::Media(c)] => {
            assert_eq!(g.duration_ms, 2500);
            assert_eq!(c.id, "c3");
        }
        other => unreachable!("expected [gap, c3] on a1t, got {other:?}"),
    }
    // Captions (absolute ranges) shifted edge-wise.
    let cap = p.track("cap1").unwrap();
    let ranges: Vec<[u64; 2]> = cap
        .clips
        .iter()
        .map(|c| match c {
            Clip::Caption(cc) => cc.range_ms,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(ranges, vec![[2500, 3500], [4000, 5000], [6500, 8500]]);
    // Markers shifted by the inserted duration (the content-relative-marker contract).
    assert_eq!(p.markers[0].at_ms, 4300);
    assert_eq!(p.markers[1].at_ms, 9500);
    // Sibling gain windows moved with their content.
    assert_eq!(
        p.track("a1t").unwrap().gain_windows[0].range_ms,
        [3500, 4500]
    );
    // Effects record the ripple per sibling + the marker shift.
    assert!(eff.iter().any(|e| e.track.as_deref() == Some("a1t")
        && e.detail.get("rippled_gap_ms") == Some(&json!([0, 2500]))));
    assert!(eff
        .iter()
        .any(|e| e.detail.get("markers_shifted") == Some(&json!(2))));
}

/// Ripple mid-clip splits the sibling's clip around the inserted gap
/// (razor-across-tracks, the NLE insert-edit convention); a marker
/// exactly AT the insert point follows the content (shifts right);
/// content strictly before the point stays.
#[test]
fn insert_ripple_mid_clip_and_boundary_marker() {
    let mut p = fixture();
    p.assets.insert(
        "a1".into(),
        crate::types::Asset {
            path: "/x.mp4".into(),
            hash: "sha256:x".into(),
            probe: Some(json!({"duration_ms": 8000})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    p.markers.push(Marker {
        id: "m3".into(),
        at_ms: 3000,
        label: "at-point".into(),
        note: None,
        color: None,
    });
    insert(&mut p, "a1", "v1", 3000, Some([0, 1000]), true).unwrap();
    // a1t's c3 [0..8000) is split at 3000 with a 1000ms gap between.
    let a1t = p.track("a1t").unwrap();
    match &a1t.clips[..] {
        [Clip::Media(l), Clip::Gap(g), Clip::Media(r)] => {
            assert_eq!((l.src_in_ms, l.src_out_ms), (0, 3000));
            assert_eq!(g.duration_ms, 1000);
            assert_eq!((r.src_in_ms, r.src_out_ms), (3000, 8000));
        }
        other => unreachable!("expected [media, gap, media] on a1t, got {other:?}"),
    }
    // m1@1800 before the point: untouched. m3@3000 AT the point: +1000.
    assert_eq!(p.markers[0].at_ms, 1800);
    assert_eq!(p.markers[2].at_ms, 4000);
}

/// ripple=false keeps the legacy single-track behavior (the documented
/// overlay/replace opt-out) — but the TARGET track's own gain windows
/// follow its shifted content in both modes.
#[test]
fn insert_no_ripple_leaves_siblings_but_remaps_target_windows() {
    let mut p = fixture();
    p.assets.insert(
        "a1".into(),
        crate::types::Asset {
            path: "/x.mp4".into(),
            hash: "sha256:x".into(),
            probe: Some(json!({"duration_ms": 8000})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    p.track_mut("a1t").unwrap().gain_windows = vec![GainWindow {
        range_ms: [4000, 5000],
        db: -12.0,
        attack_ms: 250,
    }];
    insert(&mut p, "a1", "a1t", 0, Some([0, 2000]), false).unwrap();
    // Target shifted; its windows moved with its content.
    assert_eq!(p.track("a1t").unwrap().duration_ms(), 10000);
    assert_eq!(
        p.track("a1t").unwrap().gain_windows[0].range_ms,
        [6000, 7000]
    );
    // Siblings, captions, markers untouched (deliberate desync).
    assert_eq!(p.track("v1").unwrap().duration_ms(), 8000);
    assert_eq!(p.markers[0].at_ms, 1800);
    let cap = p.track("cap1").unwrap();
    match &cap.clips[0] {
        Clip::Caption(cc) => assert_eq!(cc.range_ms, [0, 1000]),
        _ => unreachable!(),
    }
}

/// Replay compatibility: edit.insert args WITHOUT the ripple key (every
/// op logged before the ripple-sync contract) apply with ripple=false — old logs replay to
/// the exact state they produced live.
#[test]
fn insert_replay_without_ripple_key_is_legacy() {
    let mut p = fixture();
    p.assets.insert(
        "a1".into(),
        crate::types::Asset {
            path: "/x.mp4".into(),
            hash: "sha256:x".into(),
            probe: Some(json!({"duration_ms": 8000})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let args = json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 2500]});
    crate::store::apply_edit_verb(&mut p, "edit.insert", &args).unwrap();
    assert_eq!(p.track("v1").unwrap().duration_ms(), 10500);
    assert_eq!(
        p.track("a1t").unwrap().duration_ms(),
        8000,
        "legacy: no sibling ripple"
    );
    assert_eq!(p.markers[0].at_ms, 1800, "legacy: markers stay");
}

/// gain records old and new values for both target kinds.
#[test]
fn gain_sets_and_records() {
    let mut p = fixture();
    let e1 = gain(&mut p, GainTarget::Clip("c3".into()), -6.0).unwrap();
    assert_eq!(e1[0].detail["old_db"], json!(0.0));
    let e2 = gain(&mut p, GainTarget::Track("a1t".into()), 3.0).unwrap();
    assert_eq!(e2[0].detail["new_db"], json!(3.0));
    assert_eq!(p.track("a1t").unwrap().gain_db, 3.0);
    assert_eq!(
        gain(&mut p, GainTarget::Clip("c1".into()), -6.0)
            .unwrap_err()
            .code,
        codes::INVALID_ARGS,
        "video clip gain would be a render no-op"
    );
    assert_eq!(
        gain(&mut p, GainTarget::Track("v1".into()), -6.0)
            .unwrap_err()
            .code,
        codes::INVALID_ARGS,
        "video track gain would be a render no-op"
    );
}

/// transform: validates normalized geometry, video-media-clips only,
/// identity clears, split copies the transform to both halves.
#[test]
fn transform_sets_clears_and_survives_split() {
    let mut p = fixture();
    let t = |x: f64, y: f64, scale: f64| crate::types::ClipTransform {
        x,
        y,
        scale,
        opacity: 1.0,
    };
    // Validation: scale (0,1], x/y [0,1], video media clips only.
    assert_eq!(
        transform(&mut p, "c1", t(0.0, 0.0, 0.0)).unwrap_err().code,
        "invalid_args"
    );
    assert_eq!(
        transform(&mut p, "c1", t(0.0, 0.0, 1.5)).unwrap_err().code,
        "invalid_args"
    );
    assert_eq!(
        transform(&mut p, "c1", t(-0.2, 0.0, 0.5)).unwrap_err().code,
        "invalid_args"
    );
    assert_eq!(
        transform(&mut p, "c3", t(0.1, 0.1, 0.5)).unwrap_err().code,
        "invalid_args"
    ); // audio clip
    assert_eq!(
        transform(&mut p, "nope", t(0.1, 0.1, 0.5))
            .unwrap_err()
            .code,
        "not_found"
    );
    // Set → stored; effects record old (None) and new.
    let e = transform(&mut p, "c1", t(0.65, 0.05, 0.3)).unwrap();
    assert!(e[0].detail["old_transform"].is_null());
    assert_eq!(e[0].detail["new_transform"]["scale"], 0.3);
    match p.track("v1").unwrap().clips.first().unwrap() {
        Clip::Media(c) => assert_eq!(c.transform.as_ref().unwrap().x, 0.65),
        _ => unreachable!("c1 is media"),
    }
    // Split copies the transform to both halves (a PiP cut in two stays PiP).
    split(&mut p, "v1", 2000).unwrap();
    for clip in &p.track("v1").unwrap().clips[..2] {
        match clip {
            Clip::Media(c) => assert!(c.transform.is_some(), "both halves keep the transform"),
            _ => unreachable!("media expected"),
        }
    }
    // Identity clears.
    let e = transform(&mut p, "c1", t(0.0, 0.0, 1.0)).unwrap();
    assert!(e[0].detail["new_transform"].is_null());
    match p.track("v1").unwrap().clips.first().unwrap() {
        Clip::Media(c) => assert!(c.transform.is_none()),
        _ => unreachable!(),
    }
    // Opacity: out of [0,1] rejected; a full-frame clip at opacity<1 is NOT
    // identity (so it is stored), and opacity round-trips.
    let to = |x, y, scale, opacity| crate::types::ClipTransform {
        x,
        y,
        scale,
        opacity,
    };
    assert_eq!(
        transform(&mut p, "c1", to(0.0, 0.0, 1.0, 1.5))
            .unwrap_err()
            .code,
        "invalid_args"
    );
    let e = transform(&mut p, "c1", to(0.0, 0.0, 1.0, 0.4)).unwrap();
    assert_eq!(e[0].detail["new_transform"]["opacity"], 0.4);
    match p.track("v1").unwrap().clips.first().unwrap() {
        Clip::Media(c) => assert_eq!(
            c.transform
                .as_ref()
                .expect("opacity<1 full-frame is stored")
                .opacity,
            0.4
        ),
        _ => unreachable!(),
    }
}

/// reorder_track: moves a track to a new GROUP-RELATIVE z-order index within
/// its own kind, clamps an out-of-range index to the last same-kind position,
/// records from/to, errors on an unknown id. Track order = compositing stack
/// (video tracks); grouped insert keeps videos contiguous before the audio.
#[test]
fn reorder_track_moves_and_clamps() {
    let mut p = fixture(); // [v1, a1t, cap1]
    add_track(&mut p, TrackKind::Video, Some("v2")).unwrap();
    add_track(&mut p, TrackKind::Video, Some("v3")).unwrap();
    // Grouped insert puts new VIDEO tracks after the last video, so the audio
    // and caption tracks stay after them → tracks = [v1, v2, v3, a1t, cap1].
    // (Before grouping this test asserted tracks.last()=="v3"; the grouped
    // insert keeps videos contiguous up front, so v3 is at index 2.)
    assert_eq!(p.tracks.last().unwrap().id, "cap1");
    assert_eq!(p.tracks.iter().position(|t| t.id == "v3").unwrap(), 2);
    // Move v3 to the front of the VIDEO group (group index 0) → for the video
    // prefix, group index == absolute index, so it lands at absolute 0.
    let e = reorder_track(&mut p, "v3", 0).unwrap();
    assert_eq!(e[0].detail["to"], 0);
    assert_eq!(p.tracks.iter().position(|t| t.id == "v3").unwrap(), 0);
    // Out-of-range index clamps to the LAST position IN THE KIND (last video),
    // which is absolute index 2 — v3 sits after v1/v2 but before the audio.
    reorder_track(&mut p, "v3", 999).unwrap();
    assert_eq!(p.tracks.iter().position(|t| t.id == "v3").unwrap(), 2);
    // The audio + caption tracks are untouched by a video reorder (grouping
    // preserved: video reorder can never cross into the audio/caption groups).
    assert_eq!(p.tracks.last().unwrap().id, "cap1");
    assert_eq!(p.tracks.iter().position(|t| t.id == "a1t").unwrap(), 3);
    // Unknown track id → not_found.
    assert_eq!(
        reorder_track(&mut p, "nope", 0).unwrap_err().code,
        "not_found"
    );
    // Regression (the composed-frame 422): the effect must round-trip through
    // serde. A `track` key in the detail collided with OpEffect.track under
    // #[serde(flatten)] → "duplicate field `track`" when any path re-read the
    // op log. The detail must NOT carry `track`; the effect's field does.
    let eff = reorder_track(&mut p, "v3", 1).unwrap();
    assert!(
        !eff[0].detail.contains_key("track"),
        "detail must not duplicate `track`"
    );
    let wire = serde_json::to_string(&eff[0]).unwrap();
    let back: crate::ops::OpEffect =
        serde_json::from_str(&wire).expect("effect must deserialize (no dup field)");
    assert_eq!(back.track.as_deref(), Some("v3"));
    assert_eq!(back.detail.get("to").and_then(|v| v.as_u64()), Some(1));
}

/// add_track GROUPED INSERT: a new video lands at the end of the VIDEO group
/// (after the last video, still on top of prior videos), a new audio at the
/// end of the AUDIO group — never interleaved. Starting [v1, a1t]: adding a
/// video → [v1, v2, a1t]; then an audio → [v1, v2, a1t, a2t]. This is the
/// fix for the `[v1, a1t, v2, a2t]` interleaving a tail push produced.
#[test]
fn add_track_groups_by_kind() {
    // A clean project.create layout — exactly the spec's starting [v1, a1t]
    // (the shared `fixture()` adds a caption track, which would muddy the
    // tail-position assertions; grouping-with-captions is exercised by the
    // reorder test that runs on `fixture()`).
    let mut p = Project::new("t", ProjectSettings::default());
    assert_eq!(
        p.tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        vec!["v1", "a1t"]
    );
    // New video inserts AFTER the last video, BEFORE the audio.
    add_track(&mut p, TrackKind::Video, Some("v2")).unwrap();
    assert_eq!(
        p.tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        vec!["v1", "v2", "a1t"]
    );
    // New audio inserts at the end of the audio group (here, the tail).
    add_track(&mut p, TrackKind::Audio, Some("a2t")).unwrap();
    assert_eq!(
        p.tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        vec!["v1", "v2", "a1t", "a2t"]
    );
    // The new video stayed on TOP of v1 (z-order invariant: later video
    // overlays earlier), and the videos stayed contiguous before the audio.
    assert_eq!(p.tracks[0].id, "v1");
    assert_eq!(p.tracks[1].id, "v2");
}

/// normalize_track_order heals an interleaved project: [v1, a1t, v2, a2t] →
/// [v1, v2, a1t, a2t] (stable partition into [Video…, Audio…, Caption…],
/// within-kind order preserved so the video z-order is untouched). Idempotent:
/// normalizing twice yields the same result as once.
#[test]
fn normalize_track_order_groups_and_is_idempotent() {
    let mk = |id: &str, kind: TrackKind| Track {
        id: id.into(),
        kind,
        clips: vec![],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    };
    let mut p = fixture();
    // Build the legacy interleaved layout a tail-append used to produce.
    p.tracks = vec![
        mk("v1", TrackKind::Video),
        mk("a1t", TrackKind::Audio),
        mk("v2", TrackKind::Video),
        mk("a2t", TrackKind::Audio),
    ];
    p.normalize_track_order();
    let after: Vec<&str> = p.tracks.iter().map(|t| t.id.as_str()).collect();
    // Grouped, and relative order WITHIN each kind preserved (v1 before v2,
    // a1t before a2t) → video compositing z-order unchanged.
    assert_eq!(after, vec!["v1", "v2", "a1t", "a2t"]);
    // Idempotent: a second normalize is a no-op.
    let once = p.tracks.clone();
    p.normalize_track_order();
    assert_eq!(p.tracks, once);
}

/// remove_track: removes an empty extra track; refuses the last video/audio
/// track; refuses a non-empty track without force; force drops the clips.
#[test]
fn remove_track_guards_and_force() {
    let mut p = fixture();
    // Extra overlay video track, empty → removable.
    add_track(&mut p, TrackKind::Video, Some("v2")).unwrap();
    let e = remove_track(&mut p, "v2", false).unwrap();
    assert_eq!(e[0].detail["removed_track"], "v2");
    assert!(p.track("v2").is_none());
    // The LAST video track cannot be removed (renderer needs a base).
    assert_eq!(
        remove_track(&mut p, "v1", false).unwrap_err().code,
        "invalid_args"
    );
    // The LAST audio track cannot be removed either.
    assert_eq!(
        remove_track(&mut p, "a1t", false).unwrap_err().code,
        "invalid_args"
    );
    // A non-empty extra track: refused without force, dropped with it.
    add_track(&mut p, TrackKind::Video, Some("v3")).unwrap();
    let v3 = p.tracks.iter_mut().find(|t| t.id == "v3").unwrap();
    v3.clips
        .push(Clip::Media(make_media_clip("c_rt", "a1", 0, 1000)));
    assert_eq!(
        remove_track(&mut p, "v3", false).unwrap_err().code,
        "conflict"
    );
    let e = remove_track(&mut p, "v3", true).unwrap();
    assert_eq!(e[0].detail["clips_dropped"], 1);
    assert!(p.track("v3").is_none());
    // Unknown id → not_found.
    assert_eq!(
        remove_track(&mut p, "nope", false).unwrap_err().code,
        "not_found"
    );
}

/// crop: sets a source rect, validates against probed geometry,
/// clears on an identity crop, copies to both halves on a split, and
/// refuses audio clips + zero-size + out-of-bounds rects.
#[test]
fn crop_sets_clears_validates_and_survives_split() {
    use crate::types::{Asset, ClipCrop};
    let mut p = fixture();
    // Probe a1 at 3840x2160 so bounds-checking + identity-detection work.
    p.assets.insert(
        "a1".into(),
        Asset {
            path: "/x.mp4".into(),
            hash: "sha256:x".into(),
            probe: Some(json!({"width": 3840, "height": 2160, "duration_ms": 8000})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let c = |x, y, w, h| ClipCrop { x, y, w, h };
    // Validation.
    assert_eq!(
        crop(&mut p, "c1", c(0, 0, 0, 100)).unwrap_err().code,
        "invalid_args"
    ); // zero w
    assert_eq!(
        crop(&mut p, "c3", c(0, 0, 100, 100)).unwrap_err().code,
        "invalid_args"
    ); // audio clip
    assert_eq!(
        crop(&mut p, "nope", c(0, 0, 100, 100)).unwrap_err().code,
        "not_found"
    );
    assert_eq!(
        crop(&mut p, "c1", c(0, 54, 3840, 2160)).unwrap_err().code,
        "invalid_args"
    ); // y+h>height
    assert_eq!(
        crop(&mut p, "c1", c(u32::MAX, 0, 1, 1)).unwrap_err().code,
        "invalid_args",
        "overflowing crop bounds must be rejected, not panic"
    );
    // Set the real-driver crop (remove the 54px top/bottom bands).
    let e = crop(&mut p, "c1", c(0, 54, 3840, 2052)).unwrap();
    assert!(e[0].detail["old_crop"].is_null());
    assert_eq!(e[0].detail["new_crop"]["y"], 54);
    assert_eq!(e[0].detail["new_crop"]["h"], 2052);
    match p.track("v1").unwrap().clips.first().unwrap() {
        Clip::Media(m) => assert_eq!(m.crop.as_ref().unwrap().h, 2052),
        _ => unreachable!("c1 is media"),
    }
    // Split copies the crop to both halves (a cropped clip cut in two
    // keeps the same source rectangle on each side).
    split(&mut p, "v1", 2000).unwrap();
    for clip in &p.track("v1").unwrap().clips[..2] {
        match clip {
            Clip::Media(m) => {
                assert_eq!(m.crop.as_ref().unwrap().y, 54, "both halves keep the crop")
            }
            _ => unreachable!("media expected"),
        }
    }
    // Identity crop (full source frame) clears.
    let e = crop(&mut p, "c1", c(0, 0, 3840, 2160)).unwrap();
    assert!(e[0].detail["new_crop"].is_null());
    match p.track("v1").unwrap().clips.first().unwrap() {
        Clip::Media(m) => assert!(m.crop.is_none()),
        _ => unreachable!(),
    }
}

/// duck: sets/replaces windows on audio tracks only, validates window
/// shape + negative db; ripple_delete remaps windows like captions.
#[test]
fn duck_windows_set_validate_and_ripple() {
    let mut p = fixture();
    let w = |a: u64, b: u64| GainWindow {
        range_ms: [a, b],
        db: -18.0,
        attack_ms: 250,
    };
    // Audio-only target.
    assert_eq!(
        duck(&mut p, "v1", vec![w(0, 500)]).unwrap_err().code,
        "invalid_args"
    );
    assert_eq!(duck(&mut p, "nope", vec![]).unwrap_err().code, "not_found");
    // Bad windows refused.
    assert_eq!(
        duck(&mut p, "a1t", vec![w(500, 500)]).unwrap_err().code,
        "invalid_args"
    );
    assert_eq!(
        duck(
            &mut p,
            "a1t",
            vec![GainWindow {
                range_ms: [0, 500],
                db: 3.0,
                attack_ms: 250
            }]
        )
        .unwrap_err()
        .code,
        "invalid_args"
    );
    // Set, then replace (the documented refresh path).
    let e = duck(&mut p, "a1t", vec![w(1000, 2000), w(4000, 6000)]).unwrap();
    assert_eq!(e[0].detail["ducked_windows"], 2);
    assert_eq!(e[0].detail["replaced_windows"], 0);
    let e = duck(&mut p, "a1t", vec![w(1000, 2000)]).unwrap();
    assert_eq!(e[0].detail["replaced_windows"], 2);
    assert_eq!(p.track("a1t").unwrap().gain_windows.len(), 1);
    // Ripple remap: windows move with the content they duck.
    duck(&mut p, "a1t", vec![w(1000, 2000), w(4000, 6000)]).unwrap();
    ripple_delete(&mut p, None, [0, 500], true).unwrap();
    let ws = &p.track("a1t").unwrap().gain_windows;
    assert_eq!(ws[0].range_ms, [500, 1500]);
    assert_eq!(ws[1].range_ms, [3500, 5500]);
    // A window fully inside a removed range collapses and is dropped.
    ripple_delete(&mut p, None, [400, 1600], true).unwrap();
    let ws = &p.track("a1t").unwrap().gain_windows;
    assert_eq!(ws.len(), 1, "collapsed window dropped: {ws:?}");
    assert_eq!(ws[0].range_ms, [2300, 4300]);
}

/// edit.fade (the fade-edit contract): set/update/clear semantics, track-form resolution
/// to first/last media clips, kind-vs-track validation, ramp-overlap
/// refusal.
#[test]
fn fade_sets_updates_clears_and_validates() {
    let mut p = fixture(); // v1: c1[0..5000)+c2[5000..8000); a1t: c3[0..8000)
    let clip = |id: &str| FadeTarget::Clip(id.to_string());
    let track = |id: &str| FadeTarget::Track(id.to_string());
    // Set in+out on one audio clip.
    let e = fade(&mut p, clip("c3"), Some(500), Some(1000), FadeKind::Audio).unwrap();
    assert_eq!(e[0].detail["new_fade"]["in_ms"], 500);
    assert_eq!(e[0].detail["new_fade"]["out_ms"], 1000);
    assert!(e[0].detail["old_fade"].is_null());
    // Update only out_ms — in_ms survives; kind is replaced.
    let e = fade(&mut p, clip("c3"), None, Some(250), FadeKind::Both).unwrap();
    assert_eq!(e[0].detail["new_fade"]["in_ms"], 500);
    assert_eq!(e[0].detail["new_fade"]["out_ms"], 250);
    assert_eq!(e[0].detail["new_fade"]["kind"], "both");
    // Explicit zeros clear entirely.
    let e = fade(&mut p, clip("c3"), Some(0), Some(0), FadeKind::Both).unwrap();
    assert!(e[0].detail["new_fade"].is_null());
    // Track form: in → first media clip, out → last.
    let e = fade(&mut p, track("v1"), Some(400), Some(600), FadeKind::Video).unwrap();
    assert_eq!(e.len(), 2);
    assert_eq!(e[0].detail["clip"], "c1");
    assert_eq!(e[0].detail["new_fade"]["in_ms"], 400);
    assert_eq!(e[0].detail["new_fade"]["out_ms"], 0);
    assert_eq!(e[1].detail["clip"], "c2");
    assert_eq!(e[1].detail["new_fade"]["out_ms"], 600);
    // Validation: neither side given; ramps exceeding the clip; kind that
    // cannot render on the target's track; caption/non-media targets.
    assert_eq!(
        fade(&mut p, clip("c1"), None, None, FadeKind::Both)
            .unwrap_err()
            .code,
        "invalid_args"
    );
    assert_eq!(
        fade(&mut p, clip("c2"), Some(2000), Some(1500), FadeKind::Video)
            .unwrap_err()
            .code,
        "invalid_args" // 3500 > c2's 3000ms
    );
    speed(&mut p, "c2", 2.0).unwrap(); // c2 is 3000ms source, 1500ms timeline
    assert_eq!(
        fade(&mut p, clip("c2"), Some(900), Some(800), FadeKind::Video)
            .unwrap_err()
            .code,
        "invalid_args",
        "fade overlap must validate against realized timeline duration"
    );
    assert_eq!(
        fade(&mut p, clip("c1"), Some(100), None, FadeKind::Audio)
            .unwrap_err()
            .code,
        "invalid_args"
    );
    assert_eq!(
        fade(&mut p, clip("c3"), Some(100), None, FadeKind::Video)
            .unwrap_err()
            .code,
        "invalid_args"
    );
    assert_eq!(
        fade(&mut p, clip("s1"), Some(100), None, FadeKind::Both)
            .unwrap_err()
            .code,
        "invalid_args"
    );
    assert_eq!(
        fade(&mut p, clip("nope"), Some(100), None, FadeKind::Both)
            .unwrap_err()
            .code,
        "not_found"
    );
    assert_eq!(
        fade(&mut p, track("cap1"), Some(100), None, FadeKind::Both)
            .unwrap_err()
            .code,
        "not_found"
    );
}

/// Splitting a faded clip must NOT invent a mid-timeline dip: the left
/// half keeps the fade-in, the right half the fade-out — at every split
/// site (edit.split, ripple_delete remnants).
#[test]
fn fade_splits_left_in_right_out() {
    let mut p = fixture();
    fade(
        &mut p,
        FadeTarget::Clip("c1".into()),
        Some(400),
        Some(600),
        FadeKind::Video,
    )
    .unwrap();
    split(&mut p, "v1", 2000).unwrap();
    let v1 = p.track("v1").unwrap();
    match (&v1.clips[0], &v1.clips[1]) {
        (Clip::Media(l), Clip::Media(r)) => {
            assert_eq!(l.fade.as_ref().map(|f| (f.in_ms, f.out_ms)), Some((400, 0)));
            assert_eq!(r.fade.as_ref().map(|f| (f.in_ms, f.out_ms)), Some((0, 600)));
        }
        _ => unreachable!("media halves expected"),
    }
    // ripple_delete of the clip's TAIL takes the fade-out with it.
    let mut p2 = fixture();
    fade(
        &mut p2,
        FadeTarget::Clip("c1".into()),
        Some(400),
        Some(600),
        FadeKind::Video,
    )
    .unwrap();
    ripple_delete(&mut p2, Some("v1"), [4000, 5000], true).unwrap();
    match p2.track("v1").unwrap().clips.first().unwrap() {
        Clip::Media(c) => {
            assert_eq!(c.fade.as_ref().map(|f| (f.in_ms, f.out_ms)), Some((400, 0)));
        }
        _ => unreachable!("media expected"),
    }
}

/// add_track: deterministic id allocation per kind, explicit-id conflict
/// detection, caption refusal.
#[test]
fn add_track_allocates_and_guards() {
    let mut p = fixture(); // has v1, a1t, cap1
    let e = add_track(&mut p, TrackKind::Video, None).unwrap();
    assert_eq!(e[0].detail["added_track"], "v2");
    assert_eq!(e[0].detail["kind"], "video");
    let e = add_track(&mut p, TrackKind::Audio, None).unwrap();
    assert_eq!(e[0].detail["added_track"], "a2t");
    // Allocation advances past the new tracks (v3 next, not v2 again).
    let e = add_track(&mut p, TrackKind::Video, None).unwrap();
    assert_eq!(e[0].detail["added_track"], "v3");
    // Explicit id works once, conflicts after.
    let e = add_track(&mut p, TrackKind::Audio, Some("music")).unwrap();
    assert_eq!(e[0].detail["added_track"], "music");
    assert_eq!(
        add_track(&mut p, TrackKind::Audio, Some("music"))
            .unwrap_err()
            .code,
        "conflict"
    );
    // Caption tracks are captions.*'s job.
    assert_eq!(
        add_track(&mut p, TrackKind::Caption, None)
            .unwrap_err()
            .code,
        "invalid_args"
    );
    // New tracks are empty and the right kind.
    assert_eq!(p.track("v2").unwrap().kind, TrackKind::Video);
    assert!(p.track("v2").unwrap().clips.is_empty());
    assert_eq!(p.track("music").unwrap().kind, TrackKind::Audio);
}

/// markers: deterministic ids, removal records the full marker.
#[test]
fn markers_add_remove() {
    let mut p = fixture();
    let e = marker_add(&mut p, 500, "note here", Some("detail")).unwrap();
    assert_eq!(e[0].detail["added_marker"]["id"], "m3"); // m1,m2 exist
    let r = marker_remove(&mut p, "m3").unwrap();
    assert_eq!(r[0].detail["removed_marker"]["at_ms"], 500);
    assert!(marker_remove(&mut p, "m3").is_err());
}

/// edit.crossfade: sets xfade_in_ms on the RIGHT clip of a cut,
/// validates adjacency + media-only + duration clamp, clears the boundary
/// fades, and 0 clears the crossfade. The EDL then pulls the timeline back.
#[test]
fn crossfade_sets_validates_and_clears() {
    let mut p = fixture(); // v1: c1[0..5000)+c2[5000..8000); cut at 5000
                           // No cut at 1234 (mid-clip): not_found.
    assert_eq!(
        crossfade(&mut p, "v1", 1234, 500, None).unwrap_err().code,
        "not_found"
    );
    // Overlap longer than the shorter neighbour (c2 is 3000ms): refused.
    assert_eq!(
        crossfade(&mut p, "v1", 5000, 4000, None).unwrap_err().code,
        "invalid_args"
    );
    // Set a 1000ms crossfade at the 5000ms cut → stored on c2 (the right clip).
    let e = crossfade(&mut p, "v1", 5000, 1000, None).unwrap();
    assert_eq!(e[0].detail["right_clip"], "c2");
    assert_eq!(e[0].detail["left_clip"], "c1");
    assert_eq!(e[0].detail["xfade_ms"], 1000);
    match &p.track("v1").unwrap().clips[1] {
        Clip::Media(c) => assert_eq!(c.xfade_in_ms, 1000),
        _ => unreachable!("c2 is media"),
    }
    // The EDL pulls c2 back by the overlap; v1's realized end is 7000. The
    // COMPOSITION duration is still gated by the longer a1t track (8000),
    // which carries no crossfade — a crossfade on one track only shortens
    // THAT track (realistic crossfades mirror onto the audio track too).
    let edl = crate::edl::edl_from_project(&p);
    let c2seg = edl
        .segments
        .iter()
        .find(|s| s.clip_id.as_deref() == Some("c2"))
        .unwrap();
    assert_eq!(c2seg.timeline_in_ms, 4000, "c2 pulled back by 1000ms");
    assert_eq!(
        c2seg.timeline_out_ms, 7000,
        "v1 realized end after the crossfade"
    );
    assert_eq!(c2seg.xfade_in_ms, 1000);
    // Crossfade BOTH tracks (the real AV use): shorten the whole composition.
    let mut pav = fixture();
    // a1t is a single c3[0..8000) clip — split it at 5000 to make a cut.
    split(&mut pav, "a1t", 5000).unwrap();
    crossfade(&mut pav, "v1", 5000, 1000, None).unwrap();
    crossfade(&mut pav, "a1t", 5000, 1000, None).unwrap();
    assert_eq!(
        crate::edl::edl_from_project(&pav).duration_ms,
        7000,
        "both tracks crossfaded → composition shortens by the overlap"
    );
    // Boundary-fade clearing: set a fade-out on c1 + fade-in on c2, then a
    // crossfade clears exactly those (the crossfade owns the boundary).
    let mut p2 = fixture();
    fade(
        &mut p2,
        FadeTarget::Clip("c1".into()),
        Some(200),
        Some(300),
        FadeKind::Video,
    )
    .unwrap();
    fade(
        &mut p2,
        FadeTarget::Clip("c2".into()),
        Some(400),
        Some(500),
        FadeKind::Video,
    )
    .unwrap();
    crossfade(&mut p2, "v1", 5000, 800, None).unwrap();
    match (
        &p2.track("v1").unwrap().clips[0],
        &p2.track("v1").unwrap().clips[1],
    ) {
        (Clip::Media(c1), Clip::Media(c2)) => {
            // c1 keeps its fade-IN (200), loses fade-OUT (was 300).
            assert_eq!(
                c1.fade.as_ref().map(|f| (f.in_ms, f.out_ms)),
                Some((200, 0))
            );
            // c2 keeps its fade-OUT (500), loses fade-IN (was 400).
            assert_eq!(
                c2.fade.as_ref().map(|f| (f.in_ms, f.out_ms)),
                Some((0, 500))
            );
            assert_eq!(c2.xfade_in_ms, 800);
        }
        _ => unreachable!("media clips expected"),
    }
    // 0 clears the crossfade (hard cut again).
    crossfade(&mut p2, "v1", 5000, 0, None).unwrap();
    match &p2.track("v1").unwrap().clips[1] {
        Clip::Media(c) => assert_eq!(c.xfade_in_ms, 0),
        _ => unreachable!(),
    }
}

/// Transitions (edit.crossfade `transition`): a non-default style is stored on
/// the right clip + carried onto the EDL segment; "fade" normalizes to None
/// (byte-identical to a plain dissolve); clearing the crossfade clears the
/// style; and the validator (is_valid_transition) gates the exposed set.
#[test]
fn crossfade_transition_style_is_stored_and_carried() {
    let xfade_kind = |p: &Project, idx: usize| match &p.track("v1").unwrap().clips[idx] {
        Clip::Media(c) => c.xfade_kind.clone(),
        _ => None,
    };

    // A styled crossfade stores the ffmpeg xfade name on the right clip.
    let mut p = fixture();
    let e = crossfade(&mut p, "v1", 5000, 1000, Some("wipeleft")).unwrap();
    assert_eq!(e[0].detail["transition"], "wipeleft");
    assert_eq!(xfade_kind(&p, 1).as_deref(), Some("wipeleft"));
    // ...and the EDL carries it onto the dissolving segment.
    let seg = crate::edl::edl_from_project(&p)
        .segments
        .into_iter()
        .find(|s| s.clip_id.as_deref() == Some("c2"))
        .unwrap();
    assert_eq!(seg.xfade_kind.as_deref(), Some("wipeleft"));

    // "fade" normalizes to None (the canonical dissolve → byte-identical logs).
    let mut p = fixture();
    let e = crossfade(&mut p, "v1", 5000, 1000, Some("fade")).unwrap();
    assert_eq!(e[0].detail["transition"], "fade");
    assert_eq!(xfade_kind(&p, 1), None);

    // Clearing the crossfade (duration 0) also clears any style.
    let mut p = fixture();
    crossfade(&mut p, "v1", 5000, 1000, Some("circleopen")).unwrap();
    assert_eq!(xfade_kind(&p, 1).as_deref(), Some("circleopen"));
    crossfade(&mut p, "v1", 5000, 0, None).unwrap();
    assert_eq!(xfade_kind(&p, 1), None, "cleared crossfade drops the style");

    // The validator gates the exposed set.
    assert!(crate::types::is_valid_transition("wipeleft"));
    assert!(crate::types::is_valid_transition("fade"));
    assert!(crate::types::is_valid_transition("zoomin")); // Part of the full xfade set.
    assert!(!crate::types::is_valid_transition("bogus"));
    assert!(!crate::types::is_valid_transition("spiral")); // not an ffmpeg xfade style
}

/// SECURITY: chroma-key color is allowlisted (name or 0xRRGGBB hex) so it
/// can't carry a filtergraph-injection payload into the ffmpeg graph.
#[test]
fn chroma_color_allowlist_blocks_filtergraph_injection() {
    use crate::types::is_valid_chroma_color as ok;
    // Accept: bare color names + hex literals.
    assert!(ok("green"));
    assert!(ok("DarkSlateGray"));
    assert!(ok("0x00ff00"));
    assert!(ok("0x00FF00aa")); // RGBA
                               // Reject: empty, injection metacharacters, malformed hex, alpha suffix.
    assert!(!ok(""));
    assert!(!ok("green,movie=/etc/passwd")); // comma → new filter
    assert!(!ok("green;[a]")); // semicolon → new chain
    assert!(!ok("green[x]")); // label brackets
    assert!(!ok("black@0.5")); // alpha suffix (has '@')
    assert!(!ok("0xZZZZZZ")); // non-hex
    assert!(!ok("0x00ff0")); // wrong length
    assert!(!ok("green ")); // trailing space
}

/// set_effects REJECTS a chroma-key whose color isn't a name/hex literal —
/// the verb boundary stops a crafted color before it can be stored.
#[test]
fn set_effects_rejects_injection_color() {
    use crate::types::ClipEffect;
    let mut p = Project::new("t", ProjectSettings::default());
    // Base clip on v1, overlay clip on v2 (chroma needs a layer below).
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("base", "a1", 0, 1000)));
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(make_media_clip("ov", "a2", 0, 1000))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });

    let evil = vec![ClipEffect::ChromaKey {
        color: "green,movie=/etc/passwd".into(),
        similarity: 0.1,
        blend: 0.1,
    }];
    let err = set_effects(&mut p, "ov", evil).unwrap_err();
    assert_eq!(err.code, codes::INVALID_ARGS);
    assert!(
        err.message.contains("invalid chroma key color"),
        "{}",
        err.message
    );

    // The clean form is accepted.
    let good = vec![ClipEffect::ChromaKey {
        color: "green".into(),
        similarity: 0.1,
        blend: 0.1,
    }];
    assert!(set_effects(&mut p, "ov", good).is_ok());
}

/// edit.reverse: sets/clears the flag on a media clip, carries it onto the
/// EDL (so the renderer emits reverse/areverse), survives a split (BOTH
/// halves keep the parent's direction), and refuses a non-media clip.
#[test]
fn reverse_sets_clears_carries_and_splits() {
    let mut p = Project::new("t", ProjectSettings::default());
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("c1", "a1", 0, 2000)));

    let is_rev = |p: &Project, id: &str| {
        matches!(
            p.find_clip(id).and_then(|(t, i)| p.track(t).unwrap().clips.get(i)),
            Some(Clip::Media(c)) if c.reverse
        )
    };

    // SET: flag on, carried onto the EDL segment.
    reverse(&mut p, "c1", true).unwrap();
    assert!(is_rev(&p, "c1"), "reverse flag should be set");
    let edl = crate::edl::edl_from_project(&p);
    assert!(
        edl.segments
            .iter()
            .any(|s| s.clip_id.as_deref() == Some("c1") && s.reverse),
        "reverse must carry onto the EDL"
    );

    // SPLIT at the midpoint: both halves keep reverse = true.
    split(&mut p, "v1", 1000).unwrap();
    let rev_count = p
        .track("v1")
        .unwrap()
        .clips
        .iter()
        .filter(|c| matches!(c, Clip::Media(m) if m.reverse))
        .count();
    assert_eq!(rev_count, 2, "both split halves keep the parent's reverse");

    // CLEAR on the left half: identity restored (no reverse on that clip).
    reverse(&mut p, "c1", false).unwrap();
    assert!(!is_rev(&p, "c1"), "reverse flag should clear");

    // A caption clip has nothing to reverse → INVALID_ARGS.
    p.tracks.push(Track {
        id: "cap1".into(),
        kind: TrackKind::Caption,
        clips: vec![Clip::Caption(crate::types::CaptionClip {
            id: "cc1".into(),
            text: "hi".into(),
            range_ms: [0, 1000],
            style_ref: None,
        })],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let err = reverse(&mut p, "cc1", true).unwrap_err();
    assert_eq!(err.code, codes::INVALID_ARGS);
}

/// edit.stabilize: stores Some(smoothing) CLAMPED, carries onto the EDL, clears
/// on enabled:false, and refuses a non-video clip.
#[test]
fn stabilize_sets_clamps_carries_and_clears() {
    let mut p = Project::new("t", ProjectSettings::default());
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("c1", "a1", 0, 2000)));
    let st = |p: &Project, id: &str| match p
        .find_clip(id)
        .and_then(|(t, i)| p.track(t).unwrap().clips.get(i))
    {
        Some(Clip::Media(c)) => c.stabilize.clone(),
        _ => None,
    };

    // SET with an over-range smoothing → clamped to 100; carries onto the EDL.
    stabilize(&mut p, "c1", 500.0, true).unwrap();
    assert_eq!(
        st(&p, "c1").unwrap().smoothing,
        100.0,
        "smoothing clamps to 100"
    );
    let edl = crate::edl::edl_from_project(&p);
    assert!(
        edl.segments
            .iter()
            .any(|s| s.clip_id.as_deref() == Some("c1") && s.stabilize.is_some()),
        "stabilize must carry onto the EDL"
    );

    // CLEAR.
    stabilize(&mut p, "c1", 15.0, false).unwrap();
    assert!(st(&p, "c1").is_none(), "stabilize clears on enabled:false");

    // An audio-track clip has no frame → INVALID_ARGS.
    p.track_mut("a1t")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("ac", "a2", 0, 2000)));
    assert_eq!(
        stabilize(&mut p, "ac", 15.0, true).unwrap_err().code,
        codes::INVALID_ARGS
    );
}

/// edit.blend: sets an overlay video track's blend mode, "normal" clears it,
/// rejects an unknown mode + a non-video track.
#[test]
fn set_track_blend_sets_validates_clears() {
    let mut p = Project::new("t", ProjectSettings::default());
    // add an overlay video track.
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let bm = |p: &Project, id: &str| {
        p.tracks
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .blend_mode
            .clone()
    };

    // set multiply.
    set_track_blend(&mut p, "v2", "multiply").unwrap();
    assert_eq!(bm(&p, "v2").as_deref(), Some("multiply"));
    // "normal" CLEARS it.
    set_track_blend(&mut p, "v2", "normal").unwrap();
    assert!(bm(&p, "v2").is_none(), "'normal' clears the blend");
    // unknown mode → INVALID_ARGS.
    assert_eq!(
        set_track_blend(&mut p, "v2", "glow").unwrap_err().code,
        codes::INVALID_ARGS
    );
    // a non-video track → INVALID_ARGS.
    assert_eq!(
        set_track_blend(&mut p, "a1t", "screen").unwrap_err().code,
        codes::INVALID_ARGS
    );
    // an unknown track → NOT_FOUND.
    assert_eq!(
        set_track_blend(&mut p, "nope", "screen").unwrap_err().code,
        codes::NOT_FOUND
    );
}

/// Non-destructive mute/solo regression (edit.mute / edit.solo) plus the audibility rule
/// (Project::audio_track_audible). Proves: (1) the flag toggles and the track's
/// GAIN is never touched (the data-loss falsifier); (2) muted ⇒ silent, default ⇒
/// audible, solo isolates, mute wins over solo; (3) non-audio tracks are refused
/// and legacy video flags cannot affect the audio mix.
#[test]
fn mute_solo_flags_are_non_destructive_and_drive_audibility() {
    let mut p = Project::new("t", ProjectSettings::default());
    // a2t = a second audio track; dial a non-zero level on it to prove gain survives.
    p.tracks.push(Track {
        id: "a2t".into(),
        kind: TrackKind::Audio,
        clips: vec![],
        gain_db: -6.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let track = |p: &Project, id: &str| p.tracks.iter().find(|t| t.id == id).unwrap().clone();

    // DEFAULT: nothing muted/soloed → every audio track is audible.
    assert!(p.audio_track_audible(&track(&p, "a1t")));
    assert!(p.audio_track_audible(&track(&p, "a2t")));
    assert!(!p.audio_track_audible(&track(&p, "v1")));

    // Historic builds could persist a video solo even though video tracks never
    // enter the audio graph. It must not silence real audio after upgrade.
    p.track_mut("v1").unwrap().solo = true;
    assert!(p.audio_track_audible(&track(&p, "a1t")));
    assert!(p.audio_track_audible(&track(&p, "a2t")));

    // MUTE a2t: flag set, GAIN UNTOUCHED (still -6 dB), a2t no longer audible.
    let muted_fx = set_track_muted(&mut p, "a2t", true).unwrap();
    // REGRESSION (op-log replay): the effect MUST round-trip through JSON. A
    // `track` key inside the detail map would collide with OpEffect.track under
    // serde(flatten) and fail to deserialize → ops.jsonl replay (project.open)
    // would break. This guards the exact bug the headless reload test caught.
    let js = serde_json::to_string(&muted_fx).unwrap();
    let back: Vec<OpEffect> = serde_json::from_str(&js).unwrap();
    assert_eq!(back[0].track.as_deref(), Some("a2t"));
    assert_eq!(
        back[0].detail.get("muted").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert!(track(&p, "a2t").muted);
    assert_eq!(track(&p, "a2t").gain_db, -6.0, "mute must not touch gain");
    assert!(!p.audio_track_audible(&track(&p, "a2t")));
    assert!(p.audio_track_audible(&track(&p, "a1t")), "a1t still plays");
    // UN-MUTE: flag cleared, gain STILL -6 dB (the dialed level survived).
    set_track_muted(&mut p, "a2t", false).unwrap();
    assert!(!track(&p, "a2t").muted);
    assert_eq!(track(&p, "a2t").gain_db, -6.0);

    // SOLO a1t: only a1t audible; a2t silenced WITHOUT touching its gain.
    set_track_solo(&mut p, "a1t", true).unwrap();
    assert!(p.audio_track_audible(&track(&p, "a1t")));
    assert!(
        !p.audio_track_audible(&track(&p, "a2t")),
        "solo isolates a1t"
    );
    assert_eq!(track(&p, "a2t").gain_db, -6.0, "solo must not touch gain");

    // MUTE wins over SOLO: solo a1t AND mute it → a1t is silent.
    set_track_muted(&mut p, "a1t", true).unwrap();
    assert!(
        !p.audio_track_audible(&track(&p, "a1t")),
        "explicit mute beats solo"
    );

    // VIDEO and CAPTION tracks have no audio contribution → refused.
    assert_eq!(
        set_track_muted(&mut p, "v1", true).unwrap_err().code,
        codes::INVALID_ARGS
    );
    assert_eq!(
        set_track_solo(&mut p, "v1", true).unwrap_err().code,
        codes::INVALID_ARGS
    );
    p.tracks.push(Track {
        id: "cap1".into(),
        kind: TrackKind::Caption,
        clips: vec![],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    assert_eq!(
        set_track_muted(&mut p, "cap1", true).unwrap_err().code,
        codes::INVALID_ARGS
    );
    assert_eq!(
        set_track_solo(&mut p, "cap1", true).unwrap_err().code,
        codes::INVALID_ARGS
    );
    // unknown track → NOT_FOUND.
    assert_eq!(
        set_track_muted(&mut p, "nope", true).unwrap_err().code,
        codes::NOT_FOUND
    );
}

/// edit.pan: flag set + gain untouched, effect
/// round-trips the serde-flatten boundary (the replay falsifier), range /
/// NaN / caption / not-found refusals.
#[test]
fn pan_flag_is_non_destructive_and_validated() {
    let mut p = Project::new("t", ProjectSettings::default());
    p.tracks.push(Track {
        id: "a2t".into(),
        kind: TrackKind::Audio,
        clips: vec![],
        gain_db: -6.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let track = |p: &Project, id: &str| p.tracks.iter().find(|t| t.id == id).unwrap().clone();

    let fx = set_track_pan(&mut p, "a2t", 0.5).unwrap();
    // serde-flatten replay falsifier (same class as the mute/solo bug):
    // the track id must ride OpEffect.track, never a detail `track` key.
    let js = serde_json::to_string(&fx).unwrap();
    let back: Vec<OpEffect> = serde_json::from_str(&js).unwrap();
    assert_eq!(back[0].track.as_deref(), Some("a2t"));
    assert_eq!(
        back[0].detail.get("pan").and_then(|v| v.as_f64()),
        Some(0.5)
    );
    assert_eq!(
        back[0].detail.get("old_pan").and_then(|v| v.as_f64()),
        Some(0.0)
    );
    assert_eq!(track(&p, "a2t").pan, 0.5);
    assert_eq!(track(&p, "a2t").gain_db, -6.0, "pan must not touch gain");
    // back to center
    set_track_pan(&mut p, "a2t", 0.0).unwrap();
    assert_eq!(track(&p, "a2t").pan, 0.0);
    // refusals: out-of-range, NaN, non-audio track, unknown track
    assert_eq!(
        set_track_pan(&mut p, "a2t", 1.5).unwrap_err().code,
        codes::INVALID_ARGS
    );
    assert_eq!(
        set_track_pan(&mut p, "a2t", f64::NAN).unwrap_err().code,
        codes::INVALID_ARGS
    );
    assert_eq!(
        set_track_pan(&mut p, "v1", 0.3).unwrap_err().code,
        codes::INVALID_ARGS
    );
    p.tracks.push(Track {
        id: "cap2".into(),
        kind: TrackKind::Caption,
        clips: vec![],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    assert_eq!(
        set_track_pan(&mut p, "cap2", 0.3).unwrap_err().code,
        codes::INVALID_ARGS
    );
    assert_eq!(
        set_track_pan(&mut p, "nope", 0.3).unwrap_err().code,
        codes::NOT_FOUND
    );
}

/// edit.mute_range (mute_range): SOURCE-time non-destructive mute list —
/// add + normalize (merge), surgical remove (split), clear, and the
/// refusal set (non-audio track, no window intersection, ramped clip,
/// empty range, mode exclusivity).
#[test]
fn mute_range_add_remove_clear_and_validates() {
    let mut p = Project::new("t", ProjectSettings::default());
    // audio clip a1t/c_a: src window [1000, 9000)
    p.track_mut("a1t")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("c_a", "a1", 1000, 9000)));
    // video clip v1/c_v (same asset — mute must refuse the video one)
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("c_v", "a1", 1000, 9000)));
    let ranges = |p: &Project| match p.find_clip("c_a").map(|(t, i)| (t.to_string(), i)) {
        Some((t, i)) => match &p.tracks.iter().find(|x| x.id == t).unwrap().clips[i] {
            Clip::Media(c) => c.mute_ranges.clone(),
            _ => panic!(),
        },
        None => panic!(),
    };

    // add two, overlapping → merged; adjacent re-add is idempotent
    mute_range(&mut p, "c_a", Some([2000, 3000]), None, false).unwrap();
    mute_range(&mut p, "c_a", Some([2500, 4000]), None, false).unwrap();
    assert_eq!(ranges(&p), vec![[2000, 4000]]);
    mute_range(&mut p, "c_a", Some([6000, 7000]), None, false).unwrap();
    assert_eq!(ranges(&p), vec![[2000, 4000], [6000, 7000]]);

    // surgical remove SPLITS a strict superset and leaves others untouched
    let fx = mute_range(&mut p, "c_a", None, Some([2500, 3000]), false).unwrap();
    assert_eq!(ranges(&p), vec![[2000, 2500], [3000, 4000], [6000, 7000]]);
    assert_eq!(
        fx[0].detail.get("action").and_then(|v| v.as_str()),
        Some("remove")
    );
    // effect falsifier: track id on OpEffect.track (serde-flatten rule)
    let js = serde_json::to_string(&fx).unwrap();
    let back: Vec<OpEffect> = serde_json::from_str(&js).unwrap();
    assert_eq!(back[0].track.as_deref(), Some("a1t"));

    // clear wipes all
    mute_range(&mut p, "c_a", None, None, true).unwrap();
    assert!(ranges(&p).is_empty());

    // refusals
    assert_eq!(
        mute_range(&mut p, "c_v", Some([2000, 3000]), None, false)
            .unwrap_err()
            .code,
        codes::INVALID_ARGS,
        "video-track clip must refuse (inaudible mute would be a lie)"
    );
    assert_eq!(
        mute_range(&mut p, "c_a", Some([9500, 9900]), None, false)
            .unwrap_err()
            .code,
        codes::INVALID_ARGS,
        "outside the visible source window"
    );
    assert_eq!(
        mute_range(&mut p, "c_a", Some([3000, 3000]), None, false)
            .unwrap_err()
            .code,
        codes::INVALID_ARGS,
        "empty range"
    );
    assert_eq!(
        mute_range(&mut p, "c_a", Some([2000, 3000]), None, true)
            .unwrap_err()
            .code,
        codes::INVALID_ARGS,
        "mode exclusivity"
    );
    assert_eq!(
        mute_range(&mut p, "nope", Some([2000, 3000]), None, false)
            .unwrap_err()
            .code,
        codes::NOT_FOUND
    );
}

/// edit.adjustment (add_adjustment): the PURE layer logic — deterministic id
/// allocation, validation (empty/inverted range, no grade/effect, audio + chroma
/// effects refused), and that an identity grade alone is rejected (nothing to render).
#[test]
fn adjustment_allocates_ids_and_validates() {
    use crate::types::{ClipEffect, ClipGrade};
    let mut p = Project::new("t", ProjectSettings::default());
    let desat = ClipGrade {
        contrast: 1.0,
        brightness: 0.0,
        saturation: 0.0,
        gamma: 1.0,
        temperature_k: None,
        lut: None,
    };

    // First layer → adj1, stored verbatim with its grade.
    let eff = add_adjustment(&mut p, [2000, 4000], Some(desat.clone()), vec![]).unwrap();
    assert_eq!(eff[0].detail["adjustment_id"], json!("adj1"));
    assert_eq!(p.adjustments.len(), 1);
    assert_eq!(p.adjustments[0].id, "adj1");
    assert_eq!(p.adjustments[0].range_ms, [2000, 4000]);
    assert_eq!(p.adjustments[0].grade.as_ref().unwrap().saturation, 0.0);

    // Second layer → adj2 (deterministic max-index + 1).
    let eff2 = add_adjustment(
        &mut p,
        [0, 1000],
        None,
        vec![ClipEffect::Vignette { amount: 0.8 }],
    )
    .unwrap();
    assert_eq!(eff2[0].detail["adjustment_id"], json!("adj2"));
    assert_eq!(p.adjustments.len(), 2);

    // Inverted / empty range → INVALID_ARGS.
    assert_eq!(
        add_adjustment(&mut p, [4000, 2000], Some(desat.clone()), vec![])
            .unwrap_err()
            .code,
        codes::INVALID_ARGS
    );
    assert_eq!(
        add_adjustment(&mut p, [3000, 3000], Some(desat.clone()), vec![])
            .unwrap_err()
            .code,
        codes::INVALID_ARGS
    );
    // Neither grade nor effect (identity grade counts as absent) → INVALID_ARGS.
    let identity = ClipGrade {
        contrast: 1.0,
        brightness: 0.0,
        saturation: 1.0,
        gamma: 1.0,
        temperature_k: None,
        lut: None,
    };
    assert_eq!(
        add_adjustment(&mut p, [0, 1000], Some(identity), vec![])
            .unwrap_err()
            .code,
        codes::INVALID_ARGS
    );
    assert_eq!(
        add_adjustment(&mut p, [0, 1000], None, vec![])
            .unwrap_err()
            .code,
        codes::INVALID_ARGS
    );
    // Audio effect refused (an adjustment grades video, no audio chain).
    assert_eq!(
        add_adjustment(
            &mut p,
            [0, 1000],
            None,
            vec![ClipEffect::Denoise { amount: 0.5 }]
        )
        .unwrap_err()
        .code,
        codes::INVALID_ARGS
    );
    // chroma_key refused (no single layer below an adjustment to key).
    assert_eq!(
        add_adjustment(
            &mut p,
            [0, 1000],
            None,
            vec![ClipEffect::ChromaKey {
                color: "green".into(),
                similarity: 0.1,
                blend: 0.0
            }]
        )
        .unwrap_err()
        .code,
        codes::INVALID_ARGS
    );
    // No rejected call mutated the layer list (still just adj1, adj2).
    assert_eq!(p.adjustments.len(), 2);
}

/// edit.freeze: stores Some(at_ms) CLAMPED into the clip's source span,
/// carries onto the EDL, clears on enabled:false, and refuses a non-video clip.
#[test]
fn freeze_sets_clamps_carries_and_clears() {
    let mut p = Project::new("t", ProjectSettings::default());
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("c1", "a1", 0, 2000)));

    let fz = |p: &Project, id: &str| match p
        .find_clip(id)
        .and_then(|(t, i)| p.track(t).unwrap().clips.get(i))
    {
        Some(Clip::Media(c)) => c.freeze.clone(),
        _ => None,
    };

    // at_ms beyond the 2000ms span is CLAMPED to span-1 (1999).
    freeze(&mut p, "c1", 9999, true).unwrap();
    assert_eq!(fz(&p, "c1"), Some(crate::types::ClipFreeze { at_ms: 1999 }));

    // carries onto the EDL segment.
    let edl = crate::edl::edl_from_project(&p);
    assert!(
        edl.segments
            .iter()
            .any(|s| s.clip_id.as_deref() == Some("c1") && s.freeze.is_some()),
        "freeze must carry onto the EDL"
    );

    // an in-range at_ms is stored verbatim.
    freeze(&mut p, "c1", 500, true).unwrap();
    assert_eq!(fz(&p, "c1"), Some(crate::types::ClipFreeze { at_ms: 500 }));

    // enabled:false clears it.
    freeze(&mut p, "c1", 0, false).unwrap();
    assert_eq!(fz(&p, "c1"), None);

    // an audio-track clip has no frame to hold → INVALID_ARGS.
    p.tracks.push(Track {
        id: "a1t".into(),
        kind: TrackKind::Audio,
        clips: vec![Clip::Media(make_media_clip("ac1", "a2", 0, 1000))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let err = freeze(&mut p, "ac1", 0, true).unwrap_err();
    assert_eq!(err.code, codes::INVALID_ARGS);
}

/// edit.animate: stores a real pan/zoom, carries onto the EDL, clamps a sub-1
/// zoom (→ identity → cleared), clears on an identity animation, and refuses a
/// non-video clip.
#[test]
fn animate_sets_clamps_carries_and_clears() {
    use crate::types::{AnimState, ClipAnimation};
    let mut p = Project::new("t", ProjectSettings::default());
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("c1", "a1", 0, 2000)));

    let anim = |fz: f64, tz: f64| ClipAnimation {
        from: AnimState {
            zoom: fz,
            x: 0.5,
            y: 0.5,
        },
        to: AnimState {
            zoom: tz,
            x: 0.5,
            y: 0.5,
        },
    };
    let get = |p: &Project, id: &str| match p
        .find_clip(id)
        .and_then(|(t, i)| p.track(t).unwrap().clips.get(i))
    {
        Some(Clip::Media(c)) => c.animation.clone(),
        _ => None,
    };

    // A real zoom-in is stored + carried onto the EDL.
    animate(&mut p, "c1", anim(1.0, 1.3)).unwrap();
    assert!(get(&p, "c1").is_some(), "zoom-in stored");
    let edl = crate::edl::edl_from_project(&p);
    assert!(
        edl.segments
            .iter()
            .any(|s| s.clip_id.as_deref() == Some("c1") && s.animation.is_some()),
        "animation must carry onto the EDL"
    );

    // An identity animation (no zoom, centred) CLEARS it.
    animate(&mut p, "c1", anim(1.0, 1.0)).unwrap();
    assert!(get(&p, "c1").is_none(), "identity clears");

    // A sub-1 zoom clamps to 1.0 at both ends → identity → cleared.
    animate(&mut p, "c1", anim(0.5, 0.8)).unwrap();
    assert!(get(&p, "c1").is_none(), "sub-1 zoom clamps to identity");

    // An audio-track clip has no frame to pan → INVALID_ARGS.
    p.tracks.push(Track {
        id: "a1t".into(),
        kind: TrackKind::Audio,
        clips: vec![Clip::Media(make_media_clip("ac1", "a2", 0, 1000))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let err = animate(&mut p, "ac1", anim(1.0, 1.5)).unwrap_err();
    assert_eq!(err.code, codes::INVALID_ARGS);
}

/// edit.keyframe: stores per-param keyframe tracks (sorted + opacity-clamped),
/// carries onto the EDL, enforces param↔track (opacity→video, volume→audio),
/// REPLACES on re-set, and clears on empty points.
#[test]
fn keyframe_sets_validates_carries_and_clears() {
    use crate::types::{KfInterp, KfParam, KfPoint};
    let mut p = Project::new("t", ProjectSettings::default());
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("c1", "a1", 0, 2000)));
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(make_media_clip("ov", "a2", 0, 2000))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    // Use the project's DEFAULT audio track (a1t) — pushing a second "a1t"
    // would duplicate the id (find_clip then re-looks-up the empty first one).
    p.track_mut("a1t")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("ac", "a3", 0, 2000)));

    let kf = |p: &Project, id: &str, param: KfParam| -> Option<Vec<KfPoint>> {
        match p
            .find_clip(id)
            .and_then(|(t, i)| p.track(t).unwrap().clips.get(i))
        {
            Some(Clip::Media(c)) => c
                .keyframes
                .iter()
                .find(|k| k.param == param)
                .map(|k| k.points.clone()),
            _ => None,
        }
    };
    let pts = vec![
        KfPoint {
            t_ms: 2000,
            value: 0.0,
        },
        KfPoint {
            t_ms: 0,
            value: 0.0,
        },
        KfPoint {
            t_ms: 1000,
            value: 1.0,
        },
    ];

    // opacity on the overlay video clip → stored, SORTED by time.
    keyframe(
        &mut p,
        "ov",
        KfParam::Opacity,
        pts.clone(),
        KfInterp::Linear,
    )
    .unwrap();
    let got = kf(&p, "ov", KfParam::Opacity).unwrap();
    assert_eq!(got.len(), 3);
    assert!(
        got.windows(2).all(|w| w[0].t_ms <= w[1].t_ms),
        "points sorted"
    );

    // Points after the realized timeline end are refused before mutation. The
    // endpoint itself remains valid for ordinary 0-to-end animation curves.
    let err = keyframe(
        &mut p,
        "ov",
        KfParam::Opacity,
        vec![KfPoint {
            t_ms: 2001,
            value: 0.5,
        }],
        KfInterp::Linear,
    )
    .unwrap_err();
    assert_eq!(err.code, codes::INVALID_ARGS);
    assert_eq!(kf(&p, "ov", KfParam::Opacity).unwrap(), got);

    // A later retime preserves the animation by scaling clip-local keyframe
    // times to the new duration.
    let retime = speed(&mut p, "ov", 2.0).unwrap();
    assert_eq!(retime[0].detail["keyframes_rescaled"], 3);
    let Clip::Media(overlay) = &p.track("v2").unwrap().clips[0] else {
        unreachable!()
    };
    assert_eq!(overlay.speed, 2.0);
    assert_eq!(
        overlay
            .keyframes
            .iter()
            .find(|track| track.param == KfParam::Opacity)
            .unwrap()
            .points
            .iter()
            .map(|point| point.t_ms)
            .collect::<Vec<_>>(),
        vec![0, 500, 1000],
        "retime scales keyframes with the realized timeline duration",
    );

    // carries onto the EDL.
    let edl = crate::edl::edl_from_project(&p);
    assert!(
        edl.segments
            .iter()
            .any(|s| s.clip_id.as_deref() == Some("ov") && !s.keyframes.is_empty()),
        "keyframes must carry onto the EDL"
    );

    // opacity values clamp to [0,1].
    keyframe(
        &mut p,
        "ov",
        KfParam::Opacity,
        vec![KfPoint {
            t_ms: 0,
            value: 5.0,
        }],
        KfInterp::Linear,
    )
    .unwrap();
    assert_eq!(kf(&p, "ov", KfParam::Opacity).unwrap()[0].value, 1.0);

    // opacity on an AUDIO clip → rejected; volume → ok.
    assert_eq!(
        keyframe(
            &mut p,
            "ac",
            KfParam::Opacity,
            pts.clone(),
            KfInterp::Linear
        )
        .unwrap_err()
        .code,
        codes::INVALID_ARGS
    );
    keyframe(&mut p, "ac", KfParam::Volume, pts.clone(), KfInterp::Linear).unwrap();
    assert!(kf(&p, "ac", KfParam::Volume).is_some());
    // volume on a VIDEO clip → rejected.
    assert_eq!(
        keyframe(&mut p, "ov", KfParam::Volume, pts.clone(), KfInterp::Linear)
            .unwrap_err()
            .code,
        codes::INVALID_ARGS
    );

    // empty points CLEARS that param's track.
    keyframe(&mut p, "ov", KfParam::Opacity, vec![], KfInterp::Linear).unwrap();
    assert!(kf(&p, "ov", KfParam::Opacity).is_none());

    // pos_x/pos_y keyframe the overlay POSITION — allowed on a video clip, and
    // NOT clamped to [0,1] (the overlay may slide off-screen).
    keyframe(
        &mut p,
        "ov",
        KfParam::PosX,
        vec![
            KfPoint {
                t_ms: 0,
                value: -0.3,
            },
            KfPoint {
                t_ms: 1000,
                value: 1.2,
            },
        ],
        KfInterp::Linear,
    )
    .unwrap();
    let posx = kf(&p, "ov", KfParam::PosX).unwrap();
    assert_eq!(
        posx[0].value, -0.3,
        "pos_x is NOT clamped (off-screen slide-in)"
    );
    assert_eq!(
        posx[1].value, 1.2,
        "pos_x is NOT clamped (off-screen slide-out)"
    );
    // pos_y is allowed on a video clip too.
    keyframe(
        &mut p,
        "ov",
        KfParam::PosY,
        vec![KfPoint {
            t_ms: 0,
            value: 0.5,
        }],
        KfInterp::Linear,
    )
    .unwrap();
    assert!(kf(&p, "ov", KfParam::PosY).is_some());
    // pos_x on an AUDIO clip → rejected (position is a video/overlay concept).
    assert_eq!(
        keyframe(&mut p, "ac", KfParam::PosX, pts.clone(), KfInterp::Linear)
            .unwrap_err()
            .code,
        codes::INVALID_ARGS
    );
    // The verb wire form deserializes snake_case → the variant (the dispatch path
    // relies on this; opacity/volume already do, but prove pos_x/pos_y too).
    assert_eq!(
        serde_json::from_str::<KfParam>("\"pos_x\"").unwrap(),
        KfParam::PosX
    );
    assert_eq!(
        serde_json::from_str::<KfParam>("\"pos_y\"").unwrap(),
        KfParam::PosY
    );
}

/// edit.add_mask: stores onto a BASE video clip; validates point counts per shape;
/// REFUSES an overlay-track clip + an audio clip; clears with None.
#[test]
fn add_mask_validates_track_and_geometry() {
    use crate::types::{ClipMask, MaskEffect, MaskShape};
    let mut p = Project::new("t", ProjectSettings::default());
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("c1", "a1", 0, 2000)));
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(make_media_clip("ov", "a2", 0, 2000))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    p.track_mut("a1t")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("ac", "a3", 0, 2000)));
    let rect = |pts: Vec<[f64; 2]>| ClipMask {
        shape: MaskShape::Rect,
        points: pts,
        feather: 0.0,
        invert: false,
        effect: MaskEffect::Blur,
        strength: None,
        range_ms: None,
        track: None,
        regions: Vec::new(),
    };
    // Base video clip, valid rect → stored.
    add_mask(&mut p, "c1", Some(rect(vec![[0.1, 0.1], [0.5, 0.5]]))).unwrap();
    let stored = match p
        .find_clip("c1")
        .and_then(|(t, i)| p.track(t).unwrap().clips.get(i))
    {
        Some(Clip::Media(c)) => c.mask.clone(),
        _ => None,
    };
    assert!(stored.is_some(), "mask stored on the base clip");
    // Clear it.
    add_mask(&mut p, "c1", None).unwrap();
    // Rect with 1 point → rejected.
    assert!(add_mask(&mut p, "c1", Some(rect(vec![[0.1, 0.1]]))).is_err());
    // Polygon with 2 points → rejected (needs ≥3).
    let poly2 = ClipMask {
        shape: MaskShape::Polygon,
        points: vec![[0.0, 0.0], [1.0, 1.0]],
        feather: 0.0,
        invert: false,
        effect: MaskEffect::Blur,
        strength: None,
        range_ms: None,
        track: None,
        regions: Vec::new(),
    };
    assert!(add_mask(&mut p, "c1", Some(poly2)).is_err());
    // Overlay-track video clip → rejected (base-track-only v1 scope).
    assert!(add_mask(&mut p, "ov", Some(rect(vec![[0.1, 0.1], [0.5, 0.5]]))).is_err());
    // Audio clip → rejected (no frame).
    assert!(add_mask(&mut p, "ac", Some(rect(vec![[0.1, 0.1], [0.5, 0.5]]))).is_err());
    // Clearing an overlay clip's mask is allowed (None always permitted).
    add_mask(&mut p, "ov", None).unwrap();
}

/// edit.eq: stores onto an AUDIO clip, drops ~0 dB bands, carries onto the EDL,
/// REFUSES a video clip (audio chain only processes audio tracks), and an
/// identity EQ clears it.
#[test]
fn eq_sets_drops_zero_bands_carries_and_clears() {
    use crate::types::{ClipEq, EqBand};
    let mut p = Project::new("t", ProjectSettings::default());
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("vc", "a1", 0, 2000)));
    // Use the DEFAULT audio track (a1t) — see the keyframe test note.
    p.track_mut("a1t")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("ac", "a2", 0, 2000)));
    let get_eq = |p: &Project, id: &str| -> Option<ClipEq> {
        match p
            .find_clip(id)
            .and_then(|(t, i)| p.track(t).unwrap().clips.get(i))
        {
            Some(Clip::Media(c)) => c.eq.clone(),
            _ => None,
        }
    };

    // EQ on the audio clip: high-pass + one real band + one 0 dB band (dropped).
    eq(
        &mut p,
        "ac",
        ClipEq {
            high_pass_hz: Some(80.0),
            low_pass_hz: None,
            bands: vec![
                EqBand {
                    freq_hz: 3000.0,
                    gain_db: 3.0,
                    q: 1.0,
                },
                EqBand {
                    freq_hz: 200.0,
                    gain_db: 0.0,
                    q: 1.0,
                }, // no-op → dropped
            ],
        },
    )
    .unwrap();
    let stored = get_eq(&p, "ac").expect("eq stored");
    assert_eq!(stored.high_pass_hz, Some(80.0));
    assert_eq!(stored.bands.len(), 1, "0 dB band dropped");
    assert_eq!(stored.bands[0].freq_hz, 3000.0);

    // carries onto the EDL.
    let edl = crate::edl::edl_from_project(&p);
    assert!(
        edl.segments
            .iter()
            .any(|s| s.clip_id.as_deref() == Some("ac") && s.eq.is_some()),
        "eq must carry onto the EDL"
    );

    // EQ on a VIDEO clip → rejected (a video file's audio is a separate clip).
    assert_eq!(
        eq(
            &mut p,
            "vc",
            ClipEq {
                high_pass_hz: Some(80.0),
                low_pass_hz: None,
                bands: vec![]
            }
        )
        .unwrap_err()
        .code,
        codes::INVALID_ARGS
    );

    // identity EQ (nothing set) CLEARS it.
    eq(
        &mut p,
        "ac",
        ClipEq {
            high_pass_hz: None,
            low_pass_hz: None,
            bands: vec![],
        },
    )
    .unwrap();
    assert!(get_eq(&p, "ac").is_none(), "identity EQ clears");
}

/// edit.effect (set_effects): stores the list, carries it onto the EDL, and
/// REFUSES chroma key on a base-track clip (nothing under the base to reveal)
/// while allowing it on an overlay. SET semantics: [] clears.
#[test]
fn set_effects_stores_and_guards_base_chroma() {
    use crate::types::ClipEffect as E;
    let n_effects = |p: &Project, track: &str, idx: usize| match &p.track(track).unwrap().clips[idx]
    {
        Clip::Media(c) => c.effects.len(),
        _ => unreachable!("media"),
    };
    let mut p = Project::new("t", ProjectSettings::default());
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("base", "a1", 0, 2000)));
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(make_media_clip("ov", "a2", 0, 2000))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });

    // Uniform effects on the base clip: stored.
    set_effects(
        &mut p,
        "base",
        vec![E::Vignette { amount: 0.5 }, E::Grain { amount: 10.0 }],
    )
    .unwrap();
    assert_eq!(n_effects(&p, "v1", 0), 2);
    // Chroma key on the OVERLAY: allowed.
    set_effects(
        &mut p,
        "ov",
        vec![E::ChromaKey {
            color: "green".into(),
            similarity: 0.15,
            blend: 0.1,
        }],
    )
    .unwrap();
    assert_eq!(n_effects(&p, "v2", 0), 1);
    // Chroma key on the BASE clip: REFUSED, and the base list is unchanged.
    let e = set_effects(
        &mut p,
        "base",
        vec![E::ChromaKey {
            color: "green".into(),
            similarity: 0.15,
            blend: 0.1,
        }],
    )
    .unwrap_err();
    assert_eq!(e.code, codes::INVALID_ARGS);
    assert_eq!(
        n_effects(&p, "v1", 0),
        2,
        "refusal left the base effects intact"
    );

    // The EDL carries the base clip's effects through to render.
    let seg = crate::edl::edl_from_project(&p)
        .segments
        .into_iter()
        .find(|s| s.clip_id.as_deref() == Some("base"))
        .unwrap();
    assert_eq!(seg.effects.len(), 2);

    // [] clears.
    set_effects(&mut p, "ov", vec![]).unwrap();
    assert_eq!(n_effects(&p, "v2", 0), 0);

    // AUDIO effect (denoise) track validation: OK on an audio-track clip;
    // refused on a video clip; and a VISUAL effect is refused on audio.
    p.track_mut("a1t")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("aud", "a1", 0, 2000)));
    set_effects(&mut p, "aud", vec![E::Denoise { amount: 0.5 }]).unwrap();
    assert_eq!(n_effects(&p, "a1t", 0), 1);
    assert_eq!(
        set_effects(&mut p, "base", vec![E::Denoise { amount: 0.5 }])
            .unwrap_err()
            .code,
        codes::INVALID_ARGS,
        "denoise refused on a video clip"
    );
    assert_eq!(
        set_effects(&mut p, "aud", vec![E::Vignette { amount: 0.5 }])
            .unwrap_err()
            .code,
        codes::INVALID_ARGS,
        "visual effect refused on an audio clip"
    );
}

/// edit.matte: stored on an overlay clip, REFUSED (remove) on the base track,
/// allowed (replace) on the base, carried to the EDL, cleared by None, and
/// inherited by both halves of a split.
#[test]
fn matte_stores_guards_base_and_carries_to_edl() {
    use crate::types::{ClipMatte, MatteMode, MatteModel, MatteQuality};
    let matte_of = |p: &Project, track: &str, idx: usize| match &p.track(track).unwrap().clips[idx]
    {
        Clip::Media(c) => c.matte.clone(),
        _ => unreachable!("media"),
    };
    let remove = || ClipMatte {
        mode: MatteMode::Remove,
        model: MatteModel::Rvm,
        bg: None,
        quality: MatteQuality::Good,
        seed: None,
    };
    let mut p = Project::new("t", ProjectSettings::default());
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("base", "a1", 0, 2000)));
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(make_media_clip("ov", "a2", 0, 2000))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });

    // remove on the OVERLAY: stored.
    matte(&mut p, "ov", Some(remove())).unwrap();
    assert_eq!(matte_of(&p, "v2", 0), Some(remove()));
    // remove on the BASE clip: REFUSED (nothing under the canvas), base intact.
    let e = matte(&mut p, "base", Some(remove())).unwrap_err();
    assert_eq!(e.code, codes::INVALID_ARGS);
    assert_eq!(
        matte_of(&p, "v1", 0),
        None,
        "refusal left the base un-matted"
    );
    // replace on the BASE clip: ALLOWED (fills its own background).
    matte(
        &mut p,
        "base",
        Some(ClipMatte {
            mode: MatteMode::Replace,
            model: MatteModel::Rvm,
            bg: Some(crate::types::MatteBg::Color {
                color: "black".into(),
            }),
            quality: MatteQuality::Good,
            seed: None,
        }),
    )
    .unwrap();
    assert!(matches!(
        matte_of(&p, "v1", 0).unwrap().mode,
        MatteMode::Replace
    ));
    // matte on an AUDIO clip: refused (no pixels).
    p.track_mut("a1t")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("aud", "a1", 0, 2000)));
    assert_eq!(
        matte(&mut p, "aud", Some(remove())).unwrap_err().code,
        codes::INVALID_ARGS,
        "matte refused on an audio clip"
    );

    // The EDL carries the overlay clip's matte through to render.
    let seg = crate::edl::edl_from_project(&p)
        .segments
        .into_iter()
        .find(|s| s.clip_id.as_deref() == Some("ov"))
        .unwrap();
    assert_eq!(seg.matte, Some(remove()));

    // A split inherits the matte on BOTH halves (same source pixels → same alpha).
    split(&mut p, "v2", 1000).unwrap();
    assert_eq!(
        matte_of(&p, "v2", 0),
        Some(remove()),
        "left half keeps matte"
    );
    assert_eq!(
        matte_of(&p, "v2", 1),
        Some(remove()),
        "right half keeps matte"
    );

    // None clears it.
    matte(&mut p, "ov", None).unwrap();
    assert_eq!(matte_of(&p, "v2", 0), None);
}

/// A crossfade shortens its track by the overlap, so edit.duck windows
/// AFTER the seam must shift back by the same amount (windows before stay).
#[test]
fn crossfade_remaps_duck_windows() {
    let mut p = fixture(); // a1t: single c3[0..8000)
    split(&mut p, "a1t", 5000).unwrap(); // seam at 5000 on a1t
    p.track_mut("a1t").unwrap().gain_windows = vec![
        GainWindow {
            range_ms: [1000, 2000],
            db: -12.0,
            attack_ms: 250,
        }, // before seam
        GainWindow {
            range_ms: [6000, 7000],
            db: -12.0,
            attack_ms: 250,
        }, // after seam
    ];
    let e = crossfade(&mut p, "a1t", 5000, 1000, None).unwrap();
    assert_eq!(
        e[0].detail["duck_windows_remapped"], 1,
        "one window shifted"
    );
    let w = &p.track("a1t").unwrap().gain_windows;
    assert_eq!(
        w[0].range_ms,
        [1000, 2000],
        "window before the seam unchanged"
    );
    assert_eq!(
        w[1].range_ms,
        [5000, 6000],
        "window after the seam shifted back by 1000ms"
    );
}

/// A crossfade can only dissolve two MEDIA clips — a gap on either side is
/// refused (nothing to dissolve from/into).
#[test]
fn crossfade_refuses_gap_neighbour() {
    let mut p = Project::new("t", ProjectSettings::default());
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Gap(GapClip::new(2000)));
    p.track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("c1", "a1", 0, 3000)));
    // Cut at 2000: gap (left) → media (right). Refused.
    assert_eq!(
        crossfade(&mut p, "v1", 2000, 500, None).unwrap_err().code,
        "invalid_args"
    );
}

/// Agent-facing coordinate-space regression: one crossfade SHORTENS the
/// realized timeline, so the NEXT
/// seam RENDERS at a smaller position than the EDITORIAL at_ms this verb
/// requires. The not_found error must be self-correcting — map the
/// render-space position the caller passed back to the editorial cut.
#[test]
fn crossfade_not_found_maps_render_pos_to_editorial() {
    // v1: three 2000ms media clips → editorial cuts at 2000 and 4000.
    let mut p = Project::new("t", ProjectSettings::default());
    for (id, a) in [("c1", "a1"), ("c2", "a2"), ("c3", "a3")] {
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(make_media_clip(id, a, 0, 2000)));
    }
    // Crossfade the first cut → realized timeline shortens to 4000; the
    // SECOND seam now RENDERS at 3000 though it stays editorial-4000.
    crossfade(&mut p, "v1", 2000, 1000, None).unwrap();
    // Agent read "3000" off render.frame/preview and tries to crossfade there:
    let err = crossfade(&mut p, "v1", 3000, 1000, None).unwrap_err();
    assert_eq!(err.code, "not_found");
    let action = err.suggested_action.as_deref().unwrap_or("");
    assert!(
        action.contains("editorial 4000") && action.contains("at_ms=4000"),
        "maps render-pos 3000 back to the editorial cut at 4000: {action}"
    );
    // And 4000 actually applies — proving the suggestion is correct.
    assert!(crossfade(&mut p, "v1", 4000, 1000, None).is_ok());
}

/// edit.crossfade replay carries xfade through the EDL even without the key
/// on older clips (serde-skip-default → 0, hard cut, byte-identical).
#[test]
fn crossfade_absent_replays_as_hard_cut() {
    // A project with NO crossfades produces an EDL whose duration is the
    // nominal sum (legacy behavior) — proven by the existing fixtures
    // but asserted here against the realized-duration path.
    let p = fixture();
    let edl = crate::edl::edl_from_project(&p);
    assert_eq!(edl.duration_ms, 8000, "no crossfade → nominal duration");
    assert!(edl.segments.iter().all(|s| s.xfade_in_ms == 0));
}

/// edit.ripple_delete{ripple:false} = LIFT: leaves a gap of equal length,
/// nothing downstream moves, captions/markers/duck-windows stay put.
#[test]
fn lift_delete_leaves_gap_and_moves_nothing() {
    let mut p = fixture(); // v1 c1[0..5000)+c2; a1t c3[0..8000); m1@1800 m2@7000
    p.track_mut("a1t").unwrap().gain_windows = vec![GainWindow {
        range_ms: [1000, 2000],
        db: -18.0,
        attack_ms: 250,
    }];
    // Lift [1000,2000) on v1 only.
    let e = ripple_delete(&mut p, Some("v1"), [1000, 2000], false).unwrap();
    assert_eq!(e[0].detail["ripple"], json!(false));
    // v1 duration UNCHANGED (the hole stays open as a gap).
    assert_eq!(
        p.track("v1").unwrap().duration_ms(),
        8000,
        "lift keeps the length"
    );
    // c1 split: [0,1000) media, [1000,2000) gap, [2000,5000) media, c2.
    let v1 = p.track("v1").unwrap();
    match &v1.clips[..3] {
        [Clip::Media(l), Clip::Gap(g), Clip::Media(r)] => {
            assert_eq!((l.src_in_ms, l.src_out_ms), (0, 1000));
            assert_eq!(g.duration_ms, 1000);
            assert_eq!((r.src_in_ms, r.src_out_ms), (2000, 5000));
        }
        other => unreachable!("expected [media, gap, media, ...], got {other:?}"),
    }
    // Markers + audio + windows untouched (lift moves nothing).
    assert_eq!(p.markers[0].at_ms, 1800);
    assert_eq!(p.markers[1].at_ms, 7000);
    assert_eq!(p.track("a1t").unwrap().duration_ms(), 8000);
    assert_eq!(
        p.track("a1t").unwrap().gain_windows[0].range_ms,
        [1000, 2000]
    );
    // Ripple (default true) still closes the gap (regression guard).
    let mut p2 = fixture();
    ripple_delete(&mut p2, Some("v1"), [1000, 2000], true).unwrap();
    assert_eq!(p2.track("v1").unwrap().duration_ms(), 7000);
}

/// edit.move{ripple:true}: the destination splice opens an AV-sync
/// gap in sibling tracks; ripple:false (default) leaves them.
#[test]
fn move_ripple_shifts_siblings() {
    let mut p = fixture();
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    // Float move (ripple:false): a1t untouched.
    let mut pf = p.clone();
    move_clip(&mut pf, "c2", "v2", 1000, false).unwrap();
    assert_eq!(
        pf.track("a1t").unwrap().duration_ms(),
        8000,
        "float move: no sibling ripple"
    );
    // AV-sync move (ripple:true): a1t gets a gap at the dest point.
    move_clip(&mut p, "c2", "v2", 1000, true).unwrap();
    // c2 is 3000ms; a1t (8000ms, content after 1000) gains a 3000ms gap.
    assert_eq!(
        p.track("a1t").unwrap().duration_ms(),
        11000,
        "ripple move shifts siblings"
    );
    assert_eq!(
        p.markers[1].at_ms, 10000,
        "marker after the point shifted by 3000"
    );
}

#[test]
fn linked_move_ripple_excludes_both_populated_destination_tracks() {
    let mut p = fixture();
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(make_media_clip("c4", "a2", 0, 8000))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let v1_before = p.track("v1").unwrap().duration_ms();
    let a1_before = p.track("a1t").unwrap().duration_ms();
    let mut effects = Vec::new();

    ripple_open_gap_at_excluding(&mut p, &["v1", "a1t"], 1000, 500, &mut effects);

    assert_eq!(p.track("v1").unwrap().duration_ms(), v1_before);
    assert_eq!(p.track("a1t").unwrap().duration_ms(), a1_before);
    assert_eq!(p.track("v2").unwrap().duration_ms(), 8500);
    assert_eq!(p.markers[0].at_ms, 2300);
    assert!(effects.iter().any(|effect| {
        effect.track.as_deref() == Some("v2")
            && effect.detail["rippled_gap_ms"] == json!([1000, 1500])
    }));
}

/// edit.move_marker: one op, id preserved, records old/new.
#[test]
fn move_marker_preserves_id() {
    let mut p = fixture();
    let e = marker_move(&mut p, "m1", 2500).unwrap();
    assert_eq!(e[0].detail["marker_id"], "m1");
    assert_eq!(e[0].detail["old_at_ms"], 1800);
    assert_eq!(e[0].detail["at_ms"], 2500);
    assert_eq!(p.markers.iter().find(|m| m.id == "m1").unwrap().at_ms, 2500);
    assert_eq!(
        marker_move(&mut p, "nope", 0).unwrap_err().code,
        "not_found"
    );
}

/// edit.update_marker: relabel + recolor in one op, id
/// and position preserved; validation + system-marker refusal.
#[test]
fn update_marker_label_and_color() {
    let mut p = fixture();
    let e = marker_update(&mut p, "m1", Some("intro"), Some("teal"), None).unwrap();
    assert_eq!(e[0].detail["old_label"], "x");
    assert_eq!(e[0].detail["label"], "intro");
    assert_eq!(e[0].detail["old_color"], serde_json::Value::Null);
    assert_eq!(e[0].detail["color"], "teal");
    let m = p.markers.iter().find(|m| m.id == "m1").unwrap();
    assert_eq!(
        (m.label.as_str(), m.color.as_deref(), m.at_ms),
        ("intro", Some("teal"), 1800)
    );
    // color-only update keeps the label; "none" clears back to default
    marker_update(&mut p, "m1", None, Some("none"), None).unwrap();
    let m = p.markers.iter().find(|m| m.id == "m1").unwrap();
    assert_eq!((m.label.as_str(), m.color.as_deref()), ("intro", None));
    // validation: no-op args, empty label, unknown color, missing marker
    assert_eq!(
        marker_update(&mut p, "m1", None, None, None)
            .unwrap_err()
            .code,
        "invalid_args"
    );
    assert_eq!(
        marker_update(&mut p, "m1", Some("  "), None, None)
            .unwrap_err()
            .code,
        "invalid_args"
    );
    assert_eq!(
        marker_update(&mut p, "m1", None, Some("mauve"), None)
            .unwrap_err()
            .code,
        "invalid_args"
    );
    assert_eq!(
        marker_update(&mut p, "nope", Some("a"), None, None)
            .unwrap_err()
            .code,
        "not_found"
    );
    // system markers are machine-managed — refuse
    p.markers.push(Marker {
        id: "mb".into(),
        at_ms: 9000,
        label: "beat".into(),
        note: None,
        color: None,
    });
    assert_eq!(
        marker_update(&mut p, "mb", Some("boop"), None, None)
            .unwrap_err()
            .code,
        "invalid_args"
    );
}

/// Marker notes are first-class user review metadata: edit.update_marker can
/// set, replace, and clear them without changing id, label, color, or position.
#[test]
fn update_marker_note_preserves_marker_identity() {
    let mut p = fixture();
    let e = marker_update(&mut p, "m1", None, None, Some("tighten this beat")).unwrap();
    assert_eq!(e[0].detail["old_note"], serde_json::Value::Null);
    assert_eq!(e[0].detail["note"], "tighten this beat");
    let m = p.markers.iter().find(|m| m.id == "m1").unwrap();
    assert_eq!(
        (
            m.id.as_str(),
            m.label.as_str(),
            m.color.as_deref(),
            m.note.as_deref(),
            m.at_ms
        ),
        ("m1", "x", None, Some("tighten this beat"), 1800)
    );

    let e = marker_update(&mut p, "m1", None, None, Some("  ")).unwrap();
    assert_eq!(e[0].detail["old_note"], "tighten this beat");
    assert_eq!(e[0].detail["note"], serde_json::Value::Null);
    let m = p.markers.iter().find(|m| m.id == "m1").unwrap();
    assert_eq!(m.note, None);
}

/// captions.set_range: sets a caption clip's absolute range,
/// validates non-empty, refuses non-caption clips.
#[test]
fn caption_set_range_moves_caption() {
    let mut p = fixture(); // cap1 has s1[0,1000] s2[1500,2500] s3[4000,6000]
    let e = caption_set_range(&mut p, "s1", [500, 1800]).unwrap();
    assert_eq!(e[0].detail["old_range_ms"], json!([0, 1000]));
    assert_eq!(e[0].detail["range_ms"], json!([500, 1800]));
    match &p.track("cap1").unwrap().clips[0] {
        Clip::Caption(c) => assert_eq!(c.range_ms, [500, 1800]),
        _ => unreachable!(),
    }
    // Empty range refused; media clip refused; unknown clip not_found.
    assert_eq!(
        caption_set_range(&mut p, "s2", [2000, 2000])
            .unwrap_err()
            .code,
        "invalid_args"
    );
    assert_eq!(
        caption_set_range(&mut p, "c1", [0, 100]).unwrap_err().code,
        "invalid_args"
    );
    assert_eq!(
        caption_set_range(&mut p, "nope", [0, 100])
            .unwrap_err()
            .code,
        "not_found"
    );
}

#[test]
fn speed_rejects_non_positive_and_non_finite_factor() {
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let mut p = fixture();
        let err = speed(&mut p, "c1", bad).expect_err("bad speed must be rejected");
        assert_eq!(err.code, "invalid_args", "factor {bad:?}");
        match &p.track("v1").unwrap().clips[0] {
            Clip::Media(c) => assert_eq!(c.speed, 1.0, "bad factor must not persist"),
            _ => unreachable!(),
        }
    }
}

/// Build a two-point ramp.
fn ramp_pts() -> Vec<crate::types::SpeedRampPoint> {
    vec![
        crate::types::SpeedRampPoint {
            at_ms: 0,
            factor: 1.0,
        },
        crate::types::SpeedRampPoint {
            at_ms: 2500,
            factor: 3.0,
        },
        crate::types::SpeedRampPoint {
            at_ms: 5000,
            factor: 1.0,
        },
    ]
}

/// edit.speed_ramp sets the field + shortens the clip; SET on an incompatible
/// clip is refused; once ramped, the remappers (split/trim/speed/reverse) refuse
/// the clip; clearing (empty points) restores constant speed.
#[test]
fn speed_ramp_sets_guards_and_clears() {
    let mut p = fixture();
    // SET on the plain c1[0,5000): the clip shortens (fast middle).
    speed_ramp(&mut p, "c1", ramp_pts(), 24).expect("set ramp");
    let c1 = match &p.track("v1").unwrap().clips[0] {
        Clip::Media(c) => c.clone(),
        _ => unreachable!(),
    };
    assert!(c1.has_speed_ramp() && c1.is_retimed());
    assert!(Clip::Media(c1).timeline_duration_ms() < 5000);

    // The remappers refuse the ramped clip.
    assert_eq!(speed(&mut p, "c1", 2.0).unwrap_err().code, "invalid_args");
    assert_eq!(
        reverse(&mut p, "c1", true).unwrap_err().code,
        "invalid_args"
    );
    assert_eq!(
        trim(&mut p, "c1", Some(100), None).unwrap_err().code,
        "invalid_args"
    );
    // Split anywhere inside the ramped c1 (timeline [0,~) — pick 800ms) is refused.
    assert_eq!(split(&mut p, "v1", 800).unwrap_err().code, "invalid_args");

    // CLEAR restores constant speed (and is idempotent / always allowed).
    speed_ramp(&mut p, "c1", vec![], 24).expect("clear ramp");
    let c1b = match &p.track("v1").unwrap().clips[0] {
        Clip::Media(c) => c.clone(),
        _ => unreachable!(),
    };
    assert!(!c1b.has_speed_ramp());
    assert_eq!(Clip::Media(c1b).timeline_duration_ms(), 5000); // back to source length
}

/// The frame-aware clamp keeps each sub-segment ≥ MIN_FRAMES_PER_SUBSEG output
/// frames at the fastest factor, so a too-fine request is reduced (preventing
/// the sub-frame video segments that drift from audio). For a 5000ms clip @30fps
/// with a 3× peak: cap = 5000·30/(3·4·1000) = 12 → a request of 80 is stored as 12.
#[test]
fn speed_ramp_clamps_segments_to_frame_floor() {
    let mut p = fixture(); // 30fps default; c1 is [0,5000) on v1
    speed_ramp(&mut p, "c1", ramp_pts(), 80).expect("set ramp");
    let c1 = match &p.track("v1").unwrap().clips[0] {
        Clip::Media(c) => c.clone(),
        _ => unreachable!(),
    };
    let segs = c1.speed_ramp.as_ref().unwrap().segments;
    assert_eq!(
        segs, 12,
        "80 requested → clamped to the frame-floor cap of 12"
    );
    // A modest request UNDER the cap is kept as-is.
    speed_ramp(&mut p, "c2", ramp_pts(), 6).expect("set ramp c2");
    let c2 = match &p.track("v1").unwrap().clips[1] {
        Clip::Media(c) => c.clone(),
        _ => unreachable!(),
    };
    assert_eq!(
        c2.speed_ramp.as_ref().unwrap().segments,
        6,
        "under-cap request kept"
    );
}

/// SET a ramp on a clip that already carries a time-warping feature is refused
/// (the forward guard) — proven for reverse and a constant speed.
#[test]
fn speed_ramp_refuses_incompatible_clip() {
    let mut p = fixture();
    reverse(&mut p, "c2", true).expect("reverse c2");
    assert_eq!(
        speed_ramp(&mut p, "c2", ramp_pts(), 24).unwrap_err().code,
        "invalid_args"
    );
    let mut p2 = fixture();
    speed(&mut p2, "c2", 2.0).expect("speed c2");
    assert_eq!(
        speed_ramp(&mut p2, "c2", ramp_pts(), 24).unwrap_err().code,
        "invalid_args"
    );
}

// -----------------------------------------------------------------------
// edit.duplicate — clone a clip with ALL its attributes, placed right after
// it on the same track; replay-safe (the cloned id is pinned).
// -----------------------------------------------------------------------
mod duplicate_tests {
    use super::*;
    use crate::types::{ClipEffect, ClipFade, ClipGrade, FadeKind};

    /// A heavily-attributed source clip on v1 ([src 1000..6000) → 5000ms),
    /// plus a plain c2 after it (so we can prove the clone lands BETWEEN them).
    fn attributed_fixture() -> Project {
        let mut p = Project::new("t", ProjectSettings::default());
        let mut src = make_media_clip("c1", "a1", 1000, 6000);
        src.gain_db = -6.0;
        src.speed = 2.0; // retimed: timeline span = 5000/2 = 2500ms
        src.reverse = true;
        src.effects = vec![ClipEffect::Vignette { amount: 0.7 }];
        src.fade = Some(ClipFade {
            in_ms: 200,
            out_ms: 300,
            kind: FadeKind::Video,
        });
        src.grade = Some(ClipGrade {
            contrast: 1.2,
            brightness: 0.1,
            saturation: 0.8,
            gamma: 1.0,
            temperature_k: Some(5200),
            lut: None,
        });
        // A crossfade-IN on the source: the clone must RESET this to a hard cut.
        src.xfade_in_ms = 400;
        p.track_mut("v1").unwrap().clips.push(Clip::Media(src));
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(make_media_clip("c2", "a1", 6000, 8000)));
        p
    }

    /// The clone carries EVERY per-clip attribute (minus id), lands immediately
    /// after the source on the same track, and lengthens the timeline by the
    /// source's TIMELINE duration. The crossfade-in is reset to a hard cut.
    #[test]
    fn clone_carries_all_attributes_and_lands_after_source() {
        let mut p = attributed_fixture();
        let before_len = p.track("v1").unwrap().duration_ms();
        let src_tl_dur = p.track("v1").unwrap().clips[0].timeline_duration_ms();
        assert_eq!(src_tl_dur, 2500, "5000ms source at 2× = 2500ms timeline");

        let eff = duplicate(&mut p, "c1", false).expect("duplicate c1");
        let new_id = eff[0].detail["added_clip"].as_str().unwrap().to_string();
        assert_eq!(eff[0].detail["source_clip"], "c1");

        let v1 = p.track("v1").unwrap();
        // [c1, c1_dup, c2] — the clone sits BETWEEN the source and c2.
        assert_eq!(v1.clips.len(), 3);
        assert_eq!(v1.clips[0].id(), Some("c1"));
        assert_eq!(v1.clips[1].id().unwrap(), new_id);
        assert_eq!(v1.clips[2].id(), Some("c2"));
        assert_ne!(new_id, "c1");
        assert_ne!(new_id, "c2");

        let (Clip::Media(src), Clip::Media(dup)) = (&v1.clips[0], &v1.clips[1]) else {
            unreachable!("expected media clips");
        };
        // Same asset + source window.
        assert_eq!(dup.asset, src.asset);
        assert_eq!((dup.src_in_ms, dup.src_out_ms), (1000, 6000));
        // Every per-clip attribute carried.
        assert_eq!(dup.gain_db, -6.0);
        assert_eq!(dup.speed, 2.0);
        assert!(dup.reverse);
        assert_eq!(dup.effects, src.effects);
        assert_eq!(dup.fade, src.fade);
        assert_eq!(dup.grade, src.grade);
        // ...but the crossfade-in is reset to a HARD CUT (a transition is a
        // boundary property of the ORIGINAL left neighbour, not the content).
        assert_eq!(src.xfade_in_ms, 400, "source keeps its crossfade-in");
        assert_eq!(dup.xfade_in_ms, 0, "clone resets to a hard cut");
        assert_eq!(dup.xfade_kind, None);

        // Timeline lengthened by exactly the source's timeline duration.
        assert_eq!(
            p.track("v1").unwrap().duration_ms(),
            before_len + src_tl_dur
        );
    }

    /// ripple:true opens a clip-length gap on a SIBLING track (base-track AV
    /// sync); ripple:false leaves siblings untouched.
    #[test]
    fn ripple_true_opens_sibling_gap_false_does_not() {
        // v1=[c1(0,5000)], a1t=[caudio(0,8000)] — a sibling with content after
        // the insertion point (5000ms) so the ripple has something to shift.
        let mk = || {
            let mut p = Project::new("t", ProjectSettings::default());
            p.track_mut("v1")
                .unwrap()
                .clips
                .push(Clip::Media(make_media_clip("c1", "a1", 0, 5000)));
            p.track_mut("a1t")
                .unwrap()
                .clips
                .push(Clip::Media(make_media_clip("ca", "a1", 0, 8000)));
            p
        };

        let mut rip = mk();
        duplicate(&mut rip, "c1", true).expect("duplicate ripple:true");
        assert_eq!(rip.track("v1").unwrap().duration_ms(), 10000); // +5000 clone
        assert_eq!(
            rip.track("a1t").unwrap().duration_ms(),
            13000,
            "ripple:true splices a 5000ms gap into the audio sibling"
        );

        let mut no_rip = mk();
        duplicate(&mut no_rip, "c1", false).expect("duplicate ripple:false");
        assert_eq!(no_rip.track("v1").unwrap().duration_ms(), 10000);
        assert_eq!(
            no_rip.track("a1t").unwrap().duration_ms(),
            8000,
            "ripple:false leaves the audio sibling untouched"
        );
    }

    /// Errors are actionable: an unknown clip and a caption clip are refused.
    #[test]
    fn rejects_unknown_and_caption_clips() {
        let mut p = attributed_fixture();
        assert_eq!(
            duplicate(&mut p, "nope", false).unwrap_err().code,
            codes::NOT_FOUND
        );
        // A caption clip on a caption track is refused (captions.* own those).
        p.tracks.push(Track {
            id: "cap1".into(),
            kind: TrackKind::Caption,
            clips: vec![Clip::Caption(CaptionClip {
                id: "s1".into(),
                text: "hi".into(),
                style_ref: None,
                range_ms: [0, 1000],
            })],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });
        assert_eq!(
            duplicate(&mut p, "s1", false).unwrap_err().code,
            codes::INVALID_ARGS
        );
    }

    /// REPLAY-SAFETY: edit.duplicate lowers to one logged op that references
    /// the SOURCE by id and re-clones at apply time; a log rebuild must
    /// reproduce the post-duplicate timeline byte-for-byte (the cloned id is
    /// pinned via `added_clip`), AND the clone carries an attribute set on the
    /// source BEFORE the duplicate (proving attribute-carry survives replay).
    #[test]
    fn duplicate_replays_byte_identical() {
        use crate::types::Asset;
        use crate::{rebuild_from_log, ProjectStore};
        use serde_json::json;

        fn actor() -> crate::Actor {
            crate::Actor {
                kind: crate::ActorKind::Agent,
                name: "claude".into(),
                via: "test".into(),
            }
        }
        fn asset() -> Asset {
            Asset {
                path: "/testdata/clip.mp4".into(),
                hash: "sha256:deadbeef".into(),
                probe: Some(json!({ "duration_ms": 8000, "has_audio": false })),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(dir.path(), "demo", None).unwrap();
        let (aid, _) = s.record_import(None, asset(), actor(), None).unwrap();
        assert_eq!(aid, "a1");

        // Place a clip, then set an attribute on it via a real verb (edit.transform)
        // BEFORE duplicating — the clone must carry it through replay.
        s.apply(
            "edit.insert",
            json!({ "asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 5000], "ripple": false }),
            actor(),
            None,
        )
        .unwrap();
        let vid_id = s.project.track("v1").unwrap().clips[0]
            .id()
            .unwrap()
            .to_string();
        s.apply(
            "edit.transform",
            json!({ "clip": vid_id, "x": 0.2, "y": 0.1, "scale": 0.8 }),
            actor(),
            None,
        )
        .unwrap();

        // Duplicate (the live op allocates the clone id positionally + records it).
        s.apply(
            "edit.duplicate",
            json!({ "clip": vid_id, "ripple": false }),
            actor(),
            None,
        )
        .unwrap();

        let v1 = s.project.track("v1").unwrap();
        assert_eq!(v1.clips.len(), 2, "[source, clone]");
        let (Clip::Media(src), Clip::Media(dup)) = (&v1.clips[0], &v1.clips[1]) else {
            unreachable!("expected media clips");
        };
        assert_ne!(dup.id, src.id, "the clone is a distinct clip");
        assert_eq!(
            dup.transform, src.transform,
            "the clone carries the source's transform"
        );
        assert_eq!((dup.src_in_ms, dup.src_out_ms), (0, 5000));

        // Replay: rebuild the timeline from the op log and compare byte-
        // identically (the cloned id is pinned, so allocation order is stable).
        let rebuilt = rebuild_from_log(&s.log.read_all().unwrap()).unwrap();
        assert_eq!(
            serde_json::to_string(&rebuilt.tracks).unwrap(),
            serde_json::to_string(&s.project.tracks).unwrap(),
            "rebuild_from_log == live timeline (cloned id pinned via added_clip)"
        );
    }
}

// -----------------------------------------------------------------------
// edit.nest — collapse a contiguous run into a single COMPOUND CLIP / nest:
// the run MOVES into a Project::nests sub-timeline (attributes preserved),
// replaced on the parent by one nest clip spanning the combined range.
// -----------------------------------------------------------------------
mod nest_tests {
    use super::*;
    use crate::types::{ClipEffect, ClipGrade};

    /// v1 = [c1(0..2000), c2(2000..5000 graded+effected), c3(5000..6000)].
    fn fixture() -> Project {
        let mut p = Project::new("t", ProjectSettings::default());
        let c1 = make_media_clip("c1", "a1", 0, 2000); // 2000ms
        let mut c2 = make_media_clip("c2", "a1", 2000, 5000); // 3000ms
        c2.effects = vec![ClipEffect::Vignette { amount: 0.6 }];
        c2.grade = Some(ClipGrade {
            contrast: 1.3,
            brightness: 0.0,
            saturation: 0.8,
            gamma: 1.0,
            temperature_k: Some(5200),
            lut: None,
        });
        let c3 = make_media_clip("c3", "a1", 5000, 6000); // 1000ms
        let v1 = p.track_mut("v1").unwrap();
        v1.clips
            .extend([Clip::Media(c1), Clip::Media(c2), Clip::Media(c3)]);
        p
    }

    /// Nesting [c1,c2] collapses them into ONE nest clip spanning their combined
    /// [0,5000) range; c3 stays; the parent timeline LENGTH is unchanged; and the
    /// nest sub-timeline holds the two ORIGINALS with every attribute preserved.
    #[test]
    fn collapses_run_into_single_clip_preserving_attrs() {
        let mut p = fixture();
        let before_len = p.track("v1").unwrap().duration_ms();
        assert_eq!(before_len, 6000);

        let eff = nest(&mut p, &["c1".into(), "c2".into()], Some("intro")).expect("nest");
        let d = &eff[0].detail;
        let nest_clip_id = d["added_clip"].as_str().unwrap().to_string();
        let nest_id = d["added_nest"].as_str().unwrap().to_string();
        assert_eq!(nest_id, "nest1");
        assert_eq!(d["span_ms"], 5000);
        assert_eq!(d["added_ms"], serde_json::json!([0, 5000]));
        assert_eq!(d["nested_clips"], serde_json::json!(["c1", "c2"]));

        // Parent track is now [nest_clip, c3] — length unchanged (5000 + 1000).
        let v1 = p.track("v1").unwrap();
        assert_eq!(v1.clips.len(), 2, "[nest_clip, c3]");
        assert_eq!(v1.duration_ms(), 6000, "parent timeline length unchanged");
        let Clip::Media(nclip) = &v1.clips[0] else {
            unreachable!("nest clip is a media clip");
        };
        assert_eq!(nclip.id, nest_clip_id);
        assert_eq!(nclip.nest.as_deref(), Some("nest1"), "marked as a nest");
        assert_eq!(nclip.asset, "nest1", "its source IS the nest");
        assert_eq!((nclip.src_in_ms, nclip.src_out_ms), (0, 5000));
        assert_eq!(
            Clip::Media(nclip.clone()).timeline_duration_ms(),
            5000,
            "the nest clip occupies the combined span"
        );
        assert_eq!(v1.clips[1].id(), Some("c3"));

        // The nest sub-timeline holds the two ORIGINALS, rebased to start at 0,
        // with grade + effects intact.
        assert_eq!(p.nests.len(), 1);
        let n = p.nest("nest1").unwrap();
        assert_eq!(n.name.as_deref(), Some("intro"));
        assert_eq!(n.span_ms(), 5000);
        let sub = &n.tracks[0];
        assert_eq!(sub.clips.len(), 2);
        assert_eq!(sub.clips[0].id(), Some("c1"));
        assert_eq!(sub.clips[1].id(), Some("c2"));
        let Clip::Media(sub_c2) = &sub.clips[1] else {
            unreachable!("c2 is media");
        };
        assert_eq!(sub_c2.effects, vec![ClipEffect::Vignette { amount: 0.6 }]);
        assert!(sub_c2.grade.is_some(), "grade preserved inside the nest");
        assert_eq!(sub_c2.grade.as_ref().unwrap().temperature_k, Some(5200));
    }

    /// A new clip id allocated AFTER a nest never collides with a clip buried in
    /// the nest (max_clip_n scans nests too).
    #[test]
    fn new_ids_skip_clips_buried_in_a_nest() {
        let mut p = fixture();
        nest(&mut p, &["c1".into(), "c2".into()], None).expect("nest");
        // c1,c2 now live only in the nest; the nest clip got c4 (max c3 + 1).
        let nest_clip = p.track("v1").unwrap().clips[0].id().unwrap().to_string();
        assert_eq!(nest_clip, "c4");
        // A fresh id is c5 — never re-mints c1/c2 (buried) or c4 (the nest clip).
        assert_eq!(new_clip_id(&p), "c5");
    }

    /// Every refusal path is an actionable error (no half-mutation).
    #[test]
    fn rejects_bad_selections() {
        // Empty selection.
        assert_eq!(
            nest(&mut fixture(), &[], None).unwrap_err().code,
            codes::INVALID_ARGS
        );
        // Unknown clip.
        assert_eq!(
            nest(&mut fixture(), &["nope".into()], None)
                .unwrap_err()
                .code,
            codes::NOT_FOUND
        );
        // Non-contiguous (c1 + c3, skipping c2).
        assert_eq!(
            nest(&mut fixture(), &["c1".into(), "c3".into()], None)
                .unwrap_err()
                .code,
            codes::INVALID_ARGS
        );
        // Duplicate clip in the selection.
        assert_eq!(
            nest(&mut fixture(), &["c1".into(), "c1".into()], None)
                .unwrap_err()
                .code,
            codes::INVALID_ARGS
        );
        // Cross-track: add an audio clip on a1t and select it with a v1 clip.
        let mut p = fixture();
        p.track_mut("a1t")
            .unwrap()
            .clips
            .push(Clip::Media(make_media_clip("ca", "a1", 0, 1000)));
        assert_eq!(
            nest(&mut p, &["c1".into(), "ca".into()], None)
                .unwrap_err()
                .code,
            codes::INVALID_ARGS
        );
        // A gap inside the run.
        let mut pg = Project::new("t", ProjectSettings::default());
        {
            let v1 = pg.track_mut("v1").unwrap();
            v1.clips
                .push(Clip::Media(make_media_clip("c1", "a1", 0, 1000)));
            v1.clips.push(Clip::Gap(GapClip::new(500)));
            v1.clips
                .push(Clip::Media(make_media_clip("c2", "a1", 1000, 2000)));
        }
        assert_eq!(
            nest(&mut pg, &["c1".into(), "c2".into()], None)
                .unwrap_err()
                .code,
            codes::INVALID_ARGS
        );
        // No nest-of-nest: nesting a clip that is itself a nest is refused.
        let mut pn = fixture();
        nest(&mut pn, &["c1".into(), "c2".into()], None).expect("first nest");
        let nest_clip = pn.track("v1").unwrap().clips[0].id().unwrap().to_string();
        assert_eq!(
            nest(&mut pn, &[nest_clip], None).unwrap_err().code,
            codes::INVALID_ARGS
        );
    }

    /// REPLAY-SAFETY: edit.nest lowers to one logged op; a rebuild_from_log must
    /// reproduce the post-nest timeline AND the nests byte-for-byte (the nest clip
    /// id is pinned via `added_clip`; the nest id is deterministic).
    #[test]
    fn nest_replays_byte_identical() {
        use crate::types::Asset;
        use crate::{rebuild_from_log, ProjectStore};
        use serde_json::json;

        fn actor() -> crate::Actor {
            crate::Actor {
                kind: crate::ActorKind::Agent,
                name: "claude".into(),
                via: "test".into(),
            }
        }
        fn asset() -> Asset {
            Asset {
                path: "/testdata/clip.mp4".into(),
                hash: "sha256:deadbeef".into(),
                probe: Some(json!({ "duration_ms": 8000, "has_audio": false })),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(dir.path(), "demo", None).unwrap();
        s.record_import(None, asset(), actor(), None).unwrap();
        // Two clips on v1, then a grade on the second BEFORE nesting (must survive
        // replay inside the nest).
        s.apply(
            "edit.insert",
            json!({ "asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 2000], "ripple": false }),
            actor(),
            None,
        )
        .unwrap();
        s.apply(
            "edit.insert",
            json!({ "asset": "a1", "track": "v1", "at_ms": 2000, "src_range_ms": [2000, 5000], "ripple": false }),
            actor(),
            None,
        )
        .unwrap();
        let ids: Vec<String> = s
            .project
            .track("v1")
            .unwrap()
            .clips
            .iter()
            .filter_map(|c| c.id().map(String::from))
            .collect();
        s.apply(
            "edit.grade",
            json!({ "clip": ids[1], "saturation": 0.5 }),
            actor(),
            None,
        )
        .unwrap();
        s.apply(
            "edit.nest",
            json!({ "clips": ids, "name": "intro" }),
            actor(),
            None,
        )
        .unwrap();

        // Live: one nest clip on v1, two clips in the nest.
        assert_eq!(s.project.track("v1").unwrap().clips.len(), 1);
        assert_eq!(s.project.nests.len(), 1);
        assert_eq!(s.project.nests[0].tracks[0].clips.len(), 2);

        // Replay → byte-identical tracks AND nests (the cloned id is pinned).
        let rebuilt = rebuild_from_log(&s.log.read_all().unwrap()).unwrap();
        assert_eq!(
            serde_json::to_string(&rebuilt.tracks).unwrap(),
            serde_json::to_string(&s.project.tracks).unwrap(),
            "rebuild_from_log tracks == live"
        );
        assert_eq!(
            serde_json::to_string(&rebuilt.nests).unwrap(),
            serde_json::to_string(&s.project.nests).unwrap(),
            "rebuild_from_log nests == live (nest id deterministic, clip id pinned)"
        );
    }

    #[test]
    fn split_nest_clip_preserves_nest_marker_on_both_halves() {
        use crate::types::Asset;
        use crate::{rebuild_from_log, ProjectStore};
        use serde_json::json;

        fn actor() -> crate::Actor {
            crate::Actor {
                kind: crate::ActorKind::Agent,
                name: "claude".into(),
                via: "test".into(),
            }
        }
        fn asset() -> Asset {
            Asset {
                path: "/testdata/clip.mp4".into(),
                hash: "sha256:deadbeef".into(),
                probe: Some(json!({ "duration_ms": 8000, "has_audio": false })),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(dir.path(), "demo", None).unwrap();
        s.record_import(None, asset(), actor(), None).unwrap();
        s.apply(
            "edit.insert",
            json!({ "asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 2000], "ripple": false }),
            actor(),
            None,
        )
        .unwrap();
        s.apply(
            "edit.insert",
            json!({ "asset": "a1", "track": "v1", "at_ms": 2000, "src_range_ms": [2000, 5000], "ripple": false }),
            actor(),
            None,
        )
        .unwrap();
        let ids: Vec<String> = s
            .project
            .track("v1")
            .unwrap()
            .clips
            .iter()
            .filter_map(|c| c.id().map(String::from))
            .collect();
        s.apply(
            "edit.nest",
            json!({ "clips": ids, "name": "intro" }),
            actor(),
            None,
        )
        .unwrap();
        s.apply(
            "edit.split",
            json!({ "track": "v1", "at_ms": 2000 }),
            actor(),
            None,
        )
        .unwrap();

        let clips = &s.project.track("v1").unwrap().clips;
        assert_eq!(clips.len(), 2);
        for clip in clips {
            let Clip::Media(media) = clip else {
                panic!("split nest clip should stay media");
            };
            assert_eq!(media.asset, "nest1");
            assert_eq!(media.nest.as_deref(), Some("nest1"));
        }

        let rebuilt = rebuild_from_log(&s.log.read_all().unwrap()).unwrap();
        assert_eq!(
            serde_json::to_string(&rebuilt.tracks).unwrap(),
            serde_json::to_string(&s.project.tracks).unwrap()
        );
        assert_eq!(
            serde_json::to_string(&rebuilt.nests).unwrap(),
            serde_json::to_string(&s.project.nests).unwrap()
        );
    }
}

// -----------------------------------------------------------------------
// edit.replace — 3-point REPLACE EDIT: swap a clip's SOURCE in place,
// preserving its id + timeline position + slot duration. Short source is
// clamped + gap-padded; the look is kept, source-timing fields reset.
// -----------------------------------------------------------------------
mod replace_tests {
    use super::*;
    use crate::types::{Asset, ClipEffect, ClipFreeze, ClipGrade, ClipTransform};

    /// Register an asset id with a probe duration so replace/fit can read it.
    fn register_asset(p: &mut Project, id: &str, duration_ms: u64) {
        p.assets.insert(
            id.to_string(),
            Asset {
                path: format!("/testdata/{id}.mp4"),
                hash: format!("sha256:{id}"),
                probe: Some(json!({ "duration_ms": duration_ms, "has_audio": true })),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
    }

    /// v1 = [c1 (a1, 1000..6000 @ 2× → 2500ms slot, heavily attributed), c2].
    /// a2 = the long replacement asset (20000ms).
    fn fixture() -> Project {
        let mut p = Project::new("t", ProjectSettings::default());
        register_asset(&mut p, "a1", 8000);
        register_asset(&mut p, "a2", 20000);
        let mut src = make_media_clip("c1", "a1", 1000, 6000);
        src.speed = 2.0; // 5000ms source @ 2× = 2500ms timeline slot
        src.gain_db = -6.0;
        src.reverse = true;
        src.effects = vec![ClipEffect::Vignette { amount: 0.7 }];
        src.transform = Some(ClipTransform {
            x: 0.1,
            y: 0.2,
            scale: 0.5,
            opacity: 0.9,
        });
        src.grade = Some(ClipGrade {
            contrast: 1.2,
            brightness: 0.1,
            saturation: 0.8,
            gamma: 1.0,
            temperature_k: Some(5200),
            lut: None,
        });
        // A source-timing field tied to the OLD footage — must be RESET.
        src.freeze = Some(ClipFreeze { at_ms: 500 });
        p.track_mut("v1").unwrap().clips.push(Clip::Media(src));
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(make_media_clip("c2", "a1", 6000, 8000)));
        p
    }

    /// Replace preserves the slot (id + position + duration) and the look, swaps
    /// the source onto the new asset at normal speed, and resets the source-timing
    /// fields (speed→1, speed_ramp→None, freeze→None). The source is long enough
    /// (a2 = 20000ms ≥ the 2500ms slot from in=0), so no gap is padded.
    #[test]
    fn preserves_slot_and_swaps_source_keeping_look() {
        let mut p = fixture();
        let before_len = p.track("v1").unwrap().duration_ms();
        let slot = p.track("v1").unwrap().clips[0].timeline_duration_ms();
        assert_eq!(slot, 2500, "5000ms source @ 2× = 2500ms slot");

        let eff = replace(&mut p, "c1", "a2", None, None).expect("replace c1");
        assert_eq!(eff[0].detail["clip"], "c1");
        assert_eq!(eff[0].detail["asset"], "a2");
        assert_eq!(eff[0].detail["slot_ms"], 2500);
        assert_eq!(eff[0].detail["gap_ms"], 0);

        let v1 = p.track("v1").unwrap();
        assert_eq!(v1.clips.len(), 2, "[c1(replaced), c2] — no pad gap");
        assert_eq!(v1.clips[0].id(), Some("c1"), "the target keeps its id");
        assert_eq!(v1.clips[1].id(), Some("c2"), "c2 unmoved");

        let Clip::Media(c) = &v1.clips[0] else {
            unreachable!("expected media");
        };
        // Source swapped: new asset, normal speed fills the slot exactly.
        assert_eq!(c.asset, "a2");
        assert_eq!((c.src_in_ms, c.src_out_ms), (0, 2500), "in=0, span=slot");
        assert_eq!(c.speed, 1.0, "speed reset to normal");
        assert_eq!(c.speed_ramp, None);
        assert_eq!(c.freeze, None, "freeze (source-timing) reset");
        // Look KEPT (time-invariant attributes survive the swap).
        assert_eq!(c.gain_db, -6.0);
        assert!(c.reverse);
        assert_eq!(c.effects, vec![ClipEffect::Vignette { amount: 0.7 }]);
        assert!(c.transform.is_some());
        assert!(c.grade.is_some());
        // Slot duration preserved → the track length is unchanged.
        assert_eq!(p.track("v1").unwrap().duration_ms(), before_len);
        assert_eq!(p.track("v1").unwrap().clips[0].timeline_duration_ms(), 2500);
    }

    /// Insufficient media: a source window SHORTER than the slot is clamped and
    /// the remainder of the slot is padded with a gap, so the slot total — and
    /// every downstream clip — is preserved exactly (no ripple).
    #[test]
    fn short_source_clamps_and_pads_gap() {
        let mut p = fixture();
        let before_len = p.track("v1").unwrap().duration_ms();
        // Only 1000ms of a2 available [0,1000) but the slot is 2500ms.
        let eff = replace(&mut p, "c1", "a2", Some(0), Some(1000)).expect("replace");
        assert_eq!(
            eff[0].detail["gap_ms"], 1500,
            "2500 slot − 1000 used = 1500 pad"
        );

        let v1 = p.track("v1").unwrap();
        assert_eq!(v1.clips.len(), 3, "[c1(1000ms), gap(1500ms), c2]");
        let Clip::Media(c) = &v1.clips[0] else {
            unreachable!("media")
        };
        assert_eq!((c.src_in_ms, c.src_out_ms), (0, 1000));
        assert_eq!(v1.clips[0].timeline_duration_ms(), 1000);
        assert!(matches!(v1.clips[1], Clip::Gap(ref g) if g.duration_ms == 1500));
        assert_eq!(v1.clips[2].id(), Some("c2"));
        // Slot total (clip + pad) = 2500 → downstream timing preserved.
        assert_eq!(p.track("v1").unwrap().duration_ms(), before_len);
    }

    /// A source_out_ms cap and a source_in_ms offset window the replacement.
    #[test]
    fn windows_the_source() {
        let mut p = fixture();
        // in=3000, slot=2500 → wants [3000,5500); cap (out=10000, a2=20000) allows it.
        let eff = replace(&mut p, "c1", "a2", Some(3000), Some(10000)).expect("replace");
        assert_eq!(eff[0].detail["gap_ms"], 0);
        let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
            unreachable!("media")
        };
        assert_eq!((c.src_in_ms, c.src_out_ms), (3000, 5500), "in + slot");
    }

    /// Errors are actionable: unknown target, caption clip, and missing asset.
    #[test]
    fn rejects_unknown_caption_and_missing_asset() {
        let mut p = fixture();
        assert_eq!(
            replace(&mut p, "nope", "a2", None, None).unwrap_err().code,
            codes::NOT_FOUND
        );
        assert_eq!(
            replace(&mut p, "c1", "ghost", None, None).unwrap_err().code,
            codes::NOT_FOUND,
            "the replacement asset must be registered"
        );
        p.tracks.push(Track {
            id: "cap1".into(),
            kind: TrackKind::Caption,
            clips: vec![Clip::Caption(CaptionClip {
                id: "s1".into(),
                text: "hi".into(),
                style_ref: None,
                range_ms: [0, 1000],
            })],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });
        assert_eq!(
            replace(&mut p, "s1", "a2", None, None).unwrap_err().code,
            codes::INVALID_ARGS
        );
    }

    /// REPLAY-SAFETY: edit.replace lowers to one logged op that allocates NO new
    /// clip id (the target keeps its id) and re-derives the slot/window/pad
    /// deterministically; a log rebuild must reproduce the post-replace timeline
    /// byte-for-byte. Two assets are imported; the look set BEFORE the replace
    /// (edit.gain) must survive it.
    #[test]
    fn replace_replays_byte_identical() {
        use crate::{rebuild_from_log, ProjectStore};

        fn actor() -> crate::Actor {
            crate::Actor {
                kind: crate::ActorKind::Agent,
                name: "claude".into(),
                via: "test".into(),
            }
        }
        fn asset(path: &str, dur: u64) -> Asset {
            Asset {
                path: path.into(),
                hash: format!("sha256:{path}"),
                probe: Some(json!({ "duration_ms": dur, "has_audio": false })),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(dir.path(), "demo", None).unwrap();
        let (a1, _) = s
            .record_import(None, asset("/a.mp4", 8000), actor(), None)
            .unwrap();
        let (a2, _) = s
            .record_import(None, asset("/b.mp4", 20000), actor(), None)
            .unwrap();
        assert_eq!((a1.as_str(), a2.as_str()), ("a1", "a2"));

        s.apply(
            "edit.insert",
            json!({ "asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 5000], "ripple": false }),
            actor(),
            None,
        )
        .unwrap();
        let cid = s.project.track("v1").unwrap().clips[0]
            .id()
            .unwrap()
            .to_string();
        s.apply(
            "edit.transform",
            json!({ "clip": cid, "x": 0.2, "y": 0.1, "scale": 0.8 }),
            actor(),
            None,
        )
        .unwrap();
        // Replace the source with a2; the kept transform must survive.
        s.apply(
            "edit.replace",
            json!({ "clip": cid, "asset": "a2", "source_in_ms": 0 }),
            actor(),
            None,
        )
        .unwrap();

        let v1 = s.project.track("v1").unwrap();
        assert_eq!(v1.clips.len(), 1, "in-place swap, slot preserved");
        let Clip::Media(c) = &v1.clips[0] else {
            unreachable!("media")
        };
        assert_eq!(c.id, cid, "the target keeps its id");
        assert_eq!(c.asset, "a2", "swapped to the new asset");
        assert_eq!(
            (c.src_in_ms, c.src_out_ms),
            (0, 5000),
            "slot (5000ms) filled"
        );
        assert_eq!(
            c.transform.as_ref().map(|transform| transform.scale),
            Some(0.8),
            "the transform survived the replace"
        );

        let rebuilt = rebuild_from_log(&s.log.read_all().unwrap()).unwrap();
        assert_eq!(
            serde_json::to_string(&rebuilt.tracks).unwrap(),
            serde_json::to_string(&s.project.tracks).unwrap(),
            "rebuild_from_log == live timeline (no id allocation → deterministic)"
        );
    }
}

// -----------------------------------------------------------------------
// edit.fit_to_fill — FIT TO FILL: speed-adjust footage to exactly
// fill an empty slot/gap, with NO downstream shift.
// -----------------------------------------------------------------------
mod fit_to_fill_tests {
    use super::*;
    use crate::types::Asset;

    fn register_asset(p: &mut Project, id: &str, duration_ms: u64) {
        p.assets.insert(
            id.to_string(),
            Asset {
                path: format!("/testdata/{id}.mp4"),
                hash: format!("sha256:{id}"),
                probe: Some(json!({ "duration_ms": duration_ms, "has_audio": true })),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
    }

    /// v1 = [c1 (a1, 0..5000), GAP(3000ms), c2 (a1, 0..2000)]. fit 6000ms of a2
    /// into the gap → speed 2.0, the placed clip occupies exactly 3000ms, c2
    /// does not move (the gap is consumed in place, not rippled).
    #[test]
    fn fits_source_into_gap_no_downstream_shift() {
        let mut p = Project::new("t", ProjectSettings::default());
        register_asset(&mut p, "a1", 8000);
        register_asset(&mut p, "a2", 20000);
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(make_media_clip("c1", "a1", 0, 5000)));
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Gap(GapClip::new(3000)));
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(make_media_clip("c2", "a1", 0, 2000)));
        let before_len = p.track("v1").unwrap().duration_ms();
        assert_eq!(before_len, 10000); // 5000 + 3000 + 2000

        // Fill the gap at 5000ms with 6000ms of a2 → speed = 6000/3000 = 2.0.
        let eff = fit_to_fill(&mut p, "v1", 5000, 3000, "a2", 0, 6000).expect("fit");
        let new_id = eff[0].detail["added_clip"].as_str().unwrap().to_string();
        assert_eq!(eff[0].detail["speed"], 2.0);
        assert_eq!(eff[0].detail["slot_ms"], 3000);

        let v1 = p.track("v1").unwrap();
        // The gap is fully consumed (left=0, right=0) → [c1, fit, c2].
        assert_eq!(v1.clips.len(), 3);
        assert_eq!(v1.clips[0].id(), Some("c1"));
        assert_eq!(v1.clips[1].id().unwrap(), new_id);
        assert_eq!(v1.clips[2].id(), Some("c2"), "c2 unmoved");
        let Clip::Media(c) = &v1.clips[1] else {
            unreachable!("media")
        };
        assert_eq!(c.asset, "a2");
        assert_eq!(c.speed, 2.0);
        assert_eq!((c.src_in_ms, c.src_out_ms), (0, 6000));
        assert_eq!(
            v1.clips[1].timeline_duration_ms(),
            3000,
            "speed-fit clip occupies EXACTLY the slot"
        );
        // Track length unchanged → no downstream shift.
        assert_eq!(p.track("v1").unwrap().duration_ms(), before_len);
    }

    /// A partial fill of a wider gap splits it into [fit] + [leftover gap].
    #[test]
    fn partial_fill_splits_the_gap() {
        let mut p = Project::new("t", ProjectSettings::default());
        register_asset(&mut p, "a2", 20000);
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Gap(GapClip::new(8000)));
        // Fill only [1000, 4000) of the gap with 6000ms of a2 → speed 2.0, 3000ms.
        fit_to_fill(&mut p, "v1", 1000, 3000, "a2", 0, 6000).expect("fit");
        let v1 = p.track("v1").unwrap();
        // [gap(1000), fit(3000), gap(4000)] — total still 8000.
        assert_eq!(v1.clips.len(), 3);
        assert!(matches!(v1.clips[0], Clip::Gap(ref g) if g.duration_ms == 1000));
        assert_eq!(v1.clips[1].timeline_duration_ms(), 3000);
        assert!(matches!(v1.clips[2], Clip::Gap(ref g) if g.duration_ms == 4000));
        assert_eq!(p.track("v1").unwrap().duration_ms(), 8000);
    }

    /// Slow-motion fit: 2000ms of source into a 4000ms slot → speed 0.5.
    #[test]
    fn slow_motion_fit() {
        let mut p = Project::new("t", ProjectSettings::default());
        register_asset(&mut p, "a2", 20000);
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Gap(GapClip::new(4000)));
        let eff = fit_to_fill(&mut p, "v1", 0, 4000, "a2", 0, 2000).expect("fit");
        assert_eq!(eff[0].detail["speed"], 0.5);
        assert_eq!(p.track("v1").unwrap().clips[0].timeline_duration_ms(), 4000);
        let Clip::Media(c) = &p.track("v1").unwrap().clips[0] else {
            unreachable!("media")
        };
        assert_eq!(c.speed, 0.5);
    }

    /// fit_to_fill fills EMPTY space only — a slot occupied by media is refused.
    #[test]
    fn rejects_occupied_slot() {
        let mut p = Project::new("t", ProjectSettings::default());
        register_asset(&mut p, "a1", 8000);
        register_asset(&mut p, "a2", 20000);
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(make_media_clip("c1", "a1", 0, 5000)));
        // at_ms=0 lands on c1 (media), not a gap.
        assert_eq!(
            fit_to_fill(&mut p, "v1", 0, 2000, "a2", 0, 4000)
                .unwrap_err()
                .code,
            codes::CONFLICT
        );
    }

    /// A fill that overruns the gap it lands in is refused (no silent ripple).
    #[test]
    fn rejects_overrun_of_gap() {
        let mut p = Project::new("t", ProjectSettings::default());
        register_asset(&mut p, "a2", 20000);
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Gap(GapClip::new(2000)));
        // Slot 3000ms but the gap is only 2000ms → overrun.
        assert_eq!(
            fit_to_fill(&mut p, "v1", 0, 3000, "a2", 0, 6000)
                .unwrap_err()
                .code,
            codes::CONFLICT
        );
    }

    #[test]
    fn rejects_source_range_beyond_probe_duration() {
        let mut p = Project::new("t", ProjectSettings::default());
        register_asset(&mut p, "a2", 10_000);
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Gap(GapClip::new(3000)));

        let err = fit_to_fill(&mut p, "v1", 0, 3000, "a2", 9_000, 12_000).unwrap_err();

        assert_eq!(err.code, codes::INVALID_ARGS);
        assert_eq!(p.track("v1").unwrap().clips.len(), 1);
        assert!(matches!(p.track("v1").unwrap().clips[0], Clip::Gap(_)));
    }

    /// Fill the TRACK TAIL (past the last clip): pads up to at_ms then appends.
    #[test]
    fn fills_track_tail() {
        let mut p = Project::new("t", ProjectSettings::default());
        register_asset(&mut p, "a1", 8000);
        register_asset(&mut p, "a2", 20000);
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(make_media_clip("c1", "a1", 0, 5000)));
        // Fill at 5000 (the track end) — appends a 3000ms speed-fit clip.
        fit_to_fill(&mut p, "v1", 5000, 3000, "a2", 0, 6000).expect("fit tail");
        assert_eq!(p.track("v1").unwrap().duration_ms(), 8000);
        assert_eq!(p.track("v1").unwrap().clips.len(), 2);
    }

    /// REPLAY-SAFETY: edit.fit_to_fill lowers to one logged op; the placed clip's
    /// id is pinned (added_clip) and the slot/window/speed are recorded, so a log
    /// rebuild reproduces the timeline byte-for-byte.
    #[test]
    fn fit_replays_byte_identical() {
        use crate::{rebuild_from_log, ProjectStore};

        fn actor() -> crate::Actor {
            crate::Actor {
                kind: crate::ActorKind::Agent,
                name: "claude".into(),
                via: "test".into(),
            }
        }
        fn asset(path: &str, dur: u64) -> Asset {
            Asset {
                path: path.into(),
                hash: format!("sha256:{path}"),
                probe: Some(json!({ "duration_ms": dur, "has_audio": false })),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(dir.path(), "demo", None).unwrap();
        let (a1, _) = s
            .record_import(None, asset("/a.mp4", 8000), actor(), None)
            .unwrap();
        let (a2, _) = s
            .record_import(None, asset("/b.mp4", 20000), actor(), None)
            .unwrap();
        assert_eq!((a1.as_str(), a2.as_str()), ("a1", "a2"));

        // Place c1, then a clip far right so a GAP opens between them.
        s.apply(
            "edit.insert",
            json!({ "asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 5000], "ripple": false }),
            actor(),
            None,
        )
        .unwrap();
        s.apply(
            "edit.insert",
            json!({ "asset": "a1", "track": "v1", "at_ms": 8000, "src_range_ms": [0, 2000], "ripple": false }),
            actor(),
            None,
        )
        .unwrap();
        // The gap [5000, 8000) is 3000ms — fit 6000ms of a2 into it (speed 2.0).
        s.apply(
            "edit.fit_to_fill",
            json!({ "track": "v1", "at_ms": 5000, "slot_ms": 3000, "asset": "a2", "src_range_ms": [0, 6000] }),
            actor(),
            None,
        )
        .unwrap();

        let v1 = s.project.track("v1").unwrap();
        assert_eq!(v1.clips.len(), 3, "[c1, fit, c2] — gap consumed in place");
        let Clip::Media(c) = &v1.clips[1] else {
            unreachable!("media")
        };
        assert_eq!(c.asset, "a2");
        assert_eq!(c.speed, 2.0);
        assert_eq!(v1.clips[1].timeline_duration_ms(), 3000);

        let rebuilt = rebuild_from_log(&s.log.read_all().unwrap()).unwrap();
        assert_eq!(
            serde_json::to_string(&rebuilt.tracks).unwrap(),
            serde_json::to_string(&s.project.tracks).unwrap(),
            "rebuild_from_log == live timeline (placed id pinned via added_clip)"
        );
    }
}
