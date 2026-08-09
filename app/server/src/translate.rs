//! translate.rs — multilingual SUBTITLE/TRANSCRIPT TRANSLATION (TEXT only; no
//! dubbing, no audio/TTS). The PURE half of `captions.translate` +
//! `transcript.translate`: backend selection, the CLI prompt/parse, the local
//! sidecar's command shape, and the timestamp-preserving cue→cue + word→segment
//! mapping, and the bounded CLI/local sidecar execution used by the verb
//! handlers. Pure helpers remain unit-tested without spawning anything; the
//! backend orchestration is integration-tested with fake CLIs/runners.
//!
//! Backend design contract:
//! - PRIMARY = the user's OWN subscription CLI (the SAME CLI `agent.chat` /
//!   `assets.generate` drive — claude / codex / grok). High-context LLM
//!   translation is the best-quality path (idioms, names, gendered pronouns,
//!   reading-time fit); pyVideoTrans validated LLM translation as superior to
//!   pure MT. We do NOT wire the cutd MCP server (translation is pure text in →
//!   text out, no editing tools); we just prompt the CLI with the numbered cues
//!   and parse the per-cue translations back. claude is still the default
//!   preference; codex and grok are also routed through explicit command shapes
//!   and parsed tolerantly, with provenance recorded in the receipt.
//! - FALLBACK = a LOCAL self-hosted model, used ONLY when no CLI agent is
//!   available (offline). Default = Opus-MT (Helsinki-NLP, MarianMT) — light
//!   (~300 MB per pair), CC-BY-4.0 (commercial-OK), per-pair. MADLAD-400
//!   (Apache-2.0, 419 langs incl. Latvian, ~3B) is the universal-but-heavy
//!   alternative, selectable via `model`. A perception-venv python sidecar
//!   (`translate_runner.py`) mirrors the STT/matte/face runner pattern
//!   (`TRANSLATE_RUNNER_PY` / `TRANSLATE_RUNNER_SCRIPT` overrides). The model
//!   DOWNLOAD is gated behind first-use (the runner fetches on first run; an
//!   offline+uncached run fails honestly) — NOT bundled. NLLB-200 is FORBIDDEN
//!   (CC-BY-NC, non-commercial).
//! - Selection: `backend:"auto"` (default) = CLI if available, else local;
//!   `"cli"` / `"local"` force one.

use crate::jobs::{run_owned, ProcessControl, ProcessTermination};
use cut_core::{error_codes, CutError};
use std::path::PathBuf;

/// The agents the translator can route to, in preference order — the same CLIs
/// the agent chat box uses (detection is shared with gen.rs / chat.rs).
pub const TRANSLATE_AGENTS: &[&str] = &["claude", "codex", "grok"];

/// The resolved local-sidecar runtime: the perception python + the one-shot
/// `translate_runner.py` script. Mirrors faces.rs / matte.rs.
#[derive(Debug, Clone)]
pub struct Runtime {
    pub python: PathBuf,
    pub script: PathBuf,
}

/// The one-shot translate script (ships beside `instruments.py` in the sidecar
/// payload / tauri resources).
pub fn runner_script() -> PathBuf {
    let (_py, instruments) = cut_perception::sidecar_paths();
    instruments
        .parent()
        .map(|d| d.join("translate_runner.py"))
        .unwrap_or_else(|| PathBuf::from("translate_runner.py"))
}

/// `Some` when the perception python + the translate script both exist (so the
/// LOCAL backend is wired). `None` → the local backend reports a setup hint.
/// The runner surfaces a crisp error if transformers/sentencepiece is missing
/// or the model cannot be fetched offline. `TRANSLATE_RUNNER_PY` /
/// `TRANSLATE_RUNNER_SCRIPT` override the python / script (dev → the venv +
/// repo script).
pub fn runtime() -> Option<Runtime> {
    let python = std::env::var_os("TRANSLATE_RUNNER_PY")
        .map(PathBuf::from)
        .unwrap_or_else(|| cut_perception::sidecar_paths().0);
    let script = std::env::var_os("TRANSLATE_RUNNER_SCRIPT")
        .map(PathBuf::from)
        .unwrap_or_else(runner_script);
    (python.exists() && script.exists()).then_some(Runtime { python, script })
}

/// Which translation backend will run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// The user's subscription CLI agent (the resolved agent name rides separately).
    Cli,
    /// The local self-hosted MT sidecar (Opus-MT / MADLAD).
    Local,
}

