//! Red-first end-to-end export staging tests across the server verbs:
//! `render.final`, `export.range`, `export.gif`, `render.direct`,
//! `render.reframe`, `render.bundle`, `export.audio`, `export.frame`,
//! `export.otio`, plus receipts and `project.set_output_dir`.

use super::{lock_agent_cli_env, test_actor, wait_job};
use crate::dispatch::dispatch;
use crate::jobs::JobState;
use crate::output_paths::test_fixture::SessionOutputDirFixture;
use crate::state::AppState;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const STAND_IN_TOOL: &str = r#"import json, os, sys
request = json.load(sys.stdin)
if not os.path.exists(str(request["media_path"])):
    sys.exit("media not found")
if request.get("contact_sheet") is not None:
    path = os.path.join(request["contact_sheet"], "contact_sheet.jpg")
    with open(path, "wb") as sheet:
        sheet.write(b"stand-in contact sheet")
    print(json.dumps({"contact_sheet": path, "scenes": []}))
    sys.exit(0)
if request.get("qc_sheet") is not None:
    path = os.path.join(request["qc_sheet"], "qc_sheet.jpg")
    with open(path, "wb") as sheet:
        sheet.write(b"stand-in qc sheet")
    print(json.dumps({"qc_sheet": path, "scenes": []}))
    sys.exit(0)
payload = {
    "schema": "shellx-cut/perception/1",
    "asset_hash": request["asset_hash"],
    "source_path": str(request["media_path"]),
    "instruments_run": request["instruments"],
    "silences": [],
    "black_spans": [],
    "frozen_spans": [],
    "content_bbox": {"frame_width": 320, "frame_height": 180, "x": 0, "y": 0,
                     "width": 320, "height": 180, "uniform_border": False},
    "loudness": {"integrated_lufs": -16.0, "true_peak_dbtp": -12.0},
}
if "subject" in (request.get("instruments") or []):
    payload["subject_track"] = {
        "fps": 15.0,
        "frame_width": 320,
        "frame_height": 180,
        "scenes": [0],
        "frames": [{
            "f": 0, "t_ms": 0, "cx": 0.5, "cy": 0.5,
            "fx1": 0.3, "fy1": 0.1, "fx2": 0.7, "fy2": 0.9,
            "conf": 0.9, "scene": 0, "cls": "person",
        }],
    }
print(json.dumps(payload))
"#;

struct StandInPerception {
    _dir: tempfile::TempDir,
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl StandInPerception {
    fn install() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("instruments.py"), STAND_IN_TOOL).unwrap();
        let python = super::super::adapter_python::find_python_on_path().expect("python");
        let keys = [
            cut_perception::sidecar::ENV_SIDECAR_DIR,
            cut_perception::sidecar::ENV_PYTHON,
            cut_native_runtime_context::CONTEXT_ENV,
        ];
        let previous = keys.iter().map(|k| (*k, std::env::var_os(k))).collect();
        std::env::set_var(cut_perception::sidecar::ENV_SIDECAR_DIR, dir.path());
        std::env::set_var(cut_perception::sidecar::ENV_PYTHON, python);
        std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV);
        Self {
            _dir: dir,
            previous,
        }
    }
}

