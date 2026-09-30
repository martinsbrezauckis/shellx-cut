use super::super::{chapter_gaps, detect_retakes, retake_tokens, segment_chapters, RetakeKeep};
use super::*;

fn old_distance(a: &[String], b: &[String]) -> usize {
    let mut prev: Vec<_> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ta) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, tb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j + 1] + 1)
                .min(cur[j] + 1)
                .min(prev[j] + usize::from(ta != tb));
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn sequences(alphabet: &[&str], max: usize) -> Vec<Vec<String>> {
    let mut all = vec![Vec::new()];
    let mut level = vec![Vec::new()];
    for _ in 0..max {
        level = level
            .iter()
            .flat_map(|prefix| {
                alphabet.iter().map(move |token| {
                    let mut out = prefix.clone();
                    out.push((*token).to_owned());
                    out
                })
            })
            .collect();
        all.extend(level.clone());
    }
    all
}

#[test]
fn exact_threshold_matches_old_distance_exhaustively() {
    let seqs = sequences(&["rocket", "世界"], 5);
    for a in &seqs {
        for b in &seqs {
            let maxlen = a.len().max(b.len());
            let score = if maxlen == 0 {
                1.0
            } else {
                1.0 - old_distance(a, b) as f64 / maxlen as f64
            };
            let mut thresholds = vec![0.0, 0.1, 0.6, 0.9, 1.0, score];
            if score > 0.0 {
                thresholds.push(f64::from_bits(score.to_bits() - 1));
            }
            if score < 1.0 {
                thresholds.push(f64::from_bits(score.to_bits() + 1));
            }
            for threshold in thresholds {
                assert_eq!(
                    retake_matches(a, b, threshold, &mut RetakeWork::default()).unwrap(),
                    score >= threshold,
                    "{a:?} {b:?} threshold={threshold:?}"
                );
            }
        }
    }
}

fn old_depths(coh: &[f64]) -> Vec<f64> {
    coh.iter()
        .enumerate()
        .map(|(k, &ck)| {
            let (mut lpeak, mut rpeak, mut i, mut j) = (ck, ck, k, k);
            while i > 0 && coh[i - 1] >= lpeak {
                i -= 1;
                lpeak = coh[i];
            }
            while j + 1 < coh.len() && coh[j + 1] >= rpeak {
                j += 1;
                rpeak = coh[j];
            }
            (lpeak - ck) + (rpeak - ck)
        })
        .collect()
}

#[test]
fn chapter_depths_preserve_plateaus_and_valleys_exhaustively() {
    for length in 0..=8 {
        for code in 0usize..3usize.pow(length) {
            let mut digits = code;
            let values: Vec<_> = (0..length)
                .map(|_| {
                    let value = [0.0, 0.3, 1.0][digits % 3];
                    digits /= 3;
                    value
                })
                .collect();
            assert_eq!(cohesion_depths(&values), old_depths(&values));
        }
    }
}

fn words(tokens: &[&str]) -> Vec<cut_perception::WordSpan> {
    tokens
        .iter()
        .enumerate()
        .map(|(i, token)| cut_perception::WordSpan {
            idx: 100 + i,
            word: (*token).into(),
            start_ms: i as u64 * 500,
            end_ms: i as u64 * 500 + 400,
            confidence: None,
            speaker: None,
        })
        .collect()
}

#[test]
fn large_flat_transcript_keeps_chapter_title_and_indices() {
    let transcript = words(&vec!["Rocket"; 200_000]);
    let gaps = chapter_gaps(&transcript, 10);
    assert_eq!(gaps.len(), 19_999);
    assert!(gaps.iter().all(|gap| gap.depth == 0.0));
    let chapters = segment_chapters(&transcript, 50, 20_000);
    assert_eq!(chapters.len(), 1);
    assert_eq!(chapters[0].word_range, [100, 200_099]);
    assert_eq!(
        chapters[0].title,
        "Rocket Rocket Rocket Rocket Rocket Rocket"
    );
}

#[test]
fn long_identical_near_identical_and_mismatched_attempts() {
    let a = vec!["rocket".to_owned(); 100_000];
    let mut b = a.clone();
    let mut zero = RetakeWork::new(0);
    assert!(retake_matches(&a, &b, 1.0, &mut zero).unwrap());
    b[50_000] = "世界".into();
    assert!(retake_matches(&a, &b, 0.6, &mut zero).unwrap());
    b[1] = "orbit".into();
    b[99_998] = "planet".into();
    assert!(retake_matches(&a, &b, 0.6, &mut zero).unwrap());
    let mismatch = vec!["planet".to_owned(); 100_000];
    let mut work = RetakeWork::new(100);
    assert!(!retake_matches(&a, &mismatch, 1.0, &mut work).unwrap());
    assert_eq!(work.remaining, 100);
    let err = retake_matches(&a, &mismatch, 0.6, &mut work).unwrap_err();
    assert_eq!(err.code, error_codes::GUARDRAIL);
}

