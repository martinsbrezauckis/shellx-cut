//! diarize.rs — native SPEAKER DIARIZATION bridge ("who spoke when").
//!
//! Role: drive the Sortformer v2 diarization service (loopback `:9002`) to turn an
//! asset's audio into normalized speaker turns. This is the ONLY place that talks
//! to the diarization service (network) — the `media.diarize` verb (dispatch.rs)
//! triggers it; the op-log replay and the renderer stay fully OFFLINE (the turns are
//! baked into the perception receipt at plan time, exactly like matte bakes alpha).
//!
//! The work (ffmpeg audio extract + the HTTP POST) runs in the one-shot
//! `diarize_runner.py` sidecar (JSON-in on stdin, ONE JSON line out) — the SAME
//! spawn pattern as dub_runner / matte_runner. This module owns only:
//! endpoint/runtime resolution, the runner invocation, and the typed receipt. The
//! word↔speaker alignment + report writes live here too; dispatch only decides
//! when the verb runs and how to publish the resulting job receipt.
//!
//! Endpoint: `CUT_DIARIZE_ENDPOINT` (default `http://127.0.0.1:9002`) — a loopback
//! diarization service. Mirrors the dub-service loopback contract exactly.
//!
//! Dependencies: cut_core (CutError), cut_perception (SpeakerTurn/Diarization +
//! sidecar python resolution), serde. Primary caller: dispatch.rs `media_diarize`.

use std::path::{Path, PathBuf};

