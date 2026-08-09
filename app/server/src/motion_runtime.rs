//! Shared local ShellX Motion CLI runtime.
//!
//! Feature connectors provide fixed argument vectors; this module owns binary
//! discovery, bounded process execution, timeout policy, and JSON envelopes.

use crate::jobs::{run_owned, ProcessControl, ProcessTermination};
use cut_core::{error_codes, CutError};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::Duration;

/// Canonical env override for the local ShellX Motion CLI binary/wrapper. Matches
/// Canvas (which already uses this name) and Motion's own "Motion CLI" docs
/// language, unifying the divergent Cut/Canvas knob so one operator config serves
/// both hosts.
pub(crate) const ENV_MOTION_CLI: &str = "SHELLX_MOTION_CLI";
/// Legacy env override — Cut's original name. Still honored (canonical wins when
/// both are set) with a one-time deprecation trace. Prefer `ENV_MOTION_CLI`.
pub(crate) const ENV_MOTION_BIN: &str = "SHELLX_MOTION_BIN";
pub(crate) const ENV_MOTION_ROOT: &str = "SHELLX_MOTION_ROOT";
pub(crate) const ENV_MOTION_TIMEOUT_MS: &str = "SHELLX_MOTION_TIMEOUT_MS";
const DEFAULT_TIMEOUT_MS: u64 = 300_000;
const MOTION_COMMAND_OUTPUT_MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct MotionCommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub timeout_ms: u64,
}

/// One-time deprecation-trace guard for the legacy `SHELLX_MOTION_BIN` name.
static LEGACY_MOTION_ENV_WARNED: Once = Once::new();

/// Emit the legacy-env deprecation line exactly once per process (any number of
/// availability probes / command builds collapse to a single trace).
fn warn_legacy_motion_env_once() {
    LEGACY_MOTION_ENV_WARNED.call_once(|| {
        tracing::warn!(
            "{} is deprecated; set {} instead (legacy name still honored)",
            ENV_MOTION_BIN,
            ENV_MOTION_CLI
        );
    });
}

/// Precedence core: the canonical value wins over the legacy value; an empty or
/// whitespace-only value counts as unset. Returns the chosen override and whether
/// it came from the legacy name. Pure (takes the two raw values) so it is
/// unit-testable without mutating process env.
fn choose_motion_cli(canonical: Option<String>, legacy: Option<String>) -> Option<(String, bool)> {
    if let Some(value) = canonical {
        if !value.trim().is_empty() {
            return Some((value, false));
        }
    }
    if let Some(value) = legacy {
        if !value.trim().is_empty() {
            return Some((value, true));
        }
    }
    None
}

/// Resolve the Motion CLI override from the environment: canonical
/// `SHELLX_MOTION_CLI` first, then the legacy `SHELLX_MOTION_BIN`. The returned
/// bool flags a legacy hit so the caller can trace the deprecation.
fn resolve_motion_cli_env() -> Option<(String, bool)> {
    choose_motion_cli(
        std::env::var(ENV_MOTION_CLI).ok(),
        std::env::var(ENV_MOTION_BIN).ok(),
    )
}