#[test]
fn budget_counts_actual_cells_across_comparisons() {
    let a = vec!["a".into(), "b".into()];
    let b = vec!["b".into(), "a".into()];
    let mut work = RetakeWork::new(8);
    assert!(!retake_matches(&a, &b, 0.5, &mut work).unwrap());
    assert_eq!(work.remaining, 4);
    assert!(!retake_matches(&a, &b, 0.5, &mut work).unwrap());
    assert_eq!(work.remaining, 0);
    assert_eq!(
        retake_matches(&a, &b, 0.5, &mut work).unwrap_err().code,
        error_codes::GUARDRAIL
    );
}

#[test]
fn unicode_normalization_asides_and_keep_policies_remain_intact() {
    let transcript = words(&[
        "ÉCOLE,", "世界", "Rocket!", "wait.", "école", "世界", "rocket!",
    ]);
    assert_eq!(
        retake_tokens(&transcript, 0, 2),
        vec!["école", "世界", "rocket"]
    );
    for (keep, kept, removed) in [
        (RetakeKeep::First, [0, 2], [4, 6]),
        (RetakeKeep::Last, [4, 6], [0, 2]),
        (RetakeKeep::Longest, [0, 2], [4, 6]),
    ] {
        let clusters =
            detect_retakes(&transcript, 600, 1.0, keep, 3, &mut RetakeWork::new(0)).unwrap();
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].kept, kept);
        assert_eq!(clusters[0].removed, vec![removed]);
    }
}

#[tokio::test]
async fn later_asset_budget_failure_preserves_timeline_and_durable_log() {
    use crate::dispatch::{dispatch, speech_text::transcript_remove_retakes_with_work};
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.path().join("speech.cutproj");
    let state = crate::state::AppState::new();
    let actor = cut_core::Actor {
        kind: cut_core::ActorKind::Agent,
        name: "speech-test".into(),
        via: "test".into(),
        request: None,
    };
    let created = dispatch(
        &state,
        "project.create",
        serde_json::json!({"name":"speech","dir":project_dir}),
        actor.clone(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        for id in ["a1", "a2"] {
            store.record_import(Some(id.into()), cut_core::Asset {
                path: "/fixture/speech.mp4".into(), hash: format!("sha256:{id}"),
                probe: Some(serde_json::json!({"duration_ms":20_000,"kind":"video","has_audio":true})),
                transcript: Some(format!("receipts/{id}.words.json")),
                perception: None, proxy: None, filmstrip: None,
            }, actor.clone(), None).unwrap();
        }
    }
    let inserted = dispatch(&state, "edit.insert", serde_json::json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,10_000],"ripple":false}), actor.clone()).await;
    assert!(inserted.ok, "{:?}", inserted.error);
    for (id, tokens) in [
        (
            "a1",
            vec!["rocket", "launch", "today.", "rocket", "launch", "today."],
        ),
        ("a2", vec!["a", "b", "c.", "b", "c", "a."]),
    ] {
        let transcript = cut_perception::Transcript {
            asset: id.into(),
            model: "fixture".into(),
            language: None,
            words: words(&tokens),
        };
        std::fs::write(
            project_dir.join(format!("receipts/{id}.words.json")),
            serde_json::to_vec(&transcript).unwrap(),
        )
        .unwrap();
    }
    let before_project =
        serde_json::to_vec(&state.project.read().await.as_ref().unwrap().project).unwrap();
    let before_log = std::fs::read(project_dir.join("ops.jsonl")).unwrap();
    let first = detect_retakes(
        &words(&["rocket", "launch", "today.", "rocket", "launch", "today."]),
        600,
        0.6,
        RetakeKeep::Last,
        3,
        &mut RetakeWork::new(0),
    )
    .unwrap();
    assert_eq!(
        first.len(),
        1,
        "first asset has an actionable pending removal"
    );
    let result = transcript_remove_retakes_with_work(
        &state,
        serde_json::json!({}),
        actor.clone(),
        RetakeWork::new(0),
    )
    .await;
    let error = result.expect_err("second asset must exhaust analysis work");
    assert_eq!(error.code, error_codes::GUARDRAIL);
    assert_eq!(
        serde_json::to_vec(&state.project.read().await.as_ref().unwrap().project).unwrap(),
        before_project
    );
    assert_eq!(
        std::fs::read(project_dir.join("ops.jsonl")).unwrap(),
        before_log
    );
    let narrowed = transcript_remove_retakes_with_work(
        &state,
        serde_json::json!({"asset":"a1"}),
        actor,
        RetakeWork::new(0),
    )
    .await
    .unwrap();
    let receipt = narrowed.result.unwrap();
    assert_eq!(receipt["removed_takes"], 1);
    assert_eq!(
        receipt["clusters"][0]["removed"][0]["word_range"],
        serde_json::json!([100, 102])
    );
    assert_eq!(
        receipt["clusters"][0]["kept"]["word_range"],
        serde_json::json!([103, 105])
    );
    assert!(!narrowed.op_ids.unwrap().is_empty());
    assert_ne!(
        std::fs::read(project_dir.join("ops.jsonl")).unwrap(),
        before_log
    );
}
