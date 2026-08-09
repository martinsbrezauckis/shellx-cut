// ─────────────────────────────────────────────────────────────────────────────
// ShellX Cut — Tauri 2 desktop shell (library entry).
//
// ROLE
//   The thin native shell that wraps ShellX Cut as a standalone desktop app on
//   Windows / macOS / Linux. The webview owns ALL UI; Rust only opens the
//   window, exposes a tiny IPC status surface, and — the part that makes the
//   app self-contained — launches the bundled cutd ENGINE as a managed child
//   process on loopback and points the webview at it.
//
// WHY A SIDECAR CHILD, NOT A LINKED LIBRARY (the architecture decision)
//   cutd (app/server) is the single state holder for a Cut project: axum
//   REST + WS + static UI + MCP, all routed through one verb dispatcher
//   (zero-local-mutation contract invariants, the single-state-holder contract). Two ways to put it behind a Tauri window:
//     (a) link cut-server as a library and run axum in-process;
//     (b) bundle the cutd binary and spawn it as a child.
//   We chose (b):
//     * ZERO changes to the engine workspace — (a) would force a bin→lib
//       split of cut-server (phase E constraint: no restructuring crates).
//     * The shipped cutd.exe is BYTE-IDENTICAL to the headless `cutd serve`
//       binary, so the hard requirement "100% debug/agent surface preserved"
//       holds by construction: REST verbs, WS events, `cutd mcp` proxy and
//       the `cutd verb` CLI all work against the desktop app exactly as
//       against a terminal-run server.
//     * It uses the established spawn-child → wait-for-ready
//       → navigate-webview → kill-on-exit lifecycle, simplified because cutd is
//       self-contained Rust — no Node runtime floor to probe.
//   Tradeoff accepted: two processes and a few MB of duplicate runtime.
//
// PORT POLICY (debug-surface friendliness)
//   The engine's documented home is 127.0.0.1:6161 (httpc::SERVER_ADDR —
//   `cutd mcp` proxies there by default). On launch:
//     1. If a cutd already answers on 6161 (e.g. a dev `cutd serve`), REUSE it:
//        navigate the webview there, spawn nothing, kill nothing on exit.
//        engine_status reports mode "external".
//     2. Else if 6161 is free, spawn the bundled cutd there — MCP proxy and
//        every documented agent flow work out of the box.
//     3. Else (6161 held by a non-cutd process) pick an OS-chosen free port
//        and report the real URL via engine_status — honest, never silent.
//   The free-port check→spawn gap is a benign TOCTOU on a single-user
//   loopback; the ready-poll below catches a lost race as a startup failure.
//
// RUNTIME DEPENDENCIES (honest, per-verb)
//   The shell needs nothing beyond the bundled cutd + ui-dist. Media verbs
//   shell out to ffmpeg/ffprobe on PATH; transcription/perception verbs need
//   the Python sidecar. Missing tools surface as actionable per-verb errors
//   from the engine (public verb contract error contract) — the app itself always launches.
//
// WINDOWS WEBVIEW2 (tauri.conf.json → bundle.windows.webviewInstallMode)
//   downloadBootstrapper — Win11 ships WebView2; Win10 fetches the small
//   bootstrapper at install time. JSON has no comments, so the note lives here.
//
// SECURITY
//   cutd binds 127.0.0.1 ONLY (http::DEFAULT_ADDR policy); the shell never
//   passes a non-loopback addr and never exposes a port externally —
//   consistent with the loopback-only service boundary.
// ─────────────────────────────────────────────────────────────────────────────

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::Manager;

#[cfg(feature = "webdriver-test")]
const WEBDRIVER_TEST_BUILD_MARKER: &str = "shellx-cut/webdriver-test-enabled@1";
// `Emitter` brings `AppHandle::emit` into scope — used by the global-shortcut
// handler to push the `cut:record-hotkey` event to the webview. Desktop-only
// (the only emit site is the desktop-gated hotkey block); silenced on the off
// chance a future non-desktop target compiles this file without that block.
#[cfg(desktop)]
#[allow(unused_imports)]
use tauri::Emitter;

mod tools;
mod update_handoff;
mod update_identity;
mod update_settings;
mod update_state;
#[cfg(desktop)]
mod updater_key_transition;
use tools::ToolResolution;

/// The engine's documented default address (mirrors cutd httpc::SERVER_ADDR).
/// Spawning here keeps `cutd mcp` proxy + every documented agent flow working
/// with zero configuration.
const ENGINE_DEFAULT_ADDR: &str = "127.0.0.1:6161";

/// How long the setup hook waits for the spawned engine to answer on its
/// port. cutd is a native binary that binds immediately — 15 s is generous
/// headroom for first-run AV scanning on a fresh Windows install.
const ENGINE_READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Env var passed to cutd so `/api/agent` can serve the bundled skill/reference
/// files from a packaged desktop app.
const ENV_AGENT_DOCS_DIR: &str = "SHELLX_CUT_AGENT_DOCS_DIR";

/// Narrow opt-in used by Windows WebView tests. Wry
/// supplies WebView2's browser arguments programmatically, so the generic
/// WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS environment variable is not reliably
/// appended by current runtimes. Accepting only a port keeps the shipping
/// binary unchanged in normal launches and avoids an arbitrary browser-argument
/// injection surface.
#[cfg(any(windows, test))]
const ENV_WEBVIEW2_DEBUG_PORT: &str = "SHELLX_CUT_WEBVIEW2_DEBUG_PORT";
#[cfg(any(windows, test))]
const ENV_WEBVIEW2_DATA_TOKEN: &str = "SHELLX_CUT_WEBVIEW2_DATA_TOKEN";
#[cfg(any(windows, test))]
const WEBVIEW2_DEFAULT_BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
     --autoplay-policy=no-user-gesture-required";

