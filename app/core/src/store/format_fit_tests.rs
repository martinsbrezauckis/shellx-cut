//! Item 141: a received project's journal can hold a format larger than Cut
//! makes. Opening keeps the project, reduces the format to the largest frame
//! of the same shape within the limit, and reports it.

use super::*;
use crate::ops::{Actor, ActorKind};

fn actor() -> Actor {
    Actor {
        kind: ActorKind::Agent,
        name: "format-fit-test".into(),
        via: "test".into(),
        request: None,
    }
}

/// Rewrite every record of `verb` in the journal with `edit`, as a crafted
/// project would carry it; drop the caches so open replays the journal.
fn rewrite_journal(dir: &Path, verb: &str, edit: impl Fn(&mut serde_json::Value)) {
    let journal = dir.join("ops.jsonl");
    let text = std::fs::read_to_string(&journal).unwrap();
    let mut out = String::new();
    for line in text.lines() {
        let mut record: serde_json::Value = serde_json::from_str(line).unwrap();
        if record["verb"] == verb {
            edit(&mut record);
        }
        out.push_str(&serde_json::to_string(&record).unwrap());
        out.push('\n');
    }
    std::fs::write(&journal, out).unwrap();
    let _ = std::fs::remove_file(dir.join("project.json"));
    let _ = std::fs::remove_dir_all(dir.join(".history-snapshots"));
}

fn assert_within_limit(store: &ProjectStore) {
    let s = &store.project.settings;
    crate::frame_size::check_project_frame_size(s.width, s.height)
        .unwrap_or_else(|e| panic!("opened format {}x{}: {}", s.width, s.height, e.message));
    assert!(
        s.fps.is_finite() && s.fps > 0.0 && s.fps <= 240.0,
        "fps {}",
        s.fps
    );
    for sequence in &store.project.sequences {
        let q = &sequence.settings;
        crate::frame_size::check_project_frame_size(q.width, q.height).unwrap();
    }
}

#[test]
fn a_replayed_project_format_over_the_area_limit_opens_reduced_and_reported() {
    let root = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::create(root.path(), "fmt", None).unwrap();
    store
        .set_format(Some(7680), Some(4320), None, actor(), None)
        .unwrap();
    let dir = store.dir.clone();
    drop(store);
    rewrite_journal(&dir, "project.format", |record| {
        for key in ["width", "height"] {
            if record["args"].get(key).is_some() {
                record["args"][key] = serde_json::json!(8192);
            }
            if let Some(effects) = record["effects"].as_array_mut() {
                for effect in effects {
                    if effect["detail"].get(key).is_some() {
                        effect["detail"][key] = serde_json::json!(8192);
                    }
                }
            }
        }
    });

    let opened = ProjectStore::open(&dir).expect("the project still opens");
    assert_within_limit(&opened);
    assert_eq!(
        (
            opened.project.settings.width,
            opened.project.settings.height
        ),
        (5948, 5948)
    );
    let fits = &opened.open_health().format_fits;
    assert_eq!(fits.len(), 1, "{fits:?}");
    assert_eq!((fits[0].from.0, fits[0].from.1), (8192, 8192));
    assert!(fits[0].message().contains("8192x8192"));
    // Nothing was written to the journal by opening.
    let journal = std::fs::read_to_string(dir.join("ops.jsonl")).unwrap();
    assert!(journal.contains("8192"));
    // Opening again gives the same result (and the cache written for it matches).
    drop(opened);
    let again = ProjectStore::open(&dir).unwrap();
    assert_eq!(again.project.settings.width, 5948);
}

#[test]
fn a_replayed_project_create_over_the_limit_opens_reduced() {
    let root = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(root.path(), "create", None).unwrap();
    let dir = store.dir.clone();
    drop(store);
    rewrite_journal(&dir, "project.create", |record| {
        record["args"]["settings"]["width"] = serde_json::json!(16384);
        record["args"]["settings"]["height"] = serde_json::json!(16384);
        record["args"]["settings"]["fps"] = serde_json::json!(1.0e9);
    });
    let opened = ProjectStore::open(&dir).expect("the project still opens");
    assert_within_limit(&opened);
    assert_eq!(opened.project.settings.fps, 240.0);
    assert!(!opened.open_health().format_fits.is_empty());
}