/// Normalize a language tag/name: trim + lowercase. We pass it through to the
/// CLI prompt verbatim (LLMs accept "spanish" / "es" / "lv" / "Latvian"); the
/// local Opus-MT path needs a 2-letter code (validated there).
pub fn normalize_lang(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Is `agent`'s CLI installed anywhere we can launch it? (binary name == agent name
/// for all three.) Uses the FULL resolution ladder (process PATH FIRST, then the
/// explicit install dirs incl. grok's off-PATH ~/.grok/bin / a Finder-stripped-PATH
/// .app's Homebrew dirs) via `gen::resolve_agent` — NOT a process-PATH-only scan —
/// so an off-PATH grok still detects and `run_translation` then spawns the RESOLVED
/// path. Shared with `gen::detect` / `chat::detect`; the on-PATH case is unchanged
/// because `resolve_agent` searches the process PATH first.
pub fn detect_cli(agent: &str) -> bool {
    TRANSLATE_AGENTS.contains(&agent) && crate::gen::resolve_agent(agent).is_some()
}

/// Is the CLI translation path for `agent` the PROVEN path? Only claude is wired
/// + verified in v1 (matching chat.rs's `is_wired`); codex/grok are attempted
/// best-effort (tolerant parse) but not guaranteed.
pub fn is_cli_proven(agent: &str) -> bool {
    agent == "claude"
}

/// Available CLI translation agents in the same preference order as auto mode.
pub fn available_cli_agents() -> Vec<&'static str> {
    TRANSLATE_AGENTS
        .iter()
        .copied()
        .filter(|a| detect_cli(a))
        .collect()
}

/// Resolve the backend from the request + what is actually available.
/// `"auto"` (or absent): CLI if any agent is installed, else local. `"cli"` /
/// `"local"` force one (and error if that one is unavailable). Returns the
/// backend or a structured error explaining what is missing.
pub fn select_backend(
    requested: Option<&str>,
    cli_available: bool,
    local_available: bool,
) -> Result<Backend, CutError> {
    let want = requested.map(str::trim).unwrap_or("auto");
    match want {
        "cli" => {
            if cli_available {
                Ok(Backend::Cli)
            } else {
                Err(no_cli_error())
            }
        }
        "local" => {
            if local_available {
                Ok(Backend::Local)
            } else {
                Err(no_local_error())
            }
        }
        "auto" | "" => {
            if cli_available {
                Ok(Backend::Cli)
            } else if local_available {
                Ok(Backend::Local)
            } else {
                Err(CutError::new(
                    error_codes::SIDECAR,
                    "no translation backend available",
                    "neither a subscription CLI agent (claude/codex/grok) nor the local MT sidecar is set up",
                )
                .with_suggested_action(
                    "install + log in to claude (best quality), or set up the local model (Opus-MT) in the perception venv",
                ))
            }
        }
        other => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown backend '{other}'"),
            "backend is auto (default) | cli | local",
        )),
    }
}

fn no_cli_error() -> CutError {
    CutError::new(
        error_codes::SIDECAR,
        "no subscription CLI agent available for backend:\"cli\"",
        "no claude/codex/grok CLI found on PATH",
    )
    .with_suggested_action("install + log in to claude, or use backend:\"local\" / \"auto\"")
}

fn no_local_error() -> CutError {
    CutError::new(
        error_codes::SIDECAR,
        "the local MT sidecar is not available for backend:\"local\"",
        "translate_runner.py or its python (perception venv) was not found",
    )
    .with_suggested_action(
        "install the perception sidecar (transformers + sentencepiece) in its venv, or use backend:\"cli\" / \"auto\"",
    )
}

/// The result of running a translation backend (CLI agent or local sidecar) over
/// an ordered list of text segments.
pub(crate) struct TranslateOutcome {
    /// One translation per input segment, IN ORDER (length == input length).
    pub(crate) translations: Vec<String>,
    /// "cli" | "local".
    pub(crate) backend: String,
    /// The agent name (CLI) or the HF model id (local) that ran — receipt provenance.
    pub(crate) model: String,
    /// The resolved CLI agent (None for the local backend).
    pub(crate) agent: Option<String>,
    /// Honesty flag: true when this path is the VERIFIED one (claude CLI, or the
    /// local sidecar); false for a best-effort CLI agent (codex/grok) whose
    /// translation parse is attempted but not guaranteed.
    pub(crate) proven: bool,
    /// Non-fatal provenance notes from a backend. Auto mode does not hide a CLI
    /// runtime failure behind local MT fallback.
    pub(crate) warnings: Vec<String>,
}

/// Resolve the backend (auto/cli/local) given what is installed, then translate
/// `segments` (cues or sentence-ish transcript segments). The PRIMARY path is
/// the user's subscription CLI (claude/codex/grok) — pure text in → text out, NO
/// MCP/tools (unlike agent.chat); the FALLBACK is the local Opus-MT/MADLAD
/// sidecar. Spawns are bounded by `timeout_ms`. `source_lang` may be None for
/// the CLI (the LLM auto-detects); auto selects the LOCAL path only when no CLI
/// agent is available up front. Once a CLI agent launches, auth/quota/runtime
/// failures stay honest instead of silently degrading to local MT. The LOCAL
/// path REQUIRES `source_lang` (Opus-MT is per-pair). Auto/CLI mode walks the
/// available CLI agents in preference order before giving up; it never hides a
/// CLI runtime failure by silently dropping to the local script. Returns a
/// structured error on any failure (never a fake/partial translation).
pub(crate) async fn run_translation(
    backend_req: Option<&str>,
    source_lang: Option<&str>,
    target_lang: &str,
    segments: &[String],
    model: Option<&str>,
    timeout_ms: Option<u64>,
) -> Result<TranslateOutcome, CutError> {
    run_translation_once(
        backend_req,
        source_lang,
        target_lang,
        segments,
        model,
        timeout_ms,
    )
    .await
}

