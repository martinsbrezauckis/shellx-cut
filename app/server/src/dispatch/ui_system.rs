use super::*;

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
    .with_suggested_action(
        "call system.doctor to read the live loopback address, then open it in a browser (or use render.frame for composed pixels)",
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
        .ok_or_else(|| {
            CutError::new(
                error_codes::JOB_FAILED,
                "UI client returned no image",
                format!("screenshot_result lacked png_base64: {reply}"),
            )
        })?;
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
    let dir = match project_paths(state).await {
        Ok((dir, _r, _p)) => dir.join("frames"),
        Err(_) => std::env::temp_dir().join("shellx-cut"),
    };
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("ui_{}.png", chrono::Utc::now().timestamp_millis()));
    std::fs::write(&path, &bytes)?;
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
pub(super) async fn debug_screenshot(args: Value) -> Result<VerbResult, CutError> {
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
    let dir = std::env::temp_dir().join("shellx-cut");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!(
        "debug_shot_{}.png",
        chrono::Utc::now().timestamp_millis()
    ));
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
    }
    let a: Args = parse_args(args)?;
    let report = if a.refresh {
        state.doctor_rescan().await
    } else {
        state.doctor_cached().await
    };
    Ok(VerbResult::ok(serde_json::to_value(report)?))
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
    }
    let a: Args = parse_args(args)?;
    // Fail fast on an unknown tool id (the registry is the allow-list; the verb
    // schema also enum-restricts it, but we never trust only the schema).
    if a.tool != "ffmpeg" {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown tool '{}'", a.tool),
            "system.fetch_tool serves a BUILT-IN registry only (v1: \"ffmpeg\"); there is no caller-supplied URL",
        )
        .with_suggested_action("pass tool:\"ffmpeg\", or install other tools manually onto PATH"));
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
                let ffmpeg_ok = report
                    .cards
                    .iter()
                    .find(|c| c.id == "ffmpeg")
                    .map(|c| matches!(c.status, crate::doctor::CardStatus::Ok))
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
                    }),
                );
            }
            Err(e) => st.jobs.fail(&jid, e),
        }
    });
    Ok(VerbResult::ok(json!({"job_id": job_id, "tool": a.tool})))
}

/// serde default for `setup_perception.warm_model` (pre-fetch the model by default
/// so the first transcription is instant).
fn default_true() -> bool {
    true
}

/// `system.setup_perception {warm_model?, rationale?}` — consented provisioning of
/// the Python perception venv as a job (the background-job contract: returns {job_id}). Downloads uv
/// (sha256-verified via the fetch registry), installs a standalone CPython 3.12,
/// builds the app-data sidecar venv, and `uv pip install`s the bundled pinned
/// requirements (onnx-asr Parakeet/Canary STT + the perception stack). This closes the
/// cold-install gap — the bundle ships only instruments.py + requirements.txt, and
/// system python is too old on real desktops (macOS 3.9 < onnx-asr's 3.10 floor).
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
        let outcome = run_blocking("system.setup_perception", move || {
            let progress = move |frac: f32, msg: &str| {
                // Reserve 1.0 for the post-install doctor re-scan.
                jobs.progress(
                    &jid_cl,
                    (0.01 + frac * 0.97).min(0.99),
                    Some(format!("perception setup: {msg}")),
                );
            };
            crate::perception_setup::setup_perception(warm, &progress)
        })
        .await;
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
                st.jobs.finish(
                    &jid,
                    json!({
                        "venv_python": o.venv_python,
                        "uv_version": o.uv_version,
                        "onnx_asr_ready": o.onnx_asr_ready,
                        "model_warmed": o.model_warmed,
                        // BEST-EFFORT extras: false here does NOT mean failure —
                        // transcription works on the onnx-asr base regardless.
                        "full_perception_ready": o.full_perception_ready,
                        "extras_note": o.extras_note,
                        "perception_ok_after": sidecar_ok,
                    }),
                );
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
            // Consented install: a progress job (cu128 torch ~3 GB + 135 MB ckpt).
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
                let outcome = run_blocking("system.setup_matte", move || {
                    let progress = move |frac: f32, msg: &str| {
                        jobs.progress(
                            &jid_cl,
                            (0.01 + frac * 0.97).min(0.99),
                            Some(format!("premium background removal: {msg}")),
                        );
                    };
                    crate::perception_setup::setup_matanyone(&progress)
                })
                .await;
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
                                "checkpoint": o.checkpoint,
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
