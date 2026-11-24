use super::test_actor;
use crate::dispatch::dispatch;
use crate::state::AppState;
use serde_json::json;

// -----------------------------------------------------------------------
// system.doctor / system.fetch_tool
// -----------------------------------------------------------------------

#[cfg(target_os = "linux")]
const FETCH_FIXTURE_CHILD_ENV: &str = "SHELLX_CUT_FETCH_FIXTURE_CHILD";

/// Deadlock guard for the isolated end-to-end fixture child. This is not a
/// product download SLO: the child deliberately runs beside the complete Rust
/// suite, which can saturate the host while Doctor executes its own bounded
/// capability probes. Keep the guard large enough for that supported test
/// topology while still terminating a genuinely stuck fetch.
#[cfg(target_os = "linux")]
const FETCH_FIXTURE_FULL_SUITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// system.doctor returns a well-formed report with the expected cards and
/// no project required (the environment is global).
#[tokio::test]
async fn system_doctor_returns_cards_without_a_project() {
    let state = AppState::new();
    let r = dispatch(&state, "system.doctor", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let rep = r.result.unwrap();
    assert_eq!(rep["schema"], "shellx-cut/doctor/1");
    let ids: Vec<String> = rep["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    for want in ["ffmpeg", "ffprobe", "perception", "judge.claude", "disk"] {
        assert!(
            ids.contains(&want.to_string()),
            "missing card {want}: {ids:?}"
        );
    }
    // refresh:true also works and re-returns a report.
    let r2 = dispatch(
        &state,
        "system.doctor",
        json!({"refresh": true}),
        test_actor(),
    )
    .await;
    assert!(r2.ok);
}

/// fetch_tool rejects an unknown tool id BEFORE creating a job (the
/// registry is the allow-list; never a caller URL).
#[tokio::test]
async fn system_fetch_tool_rejects_unknown_tool() {
    let state = AppState::new();
    let r = dispatch(
        &state,
        "system.fetch_tool",
        json!({"tool": "curl"}),
        test_actor(),
    )
    .await;
    assert!(!r.ok);
    assert_eq!(r.error.unwrap().code, "invalid_args");
}

/// Nothing is downloaded without the person's agreement: without
/// `accept_license` the verb refuses, returns the consent text to show, and
/// starts no job.
#[tokio::test]
async fn system_fetch_tool_refuses_without_consent() {
    if !cut_media::tool_setup::builtin_ffmpeg_download_available() {
        return; // no download here: unsupported_platform answers first
    }
    let state = AppState::new();
    let r = dispatch(
        &state,
        "system.fetch_tool",
        json!({"tool": "ffmpeg"}),
        test_actor(),
    )
    .await;
    assert!(!r.ok, "a download needs consent");
    let error = r.error.unwrap();
    assert_eq!(error.code, cut_core::error_codes::INVALID_ARGS);
    assert!(error.message.contains("agreement"), "{}", error.message);
    assert!(error.cause.contains("GNU GPL version 3"), "{}", error.cause);
    assert!(error
        .suggested_action
        .unwrap()
        .contains("\"accept_license\":true"));
    let jobs = dispatch(&state, "jobs.list", json!({}), test_actor()).await;
    let jobs = jobs.result.unwrap();
    assert!(
        jobs["jobs"].as_array().is_none_or(|j| j.is_empty()),
        "no job: {jobs}"
    );
}

/// FULL fetch path against a LOCAL fixture HTTP server (honest no-network
/// proof of: pinned download -> size + SHA-256 of the archive -> unpack only
/// the pinned programs -> their size + SHA-256 -> atomic install into the
/// toolpath rung-3 dir -> doctor flips the ffmpeg card ok + doctor_updated
/// fires). It then proves a tampered archive at the same address is refused
/// and preserves those installed binaries. Uses the
/// SHELLX_CUT_FETCH_BASE_URL operator/test seam (loopback http) - the
/// "no caller URL" security property is untouched.
///
/// The Linux fixture mutates process-global toolpath env, so the normal-suite test
/// launches an exact-test child process and performs the mutation only there.
/// This keeps concurrent ffmpeg-dependent tests in the parent process isolated.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn system_fetch_tool_full_path_against_local_fixture() {
    if std::env::var_os(FETCH_FIXTURE_CHILD_ENV).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "dispatch::tests::system_tools::system_fetch_tool_full_path_against_local_fixture",
                "--nocapture",
            ])
            .env(FETCH_FIXTURE_CHILD_ENV, "1")
            .status()
            .unwrap();
        assert!(status.success(), "isolated fetch fixture test failed");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();

    // The child proves the built-in fetch/install/doctor transition, not the
    // operator's coding-agent installation. Keep Doctor's agent resolution
    // deterministic so a slow or newly installed host CLI cannot consume this
    // fixture's bounded completion budget. The child process is disposable, so
    // these process-global values cannot affect the parent test suite.
    std::env::set_var("HOME", tmp.path().join("home"));
    std::env::remove_var("USERPROFILE");
    std::env::set_var("PATH", "/usr/bin:/bin");

    // 1. Build a fake ffmpeg build: <root>/ffbuild/bin/{ffmpeg,ffprobe},
    //    each a tiny executable shell script that prints a version banner.
    let build = tmp.path().join("ffbuild");
    let bin = build.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for stem in ["ffmpeg", "ffprobe"] {
        let p = bin.join(stem);
        let script = format!(
            r#"#!/bin/sh
case "$*" in
  *-filters*)
    printf ' T.. subtitles V->V fixture\n T.. vidstabtransform V->V fixture\n T.. zscale V->V fixture\n'
    ;;
  *-encoders*)
    printf ' V....D libx265 V..... fixture\n V....D libvpx-vp9 V..... fixture\n V....D libsvtav1 V..... fixture\n'
    ;;
  *)
    echo "{stem} version N-fixture"
    ;;