pub(crate) fn build_motion_cli_command(
    mut connector_args: Vec<String>,
    caller_scope: &Path,
) -> MotionCommandSpec {
    connector_args.push("--caller-id".to_string());
    connector_args.push(motion_caller_id(caller_scope));
    let timeout_ms = std::env::var(ENV_MOTION_TIMEOUT_MS)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(1_000, 1_800_000);
    if let Some((bin, from_legacy)) = resolve_motion_cli_env() {
        if from_legacy {
            warn_legacy_motion_env_once();
        }
        return MotionCommandSpec {
            program: bin,
            args: connector_args,
            cwd: None,
            timeout_ms,
        };
    }
    if let Some(root) = find_motion_root() {
        let mut args = vec![
            "--filter".to_string(),
            "@shellx-motion/cli".to_string(),
            "run".to_string(),
            "cli".to_string(),
            "--".to_string(),
        ];
        args.extend(connector_args);
        return MotionCommandSpec {
            // On Windows pnpm is the `pnpm.cmd` shim; `resolve_spawn` launches
            // its contained Node entrypoint directly. Elsewhere it is bare pnpm.
            program: motion_pnpm_program(),
            args,
            cwd: Some(root),
            timeout_ms,
        };
    }
    // PATH fallback. `motion_available()` counts `shellx-motion.{exe,cmd,bat}` on
    // Windows, but a bare `shellx-motion` only resolves `.exe` via CreateProcess —
    // the probe/spawn contradiction. On Windows, launch the concrete
    // file the probe actually found (a standard Node `.cmd`/`.bat` shim is then
    // resolved without a shell); elsewhere keep the bare PATH-resolved name.
    let program = match find_motion_on_path() {
        Some(path) if cfg!(windows) => path.to_string_lossy().into_owned(),
        _ => "shellx-motion".to_string(),
    };
    MotionCommandSpec {
        program,
        args: connector_args,
        cwd: None,
        timeout_ms,
    }
}

pub(crate) async fn run_motion_command_spec(
    spec: MotionCommandSpec,
    operation: &str,
) -> Result<Value, CutError> {
    // Windows npm/pnpm `.cmd` launchers are resolved to their underlying Node
    // entrypoint. Passing user-controlled Motion values through `cmd.exe /c`
    // would let command metacharacters become shell syntax.
    let (program, args) = resolve_spawn(&spec.program, &spec.args).map_err(|cause| {
        CutError::new(
            error_codes::SIDECAR,
            format!("ShellX Motion CLI could not {operation}"),
            cause,
        )
        .with_suggested_action(
            "use the standard Node-installed ShellX Motion CLI or an executable wrapper",
        )
    })?;
    let mut command = tokio::process::Command::new(&program);
    command.args(&args);
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    let control = ProcessControl::for_operation(Duration::from_millis(spec.timeout_ms))
        .with_output_cap(MOTION_COMMAND_OUTPUT_MAX_BYTES as usize);
    let output = run_owned(&mut command, None, &control)
        .await
        .map_err(|error| match error.termination() {
            Some(ProcessTermination::DeadlineExceeded) => CutError::new(
                error_codes::SIDECAR,
                format!("ShellX Motion CLI timed out while trying to {operation}"),
                format!("exceeded {}ms", spec.timeout_ms),
            ),
            Some(ProcessTermination::Cancelled(reason)) => CutError::new(
                "job_cancelled",
                format!("ShellX Motion CLI cancelled ({})", reason.label()),
                "the owning background job stopped this external worker",
            ),
            None => CutError::new(
                error_codes::SIDECAR,
                format!("ShellX Motion CLI failed while trying to {operation}"),
                error.to_string(),
            ),
        })?;
    if output.diagnostics_truncated() {
        return Err(CutError::new(
            error_codes::SIDECAR,
            "ShellX Motion CLI output exceeded the safety limit",
            format!(
                "limit is {} bytes per stream",
                MOTION_COMMAND_OUTPUT_MAX_BYTES
            ),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    parse_motion_process_output(&stdout, &stderr, output.status.success(), operation)
}

/// Stable, path-private Motion owner identity for one Cut workspace.
///
/// The source path itself must not enter Motion receipts. Hashing the nearest
/// `.cutproj` ancestor keeps every process working on one project in the same
/// owner bucket while keeping independent workspaces distinct. Project-less
/// previews fall back to their stable output root.
pub(crate) fn motion_caller_id(scope: &Path) -> String {
    let absolute = if scope.is_absolute() {
        scope.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(scope)
    };
    let workspace = absolute
        .ancestors()
        .find(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("cutproj"))
        })
        .unwrap_or(absolute.as_path());
    let mut identity = workspace.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        identity.make_ascii_lowercase();
    }
    let digest = Sha256::digest(identity.as_bytes());
    let token = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("cut:{token}")
}

