use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static SCREENSHOT_SEQ: AtomicU64 = AtomicU64::new(0);

fn screenshot_name(prefix: &str) -> String {
    format!(
        "{prefix}_{}_{}.png",
        chrono::Utc::now().timestamp_millis(),
        SCREENSHOT_SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

// ---------------------------------------------------------------------------
// ui.* handlers — WS relay to the connected UI client (ui_bridge.rs)
// ---------------------------------------------------------------------------

/// ui.state{} — last state the UI client pushed; no_ui_client if never seen.
pub(super) async fn ui_state(state: &AppState) -> Result<VerbResult, CutError> {
    let client_count = state.ui_bridge.client_count();
    if client_count == 0 {
        return Err(no_ui_client());
    }
    match state.ui_state.read().await.clone() {
        Some(mut s) => {
            if let Some(object) = s.as_object_mut() {
                object.insert("connected".into(), json!(true));
                object.insert("ui_clients".into(), json!(client_count));
            }
            Ok(VerbResult::ok(s))
        }
        None => Err(no_ui_client()),
    }
}

/// Shared actionable "no UI connected" error (public verb contract ui.screenshot).
fn no_ui_client() -> CutError {
    CutError::new(
        error_codes::NO_UI_CLIENT,
        "no UI client is connected",
        "ui.* relay verbs need a browser tab running the app, connected over WS",
    )
    .with_requirement("ui_client")
    .with_suggested_action(
        "call system.doctor to read the live loopback address, then open it in a browser (or use render.frame for composed pixels)",
    )
}

/// Build the verb error for a screenshot_result that carried no png_base64.
/// The UI client sends a STRUCTURED failure frame —
/// `error: {code, stage, message, attempts, first_attempt?}` (lib/capture.ts +
/// lib/events.ts, one bounded client-side retry) — so the verb error names the
/// failing capture stage and the human cause instead of an opaque
/// `"[object Event]"` passthrough. Legacy
/// string errors and error-less replies still produce an honest message.
fn screenshot_capture_error(reply: &Value) -> CutError {
    let err = reply.get("error");
    let stage = err
        .and_then(|e| e.get("stage"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let message = err
        .and_then(|e| e.get("message"))
        .and_then(|v| v.as_str())
        // Compat: an older UI client sent `error` as a plain string.
        .or_else(|| err.and_then(|e| e.as_str()));
    let attempts = err.and_then(|e| e.get("attempts")).and_then(|v| v.as_u64());
    let (headline, cause) = match message {
        Some(m) => (
            format!("UI screenshot capture failed at stage '{stage}'"),
            match attempts {
                Some(n) => format!("{m} (after {n} attempt(s))"),
                None => m.to_string(),
            },
        ),
        None => (
            "UI client returned no image".to_string(),
            format!("screenshot_result lacked png_base64: {reply}"),
        ),
    };
    CutError::new(error_codes::JOB_FAILED, headline, cause).with_suggested_action(
        "retry ui.screenshot; if it keeps failing, debug.screenshot captures real screen pixels and render.frame composes output frames",
    )
}

/// ui.screenshot{inline?} — ask the UI client to capture its own app root
/// and relay the PNG back (UI contract: a verification PRIMITIVE).
pub(super) async fn ui_screenshot(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        #[serde(default)]
        inline: bool,
    }
    let a: Args = parse_args(args)?;
    let reply = state
        .ui_bridge
        .request(json!({"type": "screenshot_request"}))
        .await
        .map_err(|e| {
            if e.code == error_codes::NO_UI_CLIENT {
                no_ui_client()
            } else {
                e
            }
        })?;
    let b64 = reply
        .get("png_base64")
        .and_then(|v| v.as_str())
        .ok_or_else(|| screenshot_capture_error(&reply))?;
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| {
            CutError::new(
                error_codes::JOB_FAILED,
                "UI sent invalid base64",
                e.to_string(),
            )
        })?;
    // Persist next to the project (frames/) or tmp when nothing is open.
    // Item 140 sweep: inside a project the folder is created only through
    // plain folders and the file is replaced by name, never written in place.
    let name = screenshot_name("ui");
    let path = match project_paths(state).await {
        Ok((dir, _r, _p)) => {
            let frames =
                crate::output_paths::ensure_plain_project_relative_dir(&dir, Path::new("frames"))?;
            let path = frames.join(&name);
            crate::output_paths::write_output_atomic(&path, &bytes)?;
            path
        }
        Err(_) => {
            let dir = state.screenshot_temp_dir()?;
            let path = dir.join(&name);
            std::fs::write(&path, &bytes)?;
            path
        }
    };
    let mut result = json!({"path": path, "mime": "image/png"});
    if a.inline {
        result["base64"] = json!(b64);
    }
    // UI contract: panels the DOM-capture lib can't render (e.g. <video>) are
    // composited client-side from live pixels and NOTED in the metadata —
    // pass the client's notes through so the caller sees what was composited.
    if let Some(notes) = reply.get("notes") {
        result["notes"] = notes.clone();
    }
    Ok(VerbResult::ok(result))
}

/// debug.screenshot{monitor?, window?, inline?} — SERVER-SIDE screenshot of the actual
/// display via the in-process recorder. Works HEADLESS / regardless of UI-client state,
/// unlike ui.screenshot (which relays to the WebView and fails with no client). Returns
/// {path, width, height, mime} and, when `inline` (default true), the base64 PNG — so an
/// agent can SEE the app, menus and dialogs while driving cutd over the debug API.
pub(super) async fn debug_screenshot(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    fn default_true() -> bool {
        true
    }
    #[derive(serde::Deserialize, Default)]
    struct Args {
        monitor: Option<u32>,
        window: Option<String>,
        #[serde(default = "default_true")]
        inline: bool,
    }
    let a: Args = parse_args(args)?;
    let dir = state.screenshot_temp_dir()?;
    let path = dir.join(screenshot_name("debug_shot"));
    // The capture backend BLOCKS (and on Linux spins up its own tokio runtime for the XDG
    // portal), so it must run OFF cutd's async runtime — calling it inline panics with
    // "cannot start a runtime from within a runtime" (screen_record.start uses a thread for
    // the same reason). spawn_blocking moves it to the blocking pool.
    let path_c = path.clone();
    let monitor = a.monitor;
    let window = a.window.clone();
    let (w, h) = tokio::task::spawn_blocking(move || {
        crate::screen_record::capture_screenshot_png(&path_c, monitor, window)
    })
    .await
    .map_err(|e| CutError::new(error_codes::IO, "screenshot task panicked", e.to_string()))??;
    let mut result = json!({"path": path, "width": w, "height": h, "mime": "image/png"});
    if a.inline {
        let bytes = std::fs::read(&path)
            .map_err(|e| CutError::new(error_codes::IO, "read screenshot png", e.to_string()))?;
        use base64::Engine;
        result["png_base64"] = json!(base64::engine::general_purpose::STANDARD.encode(&bytes));
        result["bytes"] = json!(bytes.len());
    }
    Ok(VerbResult::ok(result))
}

fn bounded_ui_message(value: Option<&Value>, fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .map(|message| message.chars().take(500).collect())
        .filter(|message: &String| !message.trim().is_empty())
        .unwrap_or_else(|| fallback.to_owned())
}

/// ui.open / ui.playhead / ui.select / ui.highlight — request the newest UI
/// client, then return only after that exact socket reports committed,
/// observable state. A rejection remains a universal `ok:false` envelope but
/// carries the structured `applied:false` result for diagnosis.
pub(super) async fn ui_forward(
    state: &AppState,
    verb: &str,
    args: Value,
) -> Result<VerbResult, CutError> {
    let reply = state
        .ui_bridge
        .request(json!({"type": "ui_command", "verb": verb, "args": args.clone()}))
        .await
        .map_err(|error| {
            if error.code == error_codes::NO_UI_CLIENT {
                no_ui_client()
            } else {
                error
            }
        })?;
    let request_id = reply
        .get("request_id")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            CutError::new(
                error_codes::JOB_FAILED,
                "UI client returned a malformed command result",
                "ui_command_result lacked a numeric request_id",
            )
        })?;
    let applied = reply
        .get("applied")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            CutError::new(
                error_codes::JOB_FAILED,
                "UI client returned a malformed command result",
                "ui_command_result lacked applied:true|false",
            )
        })?;
    let mut result = json!({
        "applied": applied,
        "verb": verb,
        "request_id": request_id,
        "requested": args,
        "state": reply.get("state").cloned().unwrap_or(Value::Null),
    });
    for field in ["surface", "selector"] {
        if let Some(value) = reply.get(field) {
            result[field] = value.clone();
        }
    }
    if applied {
        return Ok(VerbResult::ok(result));
    }

    let ui_error = reply.get("error").and_then(Value::as_object);
    let code = ui_error
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .filter(|code| {
            matches!(
                *code,
                error_codes::INVALID_ARGS
                    | error_codes::NOT_FOUND
                    | error_codes::CONFLICT
                    | error_codes::NO_UI_CLIENT
            )
        })
        .unwrap_or(error_codes::JOB_FAILED);
    let message = bounded_ui_message(
        ui_error.and_then(|error| error.get("message")),
        "UI did not apply the requested state change",
    );
    let error = CutError::new(
        code,
        message,
        format!("the connected UI returned applied:false for {verb} request {request_id}"),
    )
    .with_suggested_action(
        "inspect ui.state and available_surface_ids, then retry a valid visible target",
    );
    result["error"] = serde_json::to_value(&error)?;
    Ok(VerbResult {
        ok: false,
        result: Some(result),
        op_ids: None,
        warnings: None,
        project_revision: None,
        error: Some(error),
    })
}