impl Drop for StandInPerception {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..) {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

struct DirTempWatcher {
    stop: Arc<AtomicBool>,
    seen_temp_names: Arc<Mutex<BTreeSet<String>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl DirTempWatcher {
    fn start(dirs: Vec<PathBuf>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(BTreeSet::new()));
        let stop_c = Arc::clone(&stop);
        let seen_c = Arc::clone(&seen);
        let handle = std::thread::spawn(move || {
            while !stop_c.load(Ordering::Relaxed) {
                for dir in &dirs {
                    scan_tree(dir, &seen_c);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            for dir in &dirs {
                scan_tree(dir, &seen_c);
            }
        });
        Self {
            stop,
            seen_temp_names: seen,
            handle: Some(handle),
        }
    }

    fn finish(mut self) -> Vec<String> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.seen_temp_names
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect()
    }
}

fn scan_tree(dir: &Path, seen: &Arc<Mutex<BTreeSet<String>>>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_reservation = name.ends_with(".shellx-cut-reservation.lock")
            || name.ends_with(".shellx-cut-reserving");
        if !is_reservation
            && (name.starts_with(".cut-ffmpeg-")
                || name.contains(".tmp.")
                || name.ends_with(".src.mp4")
                || name.starts_with(".cut-recorder-export-")
                || name.starts_with(".raw-recording-copy-")
                || name.starts_with(".range_"))
        {
            seen.lock().unwrap().insert(name);
        }
        let path = entry.path();
        if path.is_dir() && !path.is_symlink() {
            scan_tree(&path, seen);
        }
    }
}

fn make_clip(dir: &Path, name: &str, seconds: u32) -> PathBuf {
    let out = dir.join(name);
    let status = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size=320x180:rate=15:duration={seconds}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!(
            "sine=frequency=440:sample_rate=48000:duration={seconds}"
        ))
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-c:a", "aac", "-b:a", "96k"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg clip generation failed");
    out
}

