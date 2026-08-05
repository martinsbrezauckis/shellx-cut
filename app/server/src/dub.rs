//! dub.rs — native AI DUBBING synth + assembly bridge.
//!
//! Role: drive the OmniVoice TTS service (loopback `:9001`) to re-voice a set of
//! ALREADY-TRANSLATED, timestamped segments into a single continuous dubbed WAV
//! (24 kHz mono), each segment time-fit to its slot and placed at its ORIGINAL
//! start time (silence between). This is the ONLY place that talks to the TTS
//! service (network) — the `audio.dub` verb handler in this module triggers it
//! at synth time; the op-log replay and the renderer stay fully OFFLINE (they read the assembled
//! WAV asset, exactly like matte reads the cached alpha).
//!
//! The heavy lifting (HTTP synth + ffmpeg `atempo` time-fit + WAV assembly) runs
//! in the one-shot `dub_runner.py` sidecar (JSON-in on stdin, ONE JSON line out)
//! — the SAME spawn pattern as translate_runner / matte_runner. This module owns
//! only: endpoint/runtime resolution, the runner invocation, the typed receipt,
//! and the `audio.dub` handler that reads transcripts, translates segments, and
//! commits the resulting dub track.
//!
//! Endpoint: `CUT_DUB_ENDPOINT` (default `http://127.0.0.1:9001`) — a loopback
//! TTS service. Mirrors the matte-service loopback contract exactly
//! (`CUT_MATTE_ENDPOINT`).
//!
//! Dependencies: cut_core (CutError), cut_perception (sidecar python resolution),
//! serde. Primary caller: dispatch.rs route arm for `audio.dub`.

use std::path::{Path, PathBuf};

