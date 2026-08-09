//! gen.rs — `assets.generate`: image/video via the user's own agent CLI.
//!
//! Clean-room port of ShellX Canvas's `media-provider` adapter (the design, not the
//! code). NO model is hosted — the user's installed codex (gpt-image) / grok
//! (grok-imagine) CLI does the generation; cutd just (1) DETECTS the CLI, (2) spawns
//! it with a strict prompt that tells it to write the binary to an exact path, (3)
//! validates the result via the NORMAL import probe (ffprobe — a fake/placeholder
//! file fails to probe), and (4) imports it like any upload through `record_import`.
//! This is the Openverse philosophy applied to generation: integration, not hosting.
//!
//! This module is the PURE part (CLI mapping, command + prompt construction, output
//! JSON parsing) — unit-tested without spawning anything. The actual spawn (with a
//! timeout) + import live in dispatch.rs `assets_generate`. Honest degradation: the
//! CLI absent → the verb returns `ok:false` with a clear reason (never a fake asset).

use std::path::{Path, PathBuf};

/// The CLI binary for a provider (`codex` → gpt-image, `grok` → grok-imagine).
pub fn cli_for(provider: &str) -> Option<&'static str> {
    match provider {
        "codex" => Some("codex"),
        "grok" => Some("grok"),
        _ => None,
    }
}

/// Is the provider's CLI installed anywhere we can launch it? Maps the provider to
/// its binary name (`cli_for`), then uses the FULL resolution ladder (process PATH
/// FIRST, then the explicit agent install dirs including ~/.grok/bin and
/// Homebrew/npm dirs) via [`resolve_agent`] — NOT a process-PATH-only scan.
/// The on-PATH case preserves normal resolution because `resolve_agent` searches
/// the process PATH first. Mirrors
/// `chat::detect` / `translate::detect_cli`.
pub fn detect(provider: &str) -> bool {
    cli_for(provider)
        .map(|bin| resolve_agent(bin).is_some())
        .unwrap_or(false)
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

/// Which kinds a provider can generate (codex = image only; grok = image + video).
pub fn supports_kind(provider: &str, kind: &str) -> bool {
    match provider {
        "codex" => kind == "image",
        "grok" => kind == "image" || kind == "video",
        _ => false,
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

/// A resolved CLI invocation: the command, its args, the prompt, and whether the
/// prompt is delivered on STDIN (codex) or written to a `--prompt-file` (grok).
#[derive(Debug, Clone, PartialEq)]
pub struct GenCommand {
    pub cmd: String,
    pub args: Vec<String>,
    pub via_stdin: bool,
}

/// Build the agent-CLI invocation for `provider`. `workspace` is the scratch
/// cwd; `model` is optional.
pub fn build_command(provider: &str, workspace: &str, model: Option<&str>) -> Option<GenCommand> {
    match provider {
        "codex" => {
            let mut args = vec![
                "exec".into(),
                "-".into(),
                "--json".into(),
                "--sandbox".into(),
                "workspace-write".into(),
                "-C".into(),
                workspace.into(),
                "--skip-git-repo-check".into(),
            ];
            if let Some(m) = model.filter(|m| !m.is_empty()) {
                args.push("-m".into());
                args.push(m.into());
            }
            Some(GenCommand {
                cmd: "codex".into(),
                args,
                via_stdin: true,
            })
        }
        "grok" => {
            // Prompt is written to a file; the caller substitutes the path for the
            // PROMPT_FILE placeholder. The flags keep generation headless,
            // offline, and bounded.
            let mut args = vec![
                "--prompt-file".into(),
                "__PROMPT_FILE__".into(),
                "--output-format".into(),
                "json".into(),
                "--cwd".into(),
                workspace.into(),
                "--permission-mode".into(),
                "bypassPermissions".into(),
                "--always-approve".into(),
                "--disable-web-search".into(),
                "--no-memory".into(),
                "--max-turns".into(),
                "20".into(),
            ];
            args.push("--model".into());
            args.push(
                model
                    .filter(|m| !m.is_empty())
                    .unwrap_or("grok-build")
                    .into(),
            );
            Some(GenCommand {
                cmd: "grok".into(),
                args,
                via_stdin: false,
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
    let accepted = if kind == "video" {
        "mp4, webm, or ogv"
    } else {
        "png, jpg, gif, or webp"
    };
    let label = if provider == "codex" {
        "Codex (gpt-image)"
    } else {
        "Grok Build (Imagine)"
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
        lines.push("Use real image-generation tooling available to Codex, such as image_gen or the OpenAI Image API. If this CLI session has no such tool, fail honestly.".to_string());
    } else if kind == "video" {
        lines.push(format!(
            "Use Grok Build's native Imagine flow: /imagine {}.",
            serde_json::to_string(&format!("video: {description}")).unwrap_or_default()
        ));
        lines.push("If Grok exposes native video tools, use image_to_video or reference_to_video. A fixed 6 or 10 second clip is acceptable.".to_string());
        lines.push("If this Grok CLI session cannot complete a video-capable Imagine request, fail honestly without writing a fake file.".to_string());
    } else {
        lines.push(format!(
            "Use Grok Build's native Imagine flow. Prefer the slash command /imagine {}.",
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

/// Parse the agent CLI's stdout for the `{ok, path, reason}` JSON. Tries, in
/// order: the whole stdout as JSON; a
/// `{text: "...json..."}` wrapper; and codex `--json` NDJSON events
/// (`item.completed` → `agent_message.text` → JSON). Returns None if no result JSON
/// is present.
pub fn parse_output_json(stdout: &str) -> Option<GenJson> {
    fn from_value(v: &serde_json::Value) -> Option<GenJson> {
        let ok = v.get("ok")?.as_bool()?;
        Some(GenJson {
            ok,
            path: v.get("path").and_then(|x| x.as_str()).map(String::from),
            reason: v.get("reason").and_then(|x| x.as_str()).map(String::from),
        })
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
    // 1. direct JSON / last-object.
    if let Some(v) = loose(stdout) {
        if let Some(g) = from_value(&v) {
            return Some(g);
        }
        // 2. {text:"...json..."} wrapper.
        if let Some(t) = v.get("text").and_then(|x| x.as_str()) {
            if let Some(inner) = loose(t).as_ref().and_then(from_value) {
                return Some(inner);
            }
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
        assert_eq!(cli_for("dalle"), None);
        assert!(supports_kind("codex", "image"));
        assert!(!supports_kind("codex", "video")); // codex = image only
        assert!(supports_kind("grok", "video"));
        assert!(!supports_kind("nope", "image"));
    }

    #[test]
    fn codex_command_is_exec_stdin() {
        let c = build_command("codex", "/scratch", Some("gpt-image-1")).unwrap();
        assert_eq!(c.cmd, "codex");
        assert!(c.via_stdin);
        assert!(c.args.contains(&"exec".to_string()));
        assert!(c.args.contains(&"workspace-write".to_string()));
        assert!(c.args.windows(2).any(|w| w == ["-m", "gpt-image-1"]));
    }

    #[test]
    fn grok_command_uses_prompt_file() {
        let c = build_command("grok", "/scratch", None).unwrap();
        assert_eq!(c.cmd, "grok");
        assert!(!c.via_stdin);
        assert!(c.args.contains(&"__PROMPT_FILE__".to_string()));
        // default model.
        assert!(c.args.windows(2).any(|w| w == ["--model", "grok-build"]));
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
    fn prompt_lists_only_copied_reference_paths() {
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
}