#[cfg(any(windows, test))]
fn webview2_debug_browser_args(value: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "{ENV_WEBVIEW2_DEBUG_PORT} must be an integer from 1 through 65535"
        ));
    }
    let port = value.parse::<u16>().map_err(|_| {
        format!("{ENV_WEBVIEW2_DEBUG_PORT} must be an integer from 1 through 65535")
    })?;
    if port == 0 {
        return Err(format!(
            "{ENV_WEBVIEW2_DEBUG_PORT} must be an integer from 1 through 65535"
        ));
    }
    Ok(Some(format!(
        "{WEBVIEW2_DEFAULT_BROWSER_ARGS} --remote-debugging-port={port}"
    )))
}

#[cfg(any(windows, test))]
fn webview2_data_token(value: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let valid = !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if !valid {
        return Err(format!(
            "{ENV_WEBVIEW2_DATA_TOKEN} must be 1-80 ASCII letters, digits, hyphens, or underscores"
        ));
    }
    Ok(Some(value.to_string()))
}

/// Handle to the spawned cutd child, kept in Tauri managed state so we can
/// kill it on app exit. `None` when reusing an external server (mode
/// "external") or when startup failed — nothing to kill in either case.
struct EngineProcess(Mutex<Option<Child>>);

fn stop_owned_engine(app: &tauri::AppHandle) {
    update_handoff::stop_owned_engine_best_effort(app);
}

/// The desktop shell's engine state, exposed to the webview via
/// `engine_status`. Set once during setup.
enum EngineState {
    /// Engine reachable. `url` is the loopback origin (e.g.
    /// "http://127.0.0.1:6161/"); `mode` is "spawned" (our child) or
    /// "external" (a cutd that was already running); `ui` is false when the
    /// bundled ui-dist was missing (API up, fallback page stays).
    /// `tools_hint` is "" when ffmpeg + the sidecar are both present, else
    /// the actionable bootstrap text naming exactly what is missing — surfaced
    /// so a cold install is never silently degraded.
    Wired {
        url: String,
        mode: &'static str,
        ui: bool,
        tools_hint: String,
        engine_version: String,
    },
    /// Engine not reachable. `reason` is a single honest, user-surfaceable
    /// sentence (binary missing, port race lost, early exit + log path…).
    Unwired { reason: String },
}

/// Managed wrapper around [`EngineState`]. Starts `Unwired` with a generic
/// reason until the setup hook resolves the real outcome.
struct EngineStatus(Mutex<EngineState>);

// Native file picking is done from the UI via the dialog plugin's JS
// `open()`/`save()` commands, NOT a custom app command. The UI runs at the
// engine's remote loopback origin where app-command IPC is ACL-denied. Because
// the engine can move off 6161, setup grants the minimum dialog/event
// permissions to the one validated origin it actually selected. A static
// capability would either break fallback ports or over-grant every loopback
// service.

/// Report whether the desktop shell wired the engine (and on which URL), so
/// the webview / fallback page / e2e can show an honest status without
/// re-probing. Returns one of:
///   * `{ "wired": true, "url": "http://127.0.0.1:6161/", "mode": "spawned",
///        "ui": true }`
///   * `{ "wired": false, "reason": "engine unavailable: …" }`
#[tauri::command]
fn engine_status(state: tauri::State<'_, EngineStatus>) -> serde_json::Value {
    match &*state.0.lock().unwrap() {
        EngineState::Wired {
            url,
            mode,
            ui,
            tools_hint,
            engine_version,
        } => {
            serde_json::json!({
                "wired": true, "url": url, "mode": mode, "ui": ui,
                "shell_version": env!("CARGO_PKG_VERSION"),
                "engine_version": engine_version,
                // "" ⇒ all deps present; non-empty ⇒ show the bootstrap card.
                "tools_hint": tools_hint,
            })
        }
        EngineState::Unwired { reason } => {
            serde_json::json!({ "wired": false, "reason": reason })
        }
    }
}

/// Detailed tool doctor for the bootstrap UI / agent. Reports, per heavy
/// dependency, whether it resolved and from where, plus the actionable hint.
/// Mirrors the engine-side `cut_media::toolpath::doctor_media` philosophy but
/// runs in the shell so it works even before the engine is up.
#[tauri::command]
fn tools_doctor(state: tauri::State<'_, ToolResolutionState>) -> serde_json::Value {
    let r = state.0.lock().unwrap();
    serde_json::json!({
        "ffmpeg": {
            "ok": r.ffmpeg_ok,
            "dir": r.ffmpeg_dir.as_ref().map(|p| p.display().to_string()),
            "source": r.ffmpeg_source,
        },
        "sidecar": {
            "ok": r.sidecar_ok,
            "dir": r.sidecar_dir.as_ref().map(|p| p.display().to_string()),
            "source": if r.sidecar_ok { "bundled-or-appdata" } else { "missing" },
        },
        "hint": r.bootstrap_hint(),
    })
}