async fn run_translation_once(
    backend_req: Option<&str>,
    source_lang: Option<&str>,
    target_lang: &str,
    segments: &[String],
    model: Option<&str>,
    timeout_ms: Option<u64>,
) -> Result<TranslateOutcome, CutError> {
    let cli_agents = available_cli_agents();
    let runtime = runtime();
    let backend = select_backend(backend_req, !cli_agents.is_empty(), runtime.is_some())?;

    match backend {
        Backend::Cli => {
            if cli_agents.is_empty() {
                return Err(CutError::new(
                    error_codes::SIDECAR,
                    "no CLI agent resolved",
                    "claude/codex/grok not found on PATH",
                ));
            }
            let mut failures: Vec<String> = Vec::new();
            for agent in cli_agents {
                match run_translation_cli_agent(
                    agent,
                    source_lang,
                    target_lang,
                    segments,
                    model,
                    timeout_ms,
                )
                .await
                {
                    Ok(mut outcome) => {
                        if !failures.is_empty() {
                            outcome.warnings.push(format!(
                                "earlier CLI translation attempts failed: {}",
                                failures.join("; ")
                            ));
                        }
                        return Ok(outcome);
                    }
                    Err(error) => {
                        failures.push(format!("{agent}: {}", error.message));
                    }
                }
            }
            Err(CutError::new(
                error_codes::SIDECAR,
                format!("all CLI translation agents failed: {}", failures.join("; ")),
                "CLI translation failed; local MT was not used because a CLI agent existed",
            )
            .with_suggested_action(
                "retry after CLI auth/quota recovers, or explicitly pass backend:\"local\"",
            ))
        }
        Backend::Local => {
            let rt = runtime.ok_or_else(|| {
                CutError::new(
                    error_codes::SIDECAR,
                    "the local MT sidecar is not available",
                    "translate_runner.py / its python were not found",
                )
            })?;
            let src = source_lang
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    CutError::new(
                        error_codes::INVALID_ARGS,
                        "source_lang is required for the local backend",
                        "the local Opus-MT model is per-pair and cannot auto-detect the source language",
                    )
                    .with_suggested_action(
                        "pass source_lang (e.g. \"en\"), or use the CLI backend which auto-detects",
                    )
                })?;
            let timeout = std::time::Duration::from_millis(
                timeout_ms.unwrap_or(1_800_000).clamp(10_000, 3_600_000),
            );
            let mut command = tokio::process::Command::new(&rt.python);
            command
                .arg(&rt.script)
                .arg("--src")
                .arg(src)
                .arg("--tgt")
                .arg(target_lang);
            if let Some(m) = model.filter(|m| !m.is_empty()) {
                command.arg("--model").arg(m);
            }
            command
                .env("PYTHONIOENCODING", "utf-8")
                .env("PYTHONUTF8", "1");
            let stdin_payload = serde_json::json!({ "segments": segments }).to_string();
            let control = ProcessControl::for_operation(timeout);
            let out =
                match run_owned(&mut command, Some(stdin_payload.as_bytes()), &control).await {
                    Ok(output) => output,
                    Err(error) => match error.termination() {
                        Some(ProcessTermination::DeadlineExceeded) => return Err(CutError::new(
                            error_codes::SIDECAR,
                            format!(
                                "local translation timed out after {}ms",
                                timeout.as_millis()
                            ),
                            "the first run downloads the model; raise timeout_ms or pre-fetch it",
                        )),
                        Some(ProcessTermination::Cancelled(reason)) => {
                            return Err(CutError::new(
                                "job_cancelled",
                                format!("local translation cancelled ({})", reason.label()),
                                "the owning background job stopped this external worker",
                            ))
                        }
                        None => {
                            return Err(CutError::new(
                                error_codes::IO,
                                format!("the local translate runner errored: {error}"),
                                "local MT failed",
                            ))
                        }
                    },
                };
            if out.diagnostics_truncated() {
                tracing::warn!(
                    "local translation runner diagnostics exceeded the retained output cap"
                );
            }
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Err(CutError::new(
                    error_codes::SIDECAR,
                    format!("local translation failed: {}", stderr.trim()),
                    "the local MT model could not be loaded/run (offline + uncached, or an unsupported pair)",
                )
                .with_suggested_action("install transformers+sentencepiece in the perception venv and allow the first-use model download, or use the CLI backend"));
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            let parsed = parse_runner_json(&stdout).ok_or_else(|| {
                CutError::new(
                    error_codes::SIDECAR,
                    "the local translate runner returned no JSON",
                    format!("got: {}", stdout.trim()),
                )
            })?;
            if parsed.translations.len() != segments.len() {
                return Err(CutError::new(
                    error_codes::SIDECAR,
                    format!(
                        "local MT returned {} translations for {} segments",
                        parsed.translations.len(),
                        segments.len()
                    ),
                    "one translation per segment is required to preserve timestamps",
                ));
            }
            let model = match parsed.backend.as_deref() {
                Some(fam) if !fam.is_empty() => format!("{} ({fam})", parsed.model),
                _ => parsed.model,
            };
            Ok(TranslateOutcome {
                translations: parsed.translations,
                backend: "local".into(),
                model,
                agent: None,
                proven: true,
                warnings: Vec::new(),
            })
        }
    }
}

