//! `edit.trim_edges`, `transcript.remove_fillers` and `clip.candidates` work
//! from the spoken words. With no transcript they used to answer ok with an
//! empty result, which an agent reads as "nothing to cut". They now say why.

use super::captions_refusal::SidecarEnv;
use super::{dispatch, lock_agent_cli_env, narrowing_fixture, test_actor, update_asset};
use serde_json::json;

const THREE: [(&str, serde_json::Value); 3] = [
    ("edit.trim_edges", serde_json::Value::Null),
    ("transcript.remove_fillers", serde_json::Value::Null),
    ("clip.candidates", serde_json::Value::Null),
];

#[tokio::test]
async fn without_captions_the_three_verbs_give_the_shared_refusal() {
    let _guard = lock_agent_cli_env();
    let python_dir = tempfile::tempdir().unwrap();
    std::fs::write(python_dir.path().join("instruments.py"), "").unwrap();
    let _env = SidecarEnv::without_python(python_dir.path());
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    for (verb, _) in THREE {
        let r = dispatch(&state, verb, json!({}), test_actor()).await;
        assert!(!r.ok, "{verb} must not answer ok without any transcript");
        let error = r.error.unwrap();
        assert_eq!(error.code, "sidecar", "{verb}: {error:?}");
        assert_eq!(
            error.message, "Captions and transcription are not installed",
            "{verb}"
        );
        let action = error.suggested_action.unwrap();
        assert!(action.contains("Install captions"), "{verb}: {action}");
        assert!(
            action.contains("system.setup_perception"),
            "{verb}: {action}"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn with_captions_but_no_transcript_they_name_the_verb_that_makes_one() {
    let _guard = lock_agent_cli_env();
    let python_dir = tempfile::tempdir().unwrap();
    std::fs::write(python_dir.path().join("instruments.py"), "").unwrap();
    // A Python that exists: captions count as installed.
    let _env = SidecarEnv::with_python(python_dir.path(), std::path::Path::new("/bin/sh"));
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    for (verb, _) in THREE {
        let r = dispatch(&state, verb, json!({}), test_actor()).await;
        assert!(!r.ok, "{verb} must not answer ok without any transcript");
        let error = r.error.unwrap();
        assert_eq!(error.code, "not_found", "{verb}: {error:?}");
        assert!(
            error.message.contains("transcript yet"),
            "{verb}: {error:?}"
        );
        let action = error.suggested_action.unwrap();
        assert!(action.contains("media.transcribe"), "{verb}: {action}");
    }
    // A named asset that is not in the project is its own plain error.
    let r = dispatch(
        &state,
        "clip.candidates",
        json!({"asset": "nope"}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().message, "no asset 'nope'");
}

#[tokio::test]
async fn a_transcribed_asset_with_nothing_found_stays_ok_and_says_so() {
    let _guard = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset": "a1", "model": "test", "language": "en",
            "words": [],
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |a| {
        a.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();
    let r = dispatch(&state, "transcript.remove_fillers", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.unwrap()["note"], "nothing matched");
    let r = dispatch(
        &state,
        "clip.candidates",
        json!({"min_ms": 600_000, "max_ms": 1_200_000}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let result = r.result.unwrap();
    assert_eq!(result["count"], 0);
    assert!(result["note"]
        .as_str()
        .unwrap()
        .starts_with("nothing found"));
}

const THE_REST: [(&str, serde_json::Value); 6] = [
    ("transcript.remove_retakes", serde_json::Value::Null),
    ("transcript.timeline", serde_json::Value::Null),
    ("verify.delivery", serde_json::Value::Null),
    ("captions.generate", serde_json::Value::Null),
    ("captions.kinetic", serde_json::Value::Null),
    ("transcript.get", serde_json::Value::Null),
];

fn args_for(verb: &str) -> serde_json::Value {
    match verb {
        "captions.kinetic" => json!({"per_word": true}),
        "transcript.get" => json!({"asset": "a1"}),
        _ => json!({}),
    }
}

/// The other verbs that read a transcript give the same truthful answer.
#[tokio::test]
async fn the_other_transcript_verbs_say_why_when_there_is_no_transcript() {
    let _guard = lock_agent_cli_env();
    let python_dir = tempfile::tempdir().unwrap();
    std::fs::write(python_dir.path().join("instruments.py"), "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    {
        let _env = SidecarEnv::without_python(python_dir.path());
        for (verb, _) in THE_REST {
            let r = dispatch(&state, verb, args_for(verb), test_actor()).await;
            assert!(!r.ok, "{verb} must not answer ok without any transcript");
            let error = r.error.unwrap();
            assert_eq!(error.code, "sidecar", "{verb}: {error:?}");
            assert_eq!(
                error.message, "Captions and transcription are not installed",
                "{verb}"
            );
        }
    }
    #[cfg(unix)]
    {
        let _env = SidecarEnv::with_python(python_dir.path(), std::path::Path::new("/bin/sh"));
        for (verb, _) in THE_REST {
            let r = dispatch(&state, verb, args_for(verb), test_actor()).await;
            assert!(!r.ok, "{verb} must not answer ok without any transcript");
            let error = r.error.unwrap();
            assert_eq!(error.code, "not_found", "{verb}: {error:?}");
            assert!(
                error.message.contains("transcript yet"),
                "{verb}: {error:?}"
            );
            let action = error.suggested_action.unwrap();
            assert!(action.contains("media.transcribe"), "{verb}: {action}");
        }
    }
}