/// Managed wrapper for the startup-computed [`ToolResolution`] so the
/// `tools_doctor` IPC can report it without re-probing.
struct ToolResolutionState(Mutex<ToolResolution>);

/// Minimal blocking loopback HTTP/1.1 GET — the same no-dependency pattern as
/// cutd's own httpc.rs (Connection: close, read to EOF; axum honors it).
/// Returns the raw response (headers + body) or Err on connect/io failure.
fn http_get(addr: &str, path: &str, timeout: Duration) -> Result<String, String> {
    let sock_addr: std::net::SocketAddr =
        addr.parse().map_err(|e| format!("bad addr {addr}: {e}"))?;
    let mut stream = TcpStream::connect_timeout(&sock_addr, timeout).map_err(|e| e.to_string())?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// True when a LIVE cutd answers on `addr`. We GET /api/verbs — cutd serves
/// schema/verbs.json verbatim there, so the body naming a core verb is a
/// strong, cheap fingerprint that this is OUR engine and not some other
/// service that happens to hold the port.
fn is_cutd(addr: &str) -> bool {
    matches!(http_get(addr, "/api/verbs", Duration::from_millis(1500)),
             Ok(raw) if raw.contains("project.state"))
}

fn coherent_cutd_version(addr: &str) -> Result<String, String> {
    let raw = http_get(addr, "/api/agent", Duration::from_millis(1500))
        .map_err(|e| format!("engine identity unavailable: {e}"))?;
    let engine_version = update_identity::engine_version_from_http(&raw)?;
    update_identity::require_coherent_versions(env!("CARGO_PKG_VERSION"), &engine_version)?;
    Ok(engine_version)
}

fn http_status(raw: &str) -> Option<u16> {
    raw.lines().next()?.split_whitespace().nth(1)?.parse().ok()
}

fn response_looks_like_shellx_ui(raw: &str) -> bool {
    if http_status(raw) != Some(200) {
        return false;
    }
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
    let head = head.to_ascii_lowercase();
    let body = body.to_ascii_lowercase();
    head.contains("content-type: text/html")
        && body.contains("<!doctype html")
        && (body.contains("id=\"root\"") || body.contains("shellx cut"))
}

fn cutd_serves_ui(addr: &str) -> bool {
    matches!(http_get(addr, "/", Duration::from_millis(1500)),
             Ok(raw) if response_looks_like_shellx_ui(&raw))
}

/// Locate the cutd binary. Resolution order:
///   1. SHELLX_CUT_CUTD env var (explicit override for unusual setups / dev);
///   2. next to the shell executable — where Tauri's externalBin bundling
///      places it in an installed app (cutd.exe beside shellx-cut.exe);
///   3. bare "cutd" → PATH lookup (dev convenience; a missing binary then
///      surfaces as an honest spawn error).
fn cutd_program() -> PathBuf {
    if let Ok(p) = std::env::var("SHELLX_CUT_CUTD") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let name = if cfg!(windows) { "cutd.exe" } else { "cutd" };
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let beside = dir.join(name);
            if beside.exists() {
                return beside;
            }
        }
    }
    PathBuf::from(name)
}

/// Decide where the engine should live. See PORT POLICY in the header.
/// Returns `(addr, reuse)`: `reuse == true` means a cutd is ALREADY serving
/// there and we must not spawn (nor later kill) anything.
fn pick_engine_addr_for(default_addr: &str) -> Result<(String, bool), String> {
    if is_cutd(default_addr) {
        return Ok((default_addr.to_string(), true));
    }
    // 6161 free? Claim it (drop the probe listener right before spawning).
    if TcpListener::bind(default_addr).is_ok() {
        return Ok((default_addr.to_string(), false));
    }
    // 6161 held by something that is not a cutd → OS-chosen free port.
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("engine unavailable: no free loopback port ({e})"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("engine unavailable: could not read local addr ({e})"))?
        .port();
    drop(listener);
    Ok((format!("127.0.0.1:{port}"), false))
}

fn pick_engine_addr() -> Result<(String, bool), String> {
    pick_engine_addr_for(ENGINE_DEFAULT_ADDR)
}