async fn run_translation_cli_agent(
    agent: &'static str,
    source_lang: Option<&str>,
    target_lang: &str,
    segments: &[String],
    model: Option<&str>,
    timeout_ms: Option<u64>,
) -> Result<TranslateOutcome, CutError> {
    let cmd = build_cli_command(agent, model).ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            format!("no translation command for agent '{agent}'"),
            "claude|codex|grok",
        )
    })?;
    let prompt = build_cli_prompt(source_lang, target_lang, segments);
    let timeout =
        std::time::Duration::from_millis(timeout_ms.unwrap_or(180_000).clamp(10_000, 600_000));
    let suffix = {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        C.fetch_add(1, Ordering::Relaxed)
    };
    let ws = std::env::temp_dir().join(format!("cutd-xlate-{}-{suffix}", std::process::id()));
    std::fs::create_dir_all(&ws).ok();
    let resolved_args: Vec<String> = if cmd.via_stdin {
        cmd.args.clone()
    } else {
        let pf = ws.join("prompt.txt");
        if let Err(e) = std::fs::write(&pf, &prompt) {
            let _ = std::fs::remove_dir_all(&ws);
            return Err(CutError::new(
                error_codes::IO,
                "write prompt file",
                e.to_string(),
            ));
        }
        let pfs = pf.to_string_lossy().into_owned();
        cmd.args
            .iter()
            .map(|x| {
                if x == "__PROMPT_FILE__" {
                    pfs.clone()
                } else {
                    x.clone()
                }
            })
            .collect()
    };
    let agent_path = crate::gen::resolve_agent(agent)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| cmd.cmd.clone());
    let mut command =
        crate::gen::agent_tokio_command(std::path::Path::new(&agent_path), &resolved_args)
            .map_err(|e| {
                let _ = std::fs::remove_dir_all(&ws);
                CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("cannot launch the {} CLI safely: {e}", cmd.cmd),
                    "the resolved Windows batch shim received an unsafe path or argument",
                )
            })?;
    command.current_dir(&ws);
    let control = ProcessControl::for_operation(timeout);
    let out = run_owned(
        &mut command,
        cmd.via_stdin.then_some(prompt.as_bytes()),
        &control,
    )
    .await;
    let _ = std::fs::remove_dir_all(&ws);
    let out = match out {
        Ok(output) => output,
        Err(error) => match error.termination() {
            Some(ProcessTermination::DeadlineExceeded) => {
                return Err(CutError::new(
                    error_codes::SIDECAR,
                    format!(
                        "the translation turn timed out after {}ms",
                        timeout.as_millis()
                    ),
                    "raise timeout_ms or translate fewer cues at once",
                ))
            }
            Some(ProcessTermination::Cancelled(reason)) => {
                return Err(CutError::new(
                    "job_cancelled",
                    format!("translation CLI cancelled ({})", reason.label()),
                    "the owning background job stopped this external worker",
                ))
            }
            None => {
                return Err(CutError::new(
                    error_codes::IO,
                    format!("the {} CLI errored: {error}", cmd.cmd),
                    "translation CLI failed",
                ))
            }
        },
    };
    if out.diagnostics_truncated() {
        tracing::warn!(
            agent,
            "translation CLI diagnostics exceeded the retained output cap"
        );
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let env = crate::chat::parse_result(&stdout);
    if !env.ok {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(CutError::new(
            error_codes::SIDECAR,
            format!(
                "the {} CLI did not complete the translation: {}",
                cmd.cmd,
                env.reply.trim()
            ),
            if stderr.trim().is_empty() {
                "the CLI turn reported an error".to_string()
            } else {
                stderr.trim().to_string()
            },
        )
        .with_suggested_action("retry, or use backend:\"local\""));
    }
    let translations = parse_translation_array(&env.reply, segments.len())?;
    Ok(TranslateOutcome {
        translations,
        backend: "cli".into(),
        model: agent.to_string(),
        agent: Some(agent.to_string()),
        proven: is_cli_proven(agent),
        warnings: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// CLI path: command + prompt + parse.
// ---------------------------------------------------------------------------

/// A resolved CLI invocation for a translation turn.
#[derive(Debug, Clone, PartialEq)]
pub struct CliCommand {
    pub cmd: String,
    pub args: Vec<String>,
    /// Prompt delivered on STDIN (claude `-p` / codex `exec -`) vs a prompt file
    /// (grok `--prompt-file`, substituting `__PROMPT_FILE__`).
    pub via_stdin: bool,
}

/// Build the translation-CLI invocation. NO MCP / NO tools — translation is pure
/// text in → text out (unlike `agent.chat`, which wires the cutd verbs). claude
/// is the proven path; codex/grok are best-effort. Returns `None` for an unknown
/// agent.
pub fn build_cli_command(agent: &str, model: Option<&str>) -> Option<CliCommand> {
    match agent {
        "claude" => {
            let mut args = vec![
                "-p".into(),
                "--output-format".into(),
                "json".into(),
                // Pure text: forbid every tool so the turn can't web-search/edit.
                "--disallowedTools".into(),
                "*".into(),
            ];
            if let Some(m) = model.filter(|m| !m.is_empty()) {
                args.push("--model".into());
                args.push(m.into());
            }
            Some(CliCommand {
                cmd: "claude".into(),
                args,
                via_stdin: true,
            })
        }
        "codex" => {
            // Read-only sandbox (no file writes needed for text translation).
            let mut args = vec![
                "exec".into(),
                "-".into(),
                "--json".into(),
                "--sandbox".into(),
                "read-only".into(),
                "--skip-git-repo-check".into(),
            ];
            if let Some(m) = model.filter(|m| !m.is_empty()) {
                args.push("-m".into());
                args.push(m.into());
            }
            Some(CliCommand {
                cmd: "codex".into(),
                args,
                via_stdin: true,
            })
        }
        "grok" => {
            let mut args = vec![
                "--prompt-file".into(),
                "__PROMPT_FILE__".into(),
                "--output-format".into(),
                "json".into(),
                "--permission-mode".into(),
                "bypassPermissions".into(),
                "--disable-web-search".into(),
                "--no-memory".into(),
                "--max-turns".into(),
                "1".into(),
            ];
            args.push("--model".into());
            args.push(
                model
                    .filter(|m| !m.is_empty())
                    .unwrap_or("grok-build")
                    .into(),
            );
            Some(CliCommand {
                cmd: "grok".into(),
                args,
                via_stdin: false,
            })
        }
        _ => None,
    }
}

/// Build the subtitle-translation prompt: a numbered cue list + a strict
/// instruction to return ONE JSON array of per-cue translations, same order +
/// count, no merging/splitting. `source_lang` None → ask the model to
/// auto-detect. The model is told to translate cue-by-cue but use the
/// surrounding cues for context (the LLM advantage over per-line MT).
pub fn build_cli_prompt(source_lang: Option<&str>, target_lang: &str, cues: &[String]) -> String {
    let src = source_lang
        .map(|s| s.to_string())
        .unwrap_or_else(|| "auto-detect the source language".to_string());
    let mut lines = vec![
        "You are a professional subtitle translator working inside ShellX Cut, a video editor.".to_string(),
        format!("Translate the {} subtitle cues below into {target_lang}.", cues.len()),
        format!("Source language: {src}. Target language: {target_lang}."),
        "Rules:".to_string(),
        format!("- Return ONLY a JSON array with EXACTLY {} elements, one per input cue, IN THE SAME ORDER. No prose, no markdown, no code fences.", cues.len()),
        "- Each element is an object: {\"i\": <1-based cue number>, \"text\": \"<the translation>\"}.".to_string(),
        "- Translate cue-by-cue, but read the surrounding cues for context (idioms, names, gender, pronouns).".to_string(),
        "- Keep proper nouns; keep each translation concise enough to read in the same on-screen time.".to_string(),
        "- Do NOT merge or split cues. Exactly one output element per input cue. Preserve a cue that is only punctuation or numerals as-is.".to_string(),
        "Cues:".to_string(),
    ];
    for (i, c) in cues.iter().enumerate() {
        // One line per cue; newlines inside a cue are flattened so the numbered
        // list stays unambiguous (caption cues are short).
        lines.push(format!("{}. {}", i + 1, c.replace('\n', " ")));
    }
    lines.join("\n")
}

/// Parse the model's reply text into exactly `n` translations, IN ORDER.
/// Tolerant to: a flat JSON array of strings; a JSON array of `{"i","text"}`
/// objects (re-ordered by `i` when present); surrounding prose / code fences.
/// Errors (never silently wrong) when no array is found or the count differs —
/// a count mismatch means the model merged/split cues and the cue↔timestamp
/// mapping would be corrupt.
pub fn parse_translation_array(reply: &str, n: usize) -> Result<Vec<String>, CutError> {
    let arr = extract_json_array(reply).ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "the translator did not return a JSON array",
            "expected a JSON array of per-cue translations",
        )
        .with_suggested_action("retry, or switch backend (the CLI returned unparseable output)")
    })?;
    // Two accepted shapes.
    let mut out: Vec<(i64, String)> = Vec::new();
    for (idx, el) in arr.iter().enumerate() {
        match el {
            serde_json::Value::String(s) => out.push((idx as i64, s.clone())),
            serde_json::Value::Object(o) => {
                let text = o
                    .get("text")
                    .or_else(|| o.get("translation"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        CutError::new(
                            error_codes::SIDECAR,
                            "a translation element had no \"text\"",
                            "expected {\"i\":N,\"text\":\"...\"}",
                        )
                    })?;
                let i = o
                    .get("i")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(idx as i64 + 1);
                out.push((i, text.to_string()));
            }
            _ => {
                return Err(CutError::new(
                    error_codes::SIDECAR,
                    "a translation element was neither a string nor an object",
                    "expected strings or {\"i\",\"text\"} objects",
                ))
            }
        }
    }
    // Order by the declared index (stable for the string shape: idx == position).
    out.sort_by_key(|(i, _)| *i);
    let texts: Vec<String> = out.into_iter().map(|(_, t)| t).collect();
    if texts.len() != n {
        return Err(CutError::new(
            error_codes::SIDECAR,
            format!(
                "the translator returned {} cues but {n} were sent (it merged or split cues)",
                texts.len()
            ),
            "cue count must be preserved so each translation maps to its source cue's timestamp",
        )
        .with_suggested_action("retry the translation"));
    }
    Ok(texts)
}

