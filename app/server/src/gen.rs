//! gen.rs — `assets.generate`: image/video via the user's own agent CLI.
//!
//! Clean-room port of ShellX Canvas's `media-provider` adapter (the design, not the
//! code). NO model is hosted — the user's installed codex (gpt-image), grok
//! (grok-imagine), or Antigravity (`agy`) CLI does the generation; cutd just (1) DETECTS the CLI, (2) spawns
//! it with a scoped prompt (or Grok's native image_gen tool for plain images), (3)
//! validates the resulting binary via the NORMAL import probe (ffprobe — a fake/placeholder
//! file fails to probe), and (4) imports it like any upload through `record_import`.
//! This is the Openverse philosophy applied to generation: integration, not hosting.
//!
//! This module is the PURE part (CLI mapping, command + prompt construction, output
//! JSON parsing) — unit-tested without spawning anything. The actual spawn (with a
//! timeout) + import live in dispatch.rs `assets_generate`. Honest degradation: the
//! CLI absent → the verb returns `ok:false` with a clear reason (never a fake asset).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One provider command resolved for this child process.  With no Runner
/// provider context it retains Cut's ordinary resolver behavior.  With an
/// admitted context it contains only the exact executable, optional entrypoint,
/// and effective child environment selected by the Runner.
#[derive(Debug, Clone)]
pub(crate) struct ProviderChildCommand {
    executable: PathBuf,
    entrypoint: Option<PathBuf>,
    admitted_environment: Option<BTreeMap<String, String>>,
    available: bool,
}

impl ProviderChildCommand {
    pub(crate) fn executable(&self) -> &Path {
        &self.executable
    }

    pub(crate) fn available(&self) -> bool {
        self.available
    }

    pub(crate) fn admitted_environment(&self) -> Option<&BTreeMap<String, String>> {
        self.admitted_environment.as_ref()
    }

    fn prefixed_arguments(&self, arguments: &[String]) -> Vec<String> {
        let mut prefixed =
            Vec::with_capacity(arguments.len() + usize::from(self.entrypoint.is_some()));
        if let Some(entrypoint) = &self.entrypoint {
            prefixed.push(entrypoint.to_string_lossy().into_owned());
        }
        prefixed.extend(arguments.iter().cloned());
        prefixed
    }

    pub(crate) fn std_command(
        &self,
        arguments: &[String],
    ) -> Result<std::process::Command, String> {
        agent_std_command(self.executable(), &self.prefixed_arguments(arguments))
    }

    pub(crate) fn tokio_command(
        &self,
        arguments: &[String],
    ) -> Result<tokio::process::Command, String> {
        agent_tokio_command(self.executable(), &self.prefixed_arguments(arguments))
    }

    pub(crate) fn apply_admitted_environment(
        &self,
        command: &mut tokio::process::Command,
        temporary_directory: &Path,
    ) -> Result<(), String> {
        if let Some(environment) = self.admitted_environment() {
            let temporary_directory = admitted_temporary_directory(temporary_directory)?;
            command.env_clear();
            command.envs(environment);
            for name in ["TMPDIR", "TEMP", "TMP"] {
                command.env(name, temporary_directory);
            }
        }
        Ok(())
    }

    pub(crate) fn apply_admitted_std_environment(
        &self,
        command: &mut std::process::Command,
        temporary_directory: &Path,
    ) -> Result<(), String> {
        if let Some(environment) = self.admitted_environment() {
            let temporary_directory = admitted_temporary_directory(temporary_directory)?;
            command.env_clear();
            command.envs(environment);
            for name in ["TMPDIR", "TEMP", "TMP"] {
                command.env(name, temporary_directory);
            }
        }
        Ok(())
    }
}

fn admitted_temporary_directory(directory: &Path) -> Result<&Path, String> {
    if !directory.is_absolute() || !directory.is_dir() {
        return Err(format!(
            "admitted provider temporary directory must be an existing absolute directory: {}",
            directory.display()
        ));
    }
    Ok(directory)
}

/// Resolve a logical provider for one child process.  A Runner context is an
/// admission decision: bad or incomplete context is an error, never a reason to
/// search Cut's inherited PATH or home-directory ladder.
pub(crate) fn provider_child_command(
    logical_provider: &str,
    ordinary_program: &str,
) -> Result<ProviderChildCommand, String> {
    match crate::provider_runtime::selected_from_process_environment(logical_provider)? {
        Some(selected) => Ok(ProviderChildCommand {
            executable: selected.executable,
            entrypoint: selected.entrypoint,
            admitted_environment: Some(selected.environment),
            available: true,
        }),
        None => {
            let resolved = resolve_agent(ordinary_program);
            let available = resolved.is_some();
            let executable = resolved.unwrap_or_else(|| PathBuf::from(ordinary_program));
            Ok(ProviderChildCommand {
                executable,
                entrypoint: None,
                admitted_environment: None,
                available,
            })
        }
    }
}

pub(crate) fn provider_is_available(
    logical_provider: &str,
    ordinary_program: &str,
) -> Result<bool, String> {
    Ok(provider_child_command(logical_provider, ordinary_program)?.available())
}

/// The CLI binary for a provider (`codex` → gpt-image, `grok` → grok-imagine,
/// `antigravity` → `agy`).
pub fn cli_for(provider: &str) -> Option<&'static str> {
    match provider {
        "codex" => Some("codex"),
        "grok" => Some("grok"),
        "antigravity" => Some("agy"),
        _ => None,
    }
}

// ── Agent-CLI resolution (the "shellx approach": PATH is the LAST resort) ──────
//
// A process-PATH-only check misses two supported install layouts:
//   * grok self-manages a symlink at ~/.grok/bin/grok that it NEVER adds to PATH,
//     so an on-PATH-only check reports grok absent.
//   * a macOS .app launched from Finder inherits a STRIPPED PATH
//     (/usr/bin:/bin:/usr/sbin:/sbin), so claude/codex installed by Homebrew/npm in
//     /opt/homebrew/bin or ~/.local/bin are missed by the in-app cutd.
// This mirrors cut_media::toolpath::resolve_tool for ffmpeg: probe the process PATH
// FIRST (so an on-PATH install resolves exactly as before), then an explicit
// dir-ladder of the standard agent-CLI install locations (the Finder-stripped-PATH
// + self-managed-installer safety net). See toolpath.rs ~L194-202 for the same
// reasoning applied to ffmpeg.

/// The executable-name candidates for an agent stem, in match order. Windows npm
/// installs commonly contain both an extensionless Unix shim and a `.cmd` shim;
/// prefer launchable Windows formats so the Unix shim cannot cause OS error 193.
fn exe_candidates_for(stem: &str, windows: bool) -> Vec<String> {
    if windows {
        vec![
            format!("{stem}.exe"),
            format!("{stem}.cmd"),
            format!("{stem}.bat"),
            stem.to_string(),
        ]
    } else {
        vec![stem.to_string()]
    }
}

fn exe_candidates(stem: &str) -> Vec<String> {
    exe_candidates_for(stem, cfg!(windows))
}