// ---------------------------------------------------------------------------
// system.* — environment doctor + consented tool fetch
// ---------------------------------------------------------------------------

/// `system.doctor {refresh?}` — return the cached environment capability report
/// (the start wizard / Settings>Environment / agent source of truth). Fast:
/// `refresh:false` (default) returns the cached scan (or runs ONE scan if the
/// server somehow never scanned at startup); `refresh:true` re-scans and
/// re-emits `doctor_updated` on a capability change. No project required — the
/// environment is global, not project-scoped.
pub(super) async fn system_doctor(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        #[serde(default)]
        refresh: bool,
        #[serde(default)]
        feature: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let report = if a.refresh {
        state.doctor_rescan().await
    } else {
        state.doctor_cached().await
    };
    // What needs what (schema/requirements.json), read from these cards and
    // the engine's own state; nothing is probed a second time.
    let facts = crate::requirements::EngineFacts {
        open_project: state.project.read().await.is_some(),
        export_folder: crate::output_paths::output_folder_status(),
        ui_clients: state.ui_bridge.client_count(),
        os: report.os.clone(),
    };
    if let Some(feature) = a.feature.as_deref() {
        return Ok(VerbResult::ok(crate::requirements::feature_check(
            feature, &report, &facts,
        )?));
    }
    let section = crate::requirements::doctor_section(&report, &facts);
    let mut answer = serde_json::to_value(report)?;
    if let Value::Object(map) = &mut answer {
        map.insert("requirements".into(), section);
    }
    Ok(VerbResult::ok(answer))
}

