//! main.rs — cutd entrypoint (server contract).
//! Role: CLI surface of the server binary:
//!   cutd serve [--project <path>] [--headless] [--addr 127.0.0.1:6161]
//!       → axum REST+WS+static-UI; background-run friendly.
//!   cutd mcp [--project <path>]
//!       → MCP over stdio (newline JSON-RPC), tools from schema/verbs.json.
//!   cutd verb <name> ['<json-args>'] [--project <path>]
//!       → one-shot dispatch, prints the envelope — the CLI escape hatch for
//!         testing and shell scripts (e2e.sh drives this surface).
mod chat;
mod diarize;
mod dispatch;
mod doctor;
mod dub;
mod events;
mod faces;
mod fetch;
mod ffmpeg_settings;
mod framecache;
mod gen;
mod generate;
mod generate_handlers;
#[cfg(test)]
mod generate_rich_tests;
mod http;
mod httpc;
mod jobs;
mod library;
mod matte;
mod mcp;
mod motion_artifact;
mod motion_bridge;
mod motion_edit_return;
mod motion_editable_import;
mod motion_jobs;
mod motion_package;
mod motion_runtime;
mod motion_template_catalog;
#[cfg(test)]
mod motion_test_fixtures;
mod motion_tracking;
mod nest;
mod ocr;
mod output_paths;
mod paste_attributes;
mod perception_setup;
mod plugins;
mod projects_index;
mod providers;
mod recipes;
mod registry;
mod review_http;
mod schema_validation;
mod screen_record;
mod screen_record_studio;
mod state;
mod stt_settings;
mod track;
mod translate;
mod ui_bridge;
mod userdata;
mod vissearch;
use clap::{Parser, Subcommand};
use cut_core::{Actor, ActorKind, ProjectStore};
use state::AppState;
use std::path::PathBuf;
/// cutd — agent-first server; every surface routes through the same verb dispatcher.
#[derive(Parser)]
#[command(name = "cutd", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Run the REST+WS+UI server (background-run friendly).
    Serve {
        /// Open this .cutproj directory at startup.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Don't serve the UI bundle (API only). The API works headless
        /// regardless; this just skips the static-file mount.
        #[arg(long)]
        headless: bool,
        /// Bind address (loopback only by policy).
        #[arg(long, default_value = http::DEFAULT_ADDR)]
        addr: String,
        /// Path to the built UI bundle (default: ui/dist next to the repo,
        /// resolved relative to the executable's workspace in dev).
        #[arg(long)]
        ui_dist: Option<PathBuf>,
    },
    /// Run as an MCP server over stdio (newline-delimited JSON-RPC 2.0).
    /// Default: PROXIES every verb to the running `cutd serve` on
    /// 127.0.0.1:6161 (the single-state-holder contract — one state holder).
    Mcp {
        /// Open this .cutproj directory before serving tools (standalone
        /// mode only — in proxy mode the SERVER owns the project).
        #[arg(long)]
        project: Option<PathBuf>,
        /// Dispatch verbs in-process instead of proxying. Only legal when no
        /// server is running (refused otherwise to avoid split state).
        #[arg(long)]
        standalone: bool,
    },
    /// Dispatch one verb and print the result envelope (testing escape hatch).
    Verb {
        /// Verb name from schema/verbs.json, e.g. project.state
        name: String,
        /// Args as a JSON object string; defaults to {}.
        args: Option<String>,
        /// Open this .cutproj directory first.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Dispatch in-process for isolated test harnesses instead of proxying
        /// to a running desktop server.
        #[arg(long)]
        standalone: bool,
    },
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Packaged Python entrypoints live inside the signed macOS app bundle. Importing a
    // sibling module must never create __pycache__ there and invalidate the code seal.
    // Set this before any worker or child process starts so every Python sidecar and
    // adapter inherits the same read-only-package contract on all platforms.
    std::env::set_var("PYTHONDONTWRITEBYTECODE", "1");
    // Log to stderr so MCP stdout stays protocol-clean.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    // On macOS the Tauri shell PIPES cutd's stderr, so a panicking verb handler just resets
    // the HTTP connection with no visible trace (a verb returns "http 000"). Tee panics to a
    // file (in addition to the default stderr hook) so engine panics are diagnosable on every
    // platform — `$HOME/.cutd-panic.log` (falls back to the temp dir).
    {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let path = std::env::var_os("HOME")
                .map(|h| std::path::PathBuf::from(h).join(".cutd-panic.log"))
                .unwrap_or_else(|| std::env::temp_dir().join("cutd-panic.log"));
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                use std::io::Write;
                let _ = writeln!(f, "[cutd panic] {info}");
            }
            prev(info);
        }));
    }
    let cli = Cli::parse();
    match cli.command {
        Command::Serve {
            project,
            headless,
            addr,
            ui_dist,
        } => {
            let state = AppState::new();
            open_if_given(&state, project).await?;
            let dist = if headless {
                None
            } else {
                Some(resolve_ui_dist(ui_dist))
            };
            match &dist {
                Some(d) if d.join("index.html").exists() => {
                    tracing::info!("serving UI from {}", d.display())
                }
                Some(d) => tracing::warn!(
                    "UI bundle missing at {} — run scripts/dev.sh to build; API still up",
                    d.display()
                ),
                None => tracing::info!("headless mode — API only"),
            }
            // Stamp the bind address onto doctor reports, then run the
            // environment scan ONCE at startup.
            // Off the bind path so a slow probe never delays listening; the
            // first `system.doctor` call returns this cached result. A capability
            // change vs the (empty) prior cache publishes `doctor_updated`.
            state.set_addr(&addr).await;
            let startup_doctor = {
                let st = state.clone();
                tokio::spawn(async move {
                    let r = st.doctor_rescan().await;
                    tracing::info!(
                        "doctor: {} cards, ffmpeg {}",
                        r.cards.len(),
                        if r.essential_ok {
                            "ok"
                        } else {
                            "MISSING (wizard will surface)"
                        }
                    );
                })
            };
            // Refuse a non-loopback bind by default (server trust boundary).
            // BEFORE opening the socket — the header guard alone is not a network
            // boundary for a non-browser client on an exposed bind.
            if let Err(reason) = http::check_bind_addr(&addr) {
                anyhow::bail!(reason);
            }
            let router = http::build_router(state, dist);
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            // Publish the actual bound address (resolves :0 or a fallback port).
            // so `cutd mcp` / `cutd verb` proxies reach this engine even when it
            // is not on the default :6161. Cleared on graceful shutdown; a stale
            // file after a crash is made harmless by the liveness probe in
            // httpc::server_addr.
            let bound = listener
                .local_addr()
                .map(|a| a.to_string())
                .unwrap_or_else(|_| addr.clone());
            httpc::write_discovery(&bound);
            tracing::info!("cutd listening on http://{bound}/ (POST /api/verb/{{name}})");
            let result = axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await;
            startup_doctor.abort();
            let _ = startup_doctor.await;
            httpc::clear_discovery();
            result?;
            Ok(())
        }
        Command::Mcp {
            project,
            standalone,
        } => {
            // the single-state-holder contract: one state holder. Proxy when the server runs;
            // standalone (own project) only with the explicit flag AND no
            // running server.
            let server_up = httpc::server_running();
            let mode = match (server_up, standalone) {
                (true, true) => {
                    anyhow::bail!(
                        "--standalone refused: a cutd server is already running on {} — \
                         connect through it (drop --standalone)",
                        httpc::SERVER_ADDR
                    );
                }
                (true, false) => mcp::McpMode::Proxy,
                (false, true) => mcp::McpMode::Standalone,
                (false, false) => {
                    // No server yet: still proxy-mode — each tools/call
                    // re-probes, returning an actionable envelope until
                    // `cutd serve` starts. (Agents can start it and retry.)
                    tracing::warn!(
                        "no cutd server on {} — verbs will error until it starts (or rerun with --standalone)",
                        httpc::SERVER_ADDR
                    );
                    mcp::McpMode::Proxy
                }
            };
            let state = AppState::new();
            if mode == mcp::McpMode::Standalone {
                open_if_given(&state, project).await?;
            } else if project.is_some() {
                tracing::warn!("--project ignored in proxy mode — the server owns the project");
            }
            mcp::run_stdio(state, mode).await
        }
        Command::Verb {
            name,
            args,
            project,
            standalone,
        } => {
            let args: serde_json::Value = match args {
                Some(s) => serde_json::from_str(&s)
                    .map_err(|e| anyhow::anyhow!("args is not valid JSON: {e}"))?,
                None => serde_json::json!({}),
            };
            // Same one-state-holder rule as MCP by default: when a server is
            // running, the CLI passes the verb through it. `--standalone` is
            // the explicit testing escape hatch for isolated harnesses that
            // need their own project/env while a desktop server is open.
            let result_json = if !standalone && httpc::server_running() {
                if project.is_some() {
                    tracing::warn!("--project ignored — verb proxied to the running server");
                }
                httpc::post_verb(&name, &args)
                    .unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e}))
            } else {
                let state = AppState::new();
                open_if_given(&state, project).await?;
                let actor = Actor {
                    kind: ActorKind::Agent,
                    name: "cli".into(),
                    via: "cli".into(),
                };
                serde_json::to_value(dispatch::dispatch(&state, &name, args, actor).await)?
            };
            println!("{}", serde_json::to_string_pretty(&result_json)?);
            // Nonzero exit on verb failure so shell scripts can `set -e`.
            if !result_json
                .get("ok")
                .and_then(|o| o.as_bool())
                .unwrap_or(false)
            {
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

/// Open a project into state when --project was passed (shared by all
/// subcommands). Side effect: attaches the job manager to the project dir.
async fn open_if_given(state: &AppState, project: Option<PathBuf>) -> anyhow::Result<()> {
    if let Some(path) = project {
        let store = ProjectStore::open(&path).map_err(|e| anyhow::anyhow!("{e}"))?;
        state
            .jobs
            .attach_project(&store.dir)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        *state.project.write().await = Some(store);
    }
    Ok(())
}

/// Locate ui/dist: explicit flag wins; else <repo>/ui/dist relative to the
/// workspace (CARGO_MANIFEST_DIR = app/server at compile time — dev layout).
fn resolve_ui_dist(explicit: Option<PathBuf>) -> PathBuf {
    explicit.unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/dist")
            .components()
            .collect() // normalize the ../.. without touching the filesystem
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn verb_cli_accepts_explicit_standalone_testing_mode() {
        let cli = Cli::parse_from([
            "cutd",
            "verb",
            "--standalone",
            "--project",
            "/tmp/sample.cutproj",
            "project.state",
            "{}",
        ]);

        let Command::Verb {
            standalone,
            project,
            name,
            args,
        } = cli.command
        else {
            panic!("expected verb command");
        };

        assert!(standalone);
        assert_eq!(project, Some(PathBuf::from("/tmp/sample.cutproj")));
        assert_eq!(name, "project.state");
        assert_eq!(args.as_deref(), Some("{}"));
    }
}