fn parse_motion_process_output(
    stdout: &str,
    stderr: &str,
    success: bool,
    operation: &str,
) -> Result<Value, CutError> {
    match parse_motion_connector_stdout(stdout) {
        Ok(value) if success => Ok(value),
        Ok(value) => Err(CutError::new(
            error_codes::SIDECAR,
            format!("ShellX Motion CLI could not {operation}"),
            format!("process exited unsuccessfully after returning ok:true: {value}"),
        )),
        Err(error) if motion_json_envelope(stdout).is_some() => Err(error),
        Err(_) => Err(CutError::new(
            error_codes::SIDECAR,
            format!("ShellX Motion CLI could not {operation}"),
            tail(stderr, 1200),
        )),
    }
}

/// The pnpm launcher name for the current platform. On Windows pnpm ships as the
/// `pnpm.cmd` shim, which CreateProcess cannot launch directly; `resolve_spawn`
/// resolves its contained Node entrypoint so user values never cross a command
/// interpreter. Elsewhere it is the bare `pnpm` on PATH.
fn motion_pnpm_program() -> String {
    if cfg!(windows) {
        "pnpm.cmd".to_string()
    } else {
        "pnpm".to_string()
    }
}

/// The concrete ShellX Motion executable on PATH, if any — the single source of
/// truth shared by `motion_available()` (does one exist?) and the PATH fallback
/// spawn lane (launch exactly this file). On Windows every launchable form is
/// considered (`.exe`/`.cmd`/`.bat`, plus the extension-less name); elsewhere only
/// the bare `shellx-motion`. Returns the first match in PATH order.
fn find_motion_on_path() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &[
            "shellx-motion.exe",
            "shellx-motion.cmd",
            "shellx-motion.bat",
            "shellx-motion",
        ]
    } else {
        &["shellx-motion"]
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        names.iter().map(|name| dir.join(name)).find(|candidate| {
            candidate.is_file() && motion_program_supported(candidate.to_string_lossy().as_ref())
        })
    })
}

#[cfg(any(windows, test))]
fn command_file(program: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(program);
    if direct.is_file() {
        return Some(direct);
    }
    if direct.components().count() != 1 {
        return None;
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(program))
            .find(|path| path.is_file())
    })
}

/// Resolve a standard npm/pnpm Windows command shim to the JavaScript file it
/// dispatches. The accepted target is a regular `.js`/`.cjs`/`.mjs` file under
/// the shim's sibling `node_modules`; opaque batch files fail closed.
#[cfg(any(windows, test))]
fn resolve_node_cmd_shim(program: &str, args: &[String]) -> Result<(String, Vec<String>), String> {
    const MAX_SHIM_BYTES: u64 = 64 * 1024;
    let shim = command_file(program)
        .ok_or_else(|| format!("Motion command shim was not found: {program}"))?;
    let metadata = std::fs::metadata(&shim)
        .map_err(|error| format!("could not inspect Motion command shim: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_SHIM_BYTES {
        return Err("Motion command shim is not a bounded regular file".to_string());
    }
    let shim_dir = shim
        .parent()
        .ok_or_else(|| "Motion command shim has no parent directory".to_string())?;
    let root = std::fs::canonicalize(shim_dir)
        .map_err(|error| format!("could not resolve Motion command shim directory: {error}"))?;
    let body = std::fs::read_to_string(&shim)
        .map_err(|error| format!("could not read Motion command shim: {error}"))?;

    let mut entry = None;
    for segment in body.split('"').skip(1).step_by(2) {
        let lower = segment.to_ascii_lowercase().replace('\\', "/");
        let Some(index) = lower.find("node_modules/") else {
            continue;
        };
        let relative = &segment[index..];
        let lower_relative = &lower[index..];
        if !(lower_relative.ends_with(".js")
            || lower_relative.ends_with(".cjs")
            || lower_relative.ends_with(".mjs"))
        {
            continue;
        }
        let parts = relative
            .split(['\\', '/'])
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.iter().any(|part| *part == "." || *part == "..") {
            continue;
        }
        let candidate = parts
            .iter()
            .fold(shim_dir.to_path_buf(), |path, part| path.join(part));
        let Ok(candidate) = std::fs::canonicalize(candidate) else {
            continue;
        };
        if candidate.starts_with(&root) && candidate.is_file() {
            entry = Some(candidate);
            break;
        }
    }
    let entry = entry.ok_or_else(|| {
        "refusing opaque .cmd/.bat Motion wrapper; no contained Node entrypoint was found"
            .to_string()
    })?;
    let sibling_node = shim_dir.join("node.exe");
    let node = if sibling_node.is_file() {
        sibling_node.to_string_lossy().into_owned()
    } else {
        "node.exe".to_string()
    };
    let mut resolved_args = Vec::with_capacity(args.len() + 1);
    resolved_args.push(entry.to_string_lossy().into_owned());
    resolved_args.extend(args.iter().cloned());
    Ok((node, resolved_args))
}

/// Map a logical command spec to the concrete `(program, args)` to spawn. A
/// Windows npm/pnpm `.cmd`/`.bat` shim is never passed through `cmd.exe`; its
/// contained Node entrypoint is launched directly so every Motion value remains
/// an argv item. Real executables and non-Windows specs are unchanged.
#[cfg(windows)]
fn resolve_spawn(program: &str, args: &[String]) -> Result<(String, Vec<String>), String> {
    let lower = program.to_ascii_lowercase();
    if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        return resolve_node_cmd_shim(program, args);
    }
    Ok((program.to_string(), args.to_vec()))
}