use crate::jobs::{run_owned, ProcessControl, ProcessTermination};
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use cut_perception::{Diarization, SpeakerTurn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The diarization service endpoint (loopback). Override with `CUT_DIARIZE_ENDPOINT`.
pub fn endpoint() -> String {
    std::env::var("CUT_DIARIZE_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:9002".to_string())
}

/// The shared diarization secret, when the service is fronted with one (the
/// `X-LV-VA-Auth` gate; the service is loopback-open by default per its design doc).
/// `None`/empty ⇒ no header sent. Forward-compat for when the service is exposed
/// beyond localhost.
pub fn secret() -> Option<String> {
    std::env::var("CUT_DIARIZE_SECRET")
        .ok()
        .filter(|s| !s.is_empty())
}

/// The resolved local-sidecar runtime: the perception python + the one-shot
/// `diarize_runner.py` script. Mirrors dub::Runtime.
#[derive(Debug, Clone)]
pub struct Runtime {
    pub python: PathBuf,
    pub script: PathBuf,
    native_context: bool,
}

/// The one-shot diarize script (ships beside `instruments.py` in the sidecar
/// payload / tauri resources).
pub fn runner_script() -> PathBuf {
    let (_py, instruments) = cut_perception::sidecar_paths();
    instruments
        .parent()
        .map(|d| d.join("diarize_runner.py"))
        .unwrap_or_else(|| PathBuf::from("diarize_runner.py"))
}

/// `Some` when the python + the diarize script both exist (so the bridge is wired).
/// A supplied native context is authoritative and uses the bundled runner beside
/// its admitted `instruments.py`; normal runs keep the `DIARIZE_RUNNER_*` overrides.
pub fn runtime() -> Result<Option<Runtime>, CutError> {
    let sidecar = cut_perception::sidecar_runtime()?;
    Ok(runtime_from_sidecar(
        sidecar,
        std::env::var_os("DIARIZE_RUNNER_PY").map(PathBuf::from),
        std::env::var_os("DIARIZE_RUNNER_SCRIPT").map(PathBuf::from),
    ))
}

fn runtime_from_sidecar(
    sidecar: cut_perception::SidecarRuntime,
    override_python: Option<PathBuf>,
    override_script: Option<PathBuf>,
) -> Option<Runtime> {
    let native_context = sidecar.native_context.is_some();
    runtime_from_parts(
        sidecar.python,
        sidecar.script,
        native_context,
        override_python,
        override_script,
    )
}

fn runtime_from_parts(
    sidecar_python: PathBuf,
    sidecar_script: PathBuf,
    native_context: bool,
    override_python: Option<PathBuf>,
    override_script: Option<PathBuf>,
) -> Option<Runtime> {
    let (python, script) = if native_context {
        let script = sidecar_script
            .parent()
            .map(|dir| dir.join("diarize_runner.py"))
            .unwrap_or_else(|| PathBuf::from("diarize_runner.py"));
        (sidecar_python, script)
    } else {
        (
            override_python.unwrap_or(sidecar_python),
            override_script.unwrap_or_else(runner_script),
        )
    };
    (python.exists() && script.exists()).then_some(Runtime {
        python,
        script,
        native_context,
    })
}

/// media.diarize{asset, max_speakers?} — SPEAKER DIARIZATION ("who spoke when")
/// on an asset as a background job. The handler lives beside the diarization
/// runtime/receipt code so dispatch remains a router instead of the service owner.
pub(crate) async fn media_diarize(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
        max_speakers: Option<u32>,
    }
    let a: Args = crate::dispatch::parse_args(args)?;
    let (dir, receipts, _p) = crate::dispatch::project_paths(state).await?;
    // Keep the selected project stable from asset lookup through job
    // registration. Project replacement drains this tracked worker before B
    // becomes current, so its later receipt write-back cannot target B.
    let _project_transition = state.project_transition.lock().await;
    let (current_dir, _, _) = crate::dispatch::project_paths(state).await?;
    if current_dir != dir {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the open project changed before diarization started",
            "retry media.diarize against the current project",
        ));
    }
    let (src, hash) = crate::dispatch::asset_info(state, &a.asset).await?;

    // The bridge must be wired before the job starts, otherwise the UI shows a
    // running job that can only fail later with setup work the user could do now.
    let rt = runtime()?.ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "the diarization bridge is not available",
            "diarize_runner.py or its python (perception sidecar) was not found",
        )
        .with_suggested_action(cut_perception::captions_setup_action())
        .with_requirement("captions_runtime")
    })?;
    let endpoint = endpoint();
    let secret = secret();
    let max_speakers = a.max_speakers;

    let job = state.jobs.create("diarize");
    let job_id = job.job_id.clone();
    let st = state.clone();
    let jobs = state.jobs.clone();
    jobs.spawn_limited(
        &job_id,
        "analysis",
        crate::dispatch::ANALYSIS_MAX_RUNNING,
        async move {
            st.jobs.progress(&job.job_id, 0.1, Some("diarizing".into()));
            let timeout = std::time::Duration::from_secs(300);
            let receipt = match run_diarize(
                &rt,
                &endpoint,
                secret.as_deref(),
                &src,
                max_speakers,
                timeout,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    st.jobs.fail(&job.job_id, e);
                    return;
                }
            };
            st.jobs
                .progress(&job.job_id, 0.7, Some("aligning words".into()));

            let aid = a.asset.clone();
            let receipts2 = receipts.clone();
            let src2 = src.clone();
            let hash2 = hash.clone();
            let recv = receipt.clone();
            let labeled = match crate::dispatch::run_blocking("media.diarize", move || {
                merge_diarization(&receipts2, &aid, &hash2, &src2, recv)
            })
            .await
            {
                Ok(n) => n,
                Err(e) => {
                    st.jobs.fail(&job.job_id, e);
                    return;
                }
            };

            let prov = receipt.provenance();
            let rel = format!("receipts/{}.diarize.json", a.asset);
            let receipt_json = json!({
                "asset": a.asset,
                "backend": prov.backend,
                "model": receipt.model,
                "device": receipt.device,
                "endpoint": receipt.endpoint,
                "num_speakers": receipt.n_speakers,
                "n_turns": receipt.turns.len(),
                "sample_rate": receipt.sample_rate,
                "rtf": receipt.rtf,
                "audio_s": receipt.audio_s,
                "infer_s": receipt.infer_s,
                "labeled_words": labeled,
                "turns": receipt.turns,
            });
            let receipt_path = dir.join(&rel);
            if let Some(parent) = receipt_path.parent() {
                if let Err(e) = require_live_receipts_dir(parent) {
                    st.jobs.fail(
                        &job.job_id,
                        e.with_suggested_action(
                            "reopen the project and rerun diarization if the result is still needed",
                        ),
                    );
                    return;
                }
            }
            let receipt_bytes = match serde_json::to_vec_pretty(&receipt_json) {
                Ok(bytes) => bytes,
                Err(e) => {
                    st.jobs.fail(
                        &job.job_id,
                        CutError::new(
                            error_codes::IO,
                            "could not serialize diarization receipt",
                            e.to_string(),
                        ),
                    );
                    return;
                }
            };
            // Replaces the name: never writes through a link or shared file.
            if let Err(e) = cut_core::store::replace_project_file(&receipt_path, &receipt_bytes) {
                st.jobs.fail(
                    &job.job_id,
                    CutError::new(
                        error_codes::IO,
                        "could not persist diarization receipt",
                        format!("{}: {e}", receipt_path.display()),
                    ),
                );
                return;
            }
            let perc_rel = format!("receipts/{}.perception.json", a.asset);
            if let Err(e) =
                crate::dispatch::update_asset(&st, &a.asset, |x| x.perception = Some(perc_rel))
                    .await
            {
                st.jobs.fail(&job.job_id, e);
                return;
            }

            st.jobs.finish(
                &job.job_id,
                json!({
                    "diarization": rel,
                    "backend": prov.backend,
                    "model": receipt.model,
                    "num_speakers": receipt.n_speakers,
                    "n_turns": receipt.turns.len(),
                    "labeled_words": labeled,
                    "turns": receipt.turns,
                }),
            );
        },
    );
    Ok(VerbResult::ok(json!({"job_id": job_id})))
}