/// The user's home dir (HOME on unix, USERPROFILE on Windows), used to expand the
/// `~`-relative agent install dirs. Resolved via std env — never hardcoded.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Explicit dirs (beyond the process PATH) that standard agent-CLI installers drop
/// binaries into — the safety net for a Finder-stripped PATH and grok's
/// self-managed location. `home` is INJECTED (not read here) so the ladder is
/// unit-testable with a mock HOME without mutating process-global env.
///   * /opt/homebrew/bin, /usr/local/bin — Homebrew (Apple Silicon / Intel) + many
///     npm-global setups; the exact dirs toolpath.rs adds for ffmpeg's Finder case.
///   * ~/.local/bin, ~/.npm-global/bin — pipx + `npm config set prefix` globals.
///   * ~/.grok/bin — ONLY for grok: its installer symlinks the binary here and
///     never touches PATH. Added only for `agent == "grok"` so we don't probe an
///     irrelevant dir for the others.
fn agent_install_dirs(agent: &str, home: Option<PathBuf>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home {
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".npm-global").join("bin"));
        if agent == "grok" {
            dirs.push(home.join(".grok").join("bin"));
        }
    }
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs
}

/// First dir in `dirs` that holds any of the exe-name `cands`, returned as the
/// joined absolute path. Pure (no env) — the unit-test seam for the ladder.
fn first_in_dirs(dirs: &[PathBuf], cands: &[String]) -> Option<PathBuf> {
    for dir in dirs {
        for name in cands {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Core resolver with the env inputs INJECTED (PATH dirs + home) so it is fully
/// unit-testable. PATH is checked FIRST (an on-PATH install resolves exactly as the
/// old `on_path` did), then the explicit install dirs.
fn resolve_agent_with(
    agent: &str,
    path_dirs: &[PathBuf],
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    let cands = exe_candidates(agent);
    if let Some(hit) = first_in_dirs(path_dirs, &cands) {
        return Some(hit);
    }
    first_in_dirs(&agent_install_dirs(agent, home), &cands)
}

/// Resolve an agent CLI ("claude" | "codex" | "grok" | "agy" | …) to a runnable
/// absolute path, searching the process PATH first then the explicit agent install
/// dirs (so a Finder-stripped-PATH .app and grok's off-PATH ~/.grok/bin both
/// resolve). `Some(path)` ⇒ found (spawn it BY this path so an off-PATH binary
/// actually launches); `None` ⇒ nowhere on the ladder. The agent-CLI analogue of
/// cut_media::toolpath::resolve_tool.
pub fn resolve_agent(agent: &str) -> Option<PathBuf> {
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    resolve_agent_with(agent, &path_dirs, home_dir())
}

#[cfg(windows)]
fn is_windows_batch(program: &Path) -> bool {
    program
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
        .unwrap_or(false)
}

#[cfg(any(windows, test))]
fn quote_windows_batch_arg(value: &str) -> Result<String, String> {
    if value.contains('%')
        || value.contains('!')
        || value
            .chars()
            .any(|c| matches!(c, '&' | '|' | '<' | '>' | '^' | '\0' | '\r' | '\n'))
    {
        return Err(
            "Windows batch arguments cannot contain %, !, &, |, <, >, ^, NUL, CR, or LF".into(),
        );
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for ch in value.chars() {
        if ch == '\\' {
            backslashes += 1;
            continue;
        }
        if ch == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2));
            quoted.push_str("\"\"");
        } else {
            quoted.push_str(&"\\".repeat(backslashes));
            quoted.push(ch);
        }
        backslashes = 0;
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    Ok(quoted)
}

#[cfg(any(windows, test))]
pub(crate) fn windows_batch_command_line(
    program: &Path,
    args: &[String],
) -> Result<String, String> {
    let program = program
        .to_str()
        .ok_or_else(|| "Windows batch path is not valid Unicode".to_string())?;
    if program.contains('%')
        || program.contains('!')
        || program
            .chars()
            .any(|c| matches!(c, '"' | '\0' | '\r' | '\n'))
    {
        return Err("Windows batch path contains unsafe expansion or control characters".into());
    }
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(format!("\"{program}\""));
    for arg in args {
        parts.push(quote_windows_batch_arg(arg)?);
    }
    Ok(parts.join(" "))
}

/// Build a process command for a resolved agent CLI. Windows npm installs use
/// `.cmd`/`.bat` shims, which CreateProcess cannot execute directly (OS error
/// 193). Only those shims go through cmd.exe; native executables keep direct,
/// structured argument passing. Batch arguments are quoted and unsafe expansion
/// characters are rejected before cmd.exe sees them.
pub fn agent_std_command(program: &Path, args: &[String]) -> Result<std::process::Command, String> {
    #[cfg(windows)]
    if is_windows_batch(program) {
        use std::os::windows::process::CommandExt;

        let line = windows_batch_command_line(program, args)?;
        let mut command = std::process::Command::new("cmd.exe");
        command.args(["/D", "/V:OFF", "/S", "/C"]);
        command.raw_arg(format!("\"{line}\""));
        Ok(command)
    } else {
        direct_std_command(program, args)
    }

    #[cfg(not(windows))]
    direct_std_command(program, args)
}

fn direct_std_command(program: &Path, args: &[String]) -> Result<std::process::Command, String> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    Ok(command)
}

pub fn agent_tokio_command(
    program: &Path,
    args: &[String],
) -> Result<tokio::process::Command, String> {
    #[cfg(windows)]
    if is_windows_batch(program) {
        use std::os::windows::process::CommandExt;

        let line = windows_batch_command_line(program, args)?;
        let mut command = tokio::process::Command::new("cmd.exe");
        command.args(["/D", "/V:OFF", "/S", "/C"]);
        command.as_std_mut().raw_arg(format!("\"{line}\""));
        Ok(command)
    } else {
        direct_tokio_command(program, args)
    }

    #[cfg(not(windows))]
    direct_tokio_command(program, args)
}

fn direct_tokio_command(
    program: &Path,
    args: &[String],
) -> Result<tokio::process::Command, String> {
    let mut command = tokio::process::Command::new(program);
    command.args(args);
    Ok(command)
}

/// Which kinds a provider can generate. Antigravity is deliberately image-only:
/// its installed CLI contract proves a safe non-interactive turn, not a native
/// video-generation capability.
pub fn supports_kind(provider: &str, kind: &str) -> bool {
    match provider {
        "codex" | "antigravity" => kind == "image",
        "grok" => kind == "image" || kind == "video",
        _ => false,
    }
}

/// Default generation deadline when callers do not provide `timeout_ms`.
/// Image providers can legitimately spend several minutes in their native
/// generation flow. Antigravity's outer deadline stays one minute beyond its
/// own ten-minute CLI deadline so Cut can collect the provider's terminal
/// result instead of racing it.
pub fn default_timeout_ms(provider: &str) -> u64 {
    match provider {
        "grok" => 600_000,
        "antigravity" => 660_000,
        _ => 240_000,
    }
}

/// The default output filename for a kind (extension drives nothing — ffprobe
/// validates the bytes — but a sensible name helps the agent CLI).
pub fn output_filename(kind: &str) -> &'static str {
    if kind == "video" {
        "generated.mp4"
    } else {
        "generated.png"
    }
}

/// How a provider accepts the fully constructed prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptTransport {
    Stdin,
    PromptFile,
    Argument,
}

/// A resolved CLI invocation: the command, its args, and the provider's prompt
/// transport. Antigravity uses its native non-interactive `--print` contract but
/// keeps generation skill expansion enabled, unlike the Cut-MCP-only Agent Chat
/// route.
#[derive(Debug, Clone, PartialEq)]
pub struct GenCommand {
    pub cmd: String,
    pub args: Vec<String>,
    pub prompt_transport: PromptTransport,
}