/// Non-Windows: spawn the spec exactly as built (identity map).
#[cfg(not(windows))]
fn resolve_spawn(program: &str, args: &[String]) -> Result<(String, Vec<String>), String> {
    Ok((program.to_string(), args.to_vec()))
}

#[cfg(windows)]
fn motion_program_supported(program: &str) -> bool {
    let lower = program.to_ascii_lowercase();
    if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        resolve_node_cmd_shim(program, &[]).is_ok()
    } else {
        true
    }
}

#[cfg(not(windows))]
fn motion_program_supported(_program: &str) -> bool {
    true
}

pub(crate) fn motion_available() -> bool {
    if let Some((program, from_legacy)) = resolve_motion_cli_env() {
        if from_legacy {
            warn_legacy_motion_env_once();
        }
        return motion_program_supported(&program);
    }
    if find_motion_root().is_some() {
        return motion_program_supported(&motion_pnpm_program());
    }
    // Backed by the same PATH discovery the spawn lane launches, so "available"
    // now implies "spawnable" on Windows too.
    find_motion_on_path().is_some()
}

pub(crate) fn find_motion_root() -> Option<PathBuf> {
    if let Ok(root) = std::env::var(ENV_MOTION_ROOT) {
        let path = PathBuf::from(root);
        if path.join("package.json").is_file() {
            return Some(path);
        }
    }
    let cwd = std::env::current_dir().ok()?;
    for dir in cwd.ancestors().take(8) {
        for candidate in [
            dir.to_path_buf(),
            dir.join("ShellX Motion"),
            dir.join("shellx-motion"),
        ] {
            if candidate.join("package.json").is_file()
                && candidate.join("packages").join("cli").exists()
            {
                return Some(candidate);
            }
        }
    }
    None
}

pub(crate) fn parse_motion_connector_stdout(stdout: &str) -> Result<Value, CutError> {
    let json_line = motion_json_envelope(stdout).ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "ShellX Motion connector emitted no JSON result",
            tail(stdout, 800),
        )
    })?;
    let value: Value = serde_json::from_str(json_line).map_err(|error| {
        CutError::new(
            error_codes::SIDECAR,
            "ShellX Motion connector emitted invalid JSON",
            error.to_string(),
        )
    })?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(motion_result_error(&value));
    }
    Ok(value)
}

fn motion_json_envelope(stdout: &str) -> Option<&str> {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with('{') && line.ends_with('}'))
}