/// The diarization receipt the runner emits (one JSON line on stdout). `turns` is
/// the normalized speaker-turn list (arrival-order `S1..Sn`), reused as the
/// canonical `cut_perception::SpeakerTurn` so it attaches to the perception report
/// with no conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiarizeReceipt {
    #[serde(default)]
    pub schema: String,
    pub turns: Vec<SpeakerTurn>,
    pub n_speakers: u32,
    pub model: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub sample_rate: u32,
    /// Real-time factor (infer_s / audio_s) reported by the service, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtf: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infer_s: Option<f64>,
}

impl DiarizeReceipt {
    /// The provenance record to store on the perception report (receipt honesty —
    /// which backend/model/device produced these turns + the speaker count).
    pub fn provenance(&self) -> Diarization {
        Diarization {
            backend: if self.backend.is_empty() {
                "sortformer".to_string()
            } else {
                self.backend.clone()
            },
            model: self.model.clone(),
            device: self.device.clone(),
            num_speakers: self.n_speakers,
        }
    }
}

/// Build the runner stdin JSON (pure — unit-tested). The runner reads this on stdin,
/// extracts the asset's audio to 16 kHz mono WAV, POSTs it, and writes the turns.
pub fn build_runner_input(
    endpoint: &str,
    secret: Option<&str>,
    media: &Path,
    max_speakers: Option<u32>,
    timeout_s: f64,
) -> Value {
    serde_json::json!({
        "endpoint": endpoint,
        "secret": secret,
        "media": media.display().to_string(),
        "max_speakers": max_speakers,
        "timeout_s": timeout_s,
    })
}

/// Parse the runner's single JSON stdout line into a `DiarizeReceipt` (tolerant to
/// leading log lines: take the last `{...}`-looking line). Pure — unit-tested.
pub fn parse_receipt(stdout: &str) -> Option<DiarizeReceipt> {
    for line in stdout.lines().rev() {
        let t = line.trim();
        if t.starts_with('{') {
            if let Ok(r) = serde_json::from_str::<DiarizeReceipt>(t) {
                return Some(r);
            }
        }
    }
    None
}

