use super::*;

async fn other_export_refuses_before_job(verb: &str, args: Value) {
    use crate::{dispatch::dispatch, state::AppState};
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let actor = cut_core::Actor {
        kind: cut_core::ActorKind::Agent,
        name: "test".into(),
        via: "test".into(),
        request: None,
    };
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "memory", "dir": root.path().join("memory.cutproj")}),
        actor.clone(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        store.project.assets.insert("a1".into(), serde_json::from_value(json!({
            "path": Path::new(env!("CARGO_MANIFEST_DIR")).join("../media/tests/fixtures/valid-still.png"), "hash": "h"
        })).unwrap());
        store.project.tracks[0].clips.push(
            serde_json::from_value(json!({
                "id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 1000
            }))
            .unwrap(),
        );
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_TOTAL_MB.with(|m| m.set(None));
        }
    }
    let _reset = Reset;
    TEST_TOTAL_MB.with(|m| m.set(Some(1)));
    let result = dispatch(&state, verb, args, actor).await;
    let error = result
        .error
        .unwrap_or_else(|| panic!("{verb} must refuse before starting: {:?}", result.result));
    assert_eq!(error.code, "out_of_memory", "{verb}: {error:?}");
    assert!(
        error.message.starts_with("this render needs about "),
        "{error:?}"
    );
    assert!(!error.cause.is_empty());
    assert!(error
        .suggested_action
        .unwrap()
        .contains("Make the render lighter"));
    assert!(state
        .jobs
        .list()
        .iter()
        .all(|j| !["bundle", "reframe", "reframe-direct"].contains(&j.kind.as_str())));
    assert_eq!(
        state
            .project
            .read()
            .await
            .as_ref()
            .unwrap()
            .project
            .assets
            .len(),
        1,
        "a refused export must not import an asset"
    );
    let exports = root.path().join("memory.cutproj/exports");
    if exports.exists() {
        assert_eq!(
            std::fs::read_dir(exports).unwrap().count(),
            0,
            "a refused export must not start writing a movie or GIF"
        );
    }
}

#[tokio::test]
async fn range_checks_memory_before_starting() {
    other_export_refuses_before_job(
        "export.range",
        json!({"range_ms": [0, 500], "to_asset": false}),
    )
    .await;
}
#[tokio::test]
async fn gif_checks_memory_before_starting() {
    other_export_refuses_before_job(
        "export.gif",
        json!({"range_ms": [0, 500], "to_asset": false}),
    )
    .await;
}
#[tokio::test]
async fn direct_checks_memory_before_starting() {
    other_export_refuses_before_job("render.direct", json!({})).await;
}
#[tokio::test]
async fn reframe_checks_memory_before_starting() {
    other_export_refuses_before_job("render.reframe", json!({"aspect": "9:16"})).await;
}
#[tokio::test]
async fn bundle_checks_memory_before_starting() {
    other_export_refuses_before_job("render.bundle", json!({"platforms": ["9:16"]})).await;
}

fn est(total: u64, threads: Option<u32>) -> MemoryEstimate {
    MemoryEstimate {
        total_mb: total,
        encoder_threads: threads,
        ..Default::default()
    }
}

fn plan_for<'a>(project: &'a Project, edl: &'a Edl, args: &'a [String]) -> Plan<'a> {
    Plan {
        project,
        edl,
        width: 7680,
        height: 4320,
        video_args: args,
        audio_kbps: 192,
        out: Path::new("/x/exports/render_001.mp4"),
    }
}

fn project_and_edl(duration_ms: u64) -> (Project, Edl) {
    let mut project = Project::new("t", cut_core::ProjectSettings::default());
    project.settings.fps = 60.0;
    let edl = Edl {
        duration_ms,
        ..Default::default()
    };
    (project, edl)
}