/// Find the first top-level balanced JSON array `[ ... ]` in `s` and parse it.
/// Skips brackets inside string literals so a `]` inside a translation does not
/// truncate the array. Returns the parsed Vec, or None.
fn extract_json_array(s: &str) -> Option<Vec<serde_json::Value>> {
    let bytes = s.as_bytes();
    let start = s.find('[')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    let slice = &s[start..=i];
                    return serde_json::from_str::<Vec<serde_json::Value>>(slice).ok();
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Local path: Opus-MT model id + runner JSON.
// ---------------------------------------------------------------------------

/// The default local model id for a language pair. Opus-MT is per-pair; the
/// runner falls back to the `tc-big` variant when the small model 404s, and
/// `model` (e.g. a MADLAD-400 id) overrides this entirely. (Canonical Rust-side
/// reference for the id scheme the python runner mirrors; tested.)
#[allow(dead_code)]
pub fn opus_mt_model_id(src: &str, tgt: &str) -> String {
    format!("Helsinki-NLP/opus-mt-{src}-{tgt}")
}

/// The local sidecar's JSON output: the per-segment translations + provenance.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RunnerResult {
    pub translations: Vec<String>,
    pub model: String,
    #[serde(default)]
    pub backend: Option<String>,
}

/// Parse the runner's single JSON stdout line (tolerant to log lines before it:
/// take the last `{...}`-looking line).
pub fn parse_runner_json(stdout: &str) -> Option<RunnerResult> {
    for line in stdout.lines().rev() {
        let t = line.trim();
        if t.starts_with('{') {
            if let Ok(r) = serde_json::from_str::<RunnerResult>(t) {
                return Some(r);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Timestamp-preserving mapping helpers (pure).
// ---------------------------------------------------------------------------

/// Map `translations` back onto the source cues' timestamps, IN ORDER. Each
/// source cue keeps its EXACT `range_ms`; only the text changes. Errors on a
/// count mismatch (the timestamp↔text pairing would be wrong). Returns
/// `(range_ms, translated_text)` pairs.
pub fn map_translations_to_cues(
    cue_ranges: &[[u64; 2]],
    translations: &[String],
) -> Result<Vec<([u64; 2], String)>, CutError> {
    if cue_ranges.len() != translations.len() {
        return Err(CutError::new(
            error_codes::SIDECAR,
            format!(
                "{} translations for {} cues",
                translations.len(),
                cue_ranges.len()
            ),
            "one translation per cue is required to preserve timestamps",
        ));
    }
    Ok(cue_ranges
        .iter()
        .zip(translations.iter())
        .map(|(r, t)| (*r, t.trim().to_string()))
        .collect())
}

/// A segment of a word-level transcript: a contiguous run of words plus its
/// absolute time span — the unit the translator works on (sentence-ish).
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Inclusive word-index range into the source transcript.
    pub word_range: [usize; 2],
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Group a word-level transcript into translation segments: break at a speech
/// gap > `pause_ms` OR when the accumulated text would exceed `max_chars`. This
/// gives the translator coherent sentence-ish units (better quality than
/// per-word) while keeping each segment's [start,end] span for re-timing.
pub fn group_words_into_segments(
    words: &[(u64, u64, String)], // (start_ms, end_ms, word)
    pause_ms: u64,
    max_chars: usize,
) -> Vec<Segment> {
    let mut segs: Vec<Segment> = Vec::new();
    let mut cur: Option<Segment> = None;
    for (i, (s, e, w)) in words.iter().enumerate() {
        match &mut cur {
            None => {
                cur = Some(Segment {
                    word_range: [i, i],
                    start_ms: *s,
                    end_ms: *e,
                    text: w.clone(),
                });
            }
            Some(seg) => {
                let gap = s.saturating_sub(seg.end_ms);
                let would = seg.text.len() + 1 + w.len();
                if gap > pause_ms || would > max_chars {
                    segs.push(seg.clone());
                    cur = Some(Segment {
                        word_range: [i, i],
                        start_ms: *s,
                        end_ms: *e,
                        text: w.clone(),
                    });
                } else {
                    seg.text.push(' ');
                    seg.text.push_str(w);
                    seg.word_range[1] = i;
                    seg.end_ms = *e;
                }
            }
        }
    }
    if let Some(seg) = cur {
        segs.push(seg);
    }
    segs
}

/// Distribute a translated segment's text into per-token `WordSpan`-shaped
/// tuples spread across the segment's [start,end] span by token character
/// length. HONEST LIMIT: the model translates at SEGMENT grain, so per-word
/// timing here is LINEARLY INTERPOLATED inside the segment, not ASR-aligned
/// (forced alignment on the translated audio would be required for true
/// per-word timing — out of scope). Segment-level timing is exact. Returns
/// `(start_ms, end_ms, token)` in order; an empty translation yields nothing.
pub fn distribute_tokens(start_ms: u64, end_ms: u64, translated: &str) -> Vec<(u64, u64, String)> {
    let tokens: Vec<&str> = translated.split_whitespace().collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    let span = end_ms.saturating_sub(start_ms).max(tokens.len() as u64); // ≥1ms/token
    let total_chars: usize = tokens.iter().map(|t| t.chars().count().max(1)).sum();
    let total_chars = total_chars.max(1) as f64;
    let mut out = Vec::new();
    let mut acc = 0f64;
    for (k, tok) in tokens.iter().enumerate() {
        let w = (tok.chars().count().max(1)) as f64;
        let s = start_ms + (span as f64 * (acc / total_chars)).round() as u64;
        acc += w;
        let e = if k + 1 == tokens.len() {
            end_ms.max(s)
        } else {
            (start_ms + (span as f64 * (acc / total_chars)).round() as u64).max(s)
        };
        out.push((s, e, tok.to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_normalization() {
        assert_eq!(normalize_lang("  ES "), "es");
        assert_eq!(normalize_lang("Latvian"), "latvian");
    }

    #[test]
    fn backend_auto_prefers_cli_then_local() {
        assert_eq!(select_backend(None, true, true).unwrap(), Backend::Cli);
        assert_eq!(
            select_backend(Some("auto"), false, true).unwrap(),
            Backend::Local
        );
        assert_eq!(
            select_backend(Some("auto"), true, false).unwrap(),
            Backend::Cli
        );
        assert!(select_backend(Some("auto"), false, false).is_err());
    }

    #[test]
    fn backend_forced_errors_when_unavailable() {
        assert_eq!(
            select_backend(Some("cli"), true, false).unwrap(),
            Backend::Cli
        );
        assert!(select_backend(Some("cli"), false, true).is_err());
        assert_eq!(
            select_backend(Some("local"), false, true).unwrap(),
            Backend::Local
        );
        assert!(select_backend(Some("local"), true, false).is_err());
        assert!(select_backend(Some("nope"), true, true).is_err());
    }

    #[test]
    fn cli_agent_policy_rejects_unknown_and_marks_proven_path() {
        // Detection for supported agents depends on PATH, but unknown agents are
        // rejected before the resolution ladder and claude remains the proven path.
        assert!(!detect_cli("dalle"));
        assert!(is_cli_proven("claude"));
        assert!(!is_cli_proven("codex"));
    }

    #[test]
    fn claude_command_is_pure_text_no_tools() {
        let c = build_cli_command("claude", Some("claude-opus-4-8")).unwrap();
        assert_eq!(c.cmd, "claude");
        assert!(c.via_stdin);
        assert!(c.args.windows(2).any(|w| w == ["--output-format", "json"]));
        // every tool forbidden — translation needs none
        assert!(c.args.windows(2).any(|w| w == ["--disallowedTools", "*"]));
        // NO mcp wiring (unlike agent.chat)
        assert!(!c.args.iter().any(|a| a.contains("mcp")));
        assert!(c
            .args
            .windows(2)
            .any(|w| w == ["--model", "claude-opus-4-8"]));
    }

    #[test]
    fn codex_and_grok_commands_build() {
        let c = build_cli_command("codex", None).unwrap();
        assert!(c.via_stdin);
        assert!(c.args.contains(&"exec".to_string()));
        assert!(c.args.windows(2).any(|w| w == ["--sandbox", "read-only"]));
        let g = build_cli_command("grok", None).unwrap();
        assert!(!g.via_stdin);
        assert!(g.args.contains(&"__PROMPT_FILE__".to_string()));
        assert!(build_cli_command("nope", None).is_none());
    }

    #[test]
    fn prompt_numbers_cues_and_pins_count_and_order() {
        let p = build_cli_prompt(Some("en"), "es", &["Hello".into(), "World".into()]);
        assert!(p.contains("into es"));
        assert!(p.contains("Source language: en"));
        assert!(p.contains("EXACTLY 2 elements"));
        assert!(p.contains("1. Hello"));
        assert!(p.contains("2. World"));
        // auto-detect when source omitted
        let p2 = build_cli_prompt(None, "lv", &["Hi".into()]);
        assert!(p2.contains("auto-detect"));
    }

    #[test]
    fn parses_flat_string_array() {
        // Representative flat JSON returned by a CLI translation backend.
        let r = r#"["Hola, mundo.", "¿Cómo estás hoy?", "Esto es una prueba."]"#;
        let v = parse_translation_array(r, 3).unwrap();
        assert_eq!(
            v,
            ["Hola, mundo.", "¿Cómo estás hoy?", "Esto es una prueba."]
        );
    }

    #[test]
    fn parses_indexed_objects_out_of_order() {
        let r = r#"prose before [{"i":2,"text":"dos"},{"i":1,"text":"uno"}] and after"#;
        let v = parse_translation_array(r, 2).unwrap();
        assert_eq!(v, ["uno", "dos"]); // re-ordered by i
    }

    #[test]
    fn parse_rejects_count_mismatch_and_garbage() {
        assert!(parse_translation_array(r#"["only one"]"#, 2).is_err());
        assert!(parse_translation_array("no array here", 1).is_err());
    }

    #[test]
    fn extract_array_ignores_brackets_inside_strings() {
        // a ']' inside a translation must not truncate the array
        let r = r#"[{"i":1,"text":"see [note]"},{"i":2,"text":"end"}]"#;
        let v = parse_translation_array(r, 2).unwrap();
        assert_eq!(v, ["see [note]", "end"]);
    }

    #[test]
    fn cue_mapping_preserves_timestamps() {
        let ranges = [[0u64, 1000], [1000, 2000]];
        let tr = ["uno".to_string(), "dos".to_string()];
        let mapped = map_translations_to_cues(&ranges, &tr).unwrap();
        assert_eq!(mapped[0].0, [0, 1000]);
        assert_eq!(mapped[0].1, "uno");
        assert_eq!(mapped[1].0, [1000, 2000]);
        // count mismatch errors
        assert!(map_translations_to_cues(&ranges, &tr[..1]).is_err());
    }

    #[test]
    fn opus_mt_id_shape() {
        assert_eq!(opus_mt_model_id("en", "es"), "Helsinki-NLP/opus-mt-en-es");
        assert_eq!(opus_mt_model_id("en", "lv"), "Helsinki-NLP/opus-mt-en-lv");
    }

    #[test]
    fn runner_json_parse_takes_last_object() {
        let s = "loading model...\n{\"translations\":[\"hola\"],\"model\":\"Helsinki-NLP/opus-mt-en-es\",\"backend\":\"opus-mt\"}";
        let r = parse_runner_json(s).unwrap();
        assert_eq!(r.translations, ["hola"]);
        assert_eq!(r.backend.as_deref(), Some("opus-mt"));
        assert!(parse_runner_json("no json").is_none());
    }

    #[test]
    fn segment_grouping_breaks_on_gap_and_length() {
        let words = vec![
            (0u64, 200, "Hello".to_string()),
            (250, 500, "there".to_string()),
            // big gap → new segment
            (3000, 3200, "Goodbye".to_string()),
        ];
        let segs = group_words_into_segments(&words, 600, 84);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "Hello there");
        assert_eq!(segs[0].start_ms, 0);
        assert_eq!(segs[0].end_ms, 500);
        assert_eq!(segs[0].word_range, [0, 1]);
        assert_eq!(segs[1].text, "Goodbye");
        assert_eq!(segs[1].word_range, [2, 2]);
    }

    #[test]
    fn token_distribution_spans_segment_and_is_ordered() {
        let toks = distribute_tokens(1000, 2000, "hola mundo amigo");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[0].0, 1000); // first starts at segment start
        assert_eq!(toks.last().unwrap().1, 2000); // last ends at segment end
                                                  // monotonic non-decreasing
        for w in toks.windows(2) {
            assert!(w[1].0 >= w[0].0);
        }
        assert!(distribute_tokens(0, 100, "   ").is_empty());
    }
}
