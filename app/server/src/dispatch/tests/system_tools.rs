use super::test_actor;
use crate::dispatch::dispatch;
use crate::state::AppState;
use serde_json::json;

// -----------------------------------------------------------------------
// system.doctor / system.fetch_tool
// -----------------------------------------------------------------------

#[cfg(target_os = "linux")]
const FETCH_FIXTURE_CHILD_ENV: &str = "SHELLX_CUT_FETCH_FIXTURE_CHILD";

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

/// FULL fetch path against a LOCAL fixture HTTP server (honest no-network
/// proof of: pinned-URL download -> sha256 verify vs checksums.sha256 ->
/// extract -> atomic install into the toolpath rung-3 dir -> doctor flips
/// the ffmpeg card ok + doctor_updated fires). Uses the
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
    // 2. tar.xz it (asset name MUST match the registry's linux64 asset).
    let asset = "ffmpeg-master-latest-linux64-gpl.tar.xz";
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

    // 3. checksums.sha256 with the real digest of our archive.
    let bytes = std::fs::read(&archive).unwrap();
    let digest = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(&bytes);
        hex::encode(h.finalize())
    };
    std::fs::write(
        tmp.path().join("checksums.sha256"),
        format!("{digest}  {asset}\n"),
    )
    .unwrap();

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
        json!({"tool": "ffmpeg", "rationale": "fixture test"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let job_id = r.result.unwrap()["job_id"].as_str().unwrap().to_string();

    // Poll with a real deadline. The isolated child shares host resources with
    // the full server suite, so a fixed 5-second iteration budget is too tight
    // under parallel load even though the local fetch normally finishes in ~2s.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
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
                    "fetch job did not complete within 30s: state={:?}, progress={:?}, error={:?}",
                    j.state, j.progress, j.error
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