/// Run diarization by spawning `diarize_runner.py`: extract the asset's audio, POST
/// it to the service, and return the typed receipt. This is the NETWORK touch; it
/// runs with NO project lock held (the caller bakes it before recording anything,
/// exactly like dub::synthesize_track / matte::ensure_baked).
///
/// `timeout` bounds the whole spawn. A service failure / a bad asset fails the verb
/// cleanly — never faked/empty turns.
pub async fn run_diarize(
    rt: &Runtime,
    endpoint: &str,
    secret: Option<&str>,
    media: &Path,
    max_speakers: Option<u32>,
    timeout: std::time::Duration,
) -> Result<DiarizeReceipt, CutError> {
    // Give the runner its own per-call HTTP timeout (a touch under the wall budget
    // so the spawn timeout is the outer guard, not a silent inner hang).
    let inner_timeout_s = (timeout.as_secs_f64() - 5.0).max(30.0);
    let input = build_runner_input(endpoint, secret, media, max_speakers, inner_timeout_s);
    let payload = input.to_string();

    let mut command = tokio::process::Command::new(&rt.python);
    command
        .args(cut_perception::sidecar::python_command_args(
            rt.native_context,
        ))
        .arg(&rt.script)
        .env(cut_perception::sidecar::PYTHONDONTWRITEBYTECODE_ENV, "1");
    // Forward the engine-resolved ffmpeg dir so the runner's bare `ffmpeg` hits the
    // SAME binary the rest of the app uses (cold installs keep it off PATH).
    if let Some(ffdir) = std::env::var_os(cut_perception::sidecar::ENV_FFMPEG_DIR) {
        if !ffdir.is_empty() {
            command.env(cut_perception::sidecar::ENV_FFMPEG_DIR, &ffdir);
            let sep = if cfg!(windows) { ";" } else { ":" };
            let new_path = match std::env::var_os("PATH") {
                Some(p) => {
                    let mut s = ffdir.clone();
                    s.push(sep);
                    s.push(p);
                    s
                }
                None => ffdir.clone(),
            };
            command.env("PATH", new_path);
        }
    }

    let control = ProcessControl::for_operation(timeout);
    let out = match run_owned(&mut command, Some(payload.as_bytes()), &control).await {
        Ok(output) => output,
        Err(error) => match error.termination() {
            Some(ProcessTermination::DeadlineExceeded) => {
                return Err(CutError::new(
                    error_codes::SIDECAR,
                    format!("diarization timed out after {}ms", timeout.as_millis()),
                    "the diarization service was slow or stalled",
                )
                .with_suggested_action(
                    "raise timeout_ms, or check the diarization service (curl <CUT_DIARIZE_ENDPOINT>/health)",
                ))
            }
            Some(ProcessTermination::Cancelled(reason)) => {
                return Err(CutError::new(
                    "job_cancelled",
                    format!("speaker diarization cancelled ({})", reason.label()),
                    "the owning background job stopped this external worker",
                ))
            }
            None => {
            return Err(CutError::new(
                error_codes::IO,
                format!("the diarize runner errored: {error}"),
                "speaker diarization failed",
            ))
            }
        }
    };
    if out.diagnostics_truncated() {
        tracing::warn!("diarize runner diagnostics exceeded the retained output cap");
    }
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(classify_runner_failure(
            out.status.code(),
            &stderr,
            endpoint,
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_receipt(&stdout).ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "the diarize runner returned no JSON receipt",
            format!("got: {}", stdout.trim()),
        )
    })
}

/// Exit status of `diarize_runner.py` when the asset's audio is longer than one run
/// accepts. Mirrors `EXIT_AUDIO_TOO_LONG` in the runner (a test keeps them equal).
const RUNNER_EXIT_AUDIO_TOO_LONG: i32 = 3;
/// Exit status when the asset has no audio the runner can read. Mirrors
/// `EXIT_NO_USABLE_AUDIO` in the runner.
const RUNNER_EXIT_NO_USABLE_AUDIO: i32 = 4;

/// Turn a failed runner run into the error the person and the agent see.
///
/// The runner refuses some assets on its own, before it sends anything to the
/// service, and says so with a dedicated exit status. Those refusals are about the
/// asset, so the message is the runner's plain reason and the suggested action is
/// the matching next step. Every other failure (service unreachable, model still
/// loading, a bad answer) keeps the service advice. The status decides, never the
/// wording of stderr.
fn classify_runner_failure(status: Option<i32>, stderr: &str, endpoint: &str) -> CutError {
    let detail = stderr.trim();
    match status {
        Some(RUNNER_EXIT_AUDIO_TOO_LONG) => CutError::new(
            error_codes::INVALID_ARGS,
            detail,
            "the asset's audio is longer than one speaker-labelling run accepts",
        )
        .with_suggested_action("Trim or split the asset and run speaker labelling on the parts."),
        Some(RUNNER_EXIT_NO_USABLE_AUDIO) => CutError::new(
            error_codes::INVALID_ARGS,
            detail,
            "the asset has no audio that can be read",
        )
        .with_suggested_action(
            "Choose an asset that contains speech audio, or import the file again if it is damaged.",
        ),
        _ => CutError::new(
            error_codes::SIDECAR,
            format!("speaker diarization failed: {detail}"),
            format!("the diarization service ({endpoint}) could not diarize the asset"),
        )
        .with_suggested_action(
            "verify the service is reachable at <CUT_DIARIZE_ENDPOINT>/health, or set \
             CUT_DIARIZE_ENDPOINT to the running diarization service URL",
        )
        .with_requirement("diarize_service"),
    }
}

