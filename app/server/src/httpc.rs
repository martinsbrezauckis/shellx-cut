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
/// OS-assigned port). An explicit `SHELLX_CUT_HOME` keeps Runner launches in
/// their attempt state; ordinary launches use the app-data root so a separate
/// MCP process can find the same server. The OS temp dir is the last fallback.
#[cfg(test)]
fn discovery_path() -> Option<std::path::PathBuf> {
    // Tests run in parallel in ONE process, so the override is thread-local (env
    // vars are process-global). A unit test never resolves the person's real
    // engine.addr: without its own override it gets a path in a per-process temp
    // folder that nobody writes (so it reads "no engine recorded"). A guard test
    // pins this.
    Some(
        TEST_DISCOVERY_PATH
            .with(|c| c.borrow().clone())
            .unwrap_or_else(|| {
                std::env::temp_dir()
                    .join(format!("cutd-unit-test-state-{}", std::process::id()))
                    .join("engine.addr")
            }),
    )
}

#[cfg(not(test))]
fn discovery_path() -> Option<std::path::PathBuf> {
    discovery_path_for(
        std::env::var_os("SHELLX_CUT_HOME").as_deref(),
        cut_media::toolpath::appdata_tools_dir().and_then(|p| p.parent().map(|x| x.to_path_buf())),
        &std::env::temp_dir(),
    )
}

fn discovery_path_for(
    cut_home: Option<&std::ffi::OsStr>,
    appdata_root: Option<std::path::PathBuf>,
    temp_root: &std::path::Path,
) -> Option<std::path::PathBuf> {
    if let Some(home) = cut_home.filter(|home| !home.is_empty()) {
        let root = std::path::PathBuf::from(home);
        if !root.is_absolute()
            || !root.is_dir()
            || root
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return None;
        }
        return Some(root.join("engine.addr"));
    }
    Some(
        appdata_root
            .unwrap_or_else(|| temp_root.to_path_buf())
            .join("engine.addr"),
    )
}

#[cfg(test)]
thread_local! {
    /// Per-test override of [`discovery_path`] (see its comment) — `None` in production.
    static TEST_DISCOVERY_PATH: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
    /// Per-test stand-in for `CUTD_PROXY_ADDR` (the real environment variable is
    /// never read in a unit test).
    static TEST_PROXY_ENV: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
    /// Per-test stand-in for the default port. Unset, it is an address nothing
    /// listens on, so a unit test cannot reach an engine running on :6161.
    static TEST_DEFAULT_ADDR: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
    /// Per-test stand-in for an explicit `SHELLX_CUT_HOME`.
    static TEST_ISOLATED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The product name an engine reports on `/api/agent` and a client requires
/// before it sends a verb.
pub const PRODUCT_NAME: &str = "ShellX Cut";

/// Prefix of the second line of `engine.addr`.
const INSTANCE_KEY: &str = "instance=";

/// A random value fixed for the life of this process. The engine writes it into
/// `engine.addr` at start and returns it from `/api/agent`, so a client can tell
/// this engine from another program, or from an older engine that reused the
/// port, even when the product name and version match.
pub fn instance_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        let mut bytes = [0u8; 16];
        if getrandom::fill(&mut bytes).is_err() {
            // No system randomness: fall back to values that still differ per
            // process start (time and pid) rather than an empty identity.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            bytes[..8].copy_from_slice(&(now as u64).to_le_bytes());
            bytes[8..12].copy_from_slice(&std::process::id().to_le_bytes());
        }
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    })
}

/// What `engine.addr` holds: the bound address on the first line, then
/// `instance=<id>`. A file written before the instance line existed holds the
/// address alone.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscoveryRecord {
    addr: String,
    instance: Option<String>,
}

fn parse_record(text: &str) -> Option<DiscoveryRecord> {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let addr = lines.next()?.to_string();
    let instance = lines
        .find_map(|line| line.strip_prefix(INSTANCE_KEY))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Some(DiscoveryRecord { addr, instance })
}

/// The address the caller NAMED through `CUTD_PROXY_ADDR`, if any.
#[cfg(test)]
fn proxy_env() -> Option<String> {
    TEST_PROXY_ENV.with(|c| c.borrow().clone())
}

#[cfg(not(test))]
fn proxy_env() -> Option<String> {
    std::env::var("CUTD_PROXY_ADDR")
        .ok()
        .filter(|addr| !addr.is_empty())
}

/// Whether an explicit `SHELLX_CUT_HOME` selects an isolated state home.
#[cfg(test)]
fn isolated_state_home() -> bool {
    TEST_ISOLATED.with(|c| c.get())
}

#[cfg(not(test))]
fn isolated_state_home() -> bool {
    std::env::var_os("SHELLX_CUT_HOME").is_some_and(|home| !home.is_empty())
}

/// The documented default engine address.
#[cfg(test)]
fn default_addr() -> String {
    TEST_DEFAULT_ADDR
        .with(|c| c.borrow().clone())
        .unwrap_or_else(|| "127.0.0.1:1".to_string())
}