/// Build the agent-CLI invocation for `provider`. `workspace` is the scratch
/// cwd; `model` is optional. Grok images with registered references retain
/// their existing file-output route until native reference use is verified.
pub fn build_command(
    provider: &str,
    kind: &str,
    has_references: bool,
    workspace: &str,
    model: Option<&str>,
) -> Option<GenCommand> {
    match provider {
        "codex" => {
            let mut args = vec![
                "exec".into(),
                "-".into(),
                "--json".into(),
                "-C".into(),
                workspace.into(),
                "--skip-git-repo-check".into(),
                "--approve-for-me".into(),
                "--ephemeral".into(),
            ];
            if let Some(m) = model.filter(|m| !m.is_empty()) {
                args.push("-m".into());
                args.push(m.into());
            }
            Some(GenCommand {
                cmd: "codex".into(),
                args,
                prompt_transport: PromptTransport::Stdin,
            })
        }
        "grok" => {
            // Grok's native image tool reports the actual generated file in a
            // streaming tool_call_update. Video keeps the established agent
            // file-output route until a native video result is verified.
            let mut args = vec![
                "--prompt-file".into(),
                "__PROMPT_FILE__".into(),
                "--cwd".into(),
                workspace.into(),
                "--disable-web-search".into(),
                "--no-subagents".into(),
            ];
            if kind == "image" && !has_references {
                args.extend([
                    "--tools".into(),
                    "image_gen".into(),
                    "--allow".into(),
                    "image_gen".into(),
                    "--output-format".into(),
                    "streaming-json".into(),
                ]);
                // image_gen can complete on a later turn after local tool
                // discovery. The operation deadline and single completed-image
                // validator bound this route without truncating its tool result.
            } else {
                args.extend([
                    "--output-format".into(),
                    "json".into(),
                    "--permission-mode".into(),
                    "bypassPermissions".into(),
                    "--always-approve".into(),
                    "--no-plan".into(),
                    "--max-turns".into(),
                    "20".into(),
                ]);
            }
            if let Some(model) = model.filter(|model| !model.is_empty()) {
                args.push("--model".into());
                args.push(model.into());
            }
            Some(GenCommand {
                cmd: "grok".into(),
                args,
                prompt_transport: PromptTransport::PromptFile,
            })
        }
        "antigravity" => {
            let mut args = vec![
                "--new-project".into(),
                "--sandbox".into(),
                // In print mode AGY cannot display a permission prompt, so use
                // the same bounded headless approval contract as Agent Chat.
                // The provider is still confined to this create-only generation
                // workspace by the native sandbox and exact-output prompt.
                "--dangerously-skip-permissions".into(),
                "--output-format".into(),
                "json".into(),
                "--print-timeout".into(),
                "10m0s".into(),
                "--log-file".into(),
                Path::new(workspace)
                    .join("antigravity-generation.log")
                    .to_string_lossy()
                    .into_owned(),
            ];
            if let Some(model) = model.filter(|model| !model.is_empty()) {
                args.push("--model".into());
                args.push(model.into());
            }
            // Leave AGY's normal image-generation behavior available. Agent
            // Chat disables slash expansion because that route exposes only Cut
            // MCP; assets.generate sends the ordinary direct image request.
            args.push("--print".into());
            args.push("__PROMPT_TEXT__".into());
            Some(GenCommand {
                cmd: "agy".into(),
                args,
                prompt_transport: PromptTransport::Argument,
            })
        }
        _ => None,
    }
}

/// Build the generation prompt: a
/// strict instruction to generate ONE real asset and write the binary to EXACTLY
/// `output_path`, returning JSON, and to fail honestly (no fake file) if real
/// generation isn't available.
pub fn build_prompt(
    provider: &str,
    kind: &str,
    description: &str,
    output_path: &str,
    reference_paths: &[String],
) -> String {
    if provider == "grok" && kind == "image" && reference_paths.is_empty() {
        let lines = [
            "Generate exactly one real image by calling image_gen exactly once.".to_string(),
            format!("Use this user description: {}", serde_json::to_string(description).unwrap_or_default()),
            "Do not create a placeholder or retry a failed tool call. If image_gen is unavailable, report the failure.".to_string(),
        ];
        return lines.join("\n");
    }
    // AGY's native image turn works best as the direct user request it is. A
    // larger agent-protocol prompt (load a skill, emit JSON, and explain honest
    // failure) can make the CLI reason about capability admission instead of
    // creating the image. The process boundary already supplies the sandbox,
    // timeout, exact scratch cwd, media probe, and project import checks, so the
    // prompt only needs to state the creative request and final output path.
    if provider == "antigravity" {
        let mut lines = vec![
            format!(
                "Create one image from this description: {}",
                serde_json::to_string(description).unwrap_or_default()
            ),
            "Save the final PNG EXACTLY this path:".to_string(),
            output_path.to_string(),
        ];
        if !reference_paths.is_empty() {
            lines.push("Use these images as visual references:".to_string());
            for (index, path) in reference_paths.iter().enumerate() {
                lines.push(format!("Reference {}: {}", index + 1, path));
            }
            lines.push("Do not overwrite the reference images.".to_string());
        }
        return lines.join("\n");
    }

    let accepted = if kind == "video" {
        "mp4, webm, or ogv"
    } else {
        "png, jpg, gif, or webp"
    };
    let label = match provider {
        "codex" => "Codex (gpt-image)",
        "grok" => "Grok Build (Imagine)",
        "antigravity" => "Antigravity (agy)",
        _ => "the selected provider",
    };
    let mut lines = vec![
        "You are running inside ShellX Cut local media generation.".to_string(),
        format!("Task: generate one real {kind} asset using {label}."),
        format!("User description: {}.", serde_json::to_string(description).unwrap_or_default()),
        "Write the final binary file to EXACTLY this path and nowhere else:".to_string(),
        output_path.to_string(),
        format!("Accepted final file formats: {accepted}."),
        "If the media tool returns a valid file at another path, copy the binary bytes directly to the exact requested path. ShellX Cut validates bytes by probing the media.".to_string(),
        "Do not create placeholders, SVG mockups, HTML/CSS art, text files, or screenshots as a substitute.".to_string(),
        "Do not modify project files outside the scratch workspace. The only required output is the exact media file path above.".to_string(),
    ];
    if !reference_paths.is_empty() {
        lines.push("Use these registered project assets as visual references. ShellX Cut copied them into this isolated scratch workspace:".to_string());
        for (index, path) in reference_paths.iter().enumerate() {
            lines.push(format!("Reference {}: {}", index + 1, path));
        }
        lines.push("Use the references for subject, composition, palette, or motion continuity as implied by the user description. Do not overwrite them.".to_string());
    }
    if provider == "codex" {
        lines.push("Load the installed image-generation skill and use Codex's built-in image_gen tool. Generate first, then copy the selected real image into the exact workspace output path. If this CLI session has no such tool, fail honestly.".to_string());
    } else if kind == "video" {
        lines.push(format!(
            "Load and use Grok Build's native Imagine skill for this request: {}.",
            serde_json::to_string(&format!("video: {description}")).unwrap_or_default()
        ));
        lines.push("If Grok exposes native video tools, use image_to_video or reference_to_video. A fixed 6 or 10 second clip is acceptable.".to_string());
        lines.push("If this Grok CLI session cannot complete a video-capable Imagine request, fail honestly without writing a fake file.".to_string());
    } else {
        lines.push(format!(
            "Load and use Grok Build's native Imagine skill to generate this real image: {}.",
            serde_json::to_string(description).unwrap_or_default()
        ));
    }
    lines.push("After writing the file, finish with JSON only:".to_string());
    lines.push(format!(
        "{{\"ok\":true,\"path\":{},\"summary\":\"short description\"}}",
        serde_json::to_string(output_path).unwrap_or_default()
    ));
    lines.push(
        "If real media generation is unavailable, do not write a fake file. Finish with JSON only:"
            .to_string(),
    );
    lines.push(
        "{\"ok\":false,\"reason\":\"real media generation is unavailable in this CLI session\"}"
            .to_string(),
    );
    lines.join("\n")
}

