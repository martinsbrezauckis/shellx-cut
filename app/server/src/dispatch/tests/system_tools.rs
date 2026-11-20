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
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\ncase \"$*\" in\n  *-filters*) printf ' T.. subtitles V->V fixture\\n T.. vidstabtransform V->V fixture\\n T.. zscale V->V fixture\\n';;\n  *-encoders*) printf ' V....D libx265 fixture\\n V....D libvpx-vp9 fixture\\n V....D libsvtav1 fixture\\n';;\n  *) echo '{stem} version 9.9-fixture';;\nesac\n"
                ),
            )
            .unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        cut_media::toolpath::refresh_media_tools();
    };
    let state = AppState::new();
    for (missing, background) in [
        ("ffmpeg", false),
        ("ffprobe", false),
        ("ffmpeg", true),
        ("ffprobe", true),
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
                    cut_media::ffmpeg::run_bounded_command(
                        std::process::Command::new(path).arg("-version"),
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
        eprintln!("PASS {missing} background={background}: present -> ready -> removed -> tool_missing -> cached missing + doctor_updated");
    }
}