/// Merge diarization results into an asset's perception receipt (blocking IO).
/// Loads the existing `<asset>.perception.json` when its hash matches the current
/// asset (the normal post-transcribe path -> words get speaker-labeled); a stale
/// or absent report is replaced with a fresh diarize-only report for the CURRENT
/// file (turns recorded, words labeled on the next transcribe). Always rewrites
/// `<asset>.perception.json`; when a transcript is present, also refreshes
/// `<asset>.words.json` so transcript.get / the panel show speaker labels.
/// Returns the number of words that received a speaker label.
pub(crate) fn merge_diarization(
    receipts: &Path,
    asset_id: &str,
    hash: &str,
    src: &Path,
    receipt: DiarizeReceipt,
) -> Result<usize, CutError> {
    let mut report = match cut_perception::load_report(receipts, asset_id)? {
        Some(r) if r.asset_hash == hash => r,
        _ => cut_perception::PerceptionReport {
            schema: cut_perception::PERCEPTION_SCHEMA.to_string(),
            asset_hash: hash.to_string(),
            source_path: src.display().to_string(),
            instruments_run: vec![],
            words: None,
            silences: vec![],
            scenes: vec![],
            beats: None,
            loudness: None,
            black_spans: vec![],
            frozen_spans: vec![],
            content_bbox: None,
            subject_track: None,
            speaker_turns: vec![],
            diarization: None,
            runtime_context: None,
        },
    };
    let prov = receipt.provenance();
    let labeled = cut_perception::apply_diarization(&mut report, receipt.turns, prov);

    require_live_receipts_dir(receipts)?;
    // Both files are replaced by name, never written through a link or a
    // file shared with another path.
    let perc_path = receipts.join(format!("{asset_id}.perception.json"));
    cut_core::store::replace_project_file(
        &perc_path,
        serde_json::to_string_pretty(&report)
            .map_err(CutError::from)?
            .as_bytes(),
    )?;
    cut_media::cache_trust::remember(&perc_path); // item 155: this engine's report
    if let Some(t) = report.words.as_ref() {
        let words_path = receipts.join(format!("{asset_id}.words.json"));
        cut_core::store::replace_project_file(
            &words_path,
            serde_json::to_string_pretty(t)
                .map_err(CutError::from)?
                .as_bytes(),
        )?;
    }
    Ok(labeled)
}