async fn setup_project(seconds: u32) -> (tempfile::TempDir, AppState, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let state = AppState::new();
    let project_dir = root.path().join("staging.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "staging", "dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = make_clip(root.path(), "clip.mp4", seconds);
    let imported = dispatch(
        &state,
        "media.import",
        json!({"path": media, "proxy": false}),
        test_actor(),
    )
    .await;
    assert!(imported.ok, "{:?}", imported.error);
    let job = wait_job(
        &state,
        imported.result.unwrap()["job_id"].as_str().unwrap(),
        60,
    )
    .await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    (root, state, project_dir)
}

/// A reply or receipt names the same final file as before. On Windows the
/// engine names it with the long-path prefix of the project folder (`\\?\C:\...`),
/// so the two names are compared as the files they open.
fn assert_receipt_names(named: &str, expected: &Path) {
    let named_file = std::fs::canonicalize(named).unwrap_or_else(|e| panic!("{named}: {e}"));
    let expected_file =
        std::fs::canonicalize(expected).unwrap_or_else(|e| panic!("{}: {e}", expected.display()));
    assert_eq!(named_file, expected_file, "receipt names {named}");
}

#[tokio::test]
async fn all_export_kinds_stage_privately_and_land_at_expected_paths() {
    let _output_fixture = SessionOutputDirFixture::new();
    let _env = lock_agent_cli_env();
    let _tool = StandInPerception::install();
    let (root, state, project_dir) = setup_project(2).await;

    let chosen_dir = root.path().join("chosen-exports");
    std::fs::create_dir_all(&chosen_dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&chosen_dir, std::fs::Permissions::from_mode(0o775)).unwrap();
    }

    let exports_dir = project_dir.join("exports");
    let receipts_dir = project_dir.join("receipts");
    let watcher = DirTempWatcher::start(vec![exports_dir.clone(), chosen_dir.clone()]);

    // 1. render.final (default project exports + receipt)
    let rf = dispatch(
        &state,
        "render.final",
        json!({"hardware": "off", "preset": "draft"}),
        test_actor(),
    )
    .await;
    assert!(rf.ok, "render.final start: {:?}", rf.error);
    let rf_res = rf.result.unwrap();
    let render_id = rf_res["render_id"].as_str().unwrap().to_string();
    let rf_job = wait_job(&state, rf_res["job_id"].as_str().unwrap(), 120).await;
    assert_eq!(rf_job.state, JobState::Done, "{rf_job:?}");
    let expected_final = exports_dir.join(format!("{render_id}.mp4"));
    assert!(
        expected_final.is_file(),
        "render.final landed at {}",
        expected_final.display()
    );
    let receipt: Value = serde_json::from_str(
        &std::fs::read_to_string(receipts_dir.join(format!("{render_id}.json"))).unwrap(),
    )
    .unwrap();
    assert_receipt_names(receipt["output_path"].as_str().unwrap(), &expected_final);

    // 1b. comment.export (copies render + writes html + manifest beside it)
    let ce = dispatch(&state, "comment.export", json!({}), test_actor()).await;
    assert!(ce.ok, "comment.export: {:?}", ce.error);
    let ce_res = ce.result.unwrap();
    for key in ["path", "manifest_path", "media_path"] {
        let p = PathBuf::from(ce_res[key].as_str().unwrap());
        assert!(p.is_file(), "comment.export {key} missing: {}", p.display());
    }

    // 2. export.range
    let er = dispatch(
        &state,
        "export.range",
        json!({"range_ms": [0, 500], "to_asset": false}),
        test_actor(),
    )
    .await;
    assert!(er.ok, "export.range: {:?}", er.error);
    let er_path = PathBuf::from(er.result.unwrap()["path"].as_str().unwrap());
    assert_receipt_names(
        &er_path.to_string_lossy(),
        &exports_dir.join("range_0_500.mp4"),
    );
    assert!(er_path.is_file());

    // 3. export.gif
    let eg = dispatch(
        &state,
        "export.gif",
        json!({"range_ms": [0, 500], "to_asset": false, "fps": 10, "width": 160}),
        test_actor(),
    )
    .await;
    assert!(eg.ok, "export.gif: {:?}", eg.error);
    let eg_path = PathBuf::from(eg.result.unwrap()["path"].as_str().unwrap());
    assert_receipt_names(
        &eg_path.to_string_lossy(),
        &exports_dir.join("gif_0_500.gif"),
    );
    assert!(eg_path.is_file());

    // 4. export.audio + export.frame + export.otio
    let ea = dispatch(
        &state,
        "export.audio",
        json!({"format": "wav", "to_asset": false}),
        test_actor(),
    )
    .await;
    assert!(ea.ok, "export.audio: {:?}", ea.error);
    let ea_path = PathBuf::from(ea.result.unwrap()["path"].as_str().unwrap());
    assert_receipt_names(&ea_path.to_string_lossy(), &exports_dir.join("audio.wav"));
    assert!(ea_path.is_file());

    let ef = dispatch(
        &state,
        "export.frame",
        json!({"at_ms": 200, "to_asset": false}),
        test_actor(),
    )
    .await;
    assert!(ef.ok, "export.frame: {:?}", ef.error);
    let ef_path = PathBuf::from(ef.result.unwrap()["path"].as_str().unwrap());
    assert_receipt_names(
        &ef_path.to_string_lossy(),
        &exports_dir.join("frame_200.jpg"),
    );
    assert!(ef_path.is_file());

    let eo = dispatch(&state, "export.otio", json!({}), test_actor()).await;
    assert!(eo.ok, "export.otio: {:?}", eo.error);
    let eo_path = PathBuf::from(eo.result.unwrap()["path"].as_str().unwrap());
    assert_receipt_names(
        &eo_path.to_string_lossy(),
        &exports_dir.join("timeline.otio"),
    );
    assert!(eo_path.is_file());

    // 5. render.direct
    let rd = dispatch(&state, "render.direct", json!({}), test_actor()).await;
    assert!(rd.ok, "render.direct: {:?}", rd.error);
    let rd_job = wait_job(&state, rd.result.unwrap()["job_id"].as_str().unwrap(), 120).await;
    assert_eq!(rd_job.state, JobState::Done, "{rd_job:?}");
    let contact_sheet = PathBuf::from(
        rd_job.result.as_ref().unwrap()["contact_sheet"]
            .as_str()
            .unwrap(),
    );
    assert!(contact_sheet.is_file());

    // 6. render.reframe
    let rr = dispatch(
        &state,
        "render.reframe",
        json!({"aspect": "9:16", "direction": {"0": {"mode": "widen"}}}),
        test_actor(),
    )
    .await;
    assert!(rr.ok, "render.reframe start: {:?}", rr.error);
    let rr_res = rr.result.unwrap();
    let reframe_id = rr_res["reframe_id"].as_str().unwrap().to_string();
    let rr_job = wait_job(&state, rr_res["job_id"].as_str().unwrap(), 120).await;
    assert_eq!(rr_job.state, JobState::Done, "{rr_job:?}");
    let reframe_path = exports_dir.join(format!("{reframe_id}.mp4"));
    assert!(reframe_path.is_file());
    let rr_receipt: Value = serde_json::from_str(
        &std::fs::read_to_string(receipts_dir.join(format!("{reframe_id}.json"))).unwrap(),
    )
    .unwrap();
    assert_receipt_names(rr_receipt["output_path"].as_str().unwrap(), &reframe_path);

    // 7. render.bundle
    let rb = dispatch(
        &state,
        "render.bundle",
        json!({"platforms": ["9:16"], "preset": "draft", "range_ms": [0, 500]}),
        test_actor(),
    )
    .await;
    assert!(rb.ok, "render.bundle start: {:?}", rb.error);
    let rb_job = wait_job(&state, rb.result.unwrap()["job_id"].as_str().unwrap(), 120).await;
    assert_eq!(rb_job.state, JobState::Done, "{rb_job:?}");
    let rb_res = rb_job.result.unwrap();
    let manifest_path = PathBuf::from(rb_res["manifest_path"].as_str().unwrap());
    assert!(manifest_path.is_file());
    let plat_clip = PathBuf::from(rb_res["platforms"][0]["path"].as_str().unwrap());
    let plat_thumb = PathBuf::from(rb_res["platforms"][0]["thumb"].as_str().unwrap());
    assert!(plat_clip.is_file());
    assert!(plat_thumb.is_file());

    // 8. Chosen external export folder (0775 group-writable): works & reports final path
    let set_out = dispatch(
        &state,
        "project.set_output_dir",
        json!({"dir": chosen_dir}),
        test_actor(),
    )
    .await;
    assert!(set_out.ok, "set_output_dir: {:?}", set_out.error);
    let canonical_chosen = std::fs::canonicalize(&chosen_dir).unwrap();
    let rf2 = dispatch(
        &state,
        "render.final",
        json!({"hardware": "off", "preset": "draft"}),
        test_actor(),
    )
    .await;
    assert!(rf2.ok, "{:?}", rf2.error);
    let rf2_res = rf2.result.unwrap();
    let render2_id = rf2_res["render_id"].as_str().unwrap().to_string();
    let rf2_job = wait_job(&state, rf2_res["job_id"].as_str().unwrap(), 120).await;
    assert_eq!(rf2_job.state, JobState::Done, "{rf2_job:?}");
    let expected_chosen_final = canonical_chosen.join(format!("{render2_id}.mp4"));
    assert!(expected_chosen_final.is_file());
    let receipt2: Value = serde_json::from_str(
        &std::fs::read_to_string(receipts_dir.join(format!("{render2_id}.json"))).unwrap(),
    )
    .unwrap();
    assert_receipt_names(
        receipt2["output_path"].as_str().unwrap(),
        &expected_chosen_final,
    );

    let eg2 = dispatch(
        &state,
        "export.gif",
        json!({"range_ms": [0, 500], "to_asset": false, "fps": 10, "width": 160}),
        test_actor(),
    )
    .await;
    assert!(eg2.ok, "{:?}", eg2.error);
    let eg2_path = PathBuf::from(eg2.result.unwrap()["path"].as_str().unwrap());
    assert_receipt_names(
        &eg2_path.to_string_lossy(),
        &canonical_chosen.join("gif_0_500.gif"),
    );
    assert!(eg2_path.is_file());

    let seen_temps = watcher.finish();
    assert!(
        seen_temps.is_empty(),
        "temporary/staging files MUST NEVER appear inside export folders: saw {seen_temps:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn server_symlinked_parent_cannot_redirect_export() {
    let _output_fixture = SessionOutputDirFixture::new();
    let _env = lock_agent_cli_env();
    let (root, state, project_dir) = setup_project(1).await;

    let evil_dir = root.path().join("evil-target");
    std::fs::create_dir_all(evil_dir.join("sub")).unwrap();
    let exports_dir = project_dir.join("exports");
    let inside_target = exports_dir.join("inside-target");
    std::fs::create_dir_all(inside_target.join("sub")).unwrap();
    std::os::unix::fs::symlink(&evil_dir, exports_dir.join("link_parent")).unwrap();
    std::os::unix::fs::symlink(&inside_target, exports_dir.join("inside_link")).unwrap();

    for (verb, args) in [
        (
            "render.final",
            json!({"hardware": "off", "preset": "draft", "path": "exports/link_parent/sub/final.mp4"}),
        ),
        (
            "render.final",
            json!({"hardware": "off", "preset": "draft", "path": "exports/inside_link/sub/final.mp4"}),
        ),
        (
            "export.range",
            json!({"range_ms": [0, 500], "to_asset": false, "path": "exports/link_parent/sub/range.mp4"}),
        ),
        (
            "export.range",
            json!({"range_ms": [0, 500], "to_asset": false, "path": "exports/inside_link/sub/range.mp4"}),
        ),
        (
            "export.gif",
            json!({"range_ms": [0, 500], "to_asset": false, "path": "exports/link_parent/sub/clip.gif"}),
        ),
        (
            "export.audio",
            json!({"format": "wav", "to_asset": false, "path": "exports/link_parent/sub/mix.wav"}),
        ),
        (
            "export.frame",
            json!({"at_ms": 100, "to_asset": false, "path": "exports/link_parent/sub/frame.jpg"}),
        ),
        (
            "export.otio",
            json!({"path": "exports/link_parent/sub/timeline.otio"}),
        ),
    ] {
        let r = dispatch(&state, verb, args, test_actor()).await;
        if let Some(job_id) = r
            .result
            .as_ref()
            .and_then(|v| v.get("job_id"))
            .and_then(|v| v.as_str())
        {
            let job = wait_job(&state, job_id, 60).await;
            assert_eq!(
                job.state,
                JobState::Failed,
                "{verb} must refuse symlinked parent: {job:?}"
            );
        } else {
            assert!(!r.ok, "{verb} must refuse symlinked parent: {r:?}");
        }
    }

    let entries: Vec<_> = std::fs::read_dir(evil_dir.join("sub"))
        .unwrap()
        .flatten()
        .chain(
            std::fs::read_dir(inside_target.join("sub"))
                .unwrap()
                .flatten(),
        )
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        entries.is_empty(),
        "symlinked parent must receive zero exported files: {entries:?}"
    );
}