/// `system.motion_status {}` — bounded, read-only Motion discovery and
/// distribution-management posture.  This does not join Doctor's synchronous
/// cached scan: Motion discovery owns an async child-process runner and must
/// stay on the async route to avoid blocking the shared environment report.
///
/// The route never installs, repairs, updates, removes, logs in, or executes a
/// connector.  Until MOTION-DIST-01 supplies an immutable verified manifest and
/// a platform artifact, every lifecycle action is explicitly unavailable.
pub(super) async fn system_motion_status(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {}
    let _: Args = parse_args(args)?;
    Ok(VerbResult::ok(
        crate::motion_discovery::motion_management_status().await,
    ))
}

/// `system.fetch_tool {tool, rationale?}` — consented download+install of a
/// built-in tool as a job (the background-job contract: returns {job_id}). The tool id is
/// validated against the fetch registry HERE (fail fast, before a job exists),
/// then a background job streams the download, verifies sha256, atomically
/// installs into the app-data tools dir, and re-scans the doctor (which fires
/// `doctor_updated` flipping the ffmpeg card). Progress flows through the jobs
/// domain + WS job_progress. SECURITY: see fetch.rs — registry-only, no caller
/// URL, checksum-before-install.
pub(super) async fn system_fetch_tool(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        tool: String,
        #[serde(default)]
        #[allow(dead_code)] // recorded on the job below for the audit trail
        rationale: Option<String>,
        /// The person agreed to the download offer (publisher, licence, size).
        #[serde(default)]
        accept_license: bool,
    }
    let a: Args = parse_args(args)?;
    // Windows: Microsoft's Visual C++ runtime (vc_runtime.rs owns it).
    if a.tool == crate::vc_runtime::TOOL_ID {
        return fetch_vc_runtime(state);
    }
    // Fail fast on an unknown tool id (the registry is the allow-list; the verb
    // schema also enum-restricts it, but we never trust only the schema).
    if a.tool != "ffmpeg" {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown tool '{}'", a.tool),
            "system.fetch_tool serves a BUILT-IN registry only (\"ffmpeg\", and \"vc-runtime\" on Windows); there is no caller-supplied URL",
        )
        .with_suggested_action("pass tool:\"ffmpeg\" or tool:\"vc-runtime\", or install other tools manually onto PATH"));
    }
    // No job for a platform without a built-in download: answer now, with the
    // platform's way to get ffmpeg, instead of an ok:true job that then fails.
    if !cut_media::tool_setup::builtin_ffmpeg_download_available() {
        return Err(crate::fetch::no_builtin_download(
            &a.tool,
            std::env::consts::OS,
            std::env::consts::ARCH,
        ));
    }
    // Nothing is downloaded without agreement to the offer the person sees.
    let offer =
        cut_media::tool_setup::download_offer_for(std::env::consts::OS, std::env::consts::ARCH);
    if !a.accept_license {
        return Err(crate::fetch::consent_required(offer.as_ref()));
    }

    let job = state.jobs.create("fetch_tool");
    let job_id = job.job_id.clone();
    let tool = a.tool.clone();
    let st = state.clone();
    let jobs = state.jobs.clone();
    jobs.spawn(&job_id, async move {
        let jid = job.job_id.clone();
        st.jobs
            .progress(&jid, 0.01, Some(format!("fetch {tool}: starting")));
        // The install is blocking (sync HTTPS + subprocess extract); run it off
        // the async executor. A channel relays progress back so we can publish
        // WS job_progress from the async side (JobManager::progress is sync but
        // cheap; we call it directly from the blocking closure via a clone).
        let jobs = st.jobs.clone();
        let jid_cl = jid.clone();
        let tool_cl = tool.clone();
        let outcome = run_blocking("system.fetch_tool", move || {
            let progress = move |frac: f32, msg: &str| {
                // Map install stages into the 0.01..0.99 band (we keep 1.0 for
                // the post-install doctor re-scan completion).
                jobs.progress(
                    &jid_cl,
                    (0.01 + frac * 0.98).min(0.99),
                    Some(format!("fetch {tool_cl}: {msg}")),
                );
            };
            crate::fetch::install_tool(&tool, &progress)
        })
        .await;
        match outcome {
            Ok(install) => {
                // Re-scan so the doctor card flips missing→bundled-or-appdata and
                // doctor_updated fires (the wizard updates live).
                st.jobs
                    .progress(&jid, 0.99, Some("fetch: re-scanning environment".into()));
                let report = st.doctor_rescan().await;
                // Usable counts: a build without an optional filter is
                // "degraded" but imports, previews and exports.
                let ffmpeg_ok = report
                    .cards
                    .iter()
                    .find(|c| c.id == "ffmpeg")
                    .map(|c| {
                        matches!(
                            c.status,
                            crate::doctor::CardStatus::Ok | crate::doctor::CardStatus::Degraded
                        )
                    })
                    .unwrap_or(false);
                st.jobs.finish(
                    &jid,
                    json!({
                        "tool": install.tool,
                        "installed_dir": install.installed_dir,
                        "version": install.version,
                        "sha256": install.sha256,
                        "source_url": install.source_url,
                        "bytes": install.bytes,
                        "ffmpeg_ok_after": ffmpeg_ok,
                        "limitation": offer.as_ref().and_then(|o| o.limitation.clone()),
                    }),
                );
            }
            Err(e) => st.jobs.fail(&jid, e),
        }
    });
    Ok(VerbResult::ok(json!({"job_id": job_id, "tool": a.tool})))
}