#[cfg(not(test))]
fn default_addr() -> String {
    SERVER_ADDR.to_string()
}

/// Record the bound address and this engine's instance for proxy discovery
/// (best-effort: a failed write just means proxies fall back to probing the
/// default port).
pub fn write_discovery(addr: &str) {
    write_discovery_record(addr, instance_id());
}

fn write_discovery_record(addr: &str, instance: &str) {
    let Some(path) = discovery_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, format!("{addr}\n{INSTANCE_KEY}{instance}\n"));
}

/// Remove the discovery file on graceful shutdown (best-effort), unless a
/// newer engine has already rewritten it with its own instance.
pub fn clear_discovery() {
    let Some(path) = discovery_path() else { return };
    let ours = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| parse_record(&text))
        .is_none_or(|record| {
            record
                .instance
                .as_deref()
                .is_none_or(|i| i == instance_id())
        });
    if ours {
        let _ = std::fs::remove_file(path);
    }
}

fn read_record() -> Option<DiscoveryRecord> {
    parse_record(&std::fs::read_to_string(discovery_path()?).ok()?)
}

fn read_discovery() -> Option<String> {
    read_record().map(|record| record.addr)
}

/// A program answered where the engine should be, but it is not this product's
/// engine (or not this engine instance). Nothing was sent to it beyond the
/// identity request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerMismatch {
    pub addr: String,
    /// Plain-words reason, for the person or the agent.
    pub reason: String,
    source: AddrSource,
    /// For a NAMED address that failed: the verified engine that does run, if one does.
    alternative: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddrSource {
    DiscoveryFile,
    ProxyEnv,
    DefaultPort,
}

impl PeerMismatch {
    /// The refusal, with the action that is right for where the address came from.
    pub fn to_error(&self) -> CutError {
        let action = match self.source {
            AddrSource::DiscoveryFile => {
                let file = discovery_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "engine.addr".into());
                format!(
                    "engine.addr names an address that another program now uses, so it is a \
                     leftover. Open ShellX Cut (or start `cutd serve`): the engine rewrites \
                     engine.addr when it starts. If you are not going to run ShellX Cut, delete \
                     the stale file {file}"
                )
            }
            AddrSource::ProxyEnv => match &self.alternative {
                Some(other) => format!(
                    "CUTD_PROXY_ADDR names {}, but a ShellX Cut engine is running at {other}. \
                     To use that engine, unset CUTD_PROXY_ADDR (or set it to {other}); the verb \
                     was not sent to either",
                    self.addr
                ),
                None => "CUTD_PROXY_ADDR names an address that is not this ShellX Cut engine. \
                     Unset it to let the proxy find the running engine, or set it to the \
                     engine's address; open ShellX Cut if it is not running"
                    .to_string(),
            },
            AddrSource::DefaultPort => format!(
                "another program is using {}. Open ShellX Cut: its engine then takes a \
                 free port and writes engine.addr",
                self.addr
            ),
        };
        CutError::new(
            error_codes::IO,
            format!(
                "the program at {} is not this ShellX Cut engine; nothing was sent to it",
                self.addr
            ),
            self.reason.clone(),
        )
        .with_suggested_action(action)
    }
}

/// Where the engine is, after asking every candidate who it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Located {
    /// This product's engine, at this address, verified.
    Engine(String),
    /// Nothing listens at any candidate address.
    NoServer,
    /// Something listens, but it is not this engine.
    Mismatch(PeerMismatch),
}

/// Ask `addr` who it is (GET /api/agent) and accept it only when it names this
/// product, this version, and (when `expected_instance` is known) the instance
/// the engine recorded in `engine.addr`. Sends nothing else. `Err` is the reason.
fn verify_peer(addr: &str, expected_instance: Option<&str>) -> Result<(), String> {
    let raw = http_get(addr, "/api/agent")
        .map_err(|e| format!("it did not answer the identity request (GET /api/agent): {e}"))?;
    check_identity(&raw, expected_instance)
}

/// Judge an answer to the identity request. The instance rule is exact: when
/// the caller knows the instance (engine.addr names one) the peer must report
/// that value; a peer that reports none does not pass.
fn check_identity(raw: &[u8], expected_instance: Option<&str>) -> Result<(), String> {
    let head_status = raw
        .split(|b| *b == b'\r')
        .next()
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .unwrap_or_default();
    if head_status.split_whitespace().nth(1) != Some("200") {
        return Err(format!(
            "it answered the identity request with '{}', not as ShellX Cut",
            head_status.trim()
        ));
    }
    let json = parse_body(raw)
        .map_err(|_| "its answer to the identity request is not ShellX Cut's".to_string())?;
    if json.get("product").and_then(|v| v.as_str()) != Some(PRODUCT_NAME) {
        return Err(format!(
            "it does not identify itself as {PRODUCT_NAME} (product {:?})",
            json.get("product")
        ));
    }
    let version = json.get("version").and_then(|v| v.as_str()).unwrap_or("");
    if version != env!("CARGO_PKG_VERSION") {
        return Err(format!(
            "it is {PRODUCT_NAME} v{version}, but this program is v{}; use the cutd that \
             belongs to the running app, or update one of them",
            env!("CARGO_PKG_VERSION")
        ));
    }
    if let Some(expected) = expected_instance {
        let seen = json.get("instance").and_then(|v| v.as_str());
        if seen != Some(expected) {
            return Err(
                "it is a ShellX Cut engine, but not the one engine.addr was written by (an \
                 earlier or another engine instance holds the address now)"
                    .to_string(),
            );
        }
    }
    Ok(())
}

