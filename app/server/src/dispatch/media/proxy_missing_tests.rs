use super::*;
use crate::jobs::JobState;
use std::os::unix::fs::PermissionsExt;

/// A setup read probes every ffmpeg on the machine, not only the chosen one, so
/// on a Mac that has Homebrew's ffmpeg a read can take longer than 15 seconds
/// (its VideoToolbox probes run to their own bound inside a virtual machine).
/// The wait is for the refresh to arrive, not for it to be quick.
const REFRESH_WAIT: std::time::Duration = std::time::Duration::from_secs(180);

#[tokio::test]
async fn a_removed_tool_fails_the_proxy_job_and_refreshes_setup() {
    const CHILD: &str = "SHELLX_CUT_PROXY_MISSING_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "dispatch::media::proxy_missing_tests::a_removed_tool_fails_the_proxy_job_and_refreshes_setup",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("SHELLX_CUT_FFMPEG_AUTO", "0");
    for (stem, key) in [
        ("ffmpeg", "SHELLX_CUT_FFMPEG"),
        ("ffprobe", "SHELLX_CUT_FFPROBE"),
    ] {
        let path = tmp.path().join(stem);
        std::fs::write(
            &path,
            format!("#!/bin/sh\necho '{stem} version 9.9-fixture'\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var(key, path);
    }
    let state = AppState::new();
    let before = state.doctor_rescan().await;
    assert!(before.essential_ok, "{before:?}");
    let mut events = state.events.subscribe();
    std::fs::remove_file(tmp.path().join("ffmpeg")).unwrap();
    let src = tmp.path().join("source.mp4");
    std::fs::write(&src, b"input exists; the removed encoder cannot open it").unwrap();
    let hash = cut_core::hash_file(&src).unwrap();
    let mut store =
        cut_core::ProjectStore::create(&tmp.path().join("proxy.cutproj"), "proxy", None).unwrap();
    store
        .record_import(
            Some("a1".into()),
            cut_core::Asset {
                path: src.to_string_lossy().into_owned(),
                hash: hash.clone(),
                probe: Some(serde_json::json!({"kind":"video","duration_ms":1000})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
            cut_core::Actor::system(),
            None,
        )
        .unwrap();
    let dir = store.dir.clone();
    let proxies = dir.join("proxies");
    *state.project.write().await = Some(store);
    let job_id = spawn_proxy_chain(
        state.clone(),
        "a1".into(),
        src,
        hash,
        proxies.clone(),
        dir,
        "video".into(),
        Some(1000),
    );
    let record = tokio::time::timeout(REFRESH_WAIT, async {
        loop {
            let record = state.jobs.get(&job_id).unwrap();
            if matches!(record.state, JobState::Done | JobState::Failed) {
                break record;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(record.state, JobState::Failed, "{record:?}");
    let error = record.error.unwrap();
    assert_eq!(error.code, cut_core::error_codes::TOOL_MISSING);
    assert_eq!(error.message, "ffmpeg is not installed");
    let tag = error.requirement.unwrap();
    assert_eq!(tag.id, "ffmpeg");
    assert!(tag.name.is_some());
    assert_eq!(tag.get.unwrap()["verb"], "system.fetch_tool");
    assert!(!proxies.join("a1.mp4").exists());
    tokio::time::timeout(REFRESH_WAIT, async {
        loop {
            if let crate::events::Event::DoctorUpdated { report } = events.recv().await.unwrap() {
                assert!(!report.essential_ok);
                break;
            }
        }
    })
    .await
    .expect("failed proxy updates idle setup without a doctor poll");
    assert!(!state.doctor_cached().await.essential_ok);
}