/// The parsed CLI result JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct GenJson {
    pub ok: bool,
    pub path: Option<String>,
    pub reason: Option<String>,
}

/// Return the single image created by Grok's native image_gen tool. The path is
/// taken only from a completed tool result paired with its tool-call ID, never
/// from the assistant's prose or a suggested filename.
pub fn grok_image_tool_path(stdout: &str) -> Result<PathBuf, String> {
    let mut calls = std::collections::BTreeSet::new();
    let mut generated = Vec::new();
    for line in stdout.lines() {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(id) = event.get("toolCallId").and_then(|value| value.as_str()) else {
            continue;
        };
        match event.get("type").and_then(|value| value.as_str()) {
            Some("tool_call")
                if event.get("toolName").and_then(|value| value.as_str()) == Some("image_gen") =>
            {
                calls.insert(id.to_string());
            }
            Some("tool_call_update")
                if calls.contains(id)
                    && event.get("status").and_then(|value| value.as_str())
                        == Some("completed")
                    && event
                        .pointer("/rawOutput/type")
                        .and_then(|value| value.as_str())
                        == Some("ImageGen") =>
            {
                let path = event
                    .pointer("/rawOutput/path")
                    .and_then(|value| value.as_str())
                    .filter(|path| !path.trim().is_empty())
                    .ok_or("Grok image_gen completed without an image path")?;
                generated.push(PathBuf::from(path));
            }
            _ => {}
        }
    }
    if calls.len() != 1 || generated.len() != 1 {
        return Err(format!(
            "Grok image_gen made {} calls and returned {} completed images; expected one of each",
            calls.len(),
            generated.len()
        ));
    }
    Ok(generated.remove(0))
}

/// Summarize only the shape of a failed native image run. Provider output can
/// contain prompts, paths, and credentials, so none of its text is returned.
pub fn grok_image_exit_diagnostic(
    stdout: &[u8],
    stderr: &[u8],
    stdout_truncated: bool,
    stderr_truncated: bool,
) -> String {
    let mut calls = std::collections::BTreeSet::new();
    let mut completed = std::collections::BTreeSet::new();
    for line in String::from_utf8_lossy(stdout).lines() {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(id) = event.get("toolCallId").and_then(|value| value.as_str()) else {
            continue;
        };
        match event.get("type").and_then(|value| value.as_str()) {
            Some("tool_call")
                if event.get("toolName").and_then(|value| value.as_str()) == Some("image_gen") =>
            {
                calls.insert(id.to_string());
            }
            Some("tool_call_update")
                if calls.contains(id)
                    && event.get("status").and_then(|value| value.as_str())
                        == Some("completed")
                    && event
                        .pointer("/rawOutput/type")
                        .and_then(|value| value.as_str())
                        == Some("ImageGen") =>
            {
                completed.insert(id.to_string());
            }
            _ => {}
        }
    }
    let category = if stdout_truncated {
        "partial_trace"
    } else if calls.is_empty() {
        "no_image_gen_call_observed"
    } else if completed.is_empty() {
        "image_gen_not_completed"
    } else {
        "image_gen_completed_before_exit"
    };
    format!(
        "diagnostic: {category}; image_gen calls={}, completed={}; stderr_present={}; stdout_truncated={stdout_truncated}; stderr_truncated={stderr_truncated}",
        calls.len(),
        completed.len(),
        !stderr.is_empty(),
    )
}

/// Resolve Grok's generated-image session directory from the effective child
/// profile. This inspects environment names only; it never opens provider auth.
pub fn grok_sessions_root(
    admitted_environment: Option<&BTreeMap<String, String>>,
) -> Result<PathBuf, String> {
    let preferred = if cfg!(windows) {
        ["USERPROFILE", "HOME"]
    } else {
        ["HOME", "USERPROFILE"]
    };
    let home = if let Some(environment) = admitted_environment {
        preferred
            .iter()
            .find_map(|name| environment.get(*name))
            .map(PathBuf::from)
    } else {
        home_dir()
    }
    .ok_or("Grok image_gen has no admitted user home")?;
    if !home.is_absolute() {
        return Err("Grok image_gen user home is not absolute".into());
    }
    Ok(home.join(".grok").join("sessions"))
}

fn grok_image_session_relative_path_is_valid(path: &Path) -> bool {
    let components: Vec<_> = path.components().collect();
    if components.len() != 4 {
        return false;
    }
    let project = components[0].as_os_str().to_string_lossy();
    let session = components[1].as_os_str().to_string_lossy();
    let folder = components[2].as_os_str().to_string_lossy();
    let file = components[3].as_os_str().to_string_lossy();
    let uuid = session.as_bytes();
    let uuid_valid = uuid.len() == 36
        && uuid.iter().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        });
    let (number, extension) = file.rsplit_once('.').unwrap_or(("", ""));
    !project.is_empty()
        && uuid_valid
        && folder == "images"
        && !number.is_empty()
        && !number.starts_with('0')
        && number.bytes().all(|byte| byte.is_ascii_digit())
        && matches!(
            extension.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg"
        )
}

/// Copy only a numbered native image under that profile's Grok session tree.
/// Media probing still decides whether the bytes are actually an image.
pub fn copy_grok_image_tool_output(
    source: &Path,
    output: &Path,
    sessions_root: &Path,
) -> Result<(), String> {
    if !source.is_absolute() {
        return Err("Grok image_gen returned a non-absolute path".into());
    }
    let root = std::fs::canonicalize(sessions_root)
        .map_err(|_| "Grok image_gen session directory is unavailable")?;
    let canonical_source =
        std::fs::canonicalize(source).map_err(|_| "Grok image_gen output is missing")?;
    let relative = canonical_source
        .strip_prefix(&root)
        .map_err(|_| "Grok image_gen output is outside its generated-image session directory")?;
    if !grok_image_session_relative_path_is_valid(relative) {
        return Err("Grok image_gen output is not a session-generated image path".into());
    }
    let metadata = std::fs::symlink_metadata(source)
        .map_err(|error| format!("Grok image_gen output is missing: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Grok image_gen output is not a regular image file".into());
    }
    if !(64..=64 * 1024 * 1024).contains(&metadata.len()) {
        return Err("Grok image_gen output is outside the supported image size".into());
    }
    let copied = std::fs::copy(source, output)
        .map_err(|error| format!("copy Grok image_gen output: {error}"))?;
    if copied != metadata.len() || !(64..=64 * 1024 * 1024).contains(&copied) {
        let _ = std::fs::remove_file(output);
        return Err("Grok image_gen output changed during copy".into());
    }
    Ok(())
}