#[tokio::test]
async fn concurrent_exports_do_not_collide_or_touch_export_folder_with_temps() {
    let _output_fixture = SessionOutputDirFixture::new();
    let _env = lock_agent_cli_env();
    let (_root, state, project_dir) = setup_project(1).await;
    let exports_dir = project_dir.join("exports");
    std::fs::create_dir_all(&exports_dir).unwrap();
    let watcher = DirTempWatcher::start(vec![exports_dir.clone()]);

    let (r1, r2, f1, a1) = tokio::join!(
        dispatch(
            &state,
            "export.range",
            json!({"range_ms": [0, 400], "to_asset": false, "path": "exports/concurrent_a.mp4"}),
            test_actor(),
        ),
        dispatch(
            &state,
            "export.range",
            json!({"range_ms": [200, 700], "to_asset": false, "path": "exports/concurrent_b.mp4"}),
            test_actor(),
        ),
        dispatch(
            &state,
            "export.frame",
            json!({"at_ms": 100, "to_asset": false, "path": "exports/concurrent_frame.jpg"}),
            test_actor(),
        ),
        dispatch(
            &state,
            "export.audio",
            json!({"format": "wav", "to_asset": false, "path": "exports/concurrent_mix.wav"}),
            test_actor(),
        ),
    );
    assert!(r1.ok, "concurrent range a: {:?}", r1.error);
    assert!(r2.ok, "concurrent range b: {:?}", r2.error);
    assert!(f1.ok, "concurrent frame: {:?}", f1.error);
    assert!(a1.ok, "concurrent audio: {:?}", a1.error);

    let temps = watcher.finish();
    assert!(
        temps.is_empty(),
        "concurrent exports leaked temps: {temps:?}"
    );
    for name in [
        "concurrent_a.mp4",
        "concurrent_b.mp4",
        "concurrent_frame.jpg",
        "concurrent_mix.wav",
    ] {
        let p = exports_dir.join(name);
        assert!(p.is_file(), "missing concurrent output {}", p.display());
    }
}