/// Spawn (or adopt) the cutd engine and wait until it answers.
///
/// Returns `(child, url, ui_present)` — `child` is `None` when an external
/// cutd was reused. `Err(reason)` on ANY failure (no free port, binary
/// missing, child exited early, ready-poll timeout) so the caller can keep
/// the fallback page up AND surface the honest reason via `engine_status`.
/// Never panics.
///
/// `tools` is the startup tool resolution: when it found a bundled /
/// app-data ffmpeg or sidecar dir, we EXPORT it to the spawned cutd via the
/// env vars the engine's resolver reads (SHELLX_CUT_FFMPEG_DIR /
/// SHELLX_CUT_SIDECAR_DIR), so the engine + python sidecar find the same tools
/// on a cold install with zero engine-code coupling to the shell.
fn spawn_engine(
    resource_dir: &std::path::Path,
    tools: &ToolResolution,
) -> Result<(Option<Child>, String, bool, String), String> {
    let (addr, reuse) = pick_engine_addr()?;
    let url = format!("http://{addr}/");
    if reuse {
        let engine_version = coherent_cutd_version(&addr)?;
        let ui_present = cutd_serves_ui(&addr);
        eprintln!(
            "[shellx-cut] reusing external cutd v{engine_version} at {url} (ui: {ui_present})"
        );
        return Ok((None, url, ui_present, engine_version));
    }

    let program = cutd_program();
    // ui-dist ships as a Tauri resource (bundle.resources). When it is
    // missing (broken package) we still start the engine — API-first,
    // runtime-state invariant — and keep the fallback page with an honest note.
    let ui_dist = resource_dir.join("ui-dist");
    let ui_present = ui_dist.join("index.html").exists();

    // cutd logs (tracing) go to stderr; capture them to a temp file so a
    // startup failure on the INSTALLED app (no console) stays inspectable:
    // %TEMP%/shellx-cut-engine.log. Best-effort — Stdio::null() if create fails.
    let log_path = std::env::temp_dir().join("shellx-cut-engine.log");
    let log_file = std::fs::File::create(&log_path).ok();

    let mut cmd = Command::new(&program);
    cmd.arg("serve").arg("--addr").arg(&addr);

    // hand the engine the resolved tool locations. The engine's toolpath
    // resolver reads these env vars at the top of its resolution order, so a
    // bundled / app-data ffmpeg or sidecar wins over PATH. When tools were not
    // found we set NOTHING — the engine then falls through to its own beside-
    // exe / app-data / PATH rungs (identical logic), so behaviour is correct
    // either way; setting them here is just the shell sharing what it already
    // computed for the bootstrap state.
    if let Some(ff) = &tools.ffmpeg_dir {
        cmd.env(tools::ENV_FFMPEG_DIR, ff);
    }
    if let Some(sc) = &tools.sidecar_dir {
        cmd.env(tools::ENV_SIDECAR_DIR, sc);
    }
    let agent_docs_dir = resource_dir.join("agent-docs");
    if agent_docs_dir.join("skill/shellx-cut/SKILL.md").is_file() {
        cmd.env(ENV_AGENT_DOCS_DIR, &agent_docs_dir);
    }
    // Seamless GPU: let the engine auto-select the best HARDWARE-capable installed
    // ffmpeg (NVENC/cuda, QSV, VideoToolbox, …) with no user action. A user's
    // explicit SHELLX_CUT_FFMPEG override still wins; software-only machines stay
    // on the bundled build. See cut-media toolpath::ffmpeg() auto path.
    cmd.env(tools::ENV_FFMPEG_AUTO, "1");

    if ui_present {
        cmd.arg("--ui-dist").arg(&ui_dist);
    } else {
        // No bundle → don't point cutd at a dead path; API-only.
        cmd.arg("--headless");
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log_file.map(Stdio::from).unwrap_or_else(Stdio::null));

    // Windows: cutd is a console-subsystem binary; spawned from a GUI app it
    // would pop up (and keep open) a console window — the exact trap caught
    // on the installed Canvas app. CREATE_NO_WINDOW keeps the
    // engine a silent background child. No effect on POSIX.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| {
        format!(
            "engine unavailable: could not launch cutd at {} ({e})",
            program.display()
        )
    })?;

    // Ready-poll: cutd binds its listener before serving, so the first
    // successful /api/verbs answer means the full verb surface is up. Bounded
    // so a wedged child can never block app launch forever.
    let deadline = Instant::now() + ENGINE_READY_TIMEOUT;
    loop {
        if is_cutd(&addr) {
            match coherent_cutd_version(&addr) {
                Ok(engine_version) => {
                    eprintln!(
                        "[shellx-cut] engine v{engine_version} ready at {url} (ui: {ui_present})"
                    );
                    return Ok((Some(child), url, ui_present, engine_version));
                }
                Err(reason) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("engine unavailable: {reason}"));
                }
            }
        }
        // Child died (port race lost, bad args, AV kill…)? Report honestly.
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "engine unavailable: cutd exited during startup ({status}); see {}",
                log_path.display()
            ));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Err(format!(
                "engine unavailable: cutd did not answer on {addr} within {}s; see {}",
                ENGINE_READY_TIMEOUT.as_secs(),
                log_path.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Return the exact HTTP origin that may receive the desktop's narrow native
/// picker/event capability. Reject everything except a root URL on numeric
/// IPv4 loopback with an explicit non-zero port. `spawn_engine` constructs this
/// URL itself, but the validator keeps the capability boundary independently
/// fail-closed if that code changes later.
fn validated_engine_origin(url: &str) -> Result<String, String> {
    let parsed: tauri::Url = url
        .parse()
        .map_err(|e| format!("invalid engine URL '{url}': {e}"))?;
    let port = parsed
        .port()
        .filter(|port| *port != 0)
        .ok_or_else(|| format!("engine URL '{url}' has no usable port"))?;
    if parsed.scheme() != "http"
        || parsed.host_str() != Some("127.0.0.1")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(format!(
            "engine URL '{url}' is not an exact root HTTP loopback origin"
        ));
    }
    Ok(format!("http://127.0.0.1:{port}"))
}

/// Install the minimum native capability for the selected engine origin before
/// navigating the main webview. No filesystem, shell, process, updater or
/// global-shortcut commands are exposed to remote content.
fn grant_engine_origin_capability(app: &tauri::App, url: &str) -> Result<(), String> {
    let origin = validated_engine_origin(url)?;
    let capability = tauri::ipc::CapabilityBuilder::new("engine-remote-selected")
        .local(false)
        .remote(origin)
        .window("main")
        .permission("dialog:allow-open")
        .permission("dialog:allow-save")
        // Destructive confirmations and actionable errors use the dialog
        // module's supported `message` command. Do not rely on its injected
        // window.confirm/window.alert compatibility shim: dialog 2.7.x still
        // invokes removed ask/confirm command names there.
        .permission("dialog:allow-message")
        .permission("core:event:allow-listen")
        .permission("core:event:allow-unlisten")
        // Preview fullscreen fallback for WKWebView/WebKitGTK. The remote UI
        // receives only this window mutation plus the matching read-back; it
        // cannot move, resize, close, hide, or otherwise control the shell.
        .permission("core:window:allow-set-fullscreen")
        .permission("core:window:allow-is-fullscreen");
    let capability = capability
        .permission("allow-update-preferences")
        // Update-state bridge: read the snapshot, request a manual check, or
        // request an install (which still passes through the shell's native
        // confirm). No download/restart/filesystem power is granted directly —
        // the webview can only ask; update_state.rs decides.
        .permission("allow-update-state");
    // The focused native drop test must cross Tauri's real event path, but
    // engine-served production content must never be able to synthesize native
    // events. Every shipping build script rejects `webdriver-test`; only that
    // internal candidate binary receives the extra event-emission permission.
    #[cfg(feature = "webdriver-test")]
    let capability = capability
        .permission("core:event:allow-emit-to")
        // The internal WDIO plugin forwards browser console messages through
        // this command. Without its test-only permission, each forwarded log
        // becomes a page error and poisons the final console-clean gate.
        .permission("wdio:allow-log-frontend");
    app.add_capability(capability)
        .map_err(|e| format!("could not authorize native helpers for '{url}': {e}"))
}

/// Release-URL binding policy for the updater (desktop). The engine-served UI
/// is a REMOTE origin that Tauri denies plugin IPC — so the WHOLE update flow
/// lives in the shell (`update_state.rs`): quiet launch + 6-hourly checks feed
/// the topbar button / Settings > About, and install runs only on explicit
/// user request (native confirm → signature-verified download+install →
/// restart). The plugin verifies the minisign signature against the runtime
/// key registered by `updater_key_transition`, so a forged/unsigned artifact
/// is rejected; this comparator additionally requires every release URL to name
/// the manifest version, so a replayed old artifact behind a new version
/// number is rejected too. Linux packages skip all checks — see the linux-cfg
/// `run_automatic_checks` in update_state.rs.
#[cfg(desktop)]
fn updater_release_urls_match_version(current: &tauri_plugin_updater::RemoteRelease) -> bool {
    let expected_tag_segment = format!("/v{}/", current.version);
    match &current.data {
        tauri_plugin_updater::RemoteReleaseInner::Dynamic(platform) => {
            platform.url.path().contains(&expected_tag_segment)
        }
        tauri_plugin_updater::RemoteReleaseInner::Static { platforms } => {
            !platforms.is_empty()
                && platforms
                    .values()
                    .all(|platform| platform.url.path().contains(&expected_tag_segment))
        }
    }
}

/// The one CLI question the desktop shell answers by itself, before any GUI
/// toolkit, window or engine exists: `--version` (and its conventional short
/// form `-V`).
///
/// WHY THIS EXISTS
///   `argv` used to fall straight through into `tauri::Builder::run`, which
///   builds a `tao` event loop, which calls `gtk::init()` on Linux. Both
///   outcomes were wrong:
///     * headless (plain SSH shell, container, package post-install script):
///       `gtk::init()` fails, tao panics at `event_loop.rs:217` with
///       "Failed to initialize gtk backend!", and because `[profile.release]`
///       sets `panic = "abort"` the process dies with SIGABRT.
///     * with a display: no crash, but the FULL editor window opens and the
///       bundled cutd engine starts, and `--version` never returns — so
///       scripts that probe the installed version leak an app + an engine.
///
/// SCOPE — deliberately exact-match only. Anything else (notably a file path a
/// desktop file manager or `xdg-open` passes) falls through to the normal GUI
/// start, so this can never swallow a real launch. `--help` is intentionally
/// NOT claimed: this binary is a GUI shell, not a CLI; the agent/CLI surface is
/// `cutd`, which has its own argument parser.
///
/// WINDOWS NOTE: release builds link with `windows_subsystem = "windows"` and
/// have no attached console, so the printed line is only visible when the
/// process is started from a console that inherits stdout (e.g. a piped run).
/// The exit-without-launching behaviour is identical on every platform.
fn version_answer<I, S>(args: I, version: &str) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .any(|arg| matches!(arg.as_ref(), "--version" | "-V"))
        .then(|| format!("shellx-cut {version}"))
}