/// The 8K software render of the HQ test on a 16 GB machine with 6 GB
/// free: adapt to 4 encoder threads, still short, so warn with numbers.
#[test]
fn short_memory_adapts_then_warns_with_numbers() {
    let (p, e) = project_and_edl(12_000);
    let args: Vec<String> = ["-c:v", "libx264"].iter().map(|s| s.to_string()).collect();
    let machine = Machine {
        total_mb: Some(16 * 1024),
        available_mb: Some(6 * 1024),
        swap_mb: 2048,
        disk_free_mb: Some(500_000),
        limit_mb: None,
    };
    let pf = decide(&plan_for(&p, &e, &args), &machine, Path::new("/x"), |a| {
        if a.iter().any(|x| x == "-threads") {
            est(8_000, Some(4))
        } else {
            est(10_600, Some(8))
        }
    });
    assert_eq!(pf.adapt_threads, Some(4));
    assert!(pf.refusal.is_none());
    assert_eq!(pf.warnings.len(), 1);
    assert_eq!(pf.warnings[0].code, "memory_short");
    assert!(
        pf.warnings[0].message.contains("7.8 GB"),
        "{}",
        pf.warnings[0].message
    );
    assert!(pf.warnings[0].message.contains("HEVC"));
    assert!(
        pf.warnings[0]
            .message
            .contains("even with 4 encoder threads"),
        "{}",
        pf.warnings[0].message
    );
    assert_eq!(pf.json["memory"]["fits"], json!(false));
}

/// The 8K slope of libx264: about 25 GB at 54 threads, 10 GB at 12.
fn x264_8k(a: &[String]) -> MemoryEstimate {
    let n = a
        .windows(2)
        .find(|w| w[0] == "-threads")
        .and_then(|w| w[1].parse::<u64>().ok())
        .unwrap_or(54);
    est(7_000 + 330 * n, Some(n as u32))
}

fn big_machine(available_mb: u64, limit_mb: Option<u64>) -> Machine {
    Machine {
        total_mb: Some(256 * 1024),
        available_mb: Some(available_mb),
        swap_mb: 0,
        disk_free_mb: Some(500_000),
        limit_mb,
    }
}

/// Item 165: with plenty of memory the render keeps libx264's automatic
/// count (pinned as `-threads 0`) and says nothing about threads.
#[test]
fn plenty_of_memory_keeps_the_automatic_thread_count() {
    let (p, e) = project_and_edl(12_000);
    let args: Vec<String> = ["-c:v", "libx264"].iter().map(|s| s.to_string()).collect();
    let pf = decide(
        &plan_for(&p, &e, &args),
        &big_machine(200_000, None),
        Path::new("/x"),
        x264_8k,
    );
    assert_eq!(pf.adapt_threads, Some(0));
    assert!(pf.note.is_none() && pf.warnings.is_empty());
    assert_eq!(pf.json["memory"]["encoder_threads"], json!(54));
    assert_eq!(pf.json["memory"]["automatic_encoder_threads"], json!(54));
    assert_eq!(pf.json["memory"]["adapted_encoder_threads"], Value::Null);
}

/// Short memory: fewer threads only as far as needed, said in the note
/// (not a warning: the export works, slower).
#[test]
fn short_memory_reduces_threads_only_as_far_as_needed() {
    let (p, e) = project_and_edl(12_000);
    let args: Vec<String> = ["-c:v", "libx264"].iter().map(|s| s.to_string()).collect();
    let pf = decide(
        &plan_for(&p, &e, &args),
        &big_machine(16_384, None),
        Path::new("/x"),
        x264_8k,
    );
    assert_eq!(pf.adapt_threads, Some(24));
    assert!(pf.warnings.is_empty(), "{:?}", pf.warnings);
    let note = pf.note.expect("note");
    assert!(
        note.contains("24 of 54 threads") && note.contains("free now"),
        "{note}"
    );
    assert_eq!(pf.json["memory"]["note"], json!(note));
    assert_eq!(pf.json["memory"]["fits"], json!(true));
}