#[tokio::test]
async fn server_cancelled_render_leaves_zero_partial_or_temp_files() {
    // Every test in this process shares one export staging folder, and other export tests run at the
    // same time: run in a child process with a staging folder of its own so only this render counts.
    const CHILD: &str = "SHELLX_CUT_CANCELLED_RENDER_STAGING_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let home = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "dispatch::tests::export_private_staging::server_cancelled_render_leaves_zero_partial_or_temp_files",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("SHELLX_CUT_HOME", home.path())
            .status()
            .unwrap();
        assert!(status.success(), "isolated cancelled-render test failed");
        return;
    }
    let _output_fixture = SessionOutputDirFixture::new();
    let _env = lock_agent_cli_env();
    // Long enough that the render is still writing when it is cancelled, on a
    // fast box as well as a loaded one.
    let (_root, state, project_dir) = setup_project(120).await;
    let exports_dir = project_dir.join("exports");

    let rf = dispatch(
        &state,
        "render.final",
        json!({"hardware": "off", "preset": "high"}),
        test_actor(),
    )
    .await;
    assert!(rf.ok, "{:?}", rf.error);
    let job_id = rf.result.unwrap()["job_id"].as_str().unwrap().to_string();

    for _ in 0..200 {
        if state.jobs.get(&job_id).is_some_and(|j| j.progress > 0.02) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let cancel = dispatch(
        &state,
        "jobs.cancel",
        json!({"job_id": job_id}),
        test_actor(),
    )
    .await;
    if let Some(error) = cancel.error.as_ref() {
        // A busy box may not end the render step within the cancel's 1 s; the
        // job then ends as cancelled when the step ends (the documented answer).
        assert_eq!(
            error.code,
            "job_cancel_pending",
            "{error:?}; job: {:?}",
            state.jobs.get(&job_id)
        );
    }
    let record = wait_job(&state, &job_id, 120).await;
    assert_eq!(record.state, JobState::Failed);

    let remaining: Vec<_> = std::fs::read_dir(&exports_dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        remaining.is_empty(),
        "cancelled render must leave zero files in exports: {remaining:?}"
    );
    if let Some(cut_home) = std::env::var_os("SHELLX_CUT_HOME").map(PathBuf::from) {
        let staging = cut_home.join("export-staging");
        let list = || -> Vec<String> {
            std::fs::read_dir(&staging)
                .map(|it| {
                    it.flatten()
                        .map(|e| e.file_name().to_string_lossy().to_string())
                        .collect()
                })
                .unwrap_or_default()
        };
        // The job can read as ended a moment before the cancelled render step has removed its
        // staging folder; allow that moment, never a folder that stays.
        let mut staged = list();
        for _ in 0..100 {
            if staged.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            staged = list();
        }
        assert!(
            staged.is_empty(),
            "cancelled render must remove private staging: {staged:?}"
        );
    }
}