fn motion_result_error(value: &Value) -> CutError {
    let upstream_code = value
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str);
    let upstream_message = value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("ShellX Motion returned no error message");
    let suggested_action = value
        .get("error")
        .and_then(|error| error.get("suggestedAction"))
        .and_then(Value::as_str);
    if value.get("cancelled").and_then(Value::as_bool) == Some(true)
        || upstream_code == Some(error_codes::RENDER_CANCELLED)
    {
        return CutError::new(
            error_codes::RENDER_CANCELLED,
            "ShellX Motion render was stopped",
            upstream_message,
        );
    }
    if upstream_code == Some(error_codes::JOB_QUEUE_TIMEOUT) {
        return CutError::new(
            error_codes::JOB_QUEUE_TIMEOUT,
            "ShellX Motion could not start because the machine is busy",
            upstream_message,
        )
        .with_suggested_action("wait for another Motion render to finish, then retry");
    }
    if let Some(
        code @ (error_codes::JOB_UNKNOWN | error_codes::JOB_EXPIRED | error_codes::JOB_NOT_VISIBLE),
    ) = upstream_code
    {
        let error = CutError::new(code, upstream_message, "ShellX Motion job query refused");
        return suggested_action
            .map(|action| error.clone().with_suggested_action(action))
            .unwrap_or(error);
    }
    CutError::new(
        error_codes::SIDECAR,
        "ShellX Motion connector returned ok:false",
        value
            .get("error")
            .cloned()
            .unwrap_or_else(|| json!(value))
            .to_string(),
    )
}