/// The person's own limit (and the small-machine test hook) is obeyed
/// and named.
#[test]
fn memory_limit_lowers_the_budget_and_is_named() {
    let (p, e) = project_and_edl(12_000);
    let args: Vec<String> = ["-c:v", "libx264"].iter().map(|s| s.to_string()).collect();
    let pf = decide(
        &plan_for(&p, &e, &args),
        &big_machine(200_000, Some(12_000)),
        Path::new("/x"),
        x264_8k,
    );
    assert_eq!(pf.adapt_threads, Some(12));
    let note = pf.note.expect("note");
    assert!(note.contains(threads::MEMORY_LIMIT_ENV), "{note}");
    assert_eq!(pf.json["memory"]["limit_mb"], json!(12_000));
}

/// More than memory and swap together: refuse before starting.
#[test]
fn impossible_memory_is_refused_with_the_way_out() {
    let (p, e) = project_and_edl(12_000);
    let args: Vec<String> = ["-c:v", "hevc_nvenc"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let machine = Machine {
        total_mb: Some(8 * 1024),
        available_mb: Some(4 * 1024),
        swap_mb: 1024,
        disk_free_mb: Some(500_000),
        limit_mb: None,
    };
    let pf = decide(&plan_for(&p, &e, &args), &machine, Path::new("/x"), |_| {
        est(28_000, None)
    });
    let err = pf.refusal.expect("refused");
    assert_eq!(err.code, error_codes::OUT_OF_MEMORY);
    assert!(
        err.message.contains("27.3 GB") && err.message.contains("8.0 GB"),
        "{}",
        err.message
    );
    assert!(err.suggested_action.unwrap().contains("lower resolution"));
}

#[test]
fn generic_failure_after_short_memory_names_the_numbers() {
    let res = json!({"memory": {"fits": false, "estimate_mb": 10_600, "available_mb": 6_144}});
    let e = CutError::new(
        error_codes::FFMPEG,
        "ffmpeg exited with exit status: 1",
        "Error while opening encoder",
    );
    let e = note_memory(e, &res);
    assert!(
        e.message
            .contains("about 10.4 GB of memory and about 6.0 GB was free"),
        "{}",
        e.message
    );
    assert!(e.suggested_action.unwrap().contains("HEVC"));
    let ok = json!({"memory": {"fits": true, "estimate_mb": 1, "available_mb": 9}});
    let e = CutError::new(error_codes::FFMPEG, "x", "y");
    assert_eq!(note_memory(e.clone(), &ok), e);
}

/// A 10-minute 8K render on a nearly full drive is refused with how much
/// to free; a merely tight drive is a warning.
#[test]
fn disk_space_refuses_or_warns() {
    let (p, e) = project_and_edl(600_000);
    let args: Vec<String> = ["-c:v", "libx264"].iter().map(|s| s.to_string()).collect();
    let full = Machine {
        total_mb: Some(64 * 1024),
        available_mb: Some(40 * 1024),
        swap_mb: 0,
        disk_free_mb: Some(1024),
        limit_mb: None,
    };
    let pf = decide(&plan_for(&p, &e, &args), &full, Path::new("/vol"), |_| {
        est(10_000, Some(8))
    });
    let err = pf.refusal.expect("refused");
    assert_eq!(err.code, error_codes::NOT_ENOUGH_SPACE);
    assert!(err.message.contains("/vol"), "{}", err.message);
    assert!(err.suggested_action.unwrap().starts_with("Free at least"));

    let out_mb = pf.json["disk"]["estimate_mb"].as_u64().unwrap();
    let tight = Machine {
        disk_free_mb: Some(out_mb),
        ..full
    };
    let pf = decide(&plan_for(&p, &e, &args), &tight, Path::new("/vol"), |_| {
        est(10_000, Some(8))
    });
    assert!(pf.refusal.is_none());
    assert_eq!(pf.warnings[0].code, "disk_short");
}