use crate::events::Event;
use crate::state::AppState;
use cut_core::{error_codes, Actor, CutError, InverseOp, VerbResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The TTS service endpoint (loopback). Override with `CUT_DUB_ENDPOINT`.
pub fn endpoint() -> String {
    std::env::var("CUT_DUB_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:9001".to_string())
}

/// The shared TTS secret, when the service runs with `OMNIVOICE_TTS_SECRET` set
/// (the `X-LV-VA-Auth` gate). `None`/empty ⇒ loopback-open (no header sent).
pub fn secret() -> Option<String> {
    std::env::var("OMNIVOICE_TTS_SECRET")
        .ok()
        .filter(|s| !s.is_empty())
}

/// The resolved local-sidecar runtime: the perception python + the one-shot
/// `dub_runner.py` script. Mirrors translate::runtime / matte::runtime.
#[derive(Debug, Clone)]
pub struct Runtime {
    pub python: PathBuf,
    pub script: PathBuf,
}

/// The one-shot dub script (ships beside `instruments.py` in the sidecar payload
/// / tauri resources).
pub fn runner_script() -> PathBuf {
    let (_py, instruments) = cut_perception::sidecar_paths();
    instruments
        .parent()
        .map(|d| d.join("dub_runner.py"))
        .unwrap_or_else(|| PathBuf::from("dub_runner.py"))
}

/// `Some` when the python + the dub script both exist (so the synth bridge is
/// wired). `DUB_RUNNER_PY` / `DUB_RUNNER_SCRIPT` override the python / script
/// (dev → the venv + repo script). The python only needs the stdlib + ffmpeg on
/// PATH (the runner is stdlib-only), so the perception sidecar python is reused.
pub fn runtime() -> Option<Runtime> {
    let python = std::env::var_os("DUB_RUNNER_PY")
        .map(PathBuf::from)
        .unwrap_or_else(|| cut_perception::sidecar_paths().0);
    let script = std::env::var_os("DUB_RUNNER_SCRIPT")
        .map(PathBuf::from)
        .unwrap_or_else(runner_script);
    (python.exists() && script.exists()).then_some(Runtime { python, script })
}

/// audio.dub{asset, target_lang, voice?, source_lang?, backend?, model?,
/// timeout_ms?, rationale?} — native AI DUBBING: re-voice an asset's speech into
/// `target_lang` in a cloned `voice` (default "rebeka"), time-fit to the original,
/// added as a NEW audio track at the original segment timings. The original audio
/// is KEPT (a mutable mix) — dubbing ADDS a track, it does not replace.
///
/// Pipeline: read the asset's word-level transcript (it must be transcribed) →
/// group into sentence-ish segments → TRANSLATE each segment into `target_lang`
/// using the CLI-primary transcript.translate path → SYNTHESIZE each translated
/// segment via the OmniVoice TTS service → IMPORT the assembled WAV as an asset
/// → place it as per-segment clips on a fresh `dub<N>` audio track. The network
/// touch happens before the op is recorded, so failure never creates a fake track.
pub(crate) async fn audio_dub(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        asset: Option<String>,
        target_lang: String,
        voice: Option<String>,
        source_lang: Option<String>,
        backend: Option<String>,
        model: Option<String>,
        timeout_ms: Option<u64>,
        rationale: Option<String>,
    }
    let a: Args = crate::dispatch::parse_args(args.clone())?;
    let target_lang = crate::translate::normalize_lang(&a.target_lang);
    if target_lang.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "target_lang is empty",
            "pass the language to dub INTO, e.g. \"lv\"",
        ));
    }
    let voice = a
        .voice
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("rebeka")
        .to_string();

    // Resolve the asset: explicit, else the unique transcribed asset (mirrors
    // transcript.translate / media.diarize).
    let (asset_id, dir): (String, PathBuf) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(crate::dispatch::no_project)?;
        let id = if let Some(id) = a.asset.clone() {
            id
        } else {
            let with_t: Vec<&String> = store
                .project
                .assets
                .iter()
                .filter(|(_, v)| v.transcript.is_some())
                .map(|(k, _)| k)
                .collect();
            match with_t.as_slice() {
                [one] => (*one).clone(),
                [] => {
                    return Err(CutError::new(
                        error_codes::NOT_FOUND,
                        "no transcribed asset in the project",
                        "media.transcribe an asset first (dubbing re-voices its transcript)",
                    ))
                }
                many => {
                    return Err(CutError::new(
                        error_codes::INVALID_ARGS,
                        "several assets have transcripts — name one",
                        format!("pass asset; candidates: {many:?}"),
                    ))
                }
            }
        };
        (id, store.dir.clone())
    };

    // The synth bridge must be wired BEFORE we spend time translating.
    let rt = runtime().ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "the dubbing synth bridge is not available",
            "dub_runner.py or its python (perception sidecar) was not found",
        )
        .with_suggested_action(
            "install the perception sidecar, or set DUB_RUNNER_PY / DUB_RUNNER_SCRIPT (dev)",
        )
    })?;

    // Load the source transcript and split into translation/synth segments.
    let src = crate::dispatch::load_transcript(state, &asset_id).await?;
    if src.words.is_empty() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("asset '{asset_id}' transcript has no words"),
            "nothing to dub (silent footage / 0-word transcript)",
        ));
    }
    let source_lang = a.source_lang.clone().or_else(|| src.language.clone());
    let words_tuples: Vec<(u64, u64, String)> = src
        .words
        .iter()
        .map(|w| (w.start_ms, w.end_ms, w.word.clone()))
        .collect();
    let segments = crate::translate::group_words_into_segments(&words_tuples, 600, 200);
    if segments.is_empty() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("asset '{asset_id}' produced no speech segments"),
            "nothing to dub",
        ));
    }
    let seg_texts: Vec<String> = segments.iter().map(|s| s.text.clone()).collect();

    // Translate + synthesize behind the shared analysis slot. This is a synchronous
    // verb, so it does not create a JobRecord, but it still spawns CLI/Python/ffmpeg
    // work and must not overlap with transcribe/perception/diarize jobs.
    let out_wav = dir
        .join("dub")
        .join(format!("{asset_id}.{target_lang}.wav"));
    let synth_timeout = std::time::Duration::from_millis(
        (segments.len() as u64 * 30_000).clamp(120_000, 1_800_000),
    );
    let (outcome, receipt) = state
        .jobs
        .with_limit("analysis", crate::dispatch::ANALYSIS_MAX_RUNNING, async {
            let outcome = crate::translate::run_translation(
                a.backend.as_deref(),
                source_lang.as_deref(),
                &target_lang,
                &seg_texts,
                a.model.as_deref(),
                a.timeout_ms,
            )
            .await?;

            let dub_segments: Vec<DubSegment> = segments
                .iter()
                .zip(outcome.translations.iter())
                .enumerate()
                .map(|(i, (seg, tr))| DubSegment {
                    i,
                    start_ms: seg.start_ms,
                    slot_ms: seg.end_ms.saturating_sub(seg.start_ms).max(1),
                    text: tr.trim().to_string(),
                })
                .collect();

            let endpoint = endpoint();
            let secret = secret();
            let receipt = synthesize_track(
                &rt,
                &endpoint,
                &voice,
                Some(&target_lang),
                secret.as_deref(),
                24_000,
                &out_wav,
                &dub_segments,
                synth_timeout,
            )
            .await?;
            Ok::<_, CutError>((outcome, receipt))
        })
        .await?;

    // The placeable (non-silent) segments — pure-silence gaps carry no clip.
    let placements: Vec<&DubFit> = receipt
        .segments
        .iter()
        .filter(|s| s.placed_ms > 0)
        .collect();
    if placements.is_empty() {
        return Err(CutError::new(
            error_codes::SIDECAR,
            "dubbing produced no placeable audio",
            "every translated segment was empty or zero-length",
        ));
    }

    // Import the dubbed WAV as a first-class asset (its own import op + probe),
    // skipping the video proxy (audio-only). Mirrors export_frame's sub-import.
    let imp = crate::dispatch::media_import(
        state,
        json!({
            "path": out_wav.display().to_string(),
            "proxy": false,
            "rationale": format!("dubbed audio ({target_lang}, voice {voice}) for {asset_id}"),
        }),
        actor.clone(),
    )
    .await?;
    let dub_asset = imp
        .result
        .as_ref()
        .and_then(|r| r.get("asset_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            CutError::new(
                error_codes::IO,
                "dub: importing the dubbed WAV returned no asset_id",
                "the media.import sub-call did not register the dubbed track",
            )
        })?
        .to_string();

    // Allocate a fresh `dub<N>` audio track (a new dub never reuses a prior one;
    // the original audio is kept → a mutable mix).
    let dub_track = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(crate::dispatch::no_project)?;
        let mut n = 1u32;
        loop {
            let cand = format!("dub{n}");
            if !store.project.tracks.iter().any(|t| t.id == cand) {
                break cand;
            }
            n += 1;
        }
    };

    // Lower to ONE audio.dub op: create the track, then place each segment's
    // clip at its ORIGINAL start (ripple:false — the dub floats; nothing shifts).
    // The dubbed WAV is an ABSOLUTE timeline, so a segment's audio sits at
    // [start, start+placed] in the asset — src_range == the timeline window.
    let mut steps: Vec<InverseOp> = Vec::new();
    steps.push(InverseOp {
        verb: "edit.add_track".into(),
        args: json!({"kind": "audio", "id": dub_track}),
    });
    let mut clips: Vec<Value> = Vec::new();
    for p in &placements {
        let s_in = p.placed_at_ms;
        let s_out = p.placed_at_ms + p.placed_ms;
        steps.push(InverseOp {
            verb: "edit.insert".into(),
            args: json!({
                "asset": dub_asset,
                "track": dub_track,
                "at_ms": p.placed_at_ms,
                "src_range_ms": [s_in, s_out],
                "ripple": false,
            }),
        });
        clips.push(json!({
            "i": p.i,
            "at_ms": p.placed_at_ms,
            "range_ms": [s_in, s_out],
            "slot_ms": p.slot_ms,
            "fit_ratio": p.fit_ratio,
        }));
    }

    // The receipt (surfaced + persisted next to the translated transcript).
    let mean_fit = receipt.mean_fit_ratio();
    let receipt_json = json!({
        "source_lang": source_lang,
        "target_lang": target_lang,
        "voice": voice,
        "model": format!("omnivoice via {} (translate:{}:{})", receipt.endpoint, outcome.backend, outcome.model),
        "translate_backend": outcome.backend,
        "translate_backend_proven": outcome.proven,
        "translate_agent": outcome.agent,
        "translate_warnings": outcome.warnings.clone(),
        "n_segments": receipt.n_segments,
        "n_clips": placements.len(),
        "sample_rate": receipt.sample_rate,
        "total_ms": receipt.total_ms,
        "mean_fit_ratio": mean_fit,
        "compressed_segments": receipt.compressed_count(),
        "segments": receipt.segments,
    });
    let rel = format!("receipts/{asset_id}.{target_lang}.dub.json");
    let receipt_path = dir.join(&rel);
    if let Some(parent) = receipt_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            CutError::new(
                error_codes::IO,
                "could not create dubbing receipt dir",
                format!("{}: {e}", parent.display()),
            )
        })?;
    }
    let receipt_bytes = serde_json::to_vec_pretty(&receipt_json).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "could not serialize dubbing receipt",
            e.to_string(),
        )
    })?;
    std::fs::write(&receipt_path, receipt_bytes).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "could not persist dubbing receipt",
            format!("{}: {e}", receipt_path.display()),
        )
    })?;

    // Commit the placement as ONE audio.dub op (the import was its own op).
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let include_inverse = crate::dispatch::wants_inverse(&args);
    let extra = vec![crate::dispatch::effect(
        Some(&dub_track),
        json!({
            "dub_asset": dub_asset,
            "target_lang": target_lang,
            "voice": voice,
            "n_clips": placements.len(),
            "created_track": true,
        }),
    )];
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(crate::dispatch::no_project)?;
    let op = crate::dispatch::guard_call("audio.dub", || {
        store.apply_lowered("audio.dub", args, actor, rationale, steps, extra)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    Ok(VerbResult::ok_with_ops(
        json!({
            "asset": asset_id,
            "track_id": dub_track,
            "dub_asset": dub_asset,
            "dub_wav": out_wav.display().to_string(),
            "source_lang": source_lang,
            "target_lang": target_lang,
            "voice": voice,
            "n_segments": receipt.n_segments,
            "n_clips": placements.len(),
            "total_ms": receipt.total_ms,
            "sample_rate": receipt.sample_rate,
            "mean_fit_ratio": mean_fit,
            "translate_backend": outcome.backend,
            "translate_model": outcome.model,
            "translate_warnings": outcome.warnings.clone(),
            "receipt": rel,
            "clips": clips,
            "op": crate::dispatch::op_for_result(&op, include_inverse),
        }),
        vec![op_id],
    )
    .with_warnings(crate::dispatch::translation_warnings_to_verb(
        &outcome.warnings,
    )))
}

/// One translated, timestamped segment to dub — the runner input unit. `slot_ms`
/// is the segment's own `[start,end]` span (`target = end_i - start_i`),
/// the TTS `duration` target.
#[derive(Debug, Clone, Serialize)]
pub struct DubSegment {
    pub i: usize,
    pub start_ms: u64,
    pub slot_ms: u64,
    pub text: String,
}

/// Per-segment time-fit receipt (mirrors the matte stats pattern — cheap,
/// model-agnostic, surfaced so the agent sees when a translation was too long to
/// fit cleanly).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DubFit {
    pub i: usize,
    pub start_ms: u64,
    pub slot_ms: u64,
    /// Raw synthesized length BEFORE the `atempo` fit (ms).
    pub synth_ms: f64,
    /// `synth_ms / slot_ms` — how far the raw synth missed the slot (near 1.0 =
    /// the TTS `duration` param hit the target well; the quality signal).
    pub fit_ratio: f64,
    /// The `atempo` factor actually applied (1.0 = no compression — a short synth
    /// left to sit on silence).
    pub atempo: f64,
    /// Absolute timeline ms the segment was placed at (== its original start).
    pub placed_at_ms: u64,
    /// The placed clip's length (ms) after fit + the gap-clamp safety net.
    pub placed_ms: u64,
    /// True when the segment had no translated text (pure silence, no synth call).
    #[serde(default)]
    pub skipped: bool,
}