/// Desktop + mobile shared entry. Builds the Tauri app, registers the IPC
/// handlers, and — in the setup hook — starts (or adopts) the cutd engine and
/// points the main window at it. On failure the window keeps the embedded
/// fallback page, which polls `engine_status` and shows the honest reason.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();

    // Answer `--version` from the exact PackageInfo the About panel and the
    // updater report, then return — no GTK, no window, no cutd child. This has
    // to sit before the builder: `Builder::run` is what touches GTK.
    // `std::env::args()` skips argv[0] (the program path itself).
    if let Some(line) = version_answer(
        std::env::args().skip(1),
        &context.package_info().version.to_string(),
    ) {
        println!("{line}");
        return;
    }

    #[cfg(windows)]
    let mut context = context;
    #[cfg(windows)]
    let webview_test_data_directory = {
        let browser_args =
            webview2_debug_browser_args(std::env::var(ENV_WEBVIEW2_DEBUG_PORT).ok().as_deref())
                .unwrap_or_else(|reason| panic!("[shellx-cut] {reason}"));
        let data_token =
            webview2_data_token(std::env::var(ENV_WEBVIEW2_DATA_TOKEN).ok().as_deref())
                .unwrap_or_else(|reason| panic!("[shellx-cut] {reason}"));
        let data_directory = data_token.map(|token| {
            let local_app_data = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .expect("LOCALAPPDATA is required for an isolated WebView2 test profile");
            local_app_data.join("ShellX Cut WebView Tests").join(token)
        });
        if browser_args.is_some() || data_directory.is_some() {
            let main_window = context
                .config_mut()
                .app
                .windows
                .iter_mut()
                .find(|window| window.label == "main")
                .expect("ShellX Cut config must contain the main window");
            if let Some(args) = browser_args {
                main_window.additional_browser_args = Some(args);
            }
            if data_directory.is_some() {
                // Tauri 2.11 ignores WindowConfig::data_directory when it
                // converts config into WebviewAttributes. Defer this one
                // window to the setup hook and apply the builder method, which
                // reaches Wry's WebContext reliably.
                main_window.create = false;
            }
        }
        data_directory
    };

    // On X11 without a window manager, growing the display does not resize an
    // existing toplevel. Tests should size Xvfb before launch, resize the
    // window itself, or run a minimal window manager; the shell needs no
    // geometry workaround.
    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());

    // Optional WebDriver support for macOS WKWebView tests. Release packages
    // reject this feature.
    #[cfg(feature = "webdriver-test")]
    let builder = {
        // Keep an unambiguous marker in instrumented binaries. Linux tests use
        // an external driver whose own native port is open,
        // so a closed-port probe cannot distinguish the driver from an embedded
        // server. Shipping binaries must not contain this cfg-gated marker.
        eprintln!("{WEBDRIVER_TEST_BUILD_MARKER}");
        builder
            .plugin(tauri_plugin_wdio_webdriver::init())
            .plugin(tauri_plugin_wdio::init())
    };

    let builder = builder
        .manage(EngineProcess(Mutex::new(None)))
        .manage(EngineStatus(Mutex::new(EngineState::Unwired {
            reason: "engine unavailable: starting up".to_string(),
        })))
        .manage(ToolResolutionState(Mutex::new(ToolResolution::default())))
        // Update-state service: the snapshot the topbar button + Settings>About
        // read over the bridge. Seeded with the installed version; the setup
        // hook spawns the automatic (launch + 6-hourly) check driver.
        .manage(update_state::UpdateService::new(
            context.package_info().version.to_string(),
        ))
        .invoke_handler(tauri::generate_handler![
            engine_status,
            tools_doctor,
            update_settings::get_update_preferences,
            update_settings::set_update_preferences,
            update_state::get_update_state,
            update_state::update_check_now,
            update_state::update_install_now,
        ])
        .setup(move |app| {
            #[cfg(windows)]
            if let Some(data_directory) = &webview_test_data_directory {
                let main_config = app
                    .config()
                    .app
                    .windows
                    .iter()
                    .find(|window| window.label == "main")
                    .expect("ShellX Cut config must contain the main window");
                tauri::WebviewWindowBuilder::from_config(app.handle(), main_config)?
                    .data_directory(data_directory.clone())
                    .build()?;
            }

            // Bundled resources (ui-dist). In `tauri dev` this resolves into
            // the dev resource layout; in a packaged build it is the platform
            // resource dir populated from bundle.resources.
            let resource_dir = app
                .path()
                .resource_dir()
                .unwrap_or_else(|_| PathBuf::from("."));

            // resolve the heavy external tools FIRST — cutd.exe sits beside
            // the shell exe, and bundled ffmpeg/perception dirs sit beside that.
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("."));
            let tool_res = ToolResolution::detect_with_resources(&exe_dir, &resource_dir);
            let tools_hint = tool_res.bootstrap_hint();
            // `source` names the ladder rung that satisfied detection (env /
            // manual-override / bundled-or-appdata / system-dir / path /
            // missing) so a QA log line is auditable without guessing.
            eprintln!(
                "[shellx-cut] tools: ffmpeg_ok={} (source={} dir={:?}) sidecar_ok={} (dir={:?})",
                tool_res.ffmpeg_ok,
                tool_res.ffmpeg_source,
                tool_res.ffmpeg_dir,
                tool_res.sidecar_ok,
                tool_res.sidecar_dir
            );
            // also write the resolution to an app-data file. The real UI is
            // served by cutd (a remote origin), where Tauri 2 blocks custom-
            // command IPC, so this file — not the tools_doctor IPC — is the
            // runtime record of which tools the installed app resolved.
            // tools_doctor remains a local-origin diagnostic surface.
            if let Some(p) = tool_res.write_doctor_file() {
                eprintln!("[shellx-cut] tools-doctor written to {}", p.display());
            }
            *app.state::<ToolResolutionState>().0.lock().unwrap() = tool_res.clone();

            match spawn_engine(&resource_dir, &tool_res) {
                Ok((mut child, url, ui, engine_version)) => {
                    let mode = if child.is_some() { "spawned" } else { "external" };
                    let capability = if ui {
                        grant_engine_origin_capability(app, &url)
                    } else {
                        Ok(())
                    };
                    match capability {
                        Ok(()) => {
                            *app.state::<EngineProcess>().0.lock().unwrap() = child;
                            *app.state::<EngineStatus>().0.lock().unwrap() = EngineState::Wired {
                                url: url.clone(),
                                mode,
                                ui,
                                tools_hint,
                                engine_version,
                            };

                            // Navigate only after the selected origin has its
                            // exact capability. Without a UI bundle we stay on
                            // the fallback page, which reports the API-only
                            // wired state.
                            if ui {
                                if let Some(win) = app.get_webview_window("main") {
                                    if let Ok(parsed) = url.parse() {
                                        let _ = win.navigate(parsed);
                                    }
                                }
                            }
                        }
                        Err(reason) => {
                            if let Some(mut owned) = child.take() {
                                let _ = owned.kill();
                                let _ = owned.wait();
                            }
                            eprintln!("[shellx-cut] {reason} — staying on fallback page");
                            *app.state::<EngineStatus>().0.lock().unwrap() =
                                EngineState::Unwired { reason };
                        }
                    }
                }
                Err(reason) => {
                    eprintln!("[shellx-cut] {reason} — staying on fallback page");
                    *app.state::<EngineStatus>().0.lock().unwrap() =
                        EngineState::Unwired { reason };
                }
            }

            // Desktop auto-updater: register the plugin + start the QUIET
            // automatic check driver (launch check + a 6-hourly re-check while
            // the app stays open, both preference-gated — update_state.rs).
            // Lives shell-side so the engine-served remote UI never needs
            // update/restart IPC grants; results surface as the topbar update
            // button + Settings > About, never a startup dialog. Safe to run
            // regardless of engine state — it touches only the release feed.
            #[cfg(desktop)]
            {
                let handle = app.handle().clone();
                let _ = handle.plugin(
                    updater_key_transition::plugin_builder()
                        .default_version_comparator(|installed, release| {
                            release.version > installed
                                && updater_release_urls_match_version(&release)
                        })
                        .build(),
                );
                let check_handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    update_state::run_automatic_checks(check_handle).await;
                });
            }

            // Global record hotkey (desktop only). A screen recorder is, by
            // definition, NOT the focused window while it records — so an in-page
            // keydown listener can never reliably STOP a capture (the recorded app
            // owns focus). We register one OS-level key, F9, via the
            // global-shortcut plugin: its handler fires even
            // when another app is focused and EMITS `cut:record-hotkey` to the
            // webview, which toggles start⇄stop on the Record panel (the SAME
            // action as the Stop/Start button). Registration is Rust-side only, so
            // the remote engine-served origin gets no global-shortcut IPC grant —
            // it merely LISTENS for the event (core:event:allow-listen, already
            // granted). The in-page F9 keydown stays as a focused-window fallback
            // (and the sole path in the plain web/dev build, where this plugin and
            // its OS registration don't exist).
            #[cfg(desktop)]
            {
                use tauri_plugin_global_shortcut::{
                    Code, GlobalShortcutExt, Shortcut, ShortcutState,
                };

                // F9, no modifiers — a single global key.
                let record_hotkey = Shortcut::new(None, Code::F9);
                let plugin = tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(move |app, shortcut, event| {
                        // Fire once per PHYSICAL press (ignore the key-up event),
                        // otherwise a single tap would toggle twice and cancel out.
                        if shortcut == &record_hotkey && event.state() == ShortcutState::Pressed {
                            // Unit payload — the webview reads no data, it just
                            // toggles. A failed emit (no main window yet) is benign.
                            let _ = app.emit("cut:record-hotkey", ());
                        }
                    })
                    .build();
                // Register the plugin, then the key. Both are best-effort: if the
                // OS denies the global registration (another app already owns F9,
                // or a headless/CI run), the app still launches and the in-page F9
                // fallback keeps working — we log and move on, never panic.
                if let Err(e) = app.handle().plugin(plugin) {
                    eprintln!("[shellx-cut] global-shortcut plugin init failed: {e} — F9 falls back to in-app (focused-window) only");
                } else if let Err(e) = app.global_shortcut().register(record_hotkey) {
                    eprintln!("[shellx-cut] could not register global F9 record hotkey: {e} — F9 falls back to in-app (focused-window) only");
                }
            }
            Ok(())
        })
        // Kill the spawned engine when the window goes away so no orphan cutd
        // lingers. An adopted EXTERNAL server is never stored here, so it is
        // never killed — we don't own it.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                stop_owned_engine(window.app_handle());
            }
        });

    let app = builder
        .build(context)
        .expect("error while building shellx-cut desktop shell");
    app.run(|app_handle, event| {
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            stop_owned_engine(app_handle);
        }
    });
}