/// Blocking GET with short timeouts; returns the raw response.
fn http_get(addr: &str, path: &str) -> Result<Vec<u8>, String> {
    let sock: std::net::SocketAddr = addr
        .parse()
        .map_err(|e: std::net::AddrParseError| e.to_string())?;
    let mut stream = TcpStream::connect_timeout(&sock, Duration::from_millis(1000))
        .map_err(|e| e.to_string())?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(2000)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(2000)));
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    // Bounded: an identity answer is small; never read an unbounded stream.
    stream
        .take(1 << 20)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Find this product's engine. Candidates, in order: `CUTD_PROXY_ADDR` (set by
/// Agent Chat for the proxy it spawns), the address in `engine.addr`, then the
/// default port. A candidate that accepts a connection must also answer the
/// identity request as this product and version (and, for `engine.addr`, as the
/// recorded instance) before it is used; otherwise nothing is sent to it. A stale
/// file whose address nothing answers is skipped, as before.
pub fn locate() -> Located {
    let mut mismatch: Option<PeerMismatch> = None;
    for (addr, instance, source) in plan() {
        if !tcp_ok(&addr) {
            if source == AddrSource::ProxyEnv {
                mismatch = Some(named_failure(
                    addr,
                    "nothing is listening there".to_string(),
                ));
            }
            continue;
        }
        match verify_peer(&addr, instance.as_deref()) {
            Ok(()) => return Located::Engine(addr),
            Err(reason) => {
                if source == AddrSource::ProxyEnv {
                    mismatch = Some(named_failure(addr, reason));
                } else if mismatch.is_none() {
                    mismatch = Some(PeerMismatch {
                        addr,
                        reason,
                        source,
                        alternative: None,
                    });
                }
            }
        }
    }
    match mismatch {
        Some(mismatch) => Located::Mismatch(mismatch),
        None => Located::NoServer,
    }
}

/// Where to look, in order. An address the caller NAMED (`CUTD_PROXY_ADDR`) is
/// the only candidate: if it is not this engine the caller is told, and the
/// verb is never sent to another engine behind their back. With no address
/// named: the `engine.addr` record, then the default port.
fn plan() -> Vec<(String, Option<String>, AddrSource)> {
    if let Some(addr) = proxy_env() {
        return vec![(addr, None, AddrSource::ProxyEnv)];
    }
    let mut out = Vec::new();
    if let Some(record) = read_record() {
        out.push((record.addr, record.instance, AddrSource::DiscoveryFile));
    }
    // An explicit SHELLX_CUT_HOME is an isolated state home (a Runner attempt, a
    // test, a portable install): its engine is the one its engine.addr names. The
    // default port belongs to the person's own installed engine, so it is not a
    // fallback there; otherwise an isolated client could send verbs into a live
    // session that is not part of its state.
    if !isolated_state_home() {
        out.push((default_addr(), None, AddrSource::DefaultPort));
    }
    out
}

/// A named address failed: say so, and name the engine that does run, if any.
fn named_failure(addr: String, reason: String) -> PeerMismatch {
    let alternative = [
        read_record().map(|r| (r.addr, r.instance)),
        (!isolated_state_home()).then(|| (default_addr(), None)),
    ]
    .into_iter()
    .flatten()
    .find(|(other, instance)| {
        *other != addr && tcp_ok(other) && verify_peer(other, instance.as_deref()).is_ok()
    })
    .map(|(other, _)| other);
    PeerMismatch {
        addr,
        reason,
        source: AddrSource::ProxyEnv,
        alternative,
    }
}

/// The verified engine address, else the default :6161 (for callers that only
/// need a value to hand on; whoever uses it verifies again).
pub fn server_addr() -> String {
    match locate() {
        Located::Engine(addr) => addr,
        _ => SERVER_ADDR.to_string(),
    }
}

fn tcp_ok(addr: &str) -> bool {
    addr.parse::<std::net::SocketAddr>()
        .ok()
        .is_some_and(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).is_ok())
}