/// `system.fetch_tool {tool:"vc-runtime"}`: install
/// Microsoft's Visual C++ Redistributable as a job (Windows only). The job
/// downloads Microsoft's installer, runs it only after Windows' signature check
/// accepts it as Microsoft's, and waits while Windows asks the PERSON for
/// administrator permission; the result says installed, declined, cannot be
/// approved by this account, download failed, signature failed or restart needed.
fn fetch_vc_runtime(state: &AppState) -> Result<VerbResult, CutError> {
    let current = crate::vc_runtime::status();
    if !current.applicable {
        return Err(CutError::new(
            error_codes::UNSUPPORTED_PLATFORM,
            "The Microsoft Visual C++ runtime is only needed on Windows",
            format!("this is {}", std::env::consts::OS),
        ));
    }
    let job = state.jobs.create("fetch_tool");
    let job_id = job.job_id.clone();
    let st = state.clone();
    state.jobs.spawn(&job_id, async move {
        let jid = job.job_id.clone();
        let jobs = st.jobs.clone();
        let jid_cl = jid.clone();
        let cancellation = crate::jobs::current_job_cancellation();
        let worker_cancellation = cancellation.clone();
        let outcome = run_blocking("system.fetch_tool", move || {
            let progress = move |frac: f32, msg: &str| {
                jobs.progress(
                    &jid_cl,
                    (0.01 + frac * 0.97).min(0.98),
                    Some(format!("Microsoft Visual C++ runtime: {msg}")),
                );
            };
            crate::vc_runtime::install(&progress, &|| worker_cancellation.is_cancelled())
        })
        .await;
        if outcome.is_err() {
            if let Some(reason) = cancellation.reason() {
                return st.jobs.cancel_from_worker(&jid, reason);
            }
        }
        // The doctor card flips and doctor_updated fires either way.
        let _ = st.doctor_rescan().await;
        match outcome {
            Ok(result) => st.jobs.finish(&jid, result),
            Err(e) => st.jobs.fail(&jid, e),
        }
    });
    Ok(VerbResult::ok(json!({
        "job_id": job_id,
        "tool": crate::vc_runtime::TOOL_ID,
        "approval": "Windows asks the person for administrator permission while this job runs; poll the job for the outcome",
    })))
}