pub(crate) fn tail(value: &str, max: usize) -> String {
    let count = value.chars().count();
    if count <= max {
        value.to_string()
    } else {
        value.chars().skip(count.saturating_sub(max)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_last_json_envelope_and_rejects_failed_motion_results() {
        let parsed =
            parse_motion_connector_stdout("log\n{\"ok\":true,\"result\":{\"id\":\"x\"}}\n")
                .expect("json parses");
        assert_eq!(parsed["result"]["id"], "x");
        assert!(parse_motion_connector_stdout(
            "{\"ok\":false,\"error\":{\"message\":\"no render\"}}"
        )
        .is_err());
    }

    #[test]
    fn preserves_motion_cancel_and_busy_outcomes_from_failed_processes() {
        let cancelled = parse_motion_process_output(
            r#"{"ok":false,"command":"render","cancelled":true,"error":{"code":"render_cancelled","message":"stopped by user"}}"#,
            "",
            false,
            "render a clip",
        )
        .unwrap_err();
        assert_eq!(cancelled.code, error_codes::RENDER_CANCELLED);
        assert!(cancelled.message.contains("stopped"));
        assert_eq!(cancelled.suggested_action, None);

        let busy = parse_motion_process_output(
            r#"{"ok":false,"command":"render","error":{"code":"job_queue_timeout","message":"queue deadline exhausted"}}"#,
            "",
            false,
            "render a clip",
        )
        .unwrap_err();
        assert_eq!(busy.code, error_codes::JOB_QUEUE_TIMEOUT);
        assert!(busy.message.contains("machine is busy"));
        assert!(busy.suggested_action.is_some());

        let invisible = parse_motion_process_output(
            r#"{"ok":false,"command":"job.get","error":{"code":"job_not_visible","message":"belongs to another caller","suggestedAction":"open the workspace that started it"}}"#,
            "",
            false,
            "read a Motion job",
        )
        .unwrap_err();
        assert_eq!(invisible.code, error_codes::JOB_NOT_VISIBLE);
        assert_eq!(
            invisible.suggested_action.as_deref(),
            Some("open the workspace that started it")
        );
    }

    #[test]
    fn caller_identity_is_workspace_stable_distinct_and_path_private() {
        let first = motion_caller_id(Path::new("/work/a.cutproj/motion/preview/one"));
        let same = motion_caller_id(Path::new("/work/a.cutproj/motion/insert/two"));
        let other = motion_caller_id(Path::new("/work/b.cutproj/motion/preview/one"));
        assert_eq!(first, same);
        assert_ne!(first, other);
        assert!(first.starts_with("cut:"));
        assert!(!first.contains("work"));
        assert_eq!(first.len(), 28);
    }

    #[test]
    fn unicode_tail_is_character_safe() {
        assert_eq!(tail("alpha-αβγδε", 3), "γδε");
    }

    #[test]
    fn motion_cli_env_precedence_prefers_canonical_then_legacy() {
        // Canonical SHELLX_MOTION_CLI wins when both names are set.
        assert_eq!(
            choose_motion_cli(Some("/canon/cli".into()), Some("/legacy/bin".into())),
            Some(("/canon/cli".to_string(), false)),
        );
        // Legacy SHELLX_MOTION_BIN is honored (and flagged) when canonical is unset.
        assert_eq!(
            choose_motion_cli(None, Some("/legacy/bin".into())),
            Some(("/legacy/bin".to_string(), true)),
        );
        // A whitespace-only canonical counts as unset and falls through to legacy.
        assert_eq!(
            choose_motion_cli(Some("   ".into()), Some("/legacy/bin".into())),
            Some(("/legacy/bin".to_string(), true)),
        );
        // Neither set (or only blanks) => no override, discovery behavior unchanged.
        assert_eq!(choose_motion_cli(None, Some("  ".into())), None);
        assert_eq!(choose_motion_cli(None, None), None);
    }

    #[test]
    fn pnpm_program_matches_platform() {
        if cfg!(windows) {
            assert_eq!(motion_pnpm_program(), "pnpm.cmd");
        } else {
            assert_eq!(motion_pnpm_program(), "pnpm");
        }
    }

    #[test]
    fn resolve_spawn_platform_contract() {
        // A real executable is spawned verbatim on every platform. Opaque command
        // shims fail closed on Windows and remain byte-identical off Windows.
        let args = vec!["--version".to_string()];
        let resolved = resolve_spawn("pnpm.cmd", &args);
        if cfg!(windows) {
            // The host may or may not have pnpm installed; either a contained Node
            // entrypoint resolves or the opaque/missing shim is rejected.
            if let Ok((program, out_args)) = resolved {
                assert!(program.to_ascii_lowercase().ends_with("node.exe"));
                assert_eq!(out_args.last().map(String::as_str), Some("--version"));
            }
            let (exe_program, exe_args) =
                resolve_spawn("shellx-motion.exe", &args).expect("exe remains direct");
            assert_eq!(exe_program, "shellx-motion.exe");
            assert_eq!(exe_args, args);
        } else {
            let (program, out_args) = resolved.expect("non-Windows spawn is direct");
            assert_eq!(program, "pnpm.cmd");
            assert_eq!(out_args, args);
        }
    }

    #[test]
    fn standard_node_command_shim_bypasses_cmd_and_preserves_arguments() {
        let temp = tempfile::tempdir().expect("tempdir");
        let entry = temp
            .path()
            .join("node_modules")
            .join("pnpm")
            .join("bin")
            .join("pnpm.mjs");
        std::fs::create_dir_all(entry.parent().expect("entry parent")).expect("entry dir");
        std::fs::write(&entry, "// fixture\n").expect("entry");
        let shim = temp.path().join("pnpm.cmd");
        std::fs::write(
            &shim,
            "@ECHO off\r\n\"%_prog%\" \"%dp0%\\node_modules\\pnpm\\bin\\pnpm.mjs\" %*\r\n",
        )
        .expect("shim");
        let payload = "title=safe&echo.INJECTED>marker.txt".to_string();
        let (program, resolved) = resolve_node_cmd_shim(
            shim.to_str().expect("utf8 shim"),
            &["--set".to_string(), payload.clone()],
        )
        .expect("standard Node shim resolves");
        assert!(program.to_ascii_lowercase().ends_with("node.exe"));
        assert_eq!(
            resolved[0],
            std::fs::canonicalize(entry).unwrap().to_string_lossy()
        );
        assert_eq!(resolved[1], "--set");
        assert_eq!(resolved[2], payload);
    }

    #[test]
    fn opaque_command_shim_fails_closed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let shim = temp.path().join("shellx-motion.cmd");
        std::fs::write(&shim, "@echo off\r\nshellx-motion.exe %*\r\n").expect("shim");
        let error = resolve_node_cmd_shim(
            shim.to_str().expect("utf8 shim"),
            &["title=safe&whoami".to_string()],
        )
        .unwrap_err();
        assert!(error.contains("refusing opaque"));
    }
}