fn require_live_receipts_dir(receipts: &Path) -> Result<(), CutError> {
    // Item 162: the folder itself, never followed (see the perception
    // sidecar's twin): a link here would send both writes outside the project.
    let plain = std::fs::symlink_metadata(receipts).is_ok_and(|meta| {
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && !cut_core::reparse::names_another_entry(&meta)
    });
    if plain {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::IO,
        "could not persist diarization because the project is no longer open",
        format!(
            "the project receipts directory no longer exists: {}",
            receipts.display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Item 162: a receipts folder that became a link is not written through.
    #[cfg(unix)]
    #[test]
    fn linked_receipts_folder_is_not_written_through() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let receipts = dir.path().join("receipts");
        std::os::unix::fs::symlink(&outside, &receipts).unwrap();
        assert!(require_live_receipts_dir(&receipts).is_err());
        assert!(require_live_receipts_dir(&outside).is_ok());
    }

    #[test]
    fn native_selection_uses_bundled_diarize_and_normal_selection_keeps_overrides() {
        let temp = tempfile::tempdir().unwrap();
        let bundled = temp.path().join("bundled");
        std::fs::create_dir_all(&bundled).unwrap();
        let python = temp.path().join("native-python");
        let instruments = bundled.join("instruments.py");
        let runner = bundled.join("diarize_runner.py");
        let override_python = temp.path().join("override-python");
        let override_script = temp.path().join("override-diarize.py");
        for path in [
            &python,
            &instruments,
            &runner,
            &override_python,
            &override_script,
        ] {
            std::fs::write(path, b"fixture").unwrap();
        }

        let native = runtime_from_parts(
            python.clone(),
            instruments.clone(),
            true,
            Some(override_python.clone()),
            Some(override_script.clone()),
        )
        .unwrap();
        assert_eq!(native.python, python);
        assert_eq!(native.script, runner);
        assert!(native.native_context);

        let normal = runtime_from_parts(
            temp.path().join("unused-python"),
            instruments,
            false,
            Some(override_python.clone()),
            Some(override_script.clone()),
        )
        .unwrap();
        assert_eq!(normal.python, override_python);
        assert_eq!(normal.script, override_script);
        assert!(!normal.native_context);
    }

    const TOO_LONG_STDERR: &str = "this asset's audio is too long to label speakers (about 2h 01m 40s); the limit is 2h 00m 00s of audio per run.\n";

    #[test]
    fn a_too_long_asset_is_refused_with_the_plain_reason_and_a_trim_action() {
        let e = classify_runner_failure(Some(3), TOO_LONG_STDERR, "http://127.0.0.1:9002");
        assert_eq!(e.code, error_codes::INVALID_ARGS);
        assert_eq!(e.message, TOO_LONG_STDERR.trim());
        assert!(!e.message.starts_with("speaker diarization failed"));
        let action = e.suggested_action.expect("a next step");
        assert!(action.starts_with("Trim or split the asset"), "{action}");
        assert!(!action.contains("CUT_DIARIZE_ENDPOINT"), "{action}");
        assert!(!e.cause.contains("service"), "{}", e.cause);
    }

    #[test]
    fn an_asset_without_readable_audio_is_refused_with_its_own_action() {
        let msg = "this asset has no audio that can be read; speaker labelling needs an asset with speech.\n";
        let e = classify_runner_failure(Some(4), msg, "http://127.0.0.1:9002");
        assert_eq!(e.code, error_codes::INVALID_ARGS);
        assert_eq!(e.message, msg.trim());
        let action = e.suggested_action.expect("a next step");
        assert!(action.contains("speech audio"), "{action}");
        assert!(!action.contains("CUT_DIARIZE_ENDPOINT"), "{action}");
    }

    #[test]
    fn a_service_failure_keeps_the_reachability_advice() {
        let stderr =
            "diarize_runner: diarization service unreachable (http://127.0.0.1:9002): refused\n";
        for status in [Some(1), Some(2), None] {
            let e = classify_runner_failure(status, stderr, "http://127.0.0.1:9002");
            assert_eq!(e.code, error_codes::SIDECAR);
            assert_eq!(
                e.message,
                format!("speaker diarization failed: {}", stderr.trim())
            );
            assert!(e.cause.contains("http://127.0.0.1:9002"), "{}", e.cause);
            let action = e.suggested_action.expect("a next step");
            assert!(action.contains("CUT_DIARIZE_ENDPOINT"), "{action}");
        }
    }

    #[test]
    fn the_status_decides_not_the_wording() {
        // A service failure whose text happens to mention a long asset stays a
        // service failure, and a refusal keeps its class whatever it says.
        let e = classify_runner_failure(Some(1), "the audio is too long", "http://x");
        assert_eq!(e.code, error_codes::SIDECAR);
        let e = classify_runner_failure(Some(3), "anything at all", "http://x");
        assert_eq!(e.code, error_codes::INVALID_ARGS);
    }

    #[test]
    fn the_exit_statuses_match_the_runner() {
        // Compared line by line, so a checkout with Windows line endings reads the same.
        let script = include_str!("../../perception/py/diarize_runner.py");
        let has_line = |wanted: String| script.lines().any(|line| line.trim_end() == wanted);
        assert!(
            has_line(format!(
                "EXIT_AUDIO_TOO_LONG = {RUNNER_EXIT_AUDIO_TOO_LONG}"
            )),
            "runner and engine disagree on the too-long status"
        );
        assert!(
            has_line(format!(
                "EXIT_NO_USABLE_AUDIO = {RUNNER_EXIT_NO_USABLE_AUDIO}"
            )),
            "runner and engine disagree on the no-audio status"
        );
    }

    #[test]
    fn endpoint_defaults_to_loopback_9002() {
        // No env override in the test process → the documented default.
        if std::env::var("CUT_DIARIZE_ENDPOINT").is_err() {
            assert_eq!(endpoint(), "http://127.0.0.1:9002");
        }
    }

    #[test]
    fn runner_input_has_the_runner_contract_shape() {
        let v = build_runner_input(
            "http://127.0.0.1:9002",
            None,
            Path::new("/m/clip.mp4"),
            Some(2),
            90.0,
        );
        assert_eq!(v["endpoint"], "http://127.0.0.1:9002");
        assert_eq!(v["media"], "/m/clip.mp4");
        assert_eq!(v["max_speakers"], 2);
        assert!(v["secret"].is_null());
        assert_eq!(v["timeout_s"], 90.0);
        // No max_speakers → null (the service applies its own default).
        let v2 = build_runner_input("http://x", Some("sek"), Path::new("/a.wav"), None, 30.0);
        assert!(v2["max_speakers"].is_null());
        assert_eq!(v2["secret"], "sek");
    }

    #[test]
    fn parse_receipt_takes_last_json_line_tolerantly() {
        let stdout = "loading...\nffmpeg done\n{\"schema\":\"shellx-cut/diarize/1\",\"turns\":[{\"start_ms\":480,\"end_ms\":10800,\"speaker\":\"S1\"},{\"start_ms\":11040,\"end_ms\":22400,\"speaker\":\"S2\"}],\"n_speakers\":2,\"model\":\"sortformer-v2\",\"backend\":\"sortformer\",\"endpoint\":\"http://127.0.0.1:9002\",\"device\":\"cuda\",\"sample_rate\":16000,\"rtf\":0.0038,\"audio_s\":40.5,\"infer_s\":0.15}";
        let r = parse_receipt(stdout).expect("parses the last json line");
        assert_eq!(r.n_speakers, 2);
        assert_eq!(r.turns.len(), 2);
        assert_eq!(r.turns[0].speaker, "S1");
        assert_eq!(r.turns[1].start_ms, 11040);
        assert_eq!(r.model, "sortformer-v2");
        assert_eq!(r.sample_rate, 16000);
        assert_eq!(r.rtf, Some(0.0038));
        // provenance() projects the receipt onto the report's Diarization record.
        let p = r.provenance();
        assert_eq!(p.backend, "sortformer");
        assert_eq!(p.num_speakers, 2);
        assert_eq!(p.device, "cuda");
        assert!(parse_receipt("no json here").is_none());
    }

    #[test]
    fn provenance_defaults_backend_when_absent() {
        let r = DiarizeReceipt {
            schema: String::new(),
            turns: vec![],
            n_speakers: 0,
            model: "sortformer-v2".into(),
            backend: String::new(),
            endpoint: String::new(),
            device: String::new(),
            sample_rate: 16000,
            rtf: None,
            audio_s: None,
            infer_s: None,
        };
        assert_eq!(r.provenance().backend, "sortformer");
    }

    #[test]
    fn late_diarization_never_recreates_a_deleted_project() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("deleted.cutproj");
        let receipts = project.join("receipts");
        let receipt = DiarizeReceipt {
            schema: "shellx-cut/diarize/1".into(),
            turns: vec![],
            n_speakers: 0,
            model: "fixture".into(),
            backend: "fixture".into(),
            endpoint: "local".into(),
            device: "cpu".into(),
            sample_rate: 16_000,
            rtf: None,
            audio_s: None,
            infer_s: None,
        };

        let error = merge_diarization(&receipts, "a1", "hash", Path::new("source.wav"), receipt)
            .expect_err("a late diarization result must be rejected after project deletion");

        assert_eq!(error.code, error_codes::IO);
        assert!(!project.exists(), "the deleted project must stay deleted");
    }
}