esac
"#
        );
        std::fs::write(&p, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    // 2. tar.xz it, then pin that exact archive and its two programs the way
    //    a production release is pinned (the real pins cannot match a fixture).
    let asset = "ffmpeg-fixture-linux64-gpl.tar.xz";
    let archive = tmp.path().join(asset);
    let status = std::process::Command::new("tar")
        .arg("-cJf")
        .arg(&archive)
        .arg("-C")
        .arg(tmp.path())
        .arg("ffbuild")
        .status()
        .unwrap();
    assert!(status.success(), "tar fixture build failed");
    let sha = |bytes: &[u8]| -> &'static str {
        use sha2::{Digest, Sha256};
        Box::leak(hex::encode(Sha256::digest(bytes)).into_boxed_str())
    };
    let bytes = std::fs::read(&archive).unwrap();
    let digest = sha(&bytes);
    let members: Vec<cut_media::tool_setup::PinnedMember> = ["ffmpeg", "ffprobe"]
        .into_iter()
        .map(|stem| {
            let program = std::fs::read(bin.join(stem)).unwrap();
            cut_media::tool_setup::PinnedMember {
                path: Box::leak(format!("ffbuild/bin/{stem}").into_boxed_str()),
                install_as: stem,
                size: program.len() as u64,
                sha256: sha(&program),
            }
        })
        .collect();
    let assets = Box::leak(Box::new([cut_media::tool_setup::PinnedAsset {
        file: asset,
        kind: cut_media::tool_setup::ArchiveKind::TarXz,
        size: bytes.len() as u64,
        sha256: digest,
        members: Box::leak(members.into_boxed_slice()),
    }]));
    crate::fetch::use_release_for_tests(cut_media::tool_setup::PinnedRelease {
        version: "fixture",
        build: "fixture",
        base: "https://example.invalid",
        assets,
        check: cut_media::tool_setup::ProgramCheck::None,
    });

    // 4. Tiny loopback HTTP server serving the fixture dir.
    let serve_dir = tmp.path().to_path_buf();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || fixture_http_serve(listener, &serve_dir));

    // 5. Point fetch at the fixture + install into a scratch appdata.
    let appdata = tmp.path().join("appdata");
    std::env::set_var(
        crate::fetch::ENV_FETCH_BASE_URL,
        format!("http://127.0.0.1:{port}"),
    );
    std::env::set_var("XDG_DATA_HOME", &appdata);
    // Make sure no stray ffmpeg env override interferes with the rung check.
    std::env::remove_var(cut_media::toolpath::ENV_FFMPEG);
    std::env::remove_var(cut_media::toolpath::ENV_FFPROBE);
    std::env::remove_var("SHELLX_CUT_HOME");

    let state = AppState::new();
    // Subscribe to events to catch doctor_updated.
    let mut rx = state.events.subscribe();

    // BEFORE: ffmpeg not in our scratch appdata (and likely not on the test
    // PATH's appdata) - but the box may have a real ffmpeg on PATH, so we
    // assert on the install-dir resolution after, not on before-missing.
    let r = dispatch(
        &state,
        "system.fetch_tool",
        json!({"tool": "ffmpeg", "rationale": "fixture test", "accept_license": true}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let job_id = r.result.unwrap()["job_id"].as_str().unwrap().to_string();

    // Poll with a real deadlock deadline. The child shares host resources with
    // the full server suite; use the explicit isolation budget above rather
    // than treating normal suite contention as a product failure.
    let deadline = tokio::time::Instant::now() + FETCH_FIXTURE_FULL_SUITE_TIMEOUT;
    loop {
        let j = state.jobs.get(&job_id).unwrap();
        match j.state {
            crate::jobs::JobState::Done => {
                let res = j.result.unwrap();
                assert_eq!(res["sha256"].as_str().unwrap(), digest);
                assert_eq!(res["ffmpeg_ok_after"], json!(true));
                let inst = res["installed_dir"].as_str().unwrap();
                assert!(
                    std::path::Path::new(inst)
                        .join("bin")
                        .join("ffmpeg")
                        .is_file(),
                    "installed ffmpeg missing at {inst}"
                );
                break;
            }
            crate::jobs::JobState::Failed => {
                unreachable!("fetch job failed: {:?}", j.error);
            }
            _ if tokio::time::Instant::now() >= deadline => {
                panic!(
                    "fetch job did not complete within {}s: state={:?}, progress={:?}, error={:?}",
                    FETCH_FIXTURE_FULL_SUITE_TIMEOUT.as_secs(),
                    j.state,
                    j.progress,
                    j.error
                );
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
        }
    }

    // doctor_updated fired at least once during the run (capability flip).
    let mut saw_doctor_event = false;
    while let Ok(ev) = rx.try_recv() {
        if matches!(ev, crate::events::Event::DoctorUpdated { .. }) {
            saw_doctor_event = true;
        }
    }
    assert!(
        saw_doctor_event,
        "expected a doctor_updated event after install"
    );

    // If a manually chosen FFmpeg was removed, using the app's own download
    // (system.fetch_tool) clears the stale missing override and restores Ready.
    let stale_chosen = tmp.path().join("stale_chosen_ffmpeg");
    std::fs::copy(bin.join("ffmpeg"), &stale_chosen).unwrap();
    let set_stale = dispatch(
        &state,
        "system.set_ffmpeg",
        json!({"path": stale_chosen}),
        test_actor(),
    )
    .await;
    assert!(set_stale.ok, "{:?}", set_stale.error);
    std::fs::remove_file(&stale_chosen).unwrap();
    let before_repair = dispatch(&state, "system.doctor", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(before_repair["essential_ok"], false);
    let repair_req = dispatch(
        &state,
        "system.fetch_tool",
        json!({"tool": "ffmpeg", "rationale": "repair missing chosen ffmpeg", "accept_license": true}),
        test_actor(),
    )
    .await;
    assert!(repair_req.ok, "{:?}", repair_req.error);
    let repair_job_id = repair_req.result.unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let repair_deadline = tokio::time::Instant::now() + FETCH_FIXTURE_FULL_SUITE_TIMEOUT;
    loop {
        let j = state.jobs.get(&repair_job_id).unwrap();
        match j.state {
            crate::jobs::JobState::Done => {
                assert_eq!(j.result.unwrap()["ffmpeg_ok_after"], json!(true));
                break;
            }
            crate::jobs::JobState::Failed => panic!("repair fetch failed: {:?}", j.error),
            _ if tokio::time::Instant::now() >= repair_deadline => {
                panic!("repair fetch timed out: {:?}", j.state)
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
        }
    }
    let after_repair = dispatch(&state, "system.doctor", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(after_repair["essential_ok"], true);
    let repair_gpu = after_repair["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "gpu-encode")
        .unwrap();
    assert!(repair_gpu["details"]["override_setting"].is_null());

    // A different file at the pinned address (a tampered or replaced archive)
    // must fail before replacing the already verified binaries.
    let installed_bin = cut_media::toolpath::appdata_tools_dir()
        .unwrap()
        .join("ffmpeg")
        .join("bin");
    let old_ffmpeg = std::fs::read(installed_bin.join("ffmpeg")).unwrap();
    let old_ffprobe = std::fs::read(installed_bin.join("ffprobe")).unwrap();
    let mut tampered = bytes.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    std::fs::write(&archive, tampered).unwrap();

    let mismatch = dispatch(
        &state,
        "system.fetch_tool",
        json!({"tool": "ffmpeg", "rationale": "tampered archive fixture", "accept_license": true}),
        test_actor(),
    )
    .await;
    assert!(mismatch.ok, "{:?}", mismatch.error);
    let mismatch_job_id = mismatch.result.unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mismatch_deadline = tokio::time::Instant::now() + FETCH_FIXTURE_FULL_SUITE_TIMEOUT;
    loop {
        let job = state.jobs.get(&mismatch_job_id).unwrap();
        match job.state {
            crate::jobs::JobState::Failed => {
                assert!(
                    format!("{:?}", job.error).contains("SHA-256 differs from the pinned release"),
                    "a tampered archive must be refused: {:?}",
                    job.error
                );
                break;
            }
            crate::jobs::JobState::Done => panic!("checksum-mismatched archive was installed"),
            _ if tokio::time::Instant::now() >= mismatch_deadline => {
                panic!(
                    "checksum-mismatch fetch job did not settle: {:?}",
                    job.state
                )
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
        }
    }
    assert_eq!(
        std::fs::read(installed_bin.join("ffmpeg")).unwrap(),
        old_ffmpeg,
        "checksum mismatch must preserve the existing ffmpeg binary"
    );
    assert_eq!(
        std::fs::read(installed_bin.join("ffprobe")).unwrap(),
        old_ffprobe,
        "checksum mismatch must preserve the existing ffprobe binary"
    );

    // Cleanup env so other tests are unaffected.
    std::env::remove_var(crate::fetch::ENV_FETCH_BASE_URL);
    std::env::remove_var("XDG_DATA_HOME");
    drop(server); // listener dropped with tmp; the thread exits on accept err
}

/// Minimal HTTP/1.0 file server for the fetch fixture: serves any GET path
/// from `dir` with Content-Length (so the downloader's progress + read both
/// work). Single-threaded, one request per accept; loops until the listener
/// is dropped (accept errors -> return).
#[cfg(target_os = "linux")]
fn fixture_http_serve(listener: std::net::TcpListener, dir: &std::path::Path) {
    use std::io::{Read, Write};
    listener.set_nonblocking(false).ok();
    for _ in 0..16 {
        let (mut stream, _) = match listener.accept() {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        // GET /<name> HTTP/1.1
        let path = req
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/")
            .trim_start_matches('/')
            .to_string();
        let file = dir.join(&path);
        match std::fs::read(&file) {
            Ok(body) => {
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
            }
            Err(_) => {
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        }
        let _ = stream.flush();
    }
}

/// A tool removed after a successful scan must not leave the cached doctor or
/// its live clients ready after an operation has named the missing tool.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn missing_tool_refusal_refreshes_cached_doctor() {
    const CHILD: &str = "SHELLX_CUT_MISSING_TOOL_DOCTOR_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "dispatch::tests::system_tools::missing_tool_refusal_refreshes_cached_doctor",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success(), "isolated missing-tool doctor test failed");
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::env::set_var("HOME", tmp.path().join("home"));
    std::env::set_var("XDG_DATA_HOME", tmp.path().join("data"));
    std::env::set_var("SHELLX_CUT_HOME", tmp.path().join("state"));
    std::env::set_var("PATH", &bin);
    std::env::set_var("SHELLX_CUT_FFMPEG_AUTO", "0");
    std::env::remove_var("SHELLX_CUT_FFMPEG_DIR");
    for (stem, key) in [
        ("ffmpeg", "SHELLX_CUT_FFMPEG"),
        ("ffprobe", "SHELLX_CUT_FFPROBE"),
    ] {
        std::env::set_var(key, bin.join(stem));
    }
    let install = || {
        for stem in ["ffmpeg", "ffprobe"] {
            let path = bin.join(stem);
            // Write a fresh file and rename it into place: a background doctor probe from the
            // previous round may still be running the old fixture, and writing into a file that
            // is being executed fails with "Text file busy" on Linux.
            let staged = bin.join(format!(".{stem}.new"));
            std::fs::write(
                &staged,
                format!(
                    "#!/bin/sh\ncase \"$*\" in\n  *-filters*) printf ' T.. subtitles V->V fixture\\n T.. vidstabtransform V->V fixture\\n T.. zscale V->V fixture\\n';;\n  *-encoders*) printf ' V....D libx265 fixture\\n V....D libvpx-vp9 fixture\\n V....D libsvtav1 fixture\\n';;\n  *) echo '{stem} version 9.9-fixture';;\nesac\n"
                ),
            )
            .unwrap();
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::rename(&staged, &path).unwrap();
        }
        cut_media::toolpath::refresh_media_tools();
    };
    let state = AppState::new();
    for (missing, background, wrapped) in [
        ("ffmpeg", false, false),
        ("ffprobe", false, false),
        ("ffmpeg", true, false),
        ("ffprobe", true, false),
        ("ffmpeg", true, true),
        ("ffprobe", true, true),
    ] {
        install();
        let before = dispatch(
            &state,
            "system.doctor",
            json!({"refresh": true}),
            test_actor(),
        )
        .await;
        let report = before.result.as_ref().unwrap();
        assert_eq!(report["essential_ok"], true);
        for card in report["cards"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|card| card["id"] == "ffmpeg" || card["id"] == "ffprobe")
        {
            assert_eq!(card["status"], "ok", "the tool starts fully ready: {card}");
            assert_eq!(card["details"]["runnable"], true);
        }
        let mut events = state.events.subscribe();
        std::fs::remove_file(bin.join(missing)).unwrap();
        if background {
            let jobs = state.jobs.clone();
            let job = jobs.create("probe");
            let path = bin.join(missing);
            jobs.clone().spawn(&job.job_id.clone(), async move {
                let error = tokio::task::spawn_blocking(move || {
                    let mut command = if wrapped {
                        let mut command = std::process::Command::new("/usr/bin/nice");
                        command.args(["-n", "10"]).arg(path);
                        command
                    } else {
                        std::process::Command::new(path)
                    };
                    cut_media::ffmpeg::run_bounded_command(
                        command.arg("-version"),
                        "probe media tool",
                    )
                    .expect_err("the removed tool cannot spawn")
                })
                .await
                .unwrap();
                assert_eq!(error.code, "tool_missing");
                // Ordinary worker threads also need to notify an idle UI.
                std::thread::spawn(move || {
                    if missing == "ffprobe" {
                        jobs.fail_with_result(&job.job_id, error, json!({"count": 0}));
                    } else {
                        jobs.fail(&job.job_id, error);
                    }
                    let error = jobs.get(&job.job_id).unwrap().error.unwrap();
                    assert_eq!(error.message, format!("{missing} is not installed"));
                    let tag = error.requirement.unwrap();
                    assert_eq!(tag.id, missing);
                    assert!(tag.name.is_some());
                    assert_eq!(tag.get.unwrap()["verb"], "system.fetch_tool");
                })
                .join()
                .unwrap();
            });
            // No doctor poll: a background refusal must update an idle UI.
            tokio::time::timeout(std::time::Duration::from_secs(15), async {
                loop {
                    if let crate::events::Event::DoctorUpdated { report } =
                        events.recv().await.unwrap()
                    {
                        assert!(!report.essential_ok);
                        break;
                    }
                }
            })
            .await
            .expect("background refusal must publish doctor_updated without a poll");
        } else {
            let refused = dispatch(
                &state,
                "media.import",
                json!({"path": tmp.path().join("clip.mp4")}),
                test_actor(),
            )
            .await;
            let error = refused.error.expect("operation must refuse");
            assert_eq!(error.code, "tool_missing");
            assert_eq!(error.requirement.unwrap().id, missing);
        }
        let after = dispatch(&state, "system.doctor", json!({}), test_actor()).await;
        let report = after.result.unwrap();
        let card = report["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == missing)
            .unwrap();
        assert_eq!(
            card["status"], "missing",
            "cached report after {missing} refusal: {report}"
        );
        assert_eq!(card["details"]["runnable"], false);
        assert_eq!(report["essential_ok"], false);
        if !background {
            let event = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if let crate::events::Event::DoctorUpdated { report } =
                        events.recv().await.unwrap()
                    {
                        break report;
                    }
                }
            })
            .await
            .expect("live clients must receive the changed doctor");
            assert!(!event.essential_ok);
        }
        eprintln!("PASS {missing} background={background} wrapped={wrapped}: present -> ready -> removed -> tool_missing -> cached missing + doctor_updated");
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn chosen_ffmpeg_removed_is_not_ready_on_next_doctor_read() {
    const CHILD: &str = "SHELLX_CUT_CHOSEN_FFMPEG_REMOVED_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // The isolated run normally takes seconds. A run that never ends must
        // fail the test (with the child's thread states), not stall the whole
        // check until the runner's own time limit.
        const CHILD_LIMIT: std::time::Duration = std::time::Duration::from_secs(600);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "dispatch::tests::system_tools::chosen_ffmpeg_removed_is_not_ready_on_next_doctor_read",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if started.elapsed() > CHILD_LIMIT {
                let threads = std::fs::read_dir(format!("/proc/{}/task", child.id()))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|task| {
                        let read = |name: &str| {
                            std::fs::read_to_string(task.path().join(name))
                                .unwrap_or_default()
                                .trim()
                                .replace('\n', " ")
                        };
                        format!(
                            "{} {} wchan={}",
                            task.file_name().to_string_lossy(),
                            read("comm"),
                            read("wchan")
                        )
                    })
                    .collect::<Vec<_>>();
                child.kill().unwrap();
                child.wait().unwrap();
                panic!(
                    "isolated chosen-ffmpeg test did not finish within {CHILD_LIMIT:?}; child threads: {threads:?}"
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert!(status.success(), "isolated chosen-ffmpeg test failed");
        return;
    }
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let custom_dir = tmp.path().join("custom");
    let path_dir = tmp.path().join("path_bin");
    let calls_log = tmp.path().join("calls.log");
    std::fs::create_dir_all(&custom_dir).unwrap();
    std::fs::create_dir_all(&path_dir).unwrap();

    std::env::set_var("HOME", tmp.path().join("home"));
    std::env::set_var("XDG_DATA_HOME", tmp.path().join("data"));
    std::env::set_var("SHELLX_CUT_HOME", tmp.path().join("state"));
    std::env::set_var("PATH", &path_dir);
    std::env::set_var("SHELLX_CUT_FFMPEG_AUTO", "0");
    std::env::remove_var("SHELLX_CUT_FFMPEG");
    std::env::remove_var("SHELLX_CUT_FFPROBE");
    std::env::remove_var("SHELLX_CUT_FFMPEG_DIR");

    let write_fake = |dir: &std::path::Path, stem: &str, log: Option<&std::path::Path>| {
        let p = dir.join(stem);
        let log_line = log
            .map(|l| format!("echo \"{stem}:$*\" >> '{}'\n", l.display()))
            .unwrap_or_default();
        let script = format!(
            "#!/bin/sh\n{log_line}case \"$*\" in\n  *-filters*) printf ' .. ass V->V x\\n .. vidstabtransform V->V x\\n .S zscale V->V x\\n';;\n  *-encoders*) printf ' V....D libx265 x\\n V....D libvpx-vp9 x\\n V....D libsvtav1 x\\n';;\n  *) echo '{stem} version 9.0.2-chosen-fixture';;\nesac\n"
        );
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    };

    // Provide a fallback ffmpeg+ffprobe on PATH too, so we verify a deleted
    // manual choice is NOT silently hidden by a fallback binary.
    write_fake(&path_dir, "ffmpeg", Some(&calls_log));
    write_fake(&path_dir, "ffprobe", Some(&calls_log));
    let chosen_ffmpeg = write_fake(&custom_dir, "ffmpeg", Some(&calls_log));
    let _chosen_ffprobe = write_fake(&custom_dir, "ffprobe", None);

    let state = AppState::new();
    // Cold start: system.set_ffmpeg before any system.doctor call updates the
    // toolpath cache immediately and takes effect on the returned doctor report.
    let cold_set = dispatch(
        &state,
        "system.set_ffmpeg",
        json!({"path": chosen_ffmpeg}),
        test_actor(),
    )
    .await;
    assert!(
        cold_set.ok,
        "cold system.set_ffmpeg failed: {:?}",
        cold_set.error
    );
    assert_eq!(
        cold_set.result.as_ref().unwrap()["doctor"]["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "ffmpeg")
            .unwrap()["details"]["resolved"],
        chosen_ffmpeg.display().to_string()
    );

    let clear_cold = dispatch(
        &state,
        "system.set_ffmpeg",
        json!({"path": null}),
        test_actor(),
    )
    .await;
    assert!(clear_cold.ok, "{:?}", clear_cold.error);

    // Simulate normal app startup having populated the doctor cache first.
    let init_doc = dispatch(&state, "system.doctor", json!({}), test_actor()).await;
    assert!(init_doc.ok, "{:?}", init_doc.error);

    let set = dispatch(
        &state,
        "system.set_ffmpeg",
        json!({"path": chosen_ffmpeg}),
        test_actor(),
    )
    .await;
    assert!(set.ok, "system.set_ffmpeg failed: {:?}", set.error);

    // 1. Chosen path exists -> ready as today.
    let before = dispatch(&state, "system.doctor", json!({}), test_actor()).await;
    assert!(before.ok, "{:?}", before.error);
    let before_rep = before.result.unwrap();
    assert_eq!(before_rep["essential_ok"], true);
    let before_ffmpeg = before_rep["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "ffmpeg")
        .unwrap();
    assert_eq!(before_ffmpeg["status"], "ok");
    assert_eq!(before_ffmpeg["details"]["runnable"], true);
    assert_eq!(
        before_ffmpeg["details"]["resolved"],
        chosen_ffmpeg.display().to_string()
    );
    assert_eq!(before_ffmpeg["details"]["selection"]["reason"], "chosen");

    // 2. Remove the chosen FFmpeg file -> next cached read (NO refresh) notices
    // without spawning any binary probe.
    let mut events = state.events.subscribe();
    std::fs::remove_file(&chosen_ffmpeg).unwrap();
    let calls_before_read = std::fs::read_to_string(&calls_log)
        .unwrap_or_default()
        .lines()
        .count();

    let (after, concurrent_second) = tokio::join!(
        dispatch(&state, "system.doctor", json!({}), test_actor()),
        dispatch(&state, "system.doctor", json!({}), test_actor()),
    );
    assert!(after.ok, "{:?}", after.error);
    assert!(concurrent_second.ok, "{:?}", concurrent_second.error);
    let after_rep = after.result.unwrap();
    let second_rep = concurrent_second.result.unwrap();
    assert_eq!(
        second_rep["essential_ok"], false,
        "concurrent second doctor read must also see missing chosen ffmpeg"
    );

    let calls_after_read = std::fs::read_to_string(&calls_log)
        .unwrap_or_default()
        .lines()
        .count();
    assert_eq!(
        calls_after_read, calls_before_read,
        "cached system.doctor read must NOT probe binaries"
    );

    assert_eq!(
        after_rep["essential_ok"], false,
        "missing chosen ffmpeg must make essential_ok false: {after_rep}"
    );
    let after_ffmpeg = after_rep["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "ffmpeg")
        .unwrap();
    assert_eq!(after_ffmpeg["status"], "missing");
    assert_eq!(after_ffmpeg["source"], "missing");
    assert_eq!(after_ffmpeg["details"]["runnable"], false);
    assert_eq!(after_ffmpeg["details"]["chosen_missing"], true);
    let hint = after_ffmpeg["hint"].as_str().unwrap_or("");
    assert!(
        hint.contains(
            "The chosen video tool file is missing; choose it again or use the app's own download"
        ),
        "unexpected person hint: {hint}"
    );
    let reason = after_ffmpeg["details"]["reason"].as_str().unwrap_or("");
    assert!(!reason.is_empty(), "agent reason must be present");
    let action = after_ffmpeg["details"]["action"]
        .as_str()
        .or_else(|| after_ffmpeg["details"]["suggested_action"].as_str())
        .unwrap_or("");
    assert!(
        action.contains("system.set_ffmpeg"),
        "agent action must name system.set_ffmpeg: {action}"
    );

    let req_ffmpeg = after_rep["requirements"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "ffmpeg")
        .unwrap();
    assert_eq!(req_ffmpeg["state"], "missing");
    assert!(req_ffmpeg["reason"]
        .as_str()
        .unwrap_or("")
        .contains("chosen video tool file is missing"));

    let feature_check = dispatch(
        &state,
        "system.doctor",
        json!({"feature": "render.final"}),
        test_actor(),
    )
    .await;
    assert_eq!(feature_check.result.unwrap()["can_run"], "no");

    let gpu = after_rep["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "gpu-encode")
        .unwrap();
    assert_eq!(gpu["details"]["ffmpeg_missing"], true);
    assert_eq!(
        gpu["details"]["override_setting"],
        chosen_ffmpeg.display().to_string()
    );

    // doctor_updated event fires on the transition.
    let ev = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let crate::events::Event::DoctorUpdated { report } = events.recv().await.unwrap() {
                break report;
            }
        }
    })
    .await
    .expect("doctor_updated should fire when chosen ffmpeg goes missing");
    assert!(!ev.essential_ok);

    // A second cached read does not fire a duplicate event and stays missing without probing.
    let second = dispatch(&state, "system.doctor", json!({}), test_actor()).await;
    assert_eq!(second.result.unwrap()["essential_ok"], false);
    assert!(
        events.try_recv().is_err(),
        "second cached read must not emit duplicate doctor_updated"
    );
    assert_eq!(
        std::fs::read_to_string(&calls_log)
            .unwrap_or_default()
            .lines()
            .count(),
        calls_before_read
    );

    // A full re-scan while the chosen file is still missing also reports it missing
    // (must not silently fall back to PATH ffmpeg and say Ready).
    let rescanned = dispatch(
        &state,
        "system.doctor",
        json!({"refresh": true}),
        test_actor(),
    )
    .await;
    let rescan_rep = rescanned.result.unwrap();
    assert_eq!(rescan_rep["essential_ok"], false);
    let rescan_ffmpeg = rescan_rep["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "ffmpeg")
        .unwrap();
    assert_eq!(rescan_ffmpeg["status"], "missing");
    assert_eq!(rescan_ffmpeg["details"]["chosen_missing"], true);

    // Restoring the chosen file makes the next cached read notice and recover.
    write_fake(&custom_dir, "ffmpeg", Some(&calls_log));
    let restored = dispatch(&state, "system.doctor", json!({}), test_actor()).await;
    let restored_rep = restored.result.unwrap();
    assert_eq!(restored_rep["essential_ok"], true);
    let restored_ffmpeg = restored_rep["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "ffmpeg")
        .unwrap();
    assert_eq!(restored_ffmpeg["status"], "ok");

    // Replacing the chosen file with a directory is also treated as missing.
    std::fs::remove_file(&chosen_ffmpeg).unwrap();
    std::fs::create_dir_all(&chosen_ffmpeg).unwrap();
    let dir_rep = dispatch(&state, "system.doctor", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(dir_rep["essential_ok"], false);

    // Restart while chosen FFmpeg is still missing (directory or deleted file):
    // a cold AppState startup scan also reports chosen_missing instead of falling back.
    let restarted = AppState::new();
    let restarted_rep = dispatch(&restarted, "system.doctor", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(restarted_rep["essential_ok"], false);
    assert_eq!(
        restarted_rep["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "ffmpeg")
            .unwrap()["details"]["chosen_missing"],
        true
    );

    std::fs::remove_dir_all(&chosen_ffmpeg).unwrap();

    // Clearing the choice via system.set_ffmpeg {path: null} returns to automatic/PATH.
    let cleared = dispatch(
        &state,
        "system.set_ffmpeg",
        json!({"path": null}),
        test_actor(),
    )
    .await;
    assert!(cleared.ok, "{:?}", cleared.error);
    let after_clear = dispatch(&state, "system.doctor", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(after_clear["essential_ok"], true);
}