/// Parse the agent CLI's stdout for the `{ok, path, reason}` JSON. Tries, in
/// order: the whole stdout as JSON; a
/// `{text: "...json..."}` wrapper; and codex `--json` NDJSON events
/// (`item.completed` → `agent_message.text` → JSON). Returns None if no result JSON
/// is present.
pub fn parse_output_json(stdout: &str) -> Option<GenJson> {
    fn generic_unavailable_reason(reason: &str) -> bool {
        matches!(
            reason.trim().to_ascii_lowercase().as_str(),
            "real media generation is unavailable in this cli session"
                | "image generation is unavailable for this session"
                | "image generation is unavailable in this cli session"
        )
    }
    fn concrete_outer_error(v: &serde_json::Value) -> Option<String> {
        v.get("error")
            .or_else(|| v.get("message"))
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(String::from)
    }
    fn from_value(v: &serde_json::Value) -> Option<GenJson> {
        if let Some(ok) = v.get("ok").and_then(|value| value.as_bool()) {
            return Some(GenJson {
                ok,
                path: v.get("path").and_then(|x| x.as_str()).map(String::from),
                reason: v.get("reason").and_then(|x| x.as_str()).map(String::from),
            });
        }
        // Antigravity's native JSON envelope names failures with `status` and
        // keeps the human-readable detail in `response`. Preserve that detail
        // as an honest generation failure when the agent cannot return the
        // requested {ok:false,...} payload.
        let status = v.get("status").and_then(|value| value.as_str())?;
        if matches!(status, "ERROR" | "FAILED" | "FAILURE") {
            let reason = v
                .get("response")
                .or_else(|| v.get("error"))
                .or_else(|| v.get("message"))
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
                .map(String::from)
                .or_else(|| Some(format!("Antigravity reported {status}")));
            return Some(GenJson {
                ok: false,
                path: None,
                reason,
            });
        }
        None
    }
    fn loose(s: &str) -> Option<serde_json::Value> {
        // The last {...} object in the string (CLIs prepend logs).
        let start = s.find('{')?;
        let end = s.rfind('}')?;
        if end <= start {
            return None;
        }
        serde_json::from_str(&s[start..=end]).ok()
    }
    // 1. direct JSON / last-object. Provider envelopes can themselves contain
    // the requested `{ok,...}` payload in a text field. Prefer that structured
    // inner result before treating the outer Antigravity ERROR response as an
    // opaque reason; otherwise progress chatter such as "Waiting for quota
    // reset" hides the precise final provider failure.
    if let Some(v) = loose(stdout) {
        // 2. {text:"...json..."} / Antigravity {response:"...json..."} wrapper.
        if let Some(t) = v
            .get("text")
            .and_then(|x| x.as_str())
            .or_else(|| v.get("response").and_then(|x| x.as_str()))
        {
            if let Some(mut inner) = loose(t).as_ref().and_then(from_value) {
                // The generation prompt supplies a generic honest-failure JSON
                // fallback. AGY can return that fallback inside `response` while
                // retaining the concrete image-backend failure (for example a
                // 429 RESOURCE_EXHAUSTED with its model) in the outer `error`.
                // Prefer that actionable provider detail, while preserving any
                // already-specific structured inner reason.
                if !inner.ok
                    && inner
                        .reason
                        .as_deref()
                        .is_some_and(generic_unavailable_reason)
                {
                    if let Some(detail) = concrete_outer_error(&v) {
                        inner.reason = Some(detail);
                    }
                }
                return Some(inner);
            }
        }
        if let Some(g) = from_value(&v) {
            return Some(g);
        }
    }
    // 3. codex NDJSON: item.completed → agent_message.text → JSON.
    for line in stdout.lines().rev() {
        let Ok(ev) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if ev.get("type").and_then(|x| x.as_str()) == Some("item.completed") {
            if let Some(text) = ev.pointer("/item/text").and_then(|x| x.as_str()) {
                if let Some(g) = loose(text).as_ref().and_then(from_value) {
                    return Some(g);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a fake (empty) executable file at `path`, making parent dirs. The
    /// resolver keys on `is_file()`, not the exec bit, so an empty file suffices —
    /// and the tests that use it model Unix install directories with a bare stem.
    fn touch_exe(path: &std::path::Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"#!/bin/sh\n").unwrap();
    }

    #[test]
    fn provider_context_command_keeps_explicit_entrypoint_and_child_environment() {
        let command = ProviderChildCommand {
            executable: PathBuf::from("/runtime/node"),
            entrypoint: Some(PathBuf::from("/runtime/codex.mjs")),
            admitted_environment: Some(BTreeMap::from([
                ("HOME".into(), "/runtime/home".into()),
                ("PATH".into(), "/runtime/bin".into()),
            ])),
            available: true,
        };
        let process = command
            .std_command(&["exec".into(), "--help".into()])
            .unwrap();
        assert_eq!(process.get_program(), std::ffi::OsStr::new("/runtime/node"));
        assert_eq!(
            process.get_args().collect::<Vec<_>>(),
            [
                std::ffi::OsStr::new("/runtime/codex.mjs"),
                std::ffi::OsStr::new("exec"),
                std::ffi::OsStr::new("--help"),
            ]
        );
        let temporary_directory = crate::provider_runtime::test_fixture_root();
        let mut process = command.std_command(&[]).unwrap();
        command
            .apply_admitted_std_environment(&mut process, &temporary_directory)
            .unwrap();
        assert!(process
            .get_envs()
            .any(|(name, value)| name == "HOME"
                && value == Some(std::ffi::OsStr::new("/runtime/home"))));
    }

    #[cfg(unix)]
    #[test]
    fn provider_context_command_executes_with_only_selected_environment() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = tempfile::Builder::new()
            .prefix("provider-child-command-")
            .tempdir_in(crate::provider_runtime::test_fixture_root())
            .unwrap();
        let executable = fixture.path().join("fake-provider");
        std::fs::write(
            &executable,
            "#!/bin/sh\nprintf '%s|%s|%s|%s|%s|%s|%s\\n' \"$HOME\" \"$PATH\" \"$TMPDIR\" \"$TEMP\" \"$TMP\" \"$CUT_PROVIDER_TEST_SENTINEL\" \"$1\"\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&executable, permissions).unwrap();
        let command = ProviderChildCommand {
            executable,
            entrypoint: None,
            admitted_environment: Some(BTreeMap::from([
                ("HOME".into(), "/runner/home".into()),
                ("PATH".into(), "/runner/bin".into()),
            ])),
            available: true,
        };
        let temporary_directory = fixture.path().join("provider-tmp");
        std::fs::create_dir_all(&temporary_directory).unwrap();
        let mut process = command.std_command(&["argument".into()]).unwrap();
        process.env(
            "CUT_PROVIDER_TEST_SENTINEL",
            "must-not-reach-admitted-child",
        );
        command
            .apply_admitted_std_environment(&mut process, &temporary_directory)
            .unwrap();
        let output = process.output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!(
                "/runner/home|/runner/bin|{0}|{0}|{0}||argument\n",
                temporary_directory.display()
            )
        );
    }

    /// The resolver finds an agent in an explicit install dir that is NOT on PATH —
    /// the grok-in-~/.grok/bin case and the Finder-stripped-PATH case — and
    /// scopes the grok-only ~/.grok/bin to grok. PATH still wins when it has a hit.
    /// Uses the injectable `resolve_agent_with` seam (controlled PATH dirs + mock
    /// HOME) so it never mutates process-global env — parallel-safe, matching the
    /// toolpath.rs precedent of not clobbering HOME/PATH in tests.
    #[test]
    fn resolver_finds_offpath_install_dirs_and_grok_self_managed() {
        let base = std::env::temp_dir().join(format!(
            "cutd-agent-resolve-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = base.join("home");
        let empty = base.join("empty"); // an on-PATH dir that holds NOTHING
        std::fs::create_dir_all(&empty).unwrap();

        // claude lives in ~/.local/bin (a Finder-stripped-PATH / npm-global dir),
        // grok in ~/.grok/bin (its self-managed, never-on-PATH symlink dir).
        let claude = home.join(".local").join("bin").join("claude");
        let grok = home.join(".grok").join("bin").join("grok");
        touch_exe(&claude);
        touch_exe(&grok);

        // claude: NOT in the (empty) PATH dir → resolved from ~/.local/bin.
        assert_eq!(
            resolve_agent_with("claude", std::slice::from_ref(&empty), Some(home.clone()))
                .as_deref(),
            Some(claude.as_path()),
            "claude must resolve from ~/.local/bin even when not on PATH"
        );
        // grok: NOT on PATH → resolved from its self-managed ~/.grok/bin.
        assert_eq!(
            resolve_agent_with("grok", std::slice::from_ref(&empty), Some(home.clone())).as_deref(),
            Some(grok.as_path()),
            "grok must resolve from ~/.grok/bin even when not on PATH"
        );
        // ~/.grok/bin is grok-ONLY: it is not probed for other agents.
        assert!(
            !agent_install_dirs("claude", Some(home.clone()))
                .iter()
                .any(|d| d.ends_with(".grok/bin")),
            "the ~/.grok/bin dir must be scoped to grok only"
        );
        assert!(
            agent_install_dirs("grok", Some(home.clone()))
                .iter()
                .any(|d| d.ends_with(".grok/bin")),
            "grok must include its self-managed ~/.grok/bin dir"
        );

        // PATH-FIRST precedence: a grok ALSO on PATH wins over ~/.grok/bin, so an
        // on-PATH install keeps resolving exactly as the old on_path did.
        let path_grok = base.join("pathbin").join("grok");
        touch_exe(&path_grok);
        assert_eq!(
            resolve_agent_with(
                "grok",
                &[path_grok.parent().unwrap().to_path_buf()],
                Some(home.clone())
            )
            .as_deref(),
            Some(path_grok.as_path()),
            "an on-PATH grok must take precedence over ~/.grok/bin"
        );

        // A truly absent agent resolves nowhere.
        assert!(
            resolve_agent_with(
                "doesnotexist",
                std::slice::from_ref(&empty),
                Some(home.clone())
            )
            .is_none(),
            "an uninstalled agent must resolve to None"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn windows_resolver_prefers_launchable_shim_over_extensionless_npm_file() {
        let base = std::env::temp_dir().join(format!(
            "cutd-agent-windows-shim-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bare = base.join("claude");
        let command = base.join("claude.cmd");
        touch_exe(&bare);
        touch_exe(&command);

        let candidates = exe_candidates_for("claude", true);
        assert_eq!(
            first_in_dirs(std::slice::from_ref(&base), &candidates).as_deref(),
            Some(command.as_path()),
            "a mixed npm bin directory must resolve claude.cmd, not the Unix shim",
        );

        std::fs::remove_dir_all(&base).ok();
    }

    /// The GEN path (assets.generate image-gen + transcript.translate) must DETECT
    /// AND LAUNCH an OFF-PATH provider CLI — the same off-PATH-grok bug the chat path
    /// already fixed. `gen::detect` now goes provider → `cli_for` → the full
    /// `resolve_agent` ladder (process PATH FIRST, THEN ~/.grok/bin etc.), and the
    /// dispatcher spawns the RESOLVED path. This proves the resolver half for BOTH gen
    /// providers via the injectable `resolve_agent_with` seam (controlled PATH dirs +
    /// mock HOME — NO process-env mutation, parallel-safe): codex found in a
    /// Finder-stripped-PATH npm/Homebrew dir (~/.local/bin), grok in its self-managed
    /// ~/.grok/bin, with an EMPTY process PATH. It walks the SAME provider→binary
    /// mapping (`cli_for`) `detect` uses; `resolve_agent` is just
    /// `resolve_agent_with(_, real_PATH, real_HOME)`, so an on-PATH install still wins
    /// (covered by `resolver_finds_offpath_install_dirs_and_grok_self_managed`).
    #[test]
    fn gen_path_resolves_offpath_providers() {
        let base = std::env::temp_dir().join(format!(
            "cutd-gen-resolve-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = base.join("home");
        let empty = base.join("empty"); // an on-PATH dir that holds NOTHING
        std::fs::create_dir_all(&empty).unwrap();

        // codex in ~/.local/bin (a Finder-stripped-PATH / npm-global dir), grok in
        // its self-managed ~/.grok/bin — BOTH off the (empty) process PATH.
        let codex = home.join(".local").join("bin").join("codex");
        let grok = home.join(".grok").join("bin").join("grok");
        touch_exe(&codex);
        touch_exe(&grok);

        // Every gen provider must resolve off PATH (so assets.generate actually
        // launches it) — walking provider → cli_for → resolve, exactly like detect.
        for provider in ["codex", "grok"] {
            let bin = cli_for(provider).expect("a gen provider must map to a CLI binary");
            assert!(
                resolve_agent_with(bin, std::slice::from_ref(&empty), Some(home.clone())).is_some(),
                "gen provider '{provider}' must resolve off PATH so it actually launches, \
                 not just be reported installed"
            );
        }
        // …and it is the off-PATH install dir that found each (not a PATH hit).
        assert_eq!(
            resolve_agent_with("codex", std::slice::from_ref(&empty), Some(home.clone()))
                .as_deref(),
            Some(codex.as_path()),
            "codex must resolve from ~/.local/bin for the gen path"
        );
        assert_eq!(
            resolve_agent_with("grok", std::slice::from_ref(&empty), Some(home.clone())).as_deref(),
            Some(grok.as_path()),
            "grok must resolve from its self-managed ~/.grok/bin for the gen path"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn windows_batch_command_quotes_structured_arguments() {
        let line = windows_batch_command_line(
            Path::new(r"C:\Program Files\Agent\claude.cmd"),
            &[
                "--model".into(),
                "claude sonnet".into(),
                r#"mcp=x\"y\""#.into(),
                "mcp__cutd__*,Read".into(),
            ],
        )
        .unwrap();
        assert!(line.starts_with(r#""C:\Program Files\Agent\claude.cmd" "--model""#));
        assert!(line.contains(r#""claude sonnet""#));
        assert!(line.contains(r#""mcp__cutd__*,Read""#));
        assert!(line.contains(r#""mcp=x\\""y\\""""#));
    }

    #[test]
    fn windows_batch_command_rejects_expansion_and_control_chars() {
        for unsafe_arg in [
            "%PATH%",
            "!TOKEN!",
            "line\nbreak",
            "line\rbreak",
            "nul\0byte",
            "amp&command",
            "pipe|command",
            "redirect>file",
            "caret^escape",
        ] {
            assert!(
                windows_batch_command_line(Path::new(r"C:\agent.cmd"), &[unsafe_arg.to_string()],)
                    .is_err(),
                "unsafe batch argument must be rejected: {unsafe_arg:?}",
            );
        }
    }

    #[test]
    fn cli_mapping_and_kinds() {
        assert_eq!(cli_for("codex"), Some("codex"));
        assert_eq!(cli_for("grok"), Some("grok"));
        assert_eq!(cli_for("antigravity"), Some("agy"));
        assert_eq!(cli_for("dalle"), None);
        assert!(supports_kind("codex", "image"));
        assert!(!supports_kind("codex", "video")); // codex = image only
        assert!(supports_kind("grok", "video"));
        assert!(supports_kind("antigravity", "image"));
        assert!(!supports_kind("antigravity", "video"));
        assert!(!supports_kind("nope", "image"));
        assert_eq!(default_timeout_ms("codex"), 240_000);
        assert_eq!(default_timeout_ms("grok"), 600_000);
        assert_eq!(default_timeout_ms("antigravity"), 660_000);
        assert_eq!(default_timeout_ms("nope"), 240_000);
    }

    #[test]
    fn codex_command_is_exec_stdin() {
        let c = build_command("codex", "image", false, "/scratch", Some("gpt-image-1")).unwrap();
        assert_eq!(c.cmd, "codex");
        assert_eq!(c.prompt_transport, PromptTransport::Stdin);
        assert!(c.args.contains(&"exec".to_string()));
        assert!(c.args.contains(&"--approve-for-me".to_string()));
        assert!(c.args.contains(&"--ephemeral".to_string()));
        assert!(!c.args.contains(&"--sandbox".to_string()));
        assert!(c.args.windows(2).any(|w| w == ["-m", "gpt-image-1"]));
    }

    #[test]
    fn grok_image_command_admits_native_tool_and_streams_its_result() {
        let c = build_command("grok", "image", false, "/scratch", None).unwrap();
        assert_eq!(c.cmd, "grok");
        assert_eq!(c.prompt_transport, PromptTransport::PromptFile);
        assert!(c.args.contains(&"__PROMPT_FILE__".to_string()));
        assert!(!c.args.contains(&"--model".to_string()));
        assert!(c.args.contains(&"--no-subagents".to_string()));
        assert!(c.args.windows(2).any(|w| w == ["--tools", "image_gen"]));
        assert!(c.args.windows(2).any(|w| w == ["--allow", "image_gen"]));
        assert!(c
            .args
            .windows(2)
            .any(|w| w == ["--output-format", "streaming-json"]));
        assert!(!c.args.contains(&"--max-turns".to_string()));
        assert!(!c.args.contains(&"bypassPermissions".to_string()));
        assert!(!c.args.contains(&"--no-memory".to_string()));
    }

    #[test]
    fn grok_video_and_reference_images_keep_existing_file_output_route() {
        let c = build_command("grok", "video", false, "/scratch", None).unwrap();
        assert!(c.args.windows(2).any(|w| w == ["--output-format", "json"]));
        assert!(c.args.windows(2).any(|w| w == ["--max-turns", "20"]));
        assert!(!c.args.contains(&"image_gen".to_string()));
        let c = build_command("grok", "image", true, "/scratch", None).unwrap();
        assert!(c.args.windows(2).any(|w| w == ["--output-format", "json"]));
        assert!(c.args.windows(2).any(|w| w == ["--max-turns", "20"]));
        assert!(!c.args.contains(&"image_gen".to_string()));
    }

    #[test]
    fn antigravity_generation_keeps_normal_image_behavior_enabled() {
        let c = build_command(
            "antigravity",
            "image",
            false,
            "/scratch",
            Some("Gemini 3.5 Flash"),
        )
        .unwrap();
        assert_eq!(c.cmd, "agy");
        assert_eq!(c.prompt_transport, PromptTransport::Argument);
        assert!(c.args.contains(&"--new-project".to_string()));
        assert!(c.args.contains(&"--sandbox".to_string()));
        assert!(c
            .args
            .contains(&"--dangerously-skip-permissions".to_string()));
        assert!(!c.args.contains(&"--disable-slash-commands".to_string()));
        assert!(c.args.windows(2).any(|w| w == ["--output-format", "json"]));
        assert_eq!(&c.args[c.args.len() - 2..], ["--print", "__PROMPT_TEXT__"]);
    }

    #[test]
    fn prompt_pins_the_exact_output_path_and_honest_failure() {
        let p = build_prompt("codex", "image", "a red fox", "/scratch/generated.png", &[]);
        assert!(p.contains("/scratch/generated.png"));
        assert!(p.contains("a red fox"));
        assert!(
            p.to_lowercase().contains("fail honestly") || p.contains("do not write a fake file")
        );
        assert!(p.contains("\"ok\":false"));
    }

    #[test]
    fn antigravity_prompt_is_a_direct_exact_path_image_request() {
        let p = build_prompt(
            "antigravity",
            "image",
            "a red fox",
            "/scratch/generated.png",
            &[],
        );
        assert_eq!(
            p,
            "Create one image from this description: \"a red fox\"\n\
Save the final PNG EXACTLY this path:\n\
/scratch/generated.png"
        );
        assert!(!p.contains("Load and use"));
        assert!(!p.contains("\"ok\":false"));
    }

    #[test]
    fn antigravity_prompt_keeps_reference_paths_simple_and_read_only() {
        let paths = vec!["/scratch/reference-1.png".to_string()];
        let p = build_prompt(
            "antigravity",
            "image",
            "keep the palette",
            "/scratch/generated.png",
            &paths,
        );
        assert!(p.contains("Reference 1: /scratch/reference-1.png"));
        assert!(p.contains("Do not overwrite the reference images."));
    }

    #[test]
    fn grok_native_image_prompt_requests_one_tool_call() {
        let p = build_prompt(
            "grok",
            "image",
            "keep the palette",
            "/scratch/generated.png",
            &[],
        );
        assert!(p.contains("keep the palette"));
        assert!(p.contains("calling image_gen exactly once"));
    }

    #[test]
    fn grok_reference_image_prompt_keeps_copied_reference_paths() {
        let paths = vec!["/scratch/reference-1.png".to_string()];
        let p = build_prompt(
            "grok",
            "image",
            "keep the palette",
            "/scratch/generated.png",
            &paths,
        );
        assert!(p.contains("Reference 1: /scratch/reference-1.png"));
        assert!(p.contains("Do not overwrite them"));
        assert!(!p.contains("calling image_gen exactly once"));
    }

    #[test]
    fn grok_image_accepts_only_one_completed_native_tool_result() {
        let stdout = r#"{"type":"tool_call","toolName":"image_gen","toolCallId":"native-1"}
{"type":"tool_call_update","toolCallId":"other","status":"completed","rawOutput":{"type":"ImageGen","path":"/wrong.png"}}
{"type":"tool_call_update","toolCallId":"native-1","status":"completed","rawOutput":{"type":"ImageGen","path":"/real.png"}}
{"type":"result","text":"/fake.png"}"#;
        assert_eq!(
            grok_image_tool_path(stdout).unwrap(),
            PathBuf::from("/real.png")
        );
        assert!(grok_image_tool_path("{\"type\":\"result\",\"text\":\"/fake.png\"}").is_err());
        let duplicate = format!("{stdout}\n{{\"type\":\"tool_call_update\",\"toolCallId\":\"native-1\",\"status\":\"completed\",\"rawOutput\":{{\"type\":\"ImageGen\",\"path\":\"/second.png\"}}}}");
        assert!(grok_image_tool_path(&duplicate).is_err());
    }

    #[test]
    fn grok_image_exit_diagnostic_reports_only_observed_tool_state() {
        let stdout = br#"{"type":"tool_call","toolName":"image_gen","toolCallId":"private-id","prompt":"secret-looking-token"}
{"type":"tool_call_update","toolCallId":"other-id","status":"completed","rawOutput":{"type":"ImageGen","path":"/private/image.png"}}
{"type":"result","text":"secret-looking-token /private/image.png"}"#;
        let diagnostic = grok_image_exit_diagnostic(stdout, b"secret-looking-token", false, false);
        assert_eq!(diagnostic, "diagnostic: image_gen_not_completed; image_gen calls=1, completed=0; stderr_present=true; stdout_truncated=false; stderr_truncated=false");
        assert!(!diagnostic.contains("secret-looking-token"));
        assert!(!diagnostic.contains("private-id"));
        assert!(!diagnostic.contains("/private/image.png"));

        let completed = br#"{"type":"tool_call","toolName":"image_gen","toolCallId":"private-id"}
{"type":"tool_call_update","toolCallId":"private-id","status":"completed","rawOutput":{"type":"ImageGen","path":"/private/image.png"}}"#;
        assert_eq!(grok_image_exit_diagnostic(completed, b"", false, false), "diagnostic: image_gen_completed_before_exit; image_gen calls=1, completed=1; stderr_present=false; stdout_truncated=false; stderr_truncated=false");
    }

    #[test]
    fn grok_image_exit_diagnostic_keeps_unknown_output_private() {
        let diagnostic =
            grok_image_exit_diagnostic(b"secret-looking-token", b"private stderr", false, true);
        assert_eq!(diagnostic, "diagnostic: no_image_gen_call_observed; image_gen calls=0, completed=0; stderr_present=true; stdout_truncated=false; stderr_truncated=true");
        let truncated = grok_image_exit_diagnostic(b"secret-looking-token", b"", true, false);
        assert!(truncated.contains("diagnostic: partial_trace"));
        assert!(!truncated.contains("secret-looking-token"));
    }

    #[test]
    fn grok_image_copy_accepts_only_native_session_image_paths() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join(".grok/sessions");
        let image_dir = sessions.join("project/123e4567-e89b-12d3-a456-426614174000/images");
        std::fs::create_dir_all(&image_dir).unwrap();
        let output = dir.path().join("output.png");
        let unrelated = dir.path().join("provider-auth.png");
        std::fs::write(&unrelated, vec![7u8; 64]).unwrap();
        assert!(copy_grok_image_tool_output(&unrelated, &output, &sessions).is_err());
        assert!(!output.exists());
        let invalid = sessions.join("project/auth.json");
        std::fs::write(&invalid, vec![7u8; 64]).unwrap();
        assert!(copy_grok_image_tool_output(&invalid, &output, &sessions).is_err());
        let wrong_name = image_dir.join("auth.png");
        std::fs::write(&wrong_name, vec![7u8; 64]).unwrap();
        assert!(copy_grok_image_tool_output(&wrong_name, &output, &sessions).is_err());
        assert!(copy_grok_image_tool_output(&image_dir, &output, &sessions).is_err());
        let source = image_dir.join("1.png");
        std::fs::write(&source, vec![7u8; 64]).unwrap();
        copy_grok_image_tool_output(&source, &output, &sessions).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), vec![7u8; 64]);
    }

    #[cfg(unix)]
    #[test]
    fn grok_image_copy_rejects_session_symlink_to_other_file() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join(".grok/sessions");
        let image_dir = sessions.join("project/123e4567-e89b-12d3-a456-426614174000/images");
        std::fs::create_dir_all(&image_dir).unwrap();
        let unrelated = dir.path().join("private.png");
        std::fs::write(&unrelated, vec![7u8; 64]).unwrap();
        let link = image_dir.join("1.png");
        std::os::unix::fs::symlink(&unrelated, &link).unwrap();
        assert!(
            copy_grok_image_tool_output(&link, &dir.path().join("output.png"), &sessions).is_err()
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn grok_sessions_root_uses_admitted_home() {
        let environment = BTreeMap::from([("HOME".into(), "/admitted/home".into())]);
        assert_eq!(
            grok_sessions_root(Some(&environment)).unwrap(),
            PathBuf::from("/admitted/home/.grok/sessions")
        );
    }

    #[cfg(windows)]
    #[test]
    fn grok_sessions_root_uses_windows_userprofile_and_session_path_shape() {
        let environment = BTreeMap::from([
            ("HOME".into(), r"C:\other-home".into()),
            ("USERPROFILE".into(), r"C:\Users\Cut".into()),
        ]);
        assert_eq!(
            grok_sessions_root(Some(&environment)).unwrap(),
            PathBuf::from(r"C:\Users\Cut\.grok\sessions")
        );
        assert!(grok_image_session_relative_path_is_valid(Path::new(
            r"encoded-cwd\123e4567-e89b-12d3-a456-426614174000\images\1.jpg"
        )));
    }

    #[test]
    fn parses_direct_json() {
        let g = parse_output_json("noise\n{\"ok\":true,\"path\":\"/x/g.png\",\"summary\":\"s\"}\n")
            .unwrap();
        assert!(g.ok);
        assert_eq!(g.path.as_deref(), Some("/x/g.png"));
    }

    #[test]
    fn parses_codex_ndjson_agent_message() {
        let nd = r#"{"type":"thread.started"}
{"type":"item.completed","item":{"type":"agent_message","text":"{\"ok\":true,\"path\":\"/x/g.png\"}"}}"#;
        let g = parse_output_json(nd).unwrap();
        assert!(g.ok);
        assert_eq!(g.path.as_deref(), Some("/x/g.png"));
    }

    #[test]
    fn parses_honest_failure() {
        let g = parse_output_json("{\"ok\":false,\"reason\":\"no image tool\"}").unwrap();
        assert!(!g.ok);
        assert_eq!(g.reason.as_deref(), Some("no image tool"));
    }

    #[test]
    fn parses_antigravity_response_wrapped_json() {
        let g = parse_output_json(
            r#"{"status":"SUCCESS","response":"{\"ok\":true,\"path\":\"/x/g.png\"}"}"#,
        )
        .unwrap();
        assert!(g.ok);
        assert_eq!(g.path.as_deref(), Some("/x/g.png"));
    }

    #[test]
    fn parses_antigravity_envelope_failure() {
        let g = parse_output_json(
            r#"{"status":"ERROR","response":"image generation is unavailable for this session"}"#,
        )
        .unwrap();
        assert!(!g.ok);
        assert_eq!(
            g.reason.as_deref(),
            Some("image generation is unavailable for this session")
        );
    }

    #[test]
    fn prefers_antigravity_structured_failure_over_progress_chatter() {
        let g = parse_output_json(
            r#"{"status":"ERROR","response":"Waiting for quota reset.\n```json\n{\"ok\":false,\"reason\":\"Image generation model quota exhausted (429 RESOURCE_EXHAUSTED)\"}\n```","error":"429 Too Many Requests"}"#,
        )
        .unwrap();
        assert!(!g.ok);
        assert_eq!(
            g.reason.as_deref(),
            Some("Image generation model quota exhausted (429 RESOURCE_EXHAUSTED)")
        );
    }

    #[test]
    fn prefers_antigravity_concrete_outer_error_over_generic_prompt_fallback() {
        let g = parse_output_json(
            r#"{"status":"ERROR","response":"Waiting for quota reset.\n```json\n{\"ok\":false,\"reason\":\"real media generation is unavailable in this CLI session\"}\n```","error":"failed to generate content: 429 Too Many Requests (RESOURCE_EXHAUSTED; model=gemini-3.1-flash-image)"}"#,
        )
        .unwrap();
        assert!(!g.ok);
        assert_eq!(
            g.reason.as_deref(),
            Some(
                "failed to generate content: 429 Too Many Requests (RESOURCE_EXHAUSTED; model=gemini-3.1-flash-image)"
            )
        );
    }
}