/// The full dub receipt the runner emits (one JSON line on stdout).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DubReceipt {
    pub out_wav: String,
    pub sample_rate: u32,
    pub voice: String,
    pub endpoint: String,
    pub n_segments: usize,
    pub total_ms: u64,
    pub segments: Vec<DubFit>,
}

impl DubReceipt {
    /// Mean `fit_ratio` over the SYNTHESIZED (non-skipped) segments — the headline
    /// "how close did the TTS land to the slots" number for the result.
    pub fn mean_fit_ratio(&self) -> f64 {
        let synthed: Vec<f64> = self
            .segments
            .iter()
            .filter(|s| !s.skipped)
            .map(|s| s.fit_ratio)
            .collect();
        if synthed.is_empty() {
            return 0.0;
        }
        synthed.iter().sum::<f64>() / synthed.len() as f64
    }

    /// How many segments needed `atempo` compression (a fit signal).
    pub fn compressed_count(&self) -> usize {
        self.segments.iter().filter(|s| s.atempo > 1.0001).count()
    }
}

/// Build the runner stdin JSON (pure — unit-tested). The runner reads this on
/// stdin and writes the dubbed WAV to `out_wav`.
pub fn build_runner_input(
    endpoint: &str,
    voice: &str,
    lang: Option<&str>,
    secret: Option<&str>,
    sample_rate: u32,
    out_wav: &Path,
    segments: &[DubSegment],
) -> serde_json::Value {
    serde_json::json!({
        "endpoint": endpoint,
        "voice": voice,
        "lang": lang,
        "secret": secret,
        "sample_rate": sample_rate,
        "out_wav": out_wav.display().to_string(),
        "segments": segments,
    })
}