/// serde default for `setup_perception.warm_model` (pre-fetch the model by default
/// so the first transcription is instant).
fn default_true() -> bool {
    true
}

/// `system.setup_perception {warm_model?, rationale?}` — consented provisioning of
/// the Python perception venv as a job (the background-job contract: returns {job_id}). Downloads uv
/// (sha256-verified via the fetch registry), selects a compatible CPython 3.12 patch,
/// resolves its concrete executable, builds the app-data sidecar venv from that path,
/// and `uv pip install`s the bundled perception requirements (onnx-asr Parakeet/Canary
/// STT + the perception stack). This closes the cold-install gap — the bundle ships
/// only instruments.py + requirements.txt, and system python is too old on real
/// desktops (macOS 3.9 < onnx-asr's 3.10 floor).
/// On success the doctor `perception` card flips missing→ready. SECURITY: see
/// perception_setup.rs (uv via the fetch allow-list; bundled requirements only).
pub(super) async fn system_setup_perception(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        /// Pre-download the default Parakeet model so the first transcription is instant.
        #[serde(default = "default_true")]
        warm_model: bool,
        #[serde(default)]
        #[allow(dead_code)] // recorded on the job for the audit trail
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let warm = a.warm_model;
    // Windows: no doomed install without Microsoft's Visual C++ runtime; the
    // refusal points at its tools-list entry and its install verb.
    crate::vc_runtime::require_for("Captions and transcription")?;

    let job = state.jobs.create("setup_perception");
    let job_id = job.job_id.clone();
    let st = state.clone();
    let jobs = state.jobs.clone();
    jobs.spawn(&job_id, async move {
        let jid = job.job_id.clone();
        st.jobs
            .progress(&jid, 0.01, Some("perception setup: starting".into()));
        let jobs = st.jobs.clone();
        let jid_cl = jid.clone();
        let cancellation = crate::jobs::current_job_cancellation();
        let worker_cancellation = cancellation.clone();
        let outcome = run_blocking("system.setup_perception", move || {
            let progress = move |frac: f32, msg: &str| {
                // Reserve 1.0 for the post-install doctor re-scan.
                jobs.progress(
                    &jid_cl,
                    (0.01 + frac * 0.97).min(0.99),
                    Some(format!("perception setup: {msg}")),
                );
            };
            crate::perception_setup::setup_perception(warm, &progress, &worker_cancellation)
        })
        .await;
        // A cancelled install ends as cancelled, whatever step it was in when
        // its helper was stopped; the unfinished environment is already removed.
        if outcome.is_err() {
            if let Some(reason) = cancellation.reason() {
                return st.jobs.cancel_from_worker(&jid, reason);
            }
        }
        match outcome {
            Ok(o) => {
                st.jobs.progress(
                    &jid,
                    0.99,
                    Some("perception setup: re-scanning environment".into()),
                );
                let report = st.doctor_rescan().await;
                let sidecar_ok = report
                    .cards
                    .iter()
                    .find(|c| c.id == "perception")
                    .map(|c| matches!(c.status, crate::doctor::CardStatus::Ok))
                    .unwrap_or(false);
                let model_preparation_failed = o.model_warm_error.is_some();
                // The full tools (beyond base transcription) failed to install or
                // to import: a partial setup, said as such, never a plain "done".
                let extras_missing = !o.full_perception_ready;
                let warnings: Vec<String> = o
                    .extras_note
                    .iter()
                    .chain(o.model_warm_error.iter().map(|e| &e.message))
                    .cloned()
                    .collect();
                let result = json!({
                    "venv_python": o.venv_python,
                    "managed_python": o.managed_python,
                    "python_version": o.python_version,
                    "uv_version": o.uv_version,
                    "package_versions": o.package_versions,
                    "onnx_asr_ready": o.onnx_asr_ready,
                    "model_warmed": o.model_warmed,
                    "model_warm_error": o.model_warm_error,
                    // Extras and model readiness are distinct from successful
                    // base dependency installation.
                    "full_perception_ready": o.full_perception_ready,
                    "extras_note": o.extras_note,
                    "perception_ok_after": sidecar_ok,
                    "warnings": warnings,
                });
                if model_preparation_failed || extras_missing {
                    st.jobs.finish_with_warnings(&jid, result);
                } else {
                    st.jobs.finish(&jid, result);
                }
            }
            Err(e) => st.jobs.fail(&jid, e),
        }
    });
    Ok(VerbResult::ok(json!({"job_id": job_id})))
}

