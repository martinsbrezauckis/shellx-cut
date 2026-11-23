use super::picture_check::{decode_picture_with_deadline, media_check_with};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn project_with_pictures(root: &Path, count: usize) -> AppState {
    let state = AppState::new();
    let created = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name":"pictures", "dir":root.join("pictures.cutproj")}),
        Actor::system(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../media/tests/fixtures/valid-still.png");
    let mut guard = state.project.write().await;
    let store = guard.as_mut().unwrap();
    for i in 0..count {
        let source = root.join(format!("picture-{i:03}.png"));
        std::fs::copy(&fixture, &source).unwrap();
        store.project.assets.insert(
            format!("picture-{i:03}"),
            cut_core::Asset {
                path: source.display().to_string(),
                hash: "fixture".into(),
                probe: Some(json!({"kind":"image"})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
    }
    drop(guard);
    state
}

#[tokio::test]
async fn one_request_starts_at_most_the_picture_limit() {
    let root = tempfile::tempdir().unwrap();
    let state = project_with_pictures(root.path(), 65).await;
    let starts = AtomicUsize::new(0);
    let result = media_check_with(&state, json!({}), |_, _, _| {
        starts.fetch_add(1, Ordering::SeqCst);
        async { Ok(Some(true)) }
    })
    .await
    .unwrap();
    assert!(starts.load(Ordering::SeqCst) <= 64);
    let rows = result.result.unwrap()["assets"].as_array().unwrap().clone();
    assert!(rows.iter().any(|row| row.get("readable").is_none()));
}

#[tokio::test]
async fn unchanged_pictures_do_not_start_another_decode_and_changed_picture_does() {
    let root = tempfile::tempdir().unwrap();
    let state = project_with_pictures(root.path(), 3).await;
    let starts = AtomicUsize::new(0);
    for _ in 0..2 {
        media_check_with(&state, json!({}), |_, _, _| {
            starts.fetch_add(1, Ordering::SeqCst);
            async { Ok(Some(true)) }
        })
        .await
        .unwrap();
    }
    assert_eq!(starts.load(Ordering::SeqCst), 3);
    let changed = root.path().join("picture-001.png");
    let before = std::fs::metadata(&changed).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let bytes = std::fs::read(&changed).unwrap();
    std::fs::write(&changed, bytes).unwrap();
    assert_ne!(
        before,
        std::fs::metadata(&changed).unwrap().modified().unwrap()
    );
    media_check_with(&state, json!({}), |_, _, _| {
        starts.fetch_add(1, Ordering::SeqCst);
        async { Ok(Some(true)) }
    })
    .await
    .unwrap();
    assert_eq!(starts.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn failed_picture_check_is_not_offline_and_is_retried() {
    let root = tempfile::tempdir().unwrap();
    let state = project_with_pictures(root.path(), 1).await;
    let starts = AtomicUsize::new(0);
    for _ in 0..2 {
        let result = media_check_with(&state, json!({}), |_, _, _| {
            starts.fetch_add(1, Ordering::SeqCst);
            async {
                Err(CutError::new(
                    error_codes::TOOL_MISSING,
                    "picture check could not start",
                    "try again",
                ))
            }
        })
        .await
        .unwrap();
        let report = result.result.unwrap();
        assert_eq!(report["offline_count"], 0);
        assert!(report["assets"][0].get("readable").is_none());
    }
    assert_eq!(starts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn unreadable_picture_is_offline_and_its_verdict_is_remembered() {
    let root = tempfile::tempdir().unwrap();
    let state = project_with_pictures(root.path(), 1).await;
    let starts = AtomicUsize::new(0);
    for _ in 0..2 {
        let result = media_check_with(&state, json!({}), |_, _, _| {
            starts.fetch_add(1, Ordering::SeqCst);
            async { Ok(Some(false)) }
        })
        .await
        .unwrap();
        let report = result.result.unwrap();
        assert_eq!(report["offline_count"], 1);
        assert_eq!(report["assets"][0]["readable"], false);
    }
    assert_eq!(starts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn reopening_the_same_project_reuses_picture_verdicts() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = root.path().join("pictures.cutproj");
    let created = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name":"pictures", "dir":dir}),
        Actor::system(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../media/tests/fixtures/valid-still.png");
    {
        let mut guard = state.project.write().await;
        guard
            .as_mut()
            .unwrap()
            .record_import(
                None,
                cut_core::Asset {
                    path: source.display().to_string(),
                    hash: "fixture".into(),
                    probe: Some(json!({"kind":"image"})),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                Actor::system(),
                None,
            )
            .unwrap();
    }
    let starts = AtomicUsize::new(0);
    media_check_with(&state, json!({}), |_, _, _| {
        starts.fetch_add(1, Ordering::SeqCst);
        async { Ok(Some(true)) }
    })
    .await
    .unwrap();
    let reopened =
        crate::dispatch::dispatch(&state, "project.open", json!({"path":dir}), Actor::system())
            .await;
    assert!(reopened.ok, "{:?}", reopened.error);
    media_check_with(&state, json!({}), |_, _, _| {
        starts.fetch_add(1, Ordering::SeqCst);
        async { Ok(Some(true)) }
    })
    .await
    .unwrap();
    assert_eq!(starts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn overlapping_requests_start_only_one_pass() {
    let root = tempfile::tempdir().unwrap();
    let state = project_with_pictures(root.path(), 1).await;
    let starts = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let first = {
        let state = state.clone();
        let starts = starts.clone();
        let entered = entered.clone();
        let release = release.clone();
        tokio::spawn(async move {
            media_check_with(&state, json!({}), move |_, _, _| {
                starts.fetch_add(1, Ordering::SeqCst);
                let entered = entered.clone();
                let release = release.clone();
                async move {
                    entered.notify_one();
                    release.notified().await;
                    Ok(Some(true))
                }
            })
            .await
        })
    };
    entered.notified().await;
    let second = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        media_check_with(&state, json!({}), |_, _, _| {
            starts.fetch_add(1, Ordering::SeqCst);
            async { Ok(Some(true)) }
        }),
    )
    .await;
    release.notify_one();
    first.await.unwrap().unwrap();
    let second = second
        .expect("overlapping request must return without waiting")
        .unwrap();
    assert!(second.result.unwrap()["assets"][0]
        .get("readable")
        .is_none());
    assert_eq!(starts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancelled_request_keeps_the_pass_until_its_decoder_exits() {
    let root = tempfile::tempdir().unwrap();
    let state = project_with_pictures(root.path(), 1).await;
    let starts = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let (release, receiver) = std::sync::mpsc::channel::<()>();
    let receiver = Arc::new(std::sync::Mutex::new(receiver));
    let first = {
        let state = state.clone();
        let starts = starts.clone();
        let entered = entered.clone();
        tokio::spawn(async move {
            media_check_with(&state, json!({}), move |_, _, pass| {
                starts.fetch_add(1, Ordering::SeqCst);
                let entered = entered.clone();
                let receiver = receiver.clone();
                async move {
                    run_blocking("held picture", move || {
                        let _pass = pass;
                        entered.notify_one();
                        receiver
                            .lock()
                            .unwrap()
                            .recv_timeout(std::time::Duration::from_secs(5))
                            .unwrap();
                        Ok(Some(true))
                    })
                    .await
                }
            })
            .await
        })
    };
    entered.notified().await;
    first.abort();
    let _ = first.await;
    let second = media_check_with(&state, json!({}), |_, _, _| {
        starts.fetch_add(1, Ordering::SeqCst);
        async { Ok(Some(true)) }
    })
    .await
    .unwrap();
    assert!(second.result.unwrap()["assets"][0]
        .get("readable")
        .is_none());
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    release.send(()).unwrap();
}

#[tokio::test]
async fn exhausted_pass_budget_does_not_mark_a_valid_picture_unreadable() {
    let state = AppState::new();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../media/tests/fixtures/valid-still.png");
    let pass = state.picture_checks.try_begin().unwrap();
    let deadline = std::time::Instant::now() - std::time::Duration::from_millis(1);
    let verdict = decode_picture_with_deadline(source, deadline, pass)
        .await
        .unwrap();
    assert_eq!(verdict, None);
}

/// Run separately for release timing evidence; the ordinary suite uses the
/// injected no-process counter tests above.
#[tokio::test]
#[ignore = "measured 501-picture engine run"]
async fn measured_many_pictures_find_the_damaged_fixture_in_bounded_passes() {
    let root = tempfile::tempdir().unwrap();
    let state = project_with_pictures(root.path(), 500).await;
    let damaged =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../media/tests/fixtures/original-still.png");
    {
        let mut guard = state.project.write().await;
        guard.as_mut().unwrap().project.assets.insert(
            "picture-500".into(),
            cut_core::Asset {
                path: damaged.display().to_string(),
                hash: "damaged fixture".into(),
                probe: Some(json!({"kind":"image"})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
    }
    let starts = AtomicUsize::new(0);
    let mut damaged_on_call = None;
    for call in 1..=10 {
        let before = starts.load(Ordering::SeqCst);
        let began = std::time::Instant::now();
        let result = media_check_with(&state, json!({}), |source, deadline, pass| {
            starts.fetch_add(1, Ordering::SeqCst);
            decode_picture_with_deadline(source, deadline, pass)
        })
        .await
        .unwrap();
        let elapsed = began.elapsed();
        let count = starts.load(Ordering::SeqCst) - before;
        let rows = result.result.unwrap()["assets"].as_array().unwrap().clone();
        let damaged_row = rows
            .iter()
            .find(|row| row["asset"] == "picture-500")
            .unwrap();
        let valid_unreadable = rows
            .iter()
            .filter(|row| row["asset"] != "picture-500" && row["readable"] == false)
            .count();
        assert_eq!(valid_unreadable, 0);
        println!(
            "call={call} elapsed_ms={} decode_starts={count} damaged_readable={:?}",
            elapsed.as_millis(),
            damaged_row.get("readable")
        );
        if damaged_row["readable"] == false {
            damaged_on_call = Some(call);
            break;
        }
    }
    assert_eq!(damaged_on_call, Some(8));
    let before = starts.load(Ordering::SeqCst);
    media_check_with(&state, json!({}), |source, deadline, pass| {
        starts.fetch_add(1, Ordering::SeqCst);
        decode_picture_with_deadline(source, deadline, pass)
    })
    .await
    .unwrap();
    assert_eq!(starts.load(Ordering::SeqCst) - before, 0);
}