/// POST /api/verb/{name} with a JSON body; returns the parsed envelope.
/// Blocking — callers are the (already blocking) MCP stdio loop and the CLI.
pub fn post_verb(name: &str, args: &serde_json::Value) -> Result<serde_json::Value, CutError> {
    let body = args.to_string();
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
    let build = |addr: &str| {
        format!(
            "POST /api/verb/{name} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n{actor_header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    // Who is listening is verified on the SAME connection the verb then travels
    // on, so no window exists between the check and the send, and the verb (project
    // paths and content) is only ever written to a stream whose peer just
    // identified itself as this engine. A stale engine.addr plus another local
    // program on that port never receives it.
    let mut mismatch: Option<PeerMismatch> = None;
    for (addr, instance, source) in plan() {
        if !tcp_ok(&addr) {
            if source == AddrSource::ProxyEnv {
                return Err(
                    named_failure(addr, "nothing is listening there".to_string()).to_error(),
                );
            }
            continue;
        }
        match verified_roundtrip(&addr, instance.as_deref(), &build(&addr)) {
            Ok(raw) => return parse_body(&raw),
            Err(Verified::Mismatch(reason)) => {
                if source == AddrSource::ProxyEnv {
                    return Err(named_failure(addr, reason).to_error());
                }
                if mismatch.is_none() {
                    mismatch = Some(PeerMismatch {
                        addr,
                        reason,
                        source,
                        alternative: None,
                    });
                }
            }
            Err(Verified::Io(error)) => return Err(error),
        }
    }
    match mismatch {
        Some(mismatch) => Err(mismatch.to_error()),
        None => Err(no_server_error(
            SERVER_ADDR,
            "no engine answers at any known address",
            stale_discovery_note().as_deref(),
        )),
    }
}

enum Verified {
    /// The peer did not identify itself as this engine; nothing else was sent.
    Mismatch(String),
    /// The peer is this engine but the exchange failed.
    Io(CutError),
}

/// Connect, ask `GET /api/agent` on a keep-alive connection, verify the answer,
/// and only then write `req` on that same connection and read its reply.
fn verified_roundtrip(
    addr: &str,
    expected_instance: Option<&str>,
    req: &str,
) -> Result<Vec<u8>, Verified> {
    let mismatch = |reason: String| Verified::Mismatch(reason);
    let sock: std::net::SocketAddr = addr
        .parse()
        .map_err(|e: std::net::AddrParseError| mismatch(e.to_string()))?;
    let mut stream = TcpStream::connect_timeout(&sock, Duration::from_millis(1000))
        .map_err(|e| mismatch(format!("it did not accept the connection: {e}")))?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(2000)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let ask = format!("GET /api/agent HTTP/1.1\r\nHost: {addr}\r\nConnection: keep-alive\r\n\r\n");
    stream
        .write_all(ask.as_bytes())
        .map_err(|e| mismatch(format!("it did not take the identity request: {e}")))?;
    let answer = read_one_response(&mut stream).map_err(|e| {
        mismatch(format!(
            "it did not answer the identity request as HTTP: {e}"
        ))
    })?;
    check_identity(&answer, expected_instance).map_err(mismatch)?;
    // Verified. The verb goes on this same connection.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    stream.write_all(req.as_bytes()).map_err(|e| {
        Verified::Io(CutError::new(
            error_codes::IO,
            "sending the verb failed",
            e.to_string(),
        ))
    })?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).map_err(|e| {
        Verified::Io(CutError::new(
            error_codes::IO,
            "reading the verb reply failed",
            e.to_string(),
        ))
    })?;
    Ok(buf)
}