#[test]
fn rebuilding_from_the_log_applies_the_same_limit() {
    let root = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(root.path(), "rebuild", None).unwrap();
    let dir = store.dir.clone();
    drop(store);
    rewrite_journal(&dir, "project.create", |record| {
        record["args"]["settings"]["width"] = serde_json::json!(8192);
        record["args"]["settings"]["height"] = serde_json::json!(8192);
    });
    let ops = crate::ops::OpLog::open(&dir.join("ops.jsonl"))
        .unwrap()
        .read_all()
        .unwrap();
    let project = rebuild_from_log(&ops).unwrap();
    crate::frame_size::check_project_frame_size(project.settings.width, project.settings.height)
        .unwrap();
}

#[test]
fn a_live_project_create_refuses_an_out_of_range_frame_rate() {
    let root = tempfile::tempdir().unwrap();
    let settings = ProjectSettings {
        fps: 1.0e9,
        ..ProjectSettings::default()
    };
    let Err(error) = ProjectStore::create(root.path(), "fps", Some(settings)) else {
        panic!("an out-of-range frame rate is refused");
    };
    assert_eq!(error.code, codes::INVALID_ARGS);
    assert!(error.message.contains("fps"));
}

/// Owner decision 2026-11-19: an odd size within the limits is never changed
/// by the app. It opens, replays and rebuilds exactly as stored and no
/// reduction is reported; the odd-size notice asks the person instead.
#[test]
fn an_odd_size_within_the_limits_opens_replays_and_rebuilds_as_stored() {
    for (verb, w, h) in [("project.create", 1281, 721), ("project.format", 641, 361)] {
        let root = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(root.path(), "odd", None).unwrap();
        if verb == "project.format" {
            store
                .set_format(Some(640), Some(360), None, actor(), None)
                .unwrap();
        }
        let dir = store.dir.clone();
        drop(store);
        rewrite_journal(&dir, verb, |record| {
            if verb == "project.create" {
                record["args"]["settings"]["width"] = serde_json::json!(w);
                record["args"]["settings"]["height"] = serde_json::json!(h);
                return;
            }
            for (key, value) in [("width", w), ("height", h)] {
                if record["args"].get(key).is_some() {
                    record["args"][key] = serde_json::json!(value);
                }
                if let Some(effects) = record["effects"].as_array_mut() {
                    for effect in effects {
                        if effect["detail"].get(key).is_some() {
                            effect["detail"][key] = serde_json::json!(value);
                        }
                    }
                }
            }
        });
        let opened = ProjectStore::open(&dir).expect("the odd project opens");
        let s = &opened.project.settings;
        assert_eq!(
            (s.width, s.height),
            (w, h),
            "{verb}: odd size was changed at open"
        );
        assert!(
            opened.open_health().format_fits.is_empty(),
            "{verb}: a reduction was reported"
        );
        let ops = opened.log.read_all().unwrap();
        let rebuilt = rebuild_from_log(&ops).unwrap();
        assert_eq!(
            (rebuilt.settings.width, rebuilt.settings.height),
            (w, h),
            "{verb}: odd size was changed by the log rebuild"
        );
        let skipped = rebuild_skipping(&ops, ops.len()).unwrap();
        assert_eq!((skipped.settings.width, skipped.settings.height), (w, h));
        drop(opened);
        let again = ProjectStore::open(&dir).unwrap();
        assert_eq!(
            (again.project.settings.width, again.project.settings.height),
            (w, h)
        );
    }
}

/// An odd size just over the area limit is beyond the limits: reduced to the
/// largest even frame of the same shape and reported.
#[test]
fn an_odd_size_over_the_limits_is_reduced_to_even_and_reported() {
    let root = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(root.path(), "oddbig", None).unwrap();
    let dir = store.dir.clone();
    drop(store);
    rewrite_journal(&dir, "project.create", |record| {
        record["args"]["settings"]["width"] = serde_json::json!(8191);
        record["args"]["settings"]["height"] = serde_json::json!(4321);
    });
    let opened = ProjectStore::open(&dir).unwrap();
    assert_within_limit(&opened);
    assert_eq!(opened.open_health().format_fits.len(), 1);
}
