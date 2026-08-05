//! httpc.rs — minimal loopback HTTP/1.1 client (no extra dependency).
//!
//! Role: lets `cutd mcp` and `cutd verb` PROXY to a running `cutd serve` on
//! 127.0.0.1:6161 (the public single-state-holder contract: one state holder — the server owns the
//! project; other entrypoints pass verbs through instead of opening their
//! own copy). A full HTTP client crate is overkill for "POST JSON to
//! loopback": we send `Connection: close` and read to EOF, which axum
//! honors, so no chunked-decoding is needed beyond the standard case
//! (axum Json replies carry Content-Length; body = everything after the
//! blank line either way).
//!
//! Dependencies: std only. Primary callers: mcp.rs (proxy mode), main.rs
//! (`cutd verb` passthrough).

use cut_core::{error_codes, CutError};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// The default server address verbs are proxied to.
pub const SERVER_ADDR: &str = "127.0.0.1:6161";

/// Path of the engine-address discovery file. `cutd serve` writes its bound
/// addr here on start so `cutd mcp` / `cutd verb` reach it even on a FALLBACK
/// port (when :6161 was held by another process and the Tauri shell picked an
/// OS-assigned port). Deterministic per machine/user (the app-data root, NOT
/// TMPDIR) so a Tauri-spawned serve and a separately-launched mcp agree; falls
/// back to the OS temp dir only when no home/app-data var is set.
fn discovery_path() -> std::path::PathBuf {
    // Tests run in parallel in ONE process and all share this machine-global path, so a
    // concurrent test writing a LIVE addr could make another test's fallback check read
    // the wrong port (the observed flake). A thread-local override isolates each test to
    // its own file. Env vars are process-global → can't isolate parallel threads, so the
    // override is thread-local, not env-based.
    #[cfg(test)]
    if let Some(p) = TEST_DISCOVERY_PATH.with(|c| c.borrow().clone()) {
        return p;
    }
    let base = cut_media::toolpath::appdata_tools_dir()
        .and_then(|p| p.parent().map(|x| x.to_path_buf()))
        .unwrap_or_else(std::env::temp_dir);
    base.join("engine.addr")
}