/// Read exactly one HTTP response (head, then `Content-Length` body bytes) from
/// a keep-alive stream, bounded in size.
fn read_one_response(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    const LIMIT: usize = 1 << 20;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > LIMIT {
            return Err("response head too large".into());
        }
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("connection closed before a response head".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_ascii_lowercase();
    if !head.starts_with("http/1.") {
        return Err("not an HTTP response".into());
    }
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .ok_or("no Content-Length in the response")?;
    if length > LIMIT {
        return Err("response body too large".into());
    }
    while buf.len() < head_end + length {
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("connection closed inside the response body".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    buf.truncate(head_end + length);
    Ok(buf)
}

/// The error for "nothing answers at `addr`". When `engine.addr` names an address
/// where nothing listens, the file is left over from an engine that did not close
/// cleanly: say so, so the reader is not left guessing why the address in the
/// file is dead.
fn no_server_error(addr: &str, io_cause: &str, stale_note: Option<&str>) -> CutError {
    let cause = match stale_note {
        Some(note) => format!("{io_cause}; {note}"),
        None => io_cause.to_string(),
    };
    CutError::new(error_codes::IO, format!("no cutd server at {addr}"), cause)
        .with_suggested_action(
            "open ShellX Cut or start `cutd serve`, then retry (or run with --standalone); \
         engine.addr is rewritten when an engine starts",
        )
}

/// Says that the discovery file names a dead address, or None when there is no
/// file or its address answers.
fn stale_discovery_note() -> Option<String> {
    let recorded = read_discovery()?;
    (!tcp_ok(&recorded)).then(|| {
        format!(
            "engine.addr names {recorded} but nothing is listening there (the engine that \
             wrote it ended without closing cleanly)"
        )
    })
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

    #[test]
    fn discovery_uses_attempt_state_before_user_appdata_and_keeps_desktop_fallbacks() {
        let attempt_home = std::env::current_dir().unwrap();
        let user_appdata = attempt_home.join("different-user-appdata");
        let user_temp = attempt_home.join("different-user-temp");
        assert_eq!(
            discovery_path_for(
                Some(attempt_home.as_os_str()),
                Some(user_appdata.clone()),
                &user_temp,
            ),
            Some(attempt_home.join("engine.addr")),
        );
        assert_eq!(
            discovery_path_for(None, Some(user_appdata.clone()), &user_temp),
            Some(user_appdata.join("engine.addr")),
        );
        assert_eq!(
            discovery_path_for(None, None, &user_temp),
            Some(user_temp.join("engine.addr")),
        );
    }

    #[test]
    fn invalid_explicit_discovery_home_never_falls_back_into_user_appdata() {
        let root = std::env::current_dir().unwrap();
        let user_appdata = root.join("different-user-appdata");
        for invalid in [
            std::path::PathBuf::from("relative-home"),
            root.join("missing-discovery-home"),
            root.join("..").join("invalid-discovery-home"),
        ] {
            assert_eq!(
                discovery_path_for(Some(invalid.as_os_str()), Some(user_appdata.clone()), &root),
                None,
                "invalid home {invalid:?} must not select user AppData",
            );
        }
    }

    /// A refused connection names the stale `engine.addr` instead of leaving the
    /// reader with a bare "connection refused".
    #[test]
    fn unreachable_server_error_names_a_stale_discovery_file() {
        let stale = no_server_error(
            "127.0.0.1:6161",
            "Connection refused (os error 111)",
            Some("engine.addr names 127.0.0.1:7000 but nothing is listening there"),
        );
        assert_eq!(stale.message, "no cutd server at 127.0.0.1:6161");
        assert!(stale.cause.contains("Connection refused"));
        assert!(stale.cause.contains("engine.addr names 127.0.0.1:7000"));
        assert!(stale
            .suggested_action
            .as_deref()
            .is_some_and(|a| a.contains("open ShellX Cut or start `cutd serve`")));
        let plain = no_server_error("127.0.0.1:6161", "Connection refused", None);
        assert!(!plain.cause.contains("engine.addr"));
    }

    /// With the file naming a dead address the note appears; with a live one it
    /// does not.
    #[test]
    fn stale_note_only_for_a_dead_recorded_address() {
        let tmp = std::env::temp_dir().join(format!(
            "engine.addr.stale-note.{:?}",
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&tmp);
        TEST_DISCOVERY_PATH.with(|c| *c.borrow_mut() = Some(tmp.clone()));
        assert_eq!(stale_discovery_note(), None, "no file, no note");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        write_discovery(&listener.local_addr().unwrap().to_string());
        assert_eq!(stale_discovery_note(), None, "a live address is not stale");
        drop(listener);
        write_discovery("127.0.0.1:1");
        let note = stale_discovery_note().expect("a dead address is reported");
        assert!(note.contains("127.0.0.1:1"));
        let _ = std::fs::remove_file(&tmp);
        TEST_DISCOVERY_PATH.with(|c| *c.borrow_mut() = None);
    }

    /// A loopback "peer" for the tests. `agent` is the full reply to
    /// `GET /api/agent` (`None` = answer with something that is not HTTP). It
    /// serves several requests per connection and records every byte it
    /// receives, so a test can prove a verb body never reached it.
    struct FakePeer {
        addr: String,
        raw: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl FakePeer {
        fn start(agent_json: &str) -> Self {
            Self::start_with(Some(agent_json.to_string()))
        }

        fn start_with(agent_json: Option<String>) -> Self {
            use std::sync::atomic::Ordering;
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap().to_string();
            let raw = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (raw2, stop2) = (raw.clone(), stop.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop2.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(mut stream) = stream else { continue };
                    let (raw3, body) = (raw2.clone(), agent_json.clone());
                    std::thread::spawn(move || {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(1500)));
                        let mut pending: Vec<u8> = Vec::new();
                        let mut buf = [0u8; 4096];
                        loop {
                            let n = stream.read(&mut buf).unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            raw3.lock().unwrap().extend_from_slice(&buf[..n]);
                            pending.extend_from_slice(&buf[..n]);
                            // Answer the identity request as soon as its head is in.
                            let text = String::from_utf8_lossy(&pending).into_owned();
                            if text.starts_with("GET /api/agent") && text.contains("\r\n\r\n") {
                                pending.clear();
                                let reply = match &body {
                                    Some(json) => format!(
                                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{json}",
                                        json.len()
                                    ),
                                    None => "garbage that is not http\r\n\r\n".to_string(),
                                };
                                let _ = stream.write_all(reply.as_bytes());
                            } else if text.starts_with("POST ") && text.contains("\r\n\r\n") {
                                let reply = "{\"ok\":true,\"result\":{}}";
                                let _ = write!(
                                    stream,
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                                    reply.len()
                                );
                                break;
                            }
                        }
                    });
                }
            });
            Self { addr, raw, stop }
        }

        /// Everything the peer has received, as text.
        fn received(&self) -> String {
            String::from_utf8_lossy(&self.raw.lock().unwrap()).into_owned()
        }

        /// Whether any verb request (or its body) reached the peer.
        fn saw_verb(&self) -> bool {
            let all = self.received();
            all.contains("/api/verb") || all.contains("/secret/project")
        }
    }

    impl Drop for FakePeer {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = TcpStream::connect(&self.addr); // wake the accept loop
        }
    }

    fn isolate(name: &str) -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "engine.addr.{name}.{:?}.{}",
            std::thread::current().id(),
            std::process::id()
        ));
        let _ = std::fs::remove_file(&tmp);
        TEST_DISCOVERY_PATH.with(|c| *c.borrow_mut() = Some(tmp.clone()));
        tmp
    }

    fn release(tmp: &std::path::Path) {
        let _ = std::fs::remove_file(tmp);
        TEST_DISCOVERY_PATH.with(|c| *c.borrow_mut() = None);
        TEST_PROXY_ENV.with(|c| *c.borrow_mut() = None);
        TEST_DEFAULT_ADDR.with(|c| *c.borrow_mut() = None);
        TEST_ISOLATED.with(|c| c.set(false));
    }

    fn engine_json(instance: &str) -> String {
        format!(
            "{{\"product\":\"ShellX Cut\",\"version\":\"{}\",\"instance\":\"{instance}\"}}",
            env!("CARGO_PKG_VERSION")
        )
    }

    /// The file keeps the address on its first line and the instance on the
    /// second; a file from an older engine (address alone) still reads.
    #[test]
    fn engine_addr_holds_the_address_then_the_instance_and_old_files_still_read() {
        assert_eq!(
            parse_record("127.0.0.1:6161\ninstance=abc\n"),
            Some(DiscoveryRecord {
                addr: "127.0.0.1:6161".into(),
                instance: Some("abc".into())
            })
        );
        assert_eq!(
            parse_record("127.0.0.1:6161\n"),
            Some(DiscoveryRecord {
                addr: "127.0.0.1:6161".into(),
                instance: None
            })
        );
        assert_eq!(parse_record("  \n"), None);
    }

    /// THE bug: a stale `engine.addr` plus another local program on that port.
    /// The verb (project paths, content) must not reach that program.
    #[test]
    fn a_verb_is_not_sent_to_a_program_that_is_not_the_engine() {
        let tmp = isolate("stranger");
        let stranger = FakePeer::start("{\"hello\":\"i am something else\"}");
        write_discovery(&stranger.addr);
        let error = post_verb(
            "project.open",
            &serde_json::json!({"path": "/secret/project"}),
        )
        .expect_err("a stranger must not receive the verb");
        assert!(
            error.message.contains("is not this ShellX Cut engine"),
            "{error:?}"
        );
        assert!(error.message.contains("nothing was sent"), "{error:?}");
        assert!(error
            .suggested_action
            .as_deref()
            .is_some_and(|a| a.contains("Open ShellX Cut") && a.contains("delete the stale file")));
        assert!(
            !stranger.saw_verb(),
            "only the identity request may reach it: {:?}",
            stranger.received()
        );
        release(&tmp);
    }

    /// An engine of this product and version, but not the instance engine.addr
    /// names (an older engine reused the port): also refused.
    #[test]
    fn a_different_engine_instance_on_the_recorded_port_is_refused() {
        let tmp = isolate("instance");
        let other = FakePeer::start(&engine_json("older-instance"));
        write_discovery_record(&other.addr, "the-recorded-instance");
        let error = post_verb("project.state", &serde_json::json!({}))
            .expect_err("a different instance must not receive the verb");
        assert!(error
            .cause
            .contains("not the one engine.addr was written by"));
        assert!(!other.saw_verb());
        release(&tmp);
    }

    /// A different version of the product is refused too (the shell's rule).
    #[test]
    fn a_different_product_version_is_refused() {
        let tmp = isolate("version");
        let old = FakePeer::start(
            "{\"product\":\"ShellX Cut\",\"version\":\"0.0.1\",\"instance\":\"x\"}",
        );
        write_discovery_record(&old.addr, "x");
        let error = post_verb("project.state", &serde_json::json!({})).unwrap_err();
        assert!(error.cause.contains("v0.0.1"), "{error:?}");
        assert!(!old.saw_verb());
        release(&tmp);
    }

    /// The genuine engine, with the instance recorded, receives the verb; so does
    /// an engine from an older file that has no instance line.
    #[test]
    fn the_recorded_engine_receives_the_verb() {
        let tmp = isolate("genuine");
        let engine = FakePeer::start(&engine_json("inst-1"));
        write_discovery_record(&engine.addr, "inst-1");
        assert_eq!(locate(), Located::Engine(engine.addr.clone()));
        let reply = post_verb("project.state", &serde_json::json!({})).expect("verb delivered");
        assert_eq!(reply["ok"], true);
        assert!(engine.received().contains("POST /api/verb/project.state"));
        // A file written before the instance line existed: address alone.
        std::fs::write(&tmp, format!("{}\n", engine.addr)).unwrap();
        assert_eq!(locate(), Located::Engine(engine.addr.clone()));
        release(&tmp);
    }

    /// Identity answer without an instance while engine.addr names one: refused.
    #[test]
    fn a_peer_that_reports_no_instance_is_refused_when_the_file_names_one() {
        let tmp = isolate("noinstance");
        let peer = FakePeer::start(&format!(
            "{{\"product\":\"ShellX Cut\",\"version\":\"{}\"}}",
            env!("CARGO_PKG_VERSION")
        ));
        write_discovery_record(&peer.addr, "the-recorded-instance");
        let error = post_verb(
            "project.open",
            &serde_json::json!({"path": "/secret/project"}),
        )
        .expect_err("a peer without the instance must not receive the verb");
        assert!(error.message.contains("nothing was sent"), "{error:?}");
        assert!(!peer.saw_verb(), "{:?}", peer.received());
        release(&tmp);
    }

    /// Identity answer with a wrong instance: the verb body is never written.
    #[test]
    fn a_peer_with_the_wrong_instance_never_receives_the_verb_body() {
        let tmp = isolate("wronginstance");
        let peer = FakePeer::start(&engine_json("not-the-recorded-one"));
        write_discovery_record(&peer.addr, "the-recorded-instance");
        let result = post_verb(
            "project.open",
            &serde_json::json!({"path": "/secret/project"}),
        );
        assert!(result.is_err());
        assert!(!peer.saw_verb(), "{:?}", peer.received());
        release(&tmp);
    }

    /// A listener that does not speak HTTP at all: the verb body is never written.
    #[test]
    fn a_listener_that_is_not_http_never_receives_the_verb_body() {
        let tmp = isolate("nothttp");
        let peer = FakePeer::start_with(None);
        write_discovery_record(&peer.addr, "x");
        let result = post_verb(
            "project.open",
            &serde_json::json!({"path": "/secret/project"}),
        );
        let error = result.expect_err("a non-HTTP listener must not receive the verb");
        assert!(error.message.contains("nothing was sent"), "{error:?}");
        assert!(!peer.saw_verb(), "{:?}", peer.received());
        release(&tmp);
    }

    /// No listener at the recorded address: an error, and no verb goes anywhere.
    #[test]
    fn with_no_listener_the_verb_is_reported_as_not_sent() {
        let tmp = isolate("nolistener");
        write_discovery_record("127.0.0.1:1", "x");
        let error = post_verb(
            "project.open",
            &serde_json::json!({"path": "/secret/project"}),
        )
        .expect_err("nothing listens");
        assert!(error.message.starts_with("no cutd server at"), "{error:?}");
        release(&tmp);
    }

    /// GUARD: a unit test must never resolve the person's real `engine.addr`, and
    /// never reach the default port, where an installed engine may be running.
    /// (Seven tests once sent verbs, including a project change, to a live
    /// engine because the default port was a fallback candidate.)
    #[test]
    fn a_unit_test_never_resolves_the_real_engine_addr_or_the_default_port() {
        let real = discovery_path_for(
            std::env::var_os("SHELLX_CUT_HOME").as_deref(),
            cut_media::toolpath::appdata_tools_dir()
                .and_then(|p| p.parent().map(|x| x.to_path_buf())),
            &std::env::temp_dir(),
        );
        let used = discovery_path();
        assert_ne!(
            used, real,
            "a test process resolved the real per-user state folder"
        );
        assert_ne!(
            default_addr(),
            SERVER_ADDR,
            "a test would reach the real default port"
        );
        assert_eq!(
            proxy_env(),
            None,
            "a test must not read the real CUTD_PROXY_ADDR"
        );
    }

    /// GUARD: every per-user location the engine reads or writes resolves inside
    /// the test process's own temp folder (set before `main` by
    /// `cut_core::isolate_test_process!`): the state home (engine.addr, recent
    /// projects, library), the downloaded tools folder (FFmpeg, Python sidecar),
    /// the user's home and the provider CLI search.
    #[test]
    fn every_per_user_location_resolves_inside_the_test_process_folder() {
        use cut_core::test_isolation::is_isolated_path;
        let inside = |what: &str, path: Option<std::path::PathBuf>| {
            let path = path.unwrap_or_else(|| panic!("{what} is unresolved"));
            assert!(
                is_isolated_path(&path),
                "{what} resolves outside the test folder: {path:?}"
            );
        };
        inside("state home", crate::userdata::shellx_cut_home());
        inside(
            "recent projects list",
            crate::userdata::projects_index_path(),
        );
        inside("tools folder", cut_media::toolpath::appdata_tools_dir());
        inside(
            "HOME",
            std::env::var_os("HOME").map(std::path::PathBuf::from),
        );
        inside(
            "USERPROFILE",
            std::env::var_os("USERPROFILE").map(std::path::PathBuf::from),
        );
        inside(
            "SHELLX_CUT_HOME",
            std::env::var_os("SHELLX_CUT_HOME").map(std::path::PathBuf::from),
        );
        assert_eq!(
            std::env::var("SHELLX_CUT_AGENT_SEARCH").as_deref(),
            Ok("path")
        );
    }

    /// An address the caller NAMED that is not this engine is an error that names
    /// both facts; the verb goes to neither program.
    #[test]
    fn a_named_address_that_is_not_the_engine_is_an_error_naming_the_running_engine() {
        let tmp = isolate("named");
        let stranger = FakePeer::start("{\"hello\":\"other\"}");
        let engine = FakePeer::start(&engine_json("inst"));
        TEST_PROXY_ENV.with(|c| *c.borrow_mut() = Some(stranger.addr.clone()));
        TEST_DEFAULT_ADDR.with(|c| *c.borrow_mut() = Some(engine.addr.clone()));
        let error = post_verb(
            "project.open",
            &serde_json::json!({"path": "/secret/project"}),
        )
        .expect_err("a named address that fails the identity check is an error");
        assert!(error.message.contains(&stranger.addr), "{error:?}");
        let action = error.suggested_action.clone().unwrap_or_default();
        assert!(
            action.contains(&engine.addr) && action.contains("unset CUTD_PROXY_ADDR"),
            "{error:?}"
        );
        assert!(!stranger.saw_verb() && !engine.saw_verb());
        release(&tmp);
    }

    /// A named address with nothing listening is an error too: the verb does not
    /// quietly go to the engine that does run.
    #[test]
    fn a_named_address_with_no_listener_does_not_fall_back_to_another_engine() {
        let tmp = isolate("namedgone");
        let engine = FakePeer::start(&engine_json("inst"));
        TEST_PROXY_ENV.with(|c| *c.borrow_mut() = Some("127.0.0.1:1".to_string()));
        TEST_DEFAULT_ADDR.with(|c| *c.borrow_mut() = Some(engine.addr.clone()));
        let error = post_verb(
            "project.open",
            &serde_json::json!({"path": "/secret/project"}),
        )
        .expect_err("a dead named address is an error");
        assert!(error.cause.contains("nothing is listening"), "{error:?}");
        assert!(error
            .suggested_action
            .unwrap_or_default()
            .contains(&engine.addr));
        assert!(!engine.saw_verb());
        release(&tmp);
    }

    /// With no address named, the default engine is still found (the fallback
    /// that stays).
    #[test]
    fn with_no_address_named_the_default_engine_is_used() {
        let tmp = isolate("default");
        let engine = FakePeer::start(&engine_json("inst"));
        TEST_DEFAULT_ADDR.with(|c| *c.borrow_mut() = Some(engine.addr.clone()));
        post_verb("project.state", &serde_json::json!({})).expect("delivered");
        assert!(engine.received().contains("POST /api/verb/project.state"));
        release(&tmp);
    }

    /// With an explicit SHELLX_CUT_HOME the default port is not a fallback: an
    /// isolated client must not send verbs to the person's own engine. The engine
    /// its own engine.addr names is still found.
    #[test]
    fn an_isolated_state_home_does_not_fall_back_to_the_default_port() {
        let tmp = isolate("isolated");
        let installed = FakePeer::start(&engine_json("installed"));
        TEST_DEFAULT_ADDR.with(|c| *c.borrow_mut() = Some(installed.addr.clone()));
        TEST_ISOLATED.with(|c| c.set(true));
        let error = post_verb(
            "project.open",
            &serde_json::json!({"path": "/secret/project"}),
        )
        .expect_err("no engine in the isolated home");
        assert!(error.message.starts_with("no cutd server at"), "{error:?}");
        assert!(!installed.saw_verb());
        assert_eq!(locate(), Located::NoServer);
        let own = FakePeer::start(&engine_json("own"));
        write_discovery_record(&own.addr, "own");
        assert_eq!(locate(), Located::Engine(own.addr.clone()));
        release(&tmp);
    }

    /// A live discovery address that answers as the engine is used by the proxy;
    /// a stale one (server gone) falls back to the default port. Saves/restores
    /// any real discovery file so the test never disturbs a cutd running on this
    /// machine.
    #[test]
    fn discovery_roundtrip_and_liveness_fallback() {
        // Isolate the discovery file to THIS thread so a parallel test can't race us
        // (was flaky in the full suite, passed in isolation). Unique per thread id.
        let tmp = isolate("roundtrip");

        let engine = FakePeer::start(&engine_json(instance_id()));
        let addr = engine.addr.clone();
        write_discovery(&addr);
        assert_eq!(read_discovery().as_deref(), Some(addr.as_str()));
        assert_eq!(
            server_addr(),
            addr,
            "a live discovery addr is used by the proxy"
        );

        drop(engine); // stop the engine stand-in

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
        release(&tmp);
    }
}