#[cfg(all(test, desktop))]
mod updater_release_policy_tests {
    use super::*;
    use std::collections::HashMap;

    fn release(version: &str, url: &str) -> tauri_plugin_updater::RemoteRelease {
        tauri_plugin_updater::RemoteRelease {
            version: version.parse().expect("test version"),
            notes: None,
            pub_date: None,
            data: tauri_plugin_updater::RemoteReleaseInner::Static {
                platforms: HashMap::from([(
                    "windows-x86_64".to_string(),
                    tauri_plugin_updater::ReleaseManifestPlatform {
                        url: url.parse().expect("test URL"),
                        signature: "fixture-signature".to_string(),
                    },
                )]),
            },
        }
    }

    #[test]
    fn updater_release_url_must_bind_the_manifest_version() {
        assert!(updater_release_urls_match_version(&release(
            "0.6.105",
            "https://github.com/martinsbrezauckis/shellx-cut/releases/download/v0.6.105/ShellX%20Cut_0.6.105_x64-setup.exe",
        )));
        assert!(!updater_release_urls_match_version(&release(
            "9.9.9",
            "https://github.com/martinsbrezauckis/shellx-cut/releases/download/v0.6.105/ShellX%20Cut_0.6.105_x64-setup.exe",
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_flag_is_answered_without_starting_the_app() {
        // Both conventional spellings answer, and the answer names the binary
        // plus the exact version string it was handed.
        assert_eq!(
            version_answer(["--version"], "0.6.106").as_deref(),
            Some("shellx-cut 0.6.106"),
        );
        assert_eq!(
            version_answer(["-V"], "0.6.106").as_deref(),
            Some("shellx-cut 0.6.106"),
        );
        // A flag anywhere in argv still answers — argv[0] is skipped by the
        // caller, so only real arguments reach here.
        assert_eq!(
            version_answer(["/tmp/a.cutproj", "--version"], "1.2.3").as_deref(),
            Some("shellx-cut 1.2.3"),
        );
    }

    #[test]
    fn version_flag_never_swallows_a_normal_gui_launch() {
        // No arguments (double-click / .desktop launch) and file-path launches
        // must fall through to the window, as must near-miss spellings.
        for args in [
            vec![],
            vec!["/home/user/Videos/clip.mp4"],
            vec!["--versions"],
            vec!["version"],
            vec!["-v"],
            vec!["--Version"],
        ] {
            assert_eq!(
                version_answer(args.clone(), "0.6.106"),
                None,
                "must not claim {args:?}",
            );
        }
    }

    #[test]
    fn shellx_ui_probe_distinguishes_html_from_headless_api() {
        let ui = "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\n\r\n<!doctype html><html><body><div id=\"root\"></div></body></html>";
        assert!(response_looks_like_shellx_ui(ui));

        let headless_404 = "HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\n\r\n{\"error\":\"not found\"}";
        assert!(!response_looks_like_shellx_ui(headless_404));

        let api_json = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"ok\":true}";
        assert!(!response_looks_like_shellx_ui(api_json));
    }

    #[test]
    fn webview2_debug_port_is_narrow_and_preserves_wry_defaults() {
        assert_eq!(webview2_debug_browser_args(None).unwrap(), None);
        let args = webview2_debug_browser_args(Some("9223"))
            .unwrap()
            .expect("valid opt-in should produce arguments");
        assert!(args.contains("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"));
        assert!(args.contains("--autoplay-policy=no-user-gesture-required"));
        assert!(args.ends_with("--remote-debugging-port=9223"));
    }

    #[test]
    fn webview2_debug_port_rejects_ambient_argument_injection() {
        for invalid in [
            "",
            "0",
            "65536",
            " 9223",
            "9223 ",
            "9223 --disable-web-security",
        ] {
            assert!(
                webview2_debug_browser_args(Some(invalid)).is_err(),
                "must reject {invalid:?}"
            );
        }
    }

    #[test]
    fn webview2_data_token_accepts_only_a_bounded_relative_name() {
        assert_eq!(webview2_data_token(None).unwrap(), None);
        assert_eq!(
            webview2_data_token(Some("ShellXCutFinalAction-20260730"))
                .unwrap()
                .unwrap(),
            "ShellXCutFinalAction-20260730"
        );
        for invalid in [
            "",
            ".",
            "..",
            "profile/subdir",
            r"C:\Temp\profile",
            "profile with spaces",
        ] {
            assert!(
                webview2_data_token(Some(invalid)).is_err(),
                "must reject {invalid:?}"
            );
        }
    }

    #[test]
    fn engine_capability_origin_tracks_the_exact_selected_port() {
        assert_eq!(
            validated_engine_origin("http://127.0.0.1:6161/").unwrap(),
            "http://127.0.0.1:6161"
        );
        assert_eq!(
            validated_engine_origin("http://127.0.0.1:49173/").unwrap(),
            "http://127.0.0.1:49173"
        );
    }

    #[test]
    fn engine_capability_origin_rejects_non_exact_loopback_urls() {
        for url in [
            "https://127.0.0.1:6161/",
            "http://localhost:6161/",
            "http://127.0.0.2:6161/",
            "http://127.0.0.1/",
            "http://127.0.0.1:6161/admin",
            "http://127.0.0.1:6161/?debug=1",
            "http://user@127.0.0.1:6161/",
        ] {
            assert!(validated_engine_origin(url).is_err(), "must reject {url}");
        }
    }

    #[test]
    fn occupied_non_cutd_default_selects_a_different_loopback_port() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let occupied = listener.local_addr().unwrap().to_string();
        let responder = listener.try_clone().unwrap();
        let reply = std::thread::spawn(move || {
            let (mut stream, _) = responder.accept().unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });

        let (selected, reuse) = pick_engine_addr_for(&occupied).unwrap();
        reply.join().unwrap();

        assert!(!reuse, "a non-cutd listener must never be adopted");
        assert_ne!(selected, occupied, "an occupied port requires fallback");
        assert!(
            selected.starts_with("127.0.0.1:"),
            "fallback must remain on numeric loopback"
        );
        assert!(
            TcpListener::bind(&selected).is_ok(),
            "the selected fallback port must be free for cutd"
        );
    }
}