/// `system.setup_matte {path?, rationale?}` — make AI background removal usable
/// LOCALLY (the ffmpeg pattern, SEAMLESS — the user never runs pip / sees a
/// terminal). NO `path` → one-click DOWNLOAD of the 14 MB RVM model (onnxruntime
/// rides the perception venv, so the install is just the model); WITH `path` →
/// BROWSE-to-existing, point at an rvm `.onnx` already on disk. On success the
/// doctor `matte` card flips missing→ready. The runtime is invisible plumbing
/// cutd manages — one app.
pub(super) async fn system_setup_matte(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        /// `rvm` (default — automatic, license-clean, CPU-ok) or `matanyone` (the
        /// PREMIUM opt-in — NVIDIA, non-commercial, target-assigned, cleaner edges).
        #[serde(default)]
        model: Option<cut_core::MatteModel>,
        /// Browse-to-existing: a model file already on disk (rvm `.onnx` /
        /// matanyone `.pth`). Omit to download.
        #[serde(default)]
        path: Option<String>,
        /// REQUIRED to install `matanyone`: explicit acceptance that the MatAnyone2
        /// weights are NON-COMMERCIAL (NTU S-Lab License 1.0). Download-on-consent.
        #[serde(default)]
        accept_noncommercial: Option<bool>,
        #[serde(default)]
        #[allow(dead_code)] // recorded for the audit trail
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let model = a.model.unwrap_or_default();
    let browse = a.path.as_deref().map(str::trim).filter(|s| !s.is_empty());

    // A supplied prepared runtime owns the standard RVM path. It is already
    // hash-verified by Cut, so setup must report readiness without writing into
    // the immutable tree or consulting a mutable browse/app-data/download path.
    if let Some(runtime) = crate::matte::native_runtime::prepared_rvm_runtime()? {
        if matches!(model, cut_core::MatteModel::Matanyone) {
            return Err(CutError::new(
                error_codes::SIDECAR,
                "prepared native runtime does not admit the MatAnyone2/SAM2 matte stack",
                "use the standard RVM model supplied by the prepared runtime",
            ));
        }
        if browse.is_some() {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "prepared native runtime fixes the RVM model identity",
                "remove path; the prepared runtime does not accept a mutable matte model override",
            ));
        }
        let report = state.doctor_rescan().await;
        let ready = report
            .cards
            .iter()
            .find(|card| card.id == "matte")
            .map(|card| matches!(card.status, crate::doctor::CardStatus::Ok))
            .unwrap_or(false);
        return Ok(VerbResult::ok(json!({
            "model": "rvm",
            "source": "prepared-native-runtime",
            "path": runtime.model.display().to_string(),
            "matte_ready": ready,
            "prepared_runtime": {
                "contract": runtime.binding.contract,
                "manifest_sha256": runtime.binding.manifest_sha256,
                "receipt_sha256": runtime.binding.receipt_sha256,
                "model_id": runtime.binding.model_id,
                "model_sha256": runtime.binding.model_sha256,
            },
            "doctor": serde_json::to_value(report)?,
        })));
    }

    match model {
        // ---- the PREMIUM tier: MatAnyone2 (NVIDIA, non-commercial) -----------
        cut_core::MatteModel::Matanyone => {
            // Browse-to-existing: validate + persist a matanyone2 .pth (instant).
            if let Some(p) = browse {
                if !std::path::Path::new(p).is_file() {
                    return Err(CutError::new(
                        error_codes::NOT_FOUND,
                        format!("'{p}' is not a file"),
                        "point at an existing matanyone2 .pth, or omit path to download it",
                    ));
                }
                crate::matte::write_matanyone_model_setting(Some(p))?;
                let report = state.doctor_rescan().await;
                return Ok(VerbResult::ok(json!({
                    "model": "matanyone", "source": "browse", "path": p,
                    "doctor": serde_json::to_value(report)?,
                })));
            }
            // CONSENT GATE: the weights are non-commercial — require explicit accept.
            if a.accept_noncommercial != Some(true) {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "MatAnyone2 (the premium background-removal model) is licensed NON-COMMERCIALLY \
                     under the NTU S-Lab License 1.0. ShellX Cut ships only the integration; the \
                     weights are downloaded to YOUR machine on YOUR consent and responsibility.",
                    "the premium matte requires explicit consent to the non-commercial licence",
                )
                .with_suggested_action(
                    "re-run system.setup_matte{model:\"matanyone\", accept_noncommercial:true} to download it",
                ));
            }
            // Refuse only the measured dropped generation, before a job,
            // runtime prerequisite or any large download. Unknown is admitted.
            run_blocking("check premium graphics card", || {
                crate::perception_setup::require_premium_gpu_generation(
                    cut_media::hwencode::cuda_compute_capability(),
                )
            })
            .await?;
            // Windows: torch needs Microsoft's Visual C++ runtime first.
            crate::vc_runtime::require_for("Cleaner background removal")?;
            // Consented install: several GB of tools + 141 MB MatAnyone2 + 646 MB SAM2 weights.
            let job = state.jobs.create("setup_matte");
            let job_id = job.job_id.clone();
            let st = state.clone();
            let jobs = state.jobs.clone();
            jobs.spawn(&job_id, async move {
                let jid = job.job_id.clone();
                st.jobs.progress(
                    &jid,
                    0.01,
                    Some("premium background removal: starting".into()),
                );
                let jobs = st.jobs.clone();
                let jid_cl = jid.clone();
                let cancellation = crate::jobs::current_job_cancellation();
                let worker_cancellation = cancellation.clone();
                let outcome = run_blocking("system.setup_matte", move || {
                    let progress = move |frac: f32, msg: &str| {
                        jobs.progress(
                            &jid_cl,
                            (0.01 + frac * 0.97).min(0.99),
                            Some(format!("premium background removal: {msg}")),
                        );
                    };
                    crate::perception_setup::setup_matanyone(&progress, &worker_cancellation)
                })
                .await;
                // A cancelled install ends as cancelled, whatever step it was
                // in; the unfinished premium environment is already removed.
                if outcome.is_err() {
                    if let Some(reason) = cancellation.reason() {
                        return st.jobs.cancel_from_worker(&jid, reason);
                    }
                }
                match outcome {
                    Ok(o) => {
                        st.jobs.progress(
                            &jid,
                            0.99,
                            Some("premium background removal: re-scanning".into()),
                        );
                        let report = st.doctor_rescan().await;
                        let ready = report
                            .cards
                            .iter()
                            .find(|c| c.id == "matte_premium")
                            .map(|c| matches!(c.status, crate::doctor::CardStatus::Ok))
                            .unwrap_or(false);
                        st.jobs.finish(
                            &jid,
                            json!({
                                "model": "matanyone",
                                "venv_python": o.venv_python,
                                "uv_version": o.uv_version,
                                "checkpoint": o.checkpoint,
                                "matanyone_ready": o.matanyone_ready,
                                "cuda_available": o.cuda_available,
                                "matte_premium_ready": ready,
                            }),
                        );
                    }
                    Err(e) => st.jobs.fail(&jid, e),
                }
            });
            Ok(VerbResult::ok(json!({"job_id": job_id})))
        }

        // ---- the DEFAULT tier: RVM (automatic, license-clean, CPU-ok) ---------
        cut_core::MatteModel::Rvm => {
            // Browse-to-existing: validate + persist, synchronous (instant).
            if let Some(p) = browse {
                if !std::path::Path::new(p).is_file() {
                    return Err(CutError::new(
                        error_codes::NOT_FOUND,
                        format!("'{p}' is not a file"),
                        "point at an existing RVM .onnx model file, or omit path to download it",
                    ));
                }
                crate::matte::write_model_setting(Some(p))?;
                let report = state.doctor_rescan().await;
                return Ok(VerbResult::ok(json!({
                    "model": "rvm", "source": "browse", "path": p,
                    "doctor": serde_json::to_value(report)?,
                })));
            }
            // One-click download: a progress job (the model is ~14 MB).
            let job = state.jobs.create("setup_matte");
            let job_id = job.job_id.clone();
            let st = state.clone();
            let jobs = state.jobs.clone();
            jobs.spawn(&job_id, async move {
                let jid = job.job_id.clone();
                st.jobs
                    .progress(&jid, 0.01, Some("background removal: starting".into()));
                let jobs = st.jobs.clone();
                let jid_cl = jid.clone();
                let outcome = run_blocking("system.setup_matte", move || {
                    let progress = move |frac: f32, msg: &str| {
                        jobs.progress(
                            &jid_cl,
                            (0.01 + frac * 0.97).min(0.99),
                            Some(format!("background removal: {msg}")),
                        );
                    };
                    crate::matte::install_model(&progress)
                })
                .await;
                match outcome {
                    Ok(model) => {
                        st.jobs.progress(
                            &jid,
                            0.99,
                            Some("background removal: re-scanning".into()),
                        );
                        let report = st.doctor_rescan().await;
                        let ready = report
                            .cards
                            .iter()
                            .find(|c| c.id == "matte")
                            .map(|c| matches!(c.status, crate::doctor::CardStatus::Ok))
                            .unwrap_or(false);
                        st.jobs.finish(
                            &jid,
                            json!({ "model": model.display().to_string(), "matte_ready": ready }),
                        );
                    }
                    Err(e) => st.jobs.fail(&jid, e),
                }
            });
            Ok(VerbResult::ok(json!({"job_id": job_id})))
        }
    }
}