/// Parse the runner's single JSON stdout line into a `DubReceipt` (tolerant to
/// leading log lines: take the last `{...}`-looking line). Pure — unit-tested.
pub fn parse_receipt(stdout: &str) -> Option<DubReceipt> {
    for line in stdout.lines().rev() {
        let t = line.trim();
        if t.starts_with('{') {
            if let Ok(r) = serde_json::from_str::<DubReceipt>(t) {
                return Some(r);
            }
        }
    }
    None
}

/// Synthesize + time-fit + assemble the dubbed track by spawning `dub_runner.py`.
/// Writes the continuous dubbed WAV to `out_wav` and returns the typed receipt.
/// This is the NETWORK touch (the TTS service); it runs with NO project lock held
/// (the caller bakes it before recording the op, exactly like matte::ensure_baked).
///
/// `timeout` bounds the whole spawn (all segments synthesize serially behind the
/// service's GPU lock). A service failure / a bad segment fails the verb cleanly
/// — never a fake/empty track.
pub async fn synthesize_track(
    rt: &Runtime,
    endpoint: &str,
    voice: &str,
    lang: Option<&str>,
    secret: Option<&str>,
    sample_rate: u32,
    out_wav: &Path,
    segments: &[DubSegment],
    timeout: std::time::Duration,
) -> Result<DubReceipt, CutError> {
    use tokio::io::AsyncWriteExt;

    if let Some(parent) = out_wav.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            CutError::new(
                error_codes::IO,
                format!("dub: create output dir failed: {e}"),
                "could not create the dubbed-audio output directory",
            )
        })?;
    }

    let input = build_runner_input(
        endpoint,
        voice,
        lang,
        secret,
        sample_rate,
        out_wav,
        segments,
    );
    // The runner reads its own per-segment timeout from the job; give it the
    // wall budget too so a single slow synth can't hang the whole spawn forever.
    let mut input = input;
    if let Value::Object(ref mut m) = input {
        m.insert(
            "timeout_s".into(),
            serde_json::json!((timeout.as_secs_f64() / segments.len().max(1) as f64).max(30.0)),
        );
    }
    let payload = input.to_string();

    let mut command = tokio::process::Command::new(&rt.python);
    command
        .arg(&rt.script)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not start the dub runner: {e}"),
            "the perception python + dub_runner.py must exist",
        )
        .with_suggested_action(
            "install the perception sidecar, or set DUB_RUNNER_PY / DUB_RUNNER_SCRIPT",
        )
    })?;
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(payload.as_bytes()).await {
            let _ = child.kill().await;
            return Err(CutError::new(
                error_codes::IO,
                format!("dub runner stdin write failed: {e}"),
                "dubbing synthesis failed before the runner received its request",
            ));
        }
        if let Err(e) = stdin.shutdown().await {
            let _ = child.kill().await;
            return Err(CutError::new(
                error_codes::IO,
                format!("dub runner stdin close failed: {e}"),
                "dubbing synthesis failed before the runner received a complete request",
            ));
        }
    } else {
        let _ = child.kill().await;
        return Err(CutError::new(
            error_codes::IO,
            "dub runner stdin pipe unavailable",
            "dubbing synthesis failed before the runner received its request",
        ));
    }
    let waited = tokio::time::timeout(timeout, child.wait_with_output()).await;
    let out = match waited {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return Err(CutError::new(
                error_codes::IO,
                format!("the dub runner errored: {e}"),
                "dubbing synthesis failed",
            ))
        }
        Err(_) => {
            return Err(CutError::new(
                error_codes::SIDECAR,
                format!(
                    "dubbing synthesis timed out after {}ms",
                    timeout.as_millis()
                ),
                "the OmniVoice service was slow or stalled",
            )
            .with_suggested_action(
                "raise timeout_ms, dub fewer segments, or check the OmniVoice service",
            ))
        }
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(CutError::new(
            error_codes::SIDECAR,
            format!("dubbing synthesis failed: {}", stderr.trim()),
            format!("the OmniVoice TTS service ({endpoint}) could not synthesize the dub"),
        )
        .with_suggested_action(
            "verify the service is reachable at <CUT_DUB_ENDPOINT>/health, or set \
             CUT_DUB_ENDPOINT to the running dubbing service URL",
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_receipt(&stdout).ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "the dub runner returned no JSON receipt",
            format!("got: {}", stdout.trim()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(i: usize, start: u64, slot: u64, text: &str) -> DubSegment {
        DubSegment {
            i,
            start_ms: start,
            slot_ms: slot,
            text: text.into(),
        }
    }

    #[test]
    fn endpoint_defaults_to_loopback_9001() {
        // No env override in the test process → the documented default.
        if std::env::var("CUT_DUB_ENDPOINT").is_err() {
            assert_eq!(endpoint(), "http://127.0.0.1:9001");
        }
    }

    #[test]
    fn runner_input_has_the_runner_contract_shape() {
        let segs = vec![
            seg(0, 1000, 2840, "Labdien."),
            seg(1, 5000, 3200, "Paldies."),
        ];
        let v = build_runner_input(
            "http://127.0.0.1:9001",
            "rebeka",
            Some("lv"),
            None,
            24000,
            Path::new("/home/u/project/dub.wav"),
            &segs,
        );
        assert_eq!(v["endpoint"], "http://127.0.0.1:9001");
        assert_eq!(v["voice"], "rebeka");
        assert_eq!(v["sample_rate"], 24000);
        assert_eq!(v["out_wav"], "/home/u/project/dub.wav");
        assert!(v["secret"].is_null());
        let arr = v["segments"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["start_ms"], 1000);
        assert_eq!(arr[0]["slot_ms"], 2840);
        assert_eq!(arr[0]["text"], "Labdien.");
        assert_eq!(arr[1]["i"], 1);
    }

    #[test]
    fn parse_receipt_takes_last_json_line_tolerantly() {
        let stdout = "loading...\nprogress 50%\n{\"out_wav\":\"/home/u/project/dub.wav\",\"sample_rate\":24000,\"voice\":\"rebeka\",\"endpoint\":\"http://127.0.0.1:9001\",\"n_segments\":2,\"total_ms\":10982,\"segments\":[{\"i\":0,\"start_ms\":1000,\"slot_ms\":2840,\"synth_ms\":3000.0,\"fit_ratio\":1.0563,\"atempo\":1.0563,\"placed_at_ms\":1000,\"placed_ms\":2820},{\"i\":1,\"start_ms\":5000,\"slot_ms\":3200,\"synth_ms\":2200.0,\"fit_ratio\":0.6875,\"atempo\":1.0,\"placed_at_ms\":5000,\"placed_ms\":2200}]}";
        let r = parse_receipt(stdout).expect("parses the last json line");
        assert_eq!(r.sample_rate, 24000);
        assert_eq!(r.voice, "rebeka");
        assert_eq!(r.n_segments, 2);
        assert_eq!(r.total_ms, 10982);
        assert_eq!(r.segments.len(), 2);
        assert_eq!(r.segments[0].placed_at_ms, 1000);
        assert_eq!(r.segments[1].atempo, 1.0);
        // mean over both fit ratios
        assert!((r.mean_fit_ratio() - (1.0563 + 0.6875) / 2.0).abs() < 1e-9);
        // only segment 0 was compressed (atempo > 1)
        assert_eq!(r.compressed_count(), 1);
        assert!(parse_receipt("no json here").is_none());
    }

    #[test]
    fn mean_fit_ratio_ignores_skipped_segments() {
        let r = DubReceipt {
            out_wav: "/home/u/project/dub.wav".into(),
            sample_rate: 24000,
            voice: "rebeka".into(),
            endpoint: "http://127.0.0.1:9001".into(),
            n_segments: 2,
            total_ms: 5000,
            segments: vec![
                DubFit {
                    i: 0,
                    start_ms: 0,
                    slot_ms: 2000,
                    synth_ms: 2100.0,
                    fit_ratio: 1.05,
                    atempo: 1.05,
                    placed_at_ms: 0,
                    placed_ms: 2000,
                    skipped: false,
                },
                DubFit {
                    i: 1,
                    start_ms: 3000,
                    slot_ms: 1000,
                    synth_ms: 0.0,
                    fit_ratio: 0.0,
                    atempo: 1.0,
                    placed_at_ms: 3000,
                    placed_ms: 0,
                    skipped: true,
                },
            ],
        };
        // The skipped (silent) segment must not drag the mean toward 0.
        assert!((r.mean_fit_ratio() - 1.05).abs() < 1e-9);
    }
}