#[cfg(test)]
thread_local! {
    /// Per-test override of [`discovery_path`] (see its comment) — `None` in production.
    static TEST_DISCOVERY_PATH: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Record the bound address for proxy discovery (best-effort: a failed write
/// just means proxies fall back to probing the default port).
pub fn write_discovery(addr: &str) {
    let path = discovery_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, addr);
}

/// Remove the discovery file (best-effort, on graceful shutdown).
pub fn clear_discovery() {
    let _ = std::fs::remove_file(discovery_path());
}

fn read_discovery() -> Option<String> {
    std::fs::read_to_string(discovery_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A reachable loopback addr to proxy verbs to: the discovery addr if a server
/// is actually listening there (fallback-port case), else the default :6161.
/// The liveness probe makes a STALE discovery file (crashed serve) harmless.
pub fn server_addr() -> String {
    // CUTD_PROXY_ADDR override (agent.chat sets it on the spawned CLI's env so a
    // `cutd mcp` it launches proxies to the EXACT serve that started the turn,
    // not whichever serve last wrote the shared discovery file — correct even
    // with multiple cutd instances running). Honoured only if reachable.
    if let Ok(addr) = std::env::var("CUTD_PROXY_ADDR") {
        if !addr.is_empty() && tcp_ok(&addr) {
            return addr;
        }
    }
    if let Some(addr) = read_discovery() {
        if tcp_ok(&addr) {
            return addr;
        }
    }
    SERVER_ADDR.to_string()
}

fn tcp_ok(addr: &str) -> bool {
    addr.parse::<std::net::SocketAddr>()
        .ok()
        .is_some_and(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).is_ok())
}

/// True when a cutd server is listening (discovery addr or the default).
pub fn server_running() -> bool {
    tcp_ok(&server_addr())
}

/// POST /api/verb/{name} with a JSON body; returns the parsed envelope.
/// Blocking — callers are the (already blocking) MCP stdio loop and the CLI.
pub fn post_verb(name: &str, args: &serde_json::Value) -> Result<serde_json::Value, CutError> {
    let body = args.to_string();
    let addr = server_addr();
    // Agent Chat assigns every spawned MCP proxy a unique, informational actor
    // identity. Keep it out of the request unless it is a bounded header-safe
    // value; malformed inherited environment must never enable header injection.
    let actor_header = std::env::var("CUTD_PROXY_ACTOR")
        .ok()
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 160
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.' | '/'))
        })
        .map(|value| format!("X-Cut-Actor: {value}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "POST /api/verb/{name} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n{actor_header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let raw = roundtrip(&addr, &req)?;
    parse_body(&raw)
}

/// One blocking request/response cycle against the local server at `addr`.
fn roundtrip(addr: &str, req: &str) -> Result<Vec<u8>, CutError> {
    let sock: std::net::SocketAddr = addr.parse().map_err(|e: std::net::AddrParseError| {
        CutError::new(
            error_codes::IO,
            format!("invalid engine address '{addr}'"),
            e.to_string(),
        )
    })?;
    let mut stream =
        TcpStream::connect_timeout(&sock, Duration::from_millis(1000)).map_err(|e| {
            CutError::new(
                error_codes::IO,
                format!("no cutd server at {addr}"),
                e.to_string(),
            )
            .with_suggested_action("start `cutd serve` first, or run with --standalone")
        })?;
    // Generous timeouts: verbs can do real work before answering.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    stream.write_all(req.as_bytes())?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf)?; // Connection: close → EOF terminates
    Ok(buf)
}

/// Split headers/body and parse the body as JSON (handles the chunked case
/// defensively by stripping chunk-size lines when detected).
fn parse_body(raw: &[u8]) -> Result<serde_json::Value, CutError> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text.split_once("\r\n\r\n").ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "malformed HTTP response",
            "no header/body separator",
        )
    })?;
    let body = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        // Concatenate chunk payloads (sizes are hex lines between payloads).
        let mut out = String::new();
        let mut rest = body;
        while let Some((size_line, after)) = rest.split_once("\r\n") {
            let Ok(size) = usize::from_str_radix(size_line.trim(), 16) else {
                break;
            };
            if size == 0 {
                break;
            }
            out.push_str(&after[..size.min(after.len())]);
            rest = after.get(size + 2..).unwrap_or("");
        }
        out
    } else {
        body.to_string()
    };
    serde_json::from_str(body.trim()).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "server response was not JSON",
            e.to_string(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A live discovery address is used by the proxy; a stale one (server gone)
    /// falls back to the default port. Saves/restores any real discovery file so
    /// the test never disturbs a cutd running on this machine.
    #[test]
    fn discovery_roundtrip_and_liveness_fallback() {
        // Isolate the discovery file to THIS thread so a parallel test can't race us
        // (was flaky in the full suite, passed in isolation). Unique per thread id.
        let tmp = std::env::temp_dir().join(format!(
            "engine.addr.test.{:?}",
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&tmp);
        TEST_DISCOVERY_PATH.with(|c| *c.borrow_mut() = Some(tmp.clone()));

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        write_discovery(&addr);
        assert_eq!(read_discovery().as_deref(), Some(addr.as_str()));
        assert_eq!(
            server_addr(),
            addr,
            "a live discovery addr is used by the proxy"
        );

        drop(listener); // free the live port

        // Stale: an address NOTHING listens on must fall back to the default. Don't reuse
        // the just-freed ephemeral port — a parallel `:0` test can be assigned it, and then
        // the liveness probe would see it as "live" (THE flake). A fixed LOW port (127.0.0.1:1)
        // is outside the ephemeral range and effectively never listening, so the probe is
        // deterministically refused.
        write_discovery("127.0.0.1:1");
        assert_eq!(
            server_addr(),
            SERVER_ADDR,
            "stale discovery falls back to the default port"
        );

        // Isolated file → just remove it and clear the override (no real file touched).
        let _ = std::fs::remove_file(&tmp);
        TEST_DISCOVERY_PATH.with(|c| *c.borrow_mut() = None);
    }
}