#[cfg(test)]
mod screenshot_error_tests {
    use super::*;
    use serde_json::json;

    /// Structured client failure → verb error names the stage, the human
    /// cause, and the attempt count.
    #[test]
    fn structured_capture_failure_names_stage_cause_and_attempts() {
        let reply = json!({
            "type": "screenshot_result",
            "error": {
                "code": "capture_failed",
                "stage": "dom-rasterize",
                "message": "capture failed at dom-rasterize: error event on <link> (app.css)",
                "attempts": 2,
            }
        });
        let e = screenshot_capture_error(&reply);
        assert_eq!(e.code, error_codes::JOB_FAILED);
        assert!(
            e.message.contains("stage 'dom-rasterize'"),
            "message: {}",
            e.message
        );
        assert!(
            e.cause.contains("error event on <link> (app.css)"),
            "cause: {}",
            e.cause
        );
        assert!(e.cause.contains("after 2 attempt(s)"), "cause: {}", e.cause);
        assert!(
            !format!("{e:?}").contains("[object Event]"),
            "the opaque pre-hardening string must never appear"
        );
    }

    /// Compat: an old UI client that sent `error` as a plain string still
    /// yields its text (stage honestly 'unknown').
    #[test]
    fn legacy_string_error_still_surfaces_its_text() {
        let reply = json!({"type": "screenshot_result", "error": "capture blew up"});
        let e = screenshot_capture_error(&reply);
        assert!(
            e.message.contains("stage 'unknown'"),
            "message: {}",
            e.message
        );
        assert!(e.cause.contains("capture blew up"), "cause: {}", e.cause);
    }

    /// No error field at all (malformed reply) → the original honest fallback.
    #[test]
    fn errorless_reply_keeps_the_no_image_fallback() {
        let reply = json!({"type": "screenshot_result", "notes": []});
        let e = screenshot_capture_error(&reply);
        assert_eq!(e.message, "UI client returned no image");
        assert!(e.cause.contains("lacked png_base64"), "cause: {}", e.cause);
    }
}
