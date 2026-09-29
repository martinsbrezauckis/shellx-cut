//! http.rs — axum REST + WS + static UI (server contract).
//!
//! Role: the HTTP surface on 127.0.0.1:6161 —
//!   POST /api/verb/{name}   → dispatch (the ONLY mutation path)
//!   GET  /api/state         → project.state convenience alias
//!   GET  /api/verbs         → the embedded verb registry (agent discovery)
//!   GET  /api/frame?at_ms=  → composed frame JPEG (render.frame, raw bytes)
//!   GET  /api/events        → WS event stream (events.rs)
//!   /                       → ui/dist static files (the React app)
//! Dependencies: axum, tower-http, state/dispatch/events. Primary callers:
//! main.rs (serve), UI fetch/WS clients, local agent clients.

use crate::dispatch::dispatch;
use crate::events::Event;
use crate::{review_http as rh, state::AppState};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cut_core::{error_codes, Actor, ActorKind, CutError};
use serde_json::Value;
use std::collections::HashMap;
use tower_http::services::{ServeDir, ServeFile};

const MEDIA_STREAM_CHUNK: usize = 64 * 1024;

/// Build a no-Range file response without materializing the file in the server
/// heap. Callers must fence and type the path before reaching this helper.
async fn stream_file_response(
    path: &std::path::Path,
    content_type: &'static str,
    accept_ranges: bool,
) -> Response {
    use axum::body::Body;
    use axum::http::{header, StatusCode};
    use tokio_util::io::ReaderStream;

    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    let len = match file.metadata().await {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        _ => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    let stream = ReaderStream::with_capacity(file, MEDIA_STREAM_CHUNK);
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(content_type),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        axum::http::HeaderValue::from_str(&len.to_string())
            .expect("a decimal file length is a valid header"),
    );
    if accept_ranges {
        response.headers_mut().insert(
            header::ACCEPT_RANGES,
            axum::http::HeaderValue::from_static("bytes"),
        );
    }
    response
}

/// Default bind address (server contract: loopback only; Cut has no remote mode).
pub const DEFAULT_ADDR: &str = "127.0.0.1:6161";
/// Explicit development-only escape for testing a non-loopback listener. It is
/// deliberately ignored by release/packaged builds; Cut has no authenticated
/// remote-server mode.
pub const ENV_ALLOW_NON_LOCAL_DEV: &str = "SHELLX_CUT_ALLOW_NON_LOCAL";

/// Build the full router. `ui_dist` points at ui/dist (serves a 404-with-hint
/// JSON if the UI was never built — the API must work headless regardless).
pub fn build_router(state: AppState, ui_dist: Option<std::path::PathBuf>) -> Router {
    let api = Router::new()
        .route("/verb/:name", post(post_verb))
        .route("/state", get(get_state))
        .route("/verbs", get(get_verbs))
        .route("/agent", get(get_agent_info))
        .route("/agent-doc/*path", get(serve_agent_doc))
        .route("/frame", get(get_frame))
        // A disposable native rehearsal is playable only through an opaque,
        // process-local server capability.  The handler never accepts a path
        // and rechecks the owned temporary root before streaming any bytes.
        .route("/recording-rehearsal/:handle", get(serve_rehearsal_media))
        .route("/events", get(ws_events));
    // This route is absent unless the current cutd was spawned by the one
    // foreground Tauri desktop with fresh private bridge material. It is not
    // an API/verb surface for headless, MCP, or adopted engines.
    #[cfg(target_os = "macos")]
    let api = crate::screen_record::macos_region_bridge::install_route(api);
    #[cfg(windows)]
    let api = crate::screen_record::windows_region_bridge::install_route(api);

    let mut router = Router::new()
        .nest("/api", api)
        // Project media served from the CURRENT project dir (dynamic, so not a
        // static ServeDir): the preview <video src="proxies/aN.mp4"> and frame
        // images live under <project>/proxies and <project>/frames. Without
        // these routes the requests fell through to the SPA fallback and the
        // <video> got index.html → black preview (media-route regression).
        .route("/proxies/:file", get(serve_proxy))
        .route("/frames/:file", get(serve_frame_file))
        .route("/filmstrip/:file", get(serve_filmstrip))
        // Download render.bundle packs (+ any export.* artifact) from the
        // CURRENT project's exports/ subtree. Wildcard (nested bundle paths),
        // fenced to the exports subtree (see serve_export_path).
        .route("/api/export/*path", get(serve_export_path))
        // Serve ONE exact export by absolute path — the only shape that can name
        // an export written into the user's chosen output folder (which lives
        // outside the project). Fenced to the authorized export roots and never
        // falls back to a same-named file elsewhere (see serve_export_file).
        .route("/api/export-file", get(serve_export_file))
        // Stream a registered asset's ORIGINAL source for the preview
        // <video> when no proxy exists yet (edit instantly while the proxy builds,
        // or when proxy generation is toggled off). Fenced to the open project's
        // asset registry; seek + capped chunk so a multi-GB source never loads whole.
        .route("/api/source/:asset", get(serve_source))
        // Global asset library: serve content-addressed blobs (copied/portable
        // library items) for thumbnails/preview. Project-independent; fenced to
        // the library blobs dir (see serve_library_blob).
        .route("/api/library-blob/:file", get(serve_library_blob))
        // Library POSTER: a rendered single-frame thumbnail (video frame / scaled
        // image / audio waveform) for a library item, keyed by item id and resolved
        // through the library manifest (NOT an arbitrary path). Lets the panel show
        // real content for linked video/audio/image items, which have no blob to
        // serve directly. See serve_library_poster.
        .route("/api/library-poster", get(serve_library_poster));
    if let Some(dist) = ui_dist {
        // SPA fallback: unknown non-API paths get index.html.
        let index = dist.join("index.html");
        router = router.fallback_service(ServeDir::new(&dist).fallback(ServeFile::new(index)));
    }
    // N1: loopback is this server's machine-reachability boundary, not a caller
    // identity. A cross-origin web page could otherwise drive the full verb set
    // via a CORS "simple request" (the body reaches dispatch even though the
    // browser blocks the response read), and a DNS-rebound hostname could defeat
    // even that. This guard rejects browser requests whose Origin (if present)
    // or Host authority is not loopback, plus a no-Origin Fetch-Metadata
    // `cross-site` request; native callers can omit/forge headers and therefore
    // remain inside the intentionally machine-wide trust scope.
    router
        .with_state(state)
        // Content-Security-Policy for the served UI. The desktop
        // WebView loads the engine-served UI from http://127.0.0.1:6161 (a remote
        // origin — the tauri.conf `csp` field governs only tauri:// content, which
        // this app does not use), so the CSP must be a HEADER from cutd, not a
        // bundler/Tauri setting. Defense-in-depth even on a loopback app: a
        // compromised UI dependency cannot load an external script or exfiltrate to
        // an off-origin endpoint. Applied to every response — inert on JSON/media
        // (CSP is a document-level policy) and meaningful on the SPA document.
        .layer(axum::middleware::from_fn(add_csp_header))
        .layer(axum::middleware::from_fn(add_document_cache_header))
        .layer(axum::middleware::from_fn(guard_local_origin))
}

/// Index documents must be reloaded from the installed cutd after an update.
/// Hashed JS/CSS assets keep ServeDir's normal caching behavior.
async fn add_document_cache_header(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mut res = next.run(req).await;
    if res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"))
    {
        res.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
    }
    res
}

/// Content-Security-Policy for the served UI (S3). The UI is a Vite SPA: one
/// same-origin ES-module script + same-origin CSS, React inline `style={{}}`
/// (CSSOM, plus `'unsafe-inline'` for any style attribute), `data:`/`blob:`
/// images + media (frames, proxies), and the `/api/events` WebSocket. connect-src
/// is scoped to loopback at ANY port (the app falls back to an OS-chosen port when
/// 6161 is held) so the WS never breaks. `ipc.localhost` is Tauri's loopback IPC
/// bridge for remote-origin WebViews; without it WebView2 logs a CSP error before
/// falling back to postMessage. The policy still blocks off-host exfiltration.
const UI_CSP: &str = "default-src 'self'; \
base-uri 'self'; \
object-src 'none'; \
frame-ancestors 'none'; \
form-action 'self'; \
script-src 'self'; \
style-src 'self' 'unsafe-inline'; \
img-src 'self' data: blob:; \
media-src 'self' blob: data:; \
font-src 'self' data:; \
worker-src 'self' blob:; \
connect-src 'self' ws://127.0.0.1:* ws://localhost:* http://127.0.0.1:* http://localhost:* http://ipc.localhost";

/// Middleware: stamp the UI CSP on responses. Opt-out via `SHELLX_CUT_DISABLE_CSP=1`
/// (a field-deployment escape hatch should an unforeseen UI feature ever need a
/// broader policy — the loopback Origin/Host guard remains the primary boundary).
async fn add_csp_header(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let mut res = next.run(req).await;
    if should_add_default_csp(&res) {
        res.headers_mut().insert(
            axum::http::header::CONTENT_SECURITY_POLICY,
            axum::http::HeaderValue::from_static(UI_CSP),
        );
    }
    res
}

/// True if a Host/Origin authority names the loopback interface — the default
/// machine-reachability boundary, not authentication. Accepts "127.0.0.1:6161",
/// "localhost", "[::1]:6161", and bare "http://127.0.0.1:6161" origins.
///
/// Loopback is decided by PARSING the host as an IP and checking
/// `is_loopback()` — NOT a substring/prefix match. A naive `starts_with("127.")`
/// would accept `127.0.0.1.evil.com` (an attacker domain rebinding to 127.0.0.1)
/// and defeat the whole guard. The only non-IP host treated as local is the
/// literal `localhost` (after trailing-dot + case normalization).
pub(crate) fn authority_is_loopback(authority: &str) -> bool {
    let after_scheme = authority.split("://").last().unwrap_or(authority);
    let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        // IPv6 literal: [::1]:port
        rest.split(']').next().unwrap_or("")
    } else {
        host_port
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(host_port)
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(host == "localhost")
}

/// Does a build deliberately enable the non-loopback test route? A source/debug
/// executable may use the explicit environment variable for an isolated local
/// integration test. A release build — including a packaged desktop engine —
/// always returns false, even when its inherited environment contains the
/// variable. This keeps distribution mode loopback-only by construction.
fn non_local_development_escape_enabled() -> bool {
    non_local_escape_enabled_for(
        cfg!(debug_assertions),
        std::env::var(ENV_ALLOW_NON_LOCAL_DEV).as_deref() == Ok("1"),
    )
}

/// Pure policy seam for source and packaged-mode tests. Do not weaken this to
/// an environment-only check: installer, launcher, and parent-process
/// environments can all carry variables into a packaged engine.
fn non_local_escape_enabled_for(is_development_build: bool, explicit_opt_in: bool) -> bool {
    is_development_build && explicit_opt_in
}

/// Is it safe to bind `addr` as a listen address? `cutd` is a loopback-only
/// machine-reachability boundary — the HTTP guard stops browser DNS rebinding,
/// but a non-loopback BIND (`0.0.0.0`, a LAN IP, `::`) exposes the full mutation
/// surface to the network where a non-browser client can forge a `Host:
/// localhost` and slip past the header guard. Loopback does not authenticate
/// local callers. Only an explicit debug-build test route can bypass the
/// refusal; packaged builds have no remote mode or Cut authentication.
/// Returns Ok(()) when binding is allowed, Err(reason) when it must be refused.
pub fn check_bind_addr(addr: &str) -> Result<(), String> {
    check_bind_addr_with_escape(addr, non_local_development_escape_enabled())
}

fn check_bind_addr_with_escape(addr: &str, allow_non_local: bool) -> Result<(), String> {
    if allow_non_local {
        return Ok(()); // explicit development route; Cut adds no remote auth
    }
    if authority_is_loopback(addr) {
        return Ok(());
    }
    Err(format!(
        "refusing to bind non-loopback address '{addr}': cutd listens on loopback only by default \
         (server contract). Bind 127.0.0.1 / [::1] / localhost. {ENV_ALLOW_NON_LOCAL_DEV}=1 is available only to \
         an explicit debug-build development route and is ignored by packaged/release builds; Cut itself does not \
         authenticate remote callers."
    ))
}

/// N1 guard: reject browser-driven cross-origin / DNS-rebinding access. Honors
/// the explicit debug-only non-loopback route. Packaged/release builds always
/// enforce this loopback guard; a development route does not add a Cut
/// remote-auth contract.
async fn guard_local_origin(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if non_local_development_escape_enabled() {
        return next.run(req).await;
    }
    let headers = req.headers();
    // Origin (if present) is the primary defense: a cross-origin page's fetch /
    // WS handshake carries its own non-loopback Origin, and so does a DNS-rebind
    // page (its Origin is the attacker hostname).
    if let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        if !authority_is_loopback(origin) {
            return forbidden_non_local("cross-origin request rejected (Origin is not loopback)");
        }
    } else if headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|site| site.eq_ignore_ascii_case("cross-site"))
    {
        // WebKit can deliver a cross-site no-CORS image request without Origin.
        // Fetch Metadata supplies the browser-only signal that Origin cannot in
        // that case; native clients normally omit both headers and still work.
        return forbidden_non_local("cross-site browser request rejected (no Origin)");
    }
    // Host belt-and-braces: a rebound hostname resolving to 127.0.0.1 carries a
    // non-loopback Host even on a no-Origin request.
    if let Some(host) = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
    {
        if !authority_is_loopback(host) {
            return forbidden_non_local("Host header is not loopback (possible DNS rebinding)");
        }
    }
    next.run(req).await
}

/// 403 envelope for a request rejected by the loopback guard.
fn forbidden_non_local(message: &str) -> Response {
    (
        axum::http::StatusCode::FORBIDDEN,
        Json(serde_json::json!({"ok": false, "error": {
            "code": "forbidden",
            "message": message,
            "cause": "cutd serves its default listener on loopback only; this is a machine-wide reachability boundary, not local-caller authentication",
            "suggested_action": "use cutd from localhost (the desktop UI, MCP, or a local agent); Cut has no supported authenticated remote mode"
        }})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestServer {
        base_url: String,
        handle: tokio::task::JoinHandle<()>,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    async fn spawn_test_server(router: Router) -> TestServer {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        TestServer {
            base_url: format!("http://127.0.0.1:{port}"),
            handle,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn versioned_index_is_fresh_and_hashed_assets_keep_their_cache_policy() {
        let dist = tempfile::tempdir().unwrap();
        std::fs::write(dist.path().join("index.html"), "<html>current</html>").unwrap();
        std::fs::write(
            dist.path().join("app-123.js"),
            "export const current = true;",
        )
        .unwrap();
        let server =
            spawn_test_server(build_router(AppState::new(), Some(dist.path().into()))).await;

        let check = move |path: &'static str| {
            let url = format!("{}{path}", server.base_url);
            let mut response = ureq::get(&url).call().unwrap();
            let cache = response
                .headers()
                .get("cache-control")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let body = response.body_mut().read_to_string().unwrap();
            (response.status().as_u16(), cache, body)
        };
        let (old_index, new_index, external_index, direct_index, spa_fallback, asset) =
            tokio::task::spawn_blocking(move || {
                (
                    check("/?cut-ui=old"),
                    check("/?cut-ui=new"),
                    check("/?cut-session=one"),
                    check("/index.html"),
                    check("/editor"),
                    check("/app-123.js"),
                )
            })
            .await
            .unwrap();
        for (code, policy, html) in [
            old_index,
            new_index,
            external_index,
            direct_index,
            spa_fallback,
        ] {
            assert_eq!(code, 200);
            assert_eq!(policy.as_deref(), Some("no-store"));
            assert_eq!(html, "<html>current</html>");
        }
        assert_eq!(asset.0, 200);
        assert_eq!(asset.1, None);
        assert_eq!(asset.2, "export const current = true;");
    }

    /// S3: the UI CSP is well-formed and pins the directives the SPA relies on —
    /// same-origin scripts, inline styles (React), data:/blob: media, and a
    /// loopback-scoped connect-src (any port) so the /api/events WS never breaks.
    /// This unit pins the policy string itself; browser tests cover the loaded UI.
    #[test]
    fn ui_csp_is_well_formed() {
        // Parseable as a header value (no illegal bytes).
        assert!(axum::http::HeaderValue::from_static(UI_CSP)
            .to_str()
            .is_ok());
        for must in [
            "default-src 'self'",
            "script-src 'self'",
            "object-src 'none'",
            "frame-ancestors 'none'",
            "style-src 'self' 'unsafe-inline'",
            "img-src 'self' data: blob:",
            "ws://127.0.0.1:*",
            "http://ipc.localhost",
        ] {
            assert!(UI_CSP.contains(must), "CSP missing directive: {must}");
        }
        // Must NOT weaken script execution (no unsafe-inline/eval on scripts).
        assert!(!UI_CSP.contains("script-src 'self' 'unsafe-inline'"));
        assert!(!UI_CSP.contains("unsafe-eval"));
    }

    /// Packaged/release mode refuses a non-loopback address even if an inherited
    /// environment asks for the developer escape. Loopback forms still pass.
    #[test]
    fn packaged_bind_guard_refuses_non_loopback() {
        for ok in [
            "127.0.0.1:6161",
            "[::1]:6161",
            "localhost:6161",
            "127.0.0.1",
        ] {
            assert!(
                check_bind_addr_with_escape(ok, false).is_ok(),
                "{ok} should bind in packaged mode"
            );
        }
        for bad in [
            "0.0.0.0:6161",
            "203.0.113.5:6161",
            "[::]:6161",
            "10.0.0.1:6161",
        ] {
            assert!(
                check_bind_addr_with_escape(bad, false).is_err(),
                "{bad} must be refused in packaged mode"
            );
        }
    }

    /// LOCAL-BIND-HARDEN-01: the escape requires both a development build and
    /// an explicit opt-in. A release/packaged build must never become remote
    /// merely because a launcher inherited the environment variable.
    #[test]
    fn non_loopback_escape_is_explicit_and_development_only() {
        assert!(non_local_escape_enabled_for(true, true));
        assert!(!non_local_escape_enabled_for(true, false));
        assert!(
            !non_local_escape_enabled_for(false, true),
            "packaged/release mode ignores {ENV_ALLOW_NON_LOCAL_DEV}=1"
        );
        assert!(
            check_bind_addr_with_escape("0.0.0.0:6161", true).is_ok(),
            "the explicit debug integration route remains available"
        );
        assert!(
            check_bind_addr_with_escape("0.0.0.0:6161", false).is_err(),
            "the same address remains refused outside that route"
        );
    }

    #[test]
    fn authority_loopback_classification() {
        for ok in [
            "127.0.0.1:6161",
            "127.0.0.1",
            "localhost:6161",
            "localhost",
            "http://127.0.0.1:6161",
            "http://localhost:6161",
            "[::1]:6161",
            "http://[::1]:6161",
            "127.5.0.1:80",
        ] {
            assert!(authority_is_loopback(ok), "{ok} should be loopback");
        }
        for bad in [
            "evil.com",
            "evil.com:6161",
            "http://evil.com",
            "0.0.0.0:6161",
            "203.0.113.5:6161",
            "http://attacker.test:6161",
            "10.0.0.1",
            "null",
            "",
            // Unanchored-prefix bypass vectors (the bug the prefix match had):
            "127.0.0.1.evil.com",
            "127.0.0.1.evil.com:6161",
            "http://127.0.0.1.evil.com",
            "127.evil.com",
            "localhost.evil.com",
            "127.0.0.1evil.com",
            "0177.0.0.1.evil.com",
        ] {
            assert!(!authority_is_loopback(bad), "{bad} should NOT be loopback");
        }
        // Normalization: trailing dot + uppercase still classified correctly.
        assert!(authority_is_loopback("LOCALHOST:6161"));
        assert!(authority_is_loopback("localhost."));
        assert!(!authority_is_loopback("127.0.0.1.evil.com."));
    }

    /// N1 end-to-end: a real bound cutd rejects cross-origin browser requests
    /// and serves legitimate local callers. (ureq sets Host from the URL =
    /// loopback, so this exercises the browser-request guards.)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn guard_rejects_cross_origin_and_no_origin_cross_site_requests() {
        let server = spawn_test_server(build_router(AppState::new(), None)).await;
        let url = format!("{}/api/verbs", server.base_url);
        let frame_url = format!("{}/api/frame?at_ms={}&h=4096", server.base_url, u64::MAX);

        fn status(url: &str, origin: Option<&str>, fetch_site: Option<&str>) -> u16 {
            let mut req = ureq::get(url);
            if let Some(o) = origin {
                req = req.header("Origin", o);
            }
            if let Some(site) = fetch_site {
                req = req.header("Sec-Fetch-Site", site);
            }
            match req.call() {
                Ok(resp) => resp.status().as_u16(),
                Err(ureq::Error::StatusCode(code)) => code,
                Err(e) => unreachable!("transport error: {e}"),
            }
        }

        let u = url.clone();
        let no_origin = tokio::task::spawn_blocking(move || status(&u, None, None))
            .await
            .unwrap();
        assert_eq!(
            no_origin, 200,
            "a no-Origin local caller must be served; Origin is not caller authentication"
        );

        let u = url.clone();
        let local_browser =
            tokio::task::spawn_blocking(move || status(&u, None, Some("same-origin")))
                .await
                .unwrap();
        assert_eq!(
            local_browser, 200,
            "same-origin browser requests without Origin must be served"
        );

        let u = url.clone();
        let loopback =
            tokio::task::spawn_blocking(move || status(&u, Some("http://127.0.0.1"), None))
                .await
                .unwrap();
        assert_eq!(loopback, 200, "loopback Origin must be served");

        let u = url.clone();
        let cross = tokio::task::spawn_blocking(move || status(&u, Some("http://evil.com"), None))
            .await
            .unwrap();
        assert_eq!(cross, 403, "cross-origin Origin must be rejected");

        let u = frame_url;
        let cross_site_no_origin =
            tokio::task::spawn_blocking(move || status(&u, None, Some("cross-site")))
                .await
                .unwrap();
        assert_eq!(
            cross_site_no_origin, 403,
            "cross-site Fetch Metadata must reject a no-Origin image load before /api/frame work"
        );
    }

    /// media-route regression: project proxies/frames are served from the open
    /// project dir (the preview <video> source) — NOT the SPA fallback — and
    /// path traversal is rejected.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn proxy_route_serves_project_media_and_fences_traversal() {
        let state = AppState::new();
        // Open a project with a proxies/ file on disk.
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        let _ = dispatch(
            &state,
            "project.create",
            serde_json::json!({"name": "p", "dir": proj}),
            Actor {
                kind: ActorKind::Agent,
                name: "t".into(),
                via: "test".into(),
                request: None,
            },
        )
        .await;
        std::fs::create_dir_all(proj.join("proxies")).unwrap();
        std::fs::write(proj.join("proxies/a1.mp4"), b"\x00\x00\x00\x18ftypmp42stub").unwrap();
        std::fs::write(proj.join("secret.json"), b"{}").unwrap();

        let server = spawn_test_server(build_router(state, None)).await;

        fn get(url: &str) -> (u16, String) {
            match ureq::get(url).call() {
                Ok(mut r) => {
                    let ct = r
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let _ = r.body_mut().read_to_string();
                    (r.status().as_u16(), ct)
                }
                Err(ureq::Error::StatusCode(c)) => (c, String::new()),
                Err(e) => unreachable!("transport: {e}"),
            }
        }
        let base = server.base_url.clone();
        let (s, ct) = tokio::task::spawn_blocking({
            let u = format!("{base}/proxies/a1.mp4");
            move || get(&u)
        })
        .await
        .unwrap();
        assert_eq!(s, 200, "proxy must be served");
        assert_eq!(ct, "video/mp4", "served as video, not the SPA index.html");

        // Traversal out of the proxies dir is rejected (not 200).
        let (s2, _) = tokio::task::spawn_blocking({
            let u = format!("{base}/proxies/..%2fsecret.json");
            move || get(&u)
        })
        .await
        .unwrap();
        assert_ne!(s2, 200, "traversal must not serve project files");
    }

    #[test]
    fn parse_single_range_cases() {
        // len = 16
        assert_eq!(parse_single_range("bytes=4-9", 16), Some((4, 9)));
        assert_eq!(parse_single_range("bytes=4-", 16), Some((4, 15))); // open end → EOF
        assert_eq!(parse_single_range("bytes=-5", 16), Some((11, 15))); // suffix → last 5
        assert_eq!(parse_single_range("bytes=0-100", 16), Some((0, 15))); // end clamps to EOF
        assert_eq!(parse_single_range("bytes=4-9,20-30", 16), Some((4, 9))); // first range only
                                                                             // Unsatisfiable / malformed → None (caller maps to 416 or full body).
        assert_eq!(parse_single_range("bytes=20-30", 16), None); // start past EOF
        assert_eq!(parse_single_range("bytes=9-4", 16), None); // start > end
        assert_eq!(parse_single_range("bytes=-0", 16), None); // zero-length suffix
        assert_eq!(parse_single_range("items=0-1", 16), None); // wrong unit
        assert_eq!(parse_single_range("bytes=abc", 16), None); // garbage
        assert_eq!(parse_single_range("bytes=0-0", 0), None); // empty file
    }

    /// Rehearsal playback resolves only the server-issued ephemeral handle.
    /// It accepts byte ranges for immediate video playback, refuses a path-like
    /// capability, and becomes unavailable as soon as discard revokes it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rehearsal_route_is_opaque_range_capable_and_revocable() {
        let _guard = crate::screen_record::rehearsal::rehearsal_test_lock()
            .lock()
            .await;
        let handle = crate::screen_record::rehearsal::install_test_playback();
        let server = spawn_test_server(build_router(AppState::new(), None)).await;

        fn get_range(url: &str, range: &str) -> (u16, String, Vec<u8>) {
            match ureq::get(url).header("Range", range).call() {
                Ok(mut response) => {
                    let content_range = response
                        .headers()
                        .get("content-range")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let status = response.status().as_u16();
                    let body = response.body_mut().read_to_vec().unwrap_or_default();
                    (status, content_range, body)
                }
                Err(ureq::Error::StatusCode(status)) => (status, String::new(), Vec::new()),
                Err(error) => unreachable!("transport: {error}"),
            }
        }

        let url = format!("{}/api/recording-rehearsal/{handle}", server.base_url);
        let (status, content_range, body) = tokio::task::spawn_blocking({
            let url = url.clone();
            move || get_range(&url, "bytes=4-9")
        })
        .await
        .unwrap();
        assert_eq!(status, 206);
        assert_eq!(content_range, "bytes 4-9/16");
        assert_eq!(body, b"ftypmp");

        // A plain fetch is the native rehearsal proof's byte-backed read. It
        // must receive the entire file even when a take exceeds one range cap.
        let media = crate::screen_record::rehearsal::playback_media(&handle).unwrap();
        let mut large_take = vec![0x5a; SOURCE_CHUNK as usize + 17];
        large_take[..8].copy_from_slice(b"\x00\x00\x00\x18ftyp");
        std::fs::write(media, &large_take).unwrap();
        let (full_status, full_range, accepts_ranges, full_length, full_body) =
            tokio::task::spawn_blocking({
                let url = url.clone();
                move || {
                    let mut response = ureq::get(&url).call().unwrap();
                    let content_range = response.headers().get("content-range").is_some();
                    let accepts_ranges =
                        response.headers().get("accept-ranges").unwrap() == "bytes";
                    let content_length = response
                        .headers()
                        .get("content-length")
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .parse::<usize>()
                        .unwrap();
                    let status = response.status().as_u16();
                    let body = response.body_mut().read_to_vec().unwrap();
                    (status, content_range, accepts_ranges, content_length, body)
                }
            })
            .await
            .unwrap();
        assert_eq!(full_status, 200);
        assert!(!full_range);
        assert!(accepts_ranges);
        assert_eq!(full_length, large_take.len());
        assert_eq!(full_body, large_take);

        let (capped_status, capped_range, capped_body) = tokio::task::spawn_blocking({
            let url = url.clone();
            move || get_range(&url, "bytes=0-4194319")
        })
        .await
        .unwrap();
        assert_eq!(capped_status, 206);
        assert_eq!(
            capped_range,
            format!("bytes 0-{}/{}", SOURCE_CHUNK - 1, large_take.len())
        );
        assert_eq!(capped_body.len(), SOURCE_CHUNK as usize);

        let (unsatisfiable_status, _, _) = tokio::task::spawn_blocking({
            let url = url.clone();
            let range = format!("bytes={}-{}", large_take.len(), large_take.len() + 1);
            move || get_range(&url, &range)
        })
        .await
        .unwrap();
        assert_eq!(unsatisfiable_status, 416);

        let (bad_status, _, _) = tokio::task::spawn_blocking({
            let base = server.base_url.clone();
            move || {
                get_range(
                    &format!("{base}/api/recording-rehearsal/%2Ftmp%2Fsource.mp4"),
                    "bytes=0-1",
                )
            }
        })
        .await
        .unwrap();
        assert_eq!(
            bad_status, 404,
            "a filesystem-shaped route segment is not a capability"
        );

        crate::screen_record::rehearsal::discard(serde_json::json!({"handle": handle})).unwrap();
        let (revoked_status, _, _) =
            tokio::task::spawn_blocking(move || get_range(&url, "bytes=0-1"))
                .await
                .unwrap();
        assert_eq!(
            revoked_status, 404,
            "discard revokes the playback route immediately"
        );
    }

    /// The proxy route honors a byte range (206 + Content-Range) so the preview
    /// <video> can seek the proxy — and rejects an unsatisfiable one with 416,
    /// never a silent 200 (which WebView/WebKit treat as an Accept-Ranges
    /// protocol violation).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn proxy_route_honors_byte_range() {
        let state = AppState::new();
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        let _ = dispatch(
            &state,
            "project.create",
            serde_json::json!({"name": "p", "dir": proj}),
            Actor {
                kind: ActorKind::Agent,
                name: "t".into(),
                via: "test".into(),
                request: None,
            },
        )
        .await;
        std::fs::create_dir_all(proj.join("proxies")).unwrap();
        // 16 bytes; bytes 4..=9 spell "ftypmp".
        std::fs::write(proj.join("proxies/a1.mp4"), b"\x00\x00\x00\x18ftypmp42stub").unwrap();

        let server = spawn_test_server(build_router(state, None)).await;

        fn get_range(url: &str, range: &str) -> (u16, String, Vec<u8>) {
            match ureq::get(url).header("Range", range).call() {
                Ok(mut r) => {
                    let cr = r
                        .headers()
                        .get("content-range")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let st = r.status().as_u16();
                    let body = r.body_mut().read_to_vec().unwrap_or_default();
                    (st, cr, body)
                }
                Err(ureq::Error::StatusCode(c)) => (c, String::new(), Vec::new()),
                Err(e) => unreachable!("transport: {e}"),
            }
        }
        let base = server.base_url.clone();

        // Satisfiable range → 206 + exact Content-Range + the 6 sliced bytes.
        let (st, cr, body) = tokio::task::spawn_blocking({
            let u = format!("{base}/proxies/a1.mp4");
            move || get_range(&u, "bytes=4-9")
        })
        .await
        .unwrap();
        assert_eq!(st, 206, "a satisfiable range must be 206 Partial Content");
        assert_eq!(
            cr, "bytes 4-9/16",
            "Content-Range names the slice + total length"
        );
        assert_eq!(body, b"ftypmp", "body is exactly the requested byte slice");

        // Unsatisfiable range → 416, NOT a silent 200. (ureq surfaces a 4xx as a
        // StatusCode error without headers, so we assert the status only here —
        // the Content-Range on 416 is exercised by the handler, just not readable
        // off ureq's error path.)
        let (st2, _, _) = tokio::task::spawn_blocking({
            let u = format!("{base}/proxies/a1.mp4");
            move || get_range(&u, "bytes=900-999")
        })
        .await
        .unwrap();
        assert_eq!(
            st2, 416,
            "an unsatisfiable range must be 416, never a silent 200"
        );
    }

    /// /api/source/{asset} streams a REGISTERED asset's original
    /// source for the preview <video> when no proxy exists — fenced to the open
    /// project's asset registry (unknown id → 404), seek-capable (honors a byte
    /// range, including a suffix range so a moov-at-end mp4 can seek).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn source_route_streams_registered_asset_and_fences_unknown() {
        let state = AppState::new();
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p.cutproj");
        let _ = dispatch(
            &state,
            "project.create",
            serde_json::json!({"name": "p", "dir": proj}),
            Actor {
                kind: ActorKind::Agent,
                name: "t".into(),
                via: "test".into(),
                request: None,
            },
        )
        .await;
        // A 16-byte "source" file outside any project subdir (4..=9 spell "ftypmp",
        // 12..=15 spell "stub"); register it as asset a1 in the open project.
        let src = dir.path().join("clip.mp4");
        std::fs::write(&src, b"\x00\x00\x00\x18ftypmp42stub").unwrap();
        let still = dir.path().join("still.png");
        std::fs::write(&still, b"\x89PNG\r\n\x1a\nsource-still").unwrap();
        {
            let mut guard = state.project.write().await;
            let store = guard.as_mut().expect("project open");
            store.project.assets.insert(
                "a1".to_string(),
                cut_core::types::Asset {
                    path: src.to_string_lossy().to_string(),
                    hash: "sha256:test".into(),
                    probe: None,
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
            );
            store.project.assets.insert(
                "a2".to_string(),
                cut_core::types::Asset {
                    path: still.to_string_lossy().to_string(),
                    hash: "sha256:still".into(),
                    probe: None,
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
            );
        }

        let server = spawn_test_server(build_router(state, None)).await;

        fn get_range(url: &str, range: &str) -> (u16, String, String, Vec<u8>) {
            match ureq::get(url).header("Range", range).call() {
                Ok(mut r) => {
                    let cr = r
                        .headers()
                        .get("content-range")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let content_type = r
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let st = r.status().as_u16();
                    let body = r.body_mut().read_to_vec().unwrap_or_default();
                    (st, cr, content_type, body)
                }
                Err(ureq::Error::StatusCode(c)) => (c, String::new(), String::new(), Vec::new()),
                Err(e) => unreachable!("transport: {e}"),
            }
        }
        let base = server.base_url.clone();

        // Registered asset, satisfiable range → 206 + exact slice.
        let (st, cr, content_type, body) = tokio::task::spawn_blocking({
            let u = format!("{base}/api/source/a1");
            move || get_range(&u, "bytes=4-9")
        })
        .await
        .unwrap();
        assert_eq!(
            st, 206,
            "a satisfiable range on a registered source must be 206"
        );
        assert_eq!(
            cr, "bytes 4-9/16",
            "Content-Range names the slice + total length"
        );
        assert_eq!(
            body, b"ftypmp",
            "body is exactly the requested byte slice of the SOURCE"
        );
        assert_eq!(content_type, "video/mp4");

        // Source Monitor also uses this fenced route for registered stills.
        // Never label an image as video/mp4: WebKitGTK will refuse to decode it.
        let (st_i, cr_i, content_type_i, body_i) = tokio::task::spawn_blocking({
            let u = format!("{base}/api/source/a2");
            move || get_range(&u, "bytes=0-7")
        })
        .await
        .unwrap();
        assert_eq!(st_i, 206);
        assert_eq!(cr_i, "bytes 0-7/20");
        assert_eq!(content_type_i, "image/png");
        assert_eq!(body_i, b"\x89PNG\r\n\x1a\n");

        // Suffix range (moov-at-end seek) → last 4 bytes.
        let (st_s, cr_s, _, body_s) = tokio::task::spawn_blocking({
            let u = format!("{base}/api/source/a1");
            move || get_range(&u, "bytes=-4")
        })
        .await
        .unwrap();
        assert_eq!(st_s, 206, "a suffix range must be 206");
        assert_eq!(
            cr_s, "bytes 12-15/16",
            "suffix range resolves to the last bytes"
        );
        assert_eq!(
            body_s, b"stub",
            "suffix body is the file's tail (moov-at-end seek)"
        );

        // Unknown asset id → 404 (fenced to the registry, never an arbitrary path).
        let (st_u, _, _, _) = tokio::task::spawn_blocking({
            let u = format!("{base}/api/source/nope");
            move || get_range(&u, "bytes=0-3")
        })
        .await
        .unwrap();
        assert_eq!(
            st_u, 404,
            "an unregistered asset id must not serve any file"
        );
    }

    // ---------------------------------------------------------------------
    // Export serving (the 0.6.105 "exports outside the project" defect).
    //
    // The session output dir is process-global (one open project per cutd), so
    // the two tests that set it take this lock and clear it on the way out.
    // Serializing them also keeps them from reading each other's root while the
    // shared axum test servers are up.
    // ---------------------------------------------------------------------
    static OUTPUT_DIR_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Open a project at `dir` on a fresh AppState (the export routes resolve
    /// against the CURRENT project, so every export test needs one open).
    async fn open_project(dir: &std::path::Path) -> AppState {
        let state = AppState::new();
        let _ = dispatch(
            &state,
            "project.create",
            serde_json::json!({"name": "p", "dir": dir}),
            Actor {
                kind: ActorKind::Agent,
                name: "t".into(),
                via: "test".into(),
                request: None,
            },
        )
        .await;
        state
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn project_media_route_refuses_linked_read_root() {
        use std::os::unix::fs::symlink;

        let scratch = tempfile::tempdir().unwrap();
        let project = scratch.path().join("p.cutproj");
        let state = open_project(&project).await;
        let outside = scratch.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("private.mp4"), b"private").unwrap();
        std::fs::remove_dir(project.join("proxies")).unwrap();
        symlink(&outside, project.join("proxies")).unwrap();

        let response =
            serve_project_file(&state, "proxies", "private.mp4", &HeaderMap::new()).await;
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    /// GET returning (status, body bytes). 4xx/5xx surface through ureq as a
    /// StatusCode error with no readable body, so those come back empty.
    fn get_bytes(url: &str) -> (u16, Vec<u8>) {
        match ureq::get(url).call() {
            Ok(mut r) => {
                let st = r.status().as_u16();
                (st, r.body_mut().read_to_vec().unwrap_or_default())
            }
            Err(ureq::Error::StatusCode(c)) => (c, Vec::new()),
            Err(e) => unreachable!("transport: {e}"),
        }
    }

    async fn get_off_thread(url: String) -> (u16, Vec<u8>) {
        tokio::task::spawn_blocking(move || get_bytes(&url))
            .await
            .unwrap()
    }

    /// An export written into the folder the user chose with
    /// `project.set_output_dir` lives OUTSIDE `<project>/exports`, so the
    /// relative route can never name it and in-app playback 404'd. The absolute
    /// form serves it — fenced to the roots the engine is also allowed to WRITE
    /// into, and refusing everything else: another project's dir, the project's
    /// own non-export files, a `..` climb, a symlink escape, a relative path.
    ///
    /// The last case is the important one: an absolute path that does not exist
    /// must 404 EVEN THOUGH a same-named file sits in `<project>/exports` — the
    /// route must never answer with a different file than the one requested.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn export_file_route_serves_authorized_output_dir_only() {
        let _guard = OUTPUT_DIR_TEST_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("p.cutproj");
        let state = open_project(&proj).await;

        // Inside the project's exports subtree.
        std::fs::create_dir_all(proj.join("exports")).unwrap();
        let inside = proj.join("exports/audio.mp3");
        std::fs::write(&inside, b"INSIDE-EXPORT-BYTES").unwrap();
        // A project file that is NOT an export (must stay unreachable).
        std::fs::write(proj.join("project.json"), b"{}").unwrap();

        // The user's chosen delivery folder + a folder nobody authorized.
        let outside = tmp.path().join("Deliveries");
        std::fs::create_dir_all(&outside).unwrap();
        let fresh = outside.join("audio.mp3");
        std::fs::write(&fresh, b"OUTSIDE-EXPORT-BYTES-THAT-DIFFER").unwrap();
        let unauth = tmp.path().join("Private");
        std::fs::create_dir_all(&unauth).unwrap();
        let secret = unauth.join("secret.mp4");
        std::fs::write(&secret, b"NOT-AN-EXPORT").unwrap();
        // A symlink INSIDE the authorized folder pointing at the unauthorized
        // one: canonicalization must resolve it before the membership test.
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, outside.join("link.mp4")).unwrap();

        crate::output_paths::set_session_output_dir(Some(outside.clone()));
        let server = spawn_test_server(build_router(state, None)).await;
        let base = server.base_url.clone();
        let url = |p: &std::path::Path| {
            format!(
                "{base}/api/export-file?path={}",
                urlencoding_path(&p.display().to_string())
            )
        };

        let (st, body) = get_off_thread(url(&fresh)).await;
        assert_eq!(st, 200, "an export in the chosen output folder must serve");
        assert_eq!(
            body, b"OUTSIDE-EXPORT-BYTES-THAT-DIFFER",
            "and it must be the OUTSIDE file's bytes, not the same-named inside one"
        );

        let (st, body) = get_off_thread(url(&inside)).await;
        assert_eq!(st, 200, "the project's own exports stay reachable");
        assert_eq!(body, b"INSIDE-EXPORT-BYTES");

        let (st, _) = get_off_thread(url(&secret)).await;
        assert_eq!(st, 403, "an unauthorized absolute path must be refused");

        let (st, _) = get_off_thread(url(&proj.join("project.json"))).await;
        assert_eq!(
            st, 403,
            "the read fence is the exports SUBTREE — project files are not exports"
        );

        let climb = outside.join("../Private/secret.mp4");
        let (st, _) = get_off_thread(url(&climb)).await;
        assert_eq!(st, 403, "a `..` climb out of an authorized root is refused");

        #[cfg(unix)]
        {
            let (st, _) = get_off_thread(url(&outside.join("link.mp4"))).await;
            assert_eq!(st, 403, "a symlink escaping the authorized root is refused");
        }

        // Sibling-prefix: /…/Deliveries-evil must not pass as /…/Deliveries.
        let sibling = tmp.path().join("Deliveries-evil");
        std::fs::create_dir_all(&sibling).unwrap();
        let sibling_file = sibling.join("audio.mp3");
        std::fs::write(&sibling_file, b"SIBLING").unwrap();
        let (st, _) = get_off_thread(url(&sibling_file)).await;
        assert_eq!(
            st, 403,
            "root membership is per path COMPONENT, not a string prefix"
        );

        // NO FALLBACK: the requested file is gone, a same-named one exists in
        // <project>/exports — the answer is 404, never those other bytes.
        std::fs::remove_file(&fresh).unwrap();
        let (st, body) = get_off_thread(url(&fresh)).await;
        assert_eq!(st, 404, "a missing export must 404");
        assert!(
            body.is_empty(),
            "and must NOT be substituted by the same-named file inside the project"
        );

        let (st, _) =
            get_off_thread(format!("{base}/api/export-file?path=exports/audio.mp3")).await;
        assert_eq!(st, 400, "a relative path has no unambiguous meaning here");
        let (st, _) = get_off_thread(format!("{base}/api/export-file")).await;
        assert_eq!(st, 400, "a missing ?path= is a bad request");

        crate::output_paths::set_session_output_dir(None);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn export_routes_refuse_a_linked_project_exports_directory() {
        let _guard = OUTPUT_DIR_TEST_LOCK.lock().await;
        crate::output_paths::set_session_output_dir(None);
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("p.cutproj");
        let state = open_project(&proj).await;
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let private_export_like_file = outside.join("private.json");
        std::fs::write(&private_export_like_file, b"private host data").unwrap();
        std::os::unix::fs::symlink(&outside, proj.join("exports")).unwrap();

        let server = spawn_test_server(build_router(state, None)).await;
        let base = server.base_url.clone();
        let (status, _) = get_off_thread(format!("{base}/api/export/private.json")).await;
        assert_eq!(
            status, 403,
            "the project-relative route must not serve through a project exports link"
        );
        let absolute = urlencoding_path(&private_export_like_file.display().to_string());
        let (status, _) = get_off_thread(format!("{base}/api/export-file?path={absolute}")).await;
        assert_eq!(
            status, 403,
            "the absolute route must not retain a linked project exports target as a root"
        );
        crate::output_paths::set_session_output_dir(None);
    }

    #[tokio::test]
    async fn html_export_has_opaque_origin_and_streams_without_a_range() {
        let dir = tempfile::tempdir().unwrap();
        let html = dir.path().join("review.html");
        let file = std::fs::File::create(&html).unwrap();
        file.set_len(512 * 1024 * 1024).unwrap();
        let no_range = serve_authorized_export(html.clone(), &HeaderMap::new()).await;
        assert_eq!(no_range.status(), axum::http::StatusCode::OK);
        assert_eq!(no_range.headers()["content-length"], "536870912");
        assert!(no_range.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .starts_with("sandbox allow-scripts allow-downloads;"));

        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::RANGE, "bytes=0-3".parse().unwrap());
        let range = serve_authorized_export(html, &headers).await;
        assert_eq!(range.status(), axum::http::StatusCode::PARTIAL_CONTENT);
        assert!(range.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .starts_with("sandbox allow-scripts allow-downloads;"));
    }

    /// With an output folder configured, a BASENAME request may match different
    /// files inside and outside the project. Two different files, one name →
    /// refuse (409) and say so; never
    /// pick one. Unambiguous cases keep their exact previous behavior.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn export_route_refuses_ambiguous_relative_request() {
        let _guard = OUTPUT_DIR_TEST_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("p.cutproj");
        let state = open_project(&proj).await;
        std::fs::create_dir_all(proj.join("exports")).unwrap();
        std::fs::write(proj.join("exports/audio.mp3"), b"STALE-INSIDE").unwrap();
        std::fs::write(proj.join("exports/only-inside.mp3"), b"UNIQUE-INSIDE").unwrap();
        let outside = tmp.path().join("Deliveries");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("audio.mp3"), b"FRESH-OUTSIDE-DIFFERENT").unwrap();

        let server = spawn_test_server(build_router(state, None)).await;
        let base = server.base_url.clone();

        // No output folder configured → nothing is ambiguous, behavior unchanged.
        crate::output_paths::set_session_output_dir(None);
        let (st, body) = get_off_thread(format!("{base}/api/export/audio.mp3")).await;
        assert_eq!(st, 200, "the plain project-relative case must keep working");
        assert_eq!(body, b"STALE-INSIDE");

        // Output folder holds a DIFFERENT file of the same name → ambiguous.
        crate::output_paths::set_session_output_dir(Some(outside.clone()));
        let (st, _) = get_off_thread(format!("{base}/api/export/audio.mp3")).await;
        assert_eq!(
            st, 409,
            "two different files named audio.mp3 → refuse, never serve the stale one"
        );

        // A name that exists in only one place is still unambiguous.
        let (st, body) = get_off_thread(format!("{base}/api/export/only-inside.mp3")).await;
        assert_eq!(st, 200, "no rival candidate → serve as before");
        assert_eq!(body, b"UNIQUE-INSIDE");

        // Output folder POINTING AT the exports dir: same file through two
        // roots is not a conflict.
        crate::output_paths::set_session_output_dir(Some(proj.join("exports")));
        let (st, body) = get_off_thread(format!("{base}/api/export/audio.mp3")).await;
        assert_eq!(
            st, 200,
            "one file reachable through two roots is not ambiguous"
        );
        assert_eq!(body, b"STALE-INSIDE");

        // Traversal out of exports/ is still refused with the output dir set.
        crate::output_paths::set_session_output_dir(Some(outside.clone()));
        let (st, _) = get_off_thread(format!("{base}/api/export/..%2fproject.json")).await;
        assert_eq!(st, 400, "traversal stays refused");

        crate::output_paths::set_session_output_dir(None);
    }

    #[tokio::test]
    async fn sparse_multi_gigabyte_file_builds_a_bounded_stream_response() {
        use http_body_util::BodyExt;
        use std::io::{Seek, Write};

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("large.mp4");
        let mut file = std::fs::File::create(&path).unwrap();
        let logical_len = 5_u64 * 1024 * 1024 * 1024;
        file.set_len(logical_len).unwrap();
        file.seek(std::io::SeekFrom::Start(0)).unwrap();
        file.write_all(b"bounded-stream-prefix").unwrap();
        drop(file);

        let mut response = stream_file_response(&path, "video/mp4", true).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_LENGTH)
                .unwrap(),
            logical_len.to_string().as_str()
        );
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::ACCEPT_RANGES)
                .unwrap(),
            "bytes"
        );

        let frame = response
            .body_mut()
            .frame()
            .await
            .expect("the stream yields its first bounded frame")
            .expect("the first sparse-file read succeeds");
        let bytes = frame.data_ref().expect("the first frame contains data");
        assert!(bytes.starts_with(b"bounded-stream-prefix"));
        assert!(bytes.len() <= MEDIA_STREAM_CHUNK);
        drop(response); // Deliberately never consume the remaining multi-gigabyte body.
    }

    /// Percent-encode a filesystem path for the `?path=` query (test-local: the
    /// UI does this with encodeURIComponent).
    fn urlencoding_path(p: &str) -> String {
        p.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    }
}

/// Resolve the caller's Actor from the optional `x-cut-actor` header
/// ("kind:name:via", e.g. "human:ui:ui" — the UI client sends this so human
/// gestures are attributed HUMAN in the op log. Unknown /
/// malformed values fall back to the agent/rest default — attribution is
/// informational on a loopback-only surface, never a trust boundary.
fn actor_from_headers(headers: &HeaderMap) -> Actor {
    let default = Actor {
        kind: ActorKind::Agent,
        name: "rest".into(),
        via: "rest".into(),
        request: None,
    };
    let Some(raw) = headers.get("x-cut-actor").and_then(|v| v.to_str().ok()) else {
        return default;
    };
    let mut parts = raw.splitn(3, ':');
    let kind = match parts.next() {
        Some("agent") => ActorKind::Agent,
        Some("human") => ActorKind::Human,
        Some("system") => ActorKind::System,
        _ => return default,
    };
    Actor {
        kind,
        name: parts
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("rest")
            .to_string(),
        via: parts
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("rest")
            .to_string(),
        request: None,
    }
}

/// POST /api/verb/{name} — body = verb args JSON (defaults to {}).
/// Response: the universal envelope, HTTP 200 even on verb errors (the
/// envelope's `ok` is the contract; HTTP status only signals transport).
///
/// The body is parsed as JSON REGARDLESS of Content-Type (the JSON-body compatibility contract): the old
/// `Option<Json<…>>` extractor silently dropped the body of a bare
/// `curl -d '{…}'` (Content-Type: x-www-form-urlencoded) and dispatch then
/// reported a phantom "missing field" — the misleading error named a field,
/// not the real problem. Loopback agent surface: the body either parses as
/// JSON (used) or the error SAYS it's a body-parse problem.
async fn post_verb(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Json<cut_core::VerbResult> {
    let args = if body.is_empty() {
        Value::Object(Default::default())
    } else {
        match serde_json::from_slice::<Value>(&body) {
            Ok(v) => v,
            Err(e) => {
                return Json(cut_core::VerbResult::err(
                    CutError::new(
                        error_codes::INVALID_ARGS,
                        "request body is not valid JSON",
                        e.to_string(),
                    )
                    .with_suggested_action(
                        "send the verb args as a JSON object — the body is parsed as JSON whatever the Content-Type header says",
                    ),
                ));
            }
        }
    };
    // REST callers are agents by default; the UI announces itself per-request
    // via x-cut-actor (human:ui:ui) so the op log attributes humans correctly.
    let actor = actor_from_headers(&headers);
    Json(dispatch(&state, &name, args, actor).await)
}

/// GET /proxies/{file} — serve a per-asset proxy from the current project's
/// proxies dir (the preview <video> source). GET /frames/{file} mirrors it for
/// frame images. Dynamic (the project dir changes at runtime), path-fenced to a
/// bare filename inside the subdir — no traversal.
async fn serve_proxy(
    State(state): State<AppState>,
    Path(file): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    serve_project_file(&state, "proxies", &file, &headers).await
}

async fn serve_frame_file(
    State(state): State<AppState>,
    Path(file): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    serve_project_file(&state, "frames", &file, &headers).await
}

async fn serve_filmstrip(
    State(state): State<AppState>,
    Path(file): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    serve_project_file(&state, "filmstrip", &file, &headers).await
}

/// GET /api/export/*path — serve a file from the CURRENT project's `exports/`
/// subtree (render.bundle packs + export.* artifacts) for download/preview.
/// The path is PROJECT-RELATIVE by contract: it is resolved against
/// `<project>/exports` and nothing else, which keeps the URL portable when a
/// project folder is moved or copied (an old receipt still resolves).
/// FENCED: rejects `..`/backslashes, canonicalizes, verifies the target stays
/// inside `exports/`, and suffix-allowlists media/caption/interchange types.
///
/// AMBIGUITY REFUSAL (the silent-wrong-file half of the 0.6.105 export defect):
/// a relative request carries no information about WHICH root the caller meant.
/// When the same relative path also names a DIFFERENT existing file under
/// another authorized export root — the state a user reaches by pointing
/// "Choose default export folder…" at a folder that already holds a same-named
/// export — this route used to answer 200 with the `<project>/exports/` copy.
/// Proven byte-for-byte: a fresh outside `audio.mp3` (782,324 B) while the
/// request returned the stale inside `audio.mp3` (760,052 B), no error at all.
/// Showing the user a different file than the one they exported is worse than
/// any failure, so an ambiguous request is refused (409) and names both
/// candidates plus the unambiguous URL to use. Only the genuinely ambiguous
/// case refuses: with no second candidate, or when both candidates resolve to
/// the SAME file, behavior is exactly as before.
async fn serve_export_path(
    State(state): State<AppState>,
    Path(path): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    use axum::http::StatusCode;
    if path.is_empty() || path.contains("..") || path.contains('\\') {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
    }
    let project_dir = {
        let guard = state.project.read().await;
        match guard.as_ref() {
            Some(store) => store.dir.clone(),
            None => return (StatusCode::NOT_FOUND, "no project open").into_response(),
        }
    };
    let roots = crate::output_paths::authorized_export_read_roots(&project_dir);
    let dir = project_dir.join("exports");
    let (canon_dir, canon_path) = match (dir.canonicalize(), dir.join(&path).canonicalize()) {
        (Ok(d), Ok(p)) => (d, p),
        _ => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    if !roots.iter().any(|root| root == &canon_dir) {
        return (
            StatusCode::FORBIDDEN,
            "project exports directory is not an authorized plain local directory",
        )
            .into_response();
    }
    // Defence in depth on top of the `..` check — the canonical target must
    // stay inside the canonical exports dir (rejects symlink escapes too).
    if !canon_path.starts_with(&canon_dir) {
        return (StatusCode::BAD_REQUEST, "path escapes exports dir").into_response();
    }
    // Ambiguity refusal: does the SAME relative path name a different existing
    // file under another authorized export root? `starts_with` on the candidate
    // keeps a symlink that escapes its root from counting as a rival (it is not
    // authorized, so it must not turn a good request into a refusal either).
    for root in roots {
        if root == canon_dir {
            continue;
        }
        if let Ok(rival) = root.join(&path).canonicalize() {
            if rival != canon_path && rival.starts_with(&root) && rival.is_file() {
                return (
                    StatusCode::CONFLICT,
                    format!(
                        "ambiguous export request: '{path}' names two different files — {} and {}. \
                         Request the exact file with GET /api/export-file?path=<absolute path>.",
                        canon_path.display(),
                        rival.display()
                    ),
                )
                    .into_response();
            }
        }
    }
    serve_authorized_export(canon_path, &headers).await
}

/// GET /api/export-file?path=<ABSOLUTE path> — serve one exact export file.
///
/// Why this exists next to `/api/export/*path`: the relative form cannot name a
/// file outside `<project>/exports`, yet the engine legitimately writes exports
/// into the folder the user chose with `project.set_output_dir` ("Choose default
/// export folder…"). The UI used to fold such an absolute path into the relative
/// route, which either 404'd (path outside the fence) or — worse — resolved to a
/// same-named file inside the project and played the WRONG export. This route
/// takes the absolute path verbatim, so the request states exactly one file.
///
/// FENCED to explicitly authorized roots only
/// (`output_paths::authorized_export_read_roots`: the project's exports subtree,
/// `CUTD_OUTPUTS_DIR`, the session output dir). Canonicalization happens FIRST,
/// so `..` segments and symlinks are resolved before the membership test —
/// `starts_with` then compares whole path components, never a string prefix
/// (`/out-evil` is not inside `/out`). NO FALLBACK: a path that does not exist,
/// is not a file, or is not inside an authorized root is refused. It is never
/// retried as a name inside `<project>/exports`, because answering with a
/// different file than the one asked for is the defect this route fixes.
async fn serve_export_file(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> Response {
    use axum::http::StatusCode;
    let Some(raw) = params
        .get("path")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            "missing ?path= (absolute path of the export to serve)",
        )
            .into_response();
    };
    let requested = std::path::PathBuf::from(raw);
    if !requested.is_absolute() {
        // A relative path has no meaning here — it would have to be guessed
        // against a root, which is exactly the guessing this route removes.
        return (
            StatusCode::BAD_REQUEST,
            "path must be absolute — use GET /api/export/<relative path> for a project-relative export",
        )
            .into_response();
    }
    let project_dir = {
        let guard = state.project.read().await;
        match guard.as_ref() {
            Some(store) => store.dir.clone(),
            None => return (StatusCode::NOT_FOUND, "no project open").into_response(),
        }
    };
    let Ok(canon_path) = requested.canonicalize() else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    if !canon_path.is_file() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let roots = crate::output_paths::authorized_export_read_roots(&project_dir);
    if !roots.iter().any(|root| canon_path.starts_with(root)) {
        return (
            StatusCode::FORBIDDEN,
            "path is outside the authorized export folders (the project's exports/ subtree and the chosen output folder)",
        )
            .into_response();
    }
    serve_authorized_export(canon_path, &headers).await
}

/// Serve an export file whose path a caller-facing route has ALREADY fenced to
/// an authorized root. Owns the suffix allowlist, the range read and the
/// response construction for both export routes so the two can never drift —
/// the fencing lives in the callers, the byte-serving lives here, and nothing
/// in this function is reachable with a path the callers did not authorize.
async fn serve_authorized_export(
    canon_path: std::path::PathBuf,
    headers: &axum::http::HeaderMap,
) -> Response {
    use axum::http::StatusCode;
    let ext = canon_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ct = match ext.as_str() {
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "gif" => "image/gif",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        // Audio exports (export.audio) are downloadable too.
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "opus" | "ogg" => "audio/ogg",
        "srt" => "application/x-subrip",
        "vtt" => "text/vtt",
        kind if rh::text_type(kind).is_some() => rh::text_type(kind).unwrap(),
        "xml" | "fcpxml" => "application/xml",
        // Anything else is not a publishable artifact — refuse rather than guess.
        _ => return (StatusCode::BAD_REQUEST, "unsupported file type").into_response(),
    };
    // RANGE (a <video> previewing an exported render seeks): seek + capped chunk
    // so a large render never loads whole into RAM. HTML stays sandboxed even
    // for range requests: project files must not inherit the editor origin.
    if let Some(spec) = headers
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok())
    {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let mut f = match tokio::fs::File::open(&canon_path).await {
            Ok(f) => f,
            Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
        };
        let len = match f.metadata().await {
            Ok(m) => m.len(),
            Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
        };
        match parse_single_range(spec, len) {
            Some((start, end)) => {
                let end = end.min(start + SOURCE_CHUNK - 1);
                let slice_len = (end - start + 1) as usize;
                if f.seek(std::io::SeekFrom::Start(start)).await.is_err() {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "seek failed").into_response();
                }
                let mut buf = vec![0u8; slice_len];
                if f.read_exact(&mut buf).await.is_err() {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "read failed").into_response();
                }
                let mut response = (
                    StatusCode::PARTIAL_CONTENT,
                    [
                        (axum::http::header::CONTENT_TYPE, ct.to_string()),
                        (axum::http::header::ACCEPT_RANGES, "bytes".to_string()),
                        (
                            axum::http::header::CONTENT_RANGE,
                            format!("bytes {start}-{end}/{len}"),
                        ),
                    ],
                    buf,
                )
                    .into_response();
                if ext == "html" {
                    rh::isolate_html_export(&mut response);
                }
                return response;
            }
            None => {
                return (
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    [
                        (axum::http::header::ACCEPT_RANGES, "bytes".to_string()),
                        (axum::http::header::CONTENT_RANGE, format!("bytes */{len}")),
                    ],
                )
                    .into_response();
            }
        }
    }
    // Stream every no-Range export, including untrusted HTML, in fixed chunks.
    let mut response = stream_file_response(&canon_path, ct, true).await;
    if ext == "html" && response.status() == StatusCode::OK {
        rh::isolate_html_export(&mut response);
    }
    response
}

/// GET /api/library-blob/:file — serve a content-addressed blob from the GLOBAL
/// asset library blob store (~/.shellx-cut/library/blobs/) for thumbnails/preview.
/// Project-INDEPENDENT (the library is global). FENCED: bare filename only (no
/// separators/traversal), canonicalized inside the blobs dir, suffix-allowlisted
/// to media/image types. No-Range 200 responses stream in bounded chunks.
async fn serve_library_blob(Path(file): Path<String>) -> Response {
    use axum::http::StatusCode;
    if file.is_empty() || file.contains('/') || file.contains('\\') || file.contains("..") {
        return (StatusCode::BAD_REQUEST, "invalid file name").into_response();
    }
    let Some(dir) = crate::userdata::library_blobs_dir() else {
        return (StatusCode::NOT_FOUND, "no library").into_response();
    };
    let (canon_dir, canon_path) = match (dir.canonicalize(), dir.join(&file).canonicalize()) {
        (Ok(d), Ok(p)) => (d, p),
        _ => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    if !canon_path.starts_with(&canon_dir) {
        return (StatusCode::BAD_REQUEST, "path escapes blobs dir").into_response();
    }
    let ext = canon_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ct = match ext.as_str() {
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" | "aac" => "audio/mp4",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        // Not an allowed media type — refuse rather than guess.
        _ => return (StatusCode::BAD_REQUEST, "unsupported file type").into_response(),
    };
    stream_file_response(&canon_path, ct, false).await
}

/// GET /api/library-poster?id=<item-id>[&h=<px>] — a rendered thumbnail for a
/// GLOBAL asset Library item: a representative FRAME (video), a scaled still
/// (image) or a WAVEFORM strip (audio).
///
/// Why an id, not a path: library items are cross-project file PATHS with no
/// project asset id, so the project-scoped `/filmstrip` + `/api/frame` routes
/// (which resolve through the open project's asset registry) can't thumbnail them.
/// Rather than accept an arbitrary `?path=` — an UNFENCED arbitrary-file ffmpeg
/// primitive — the caller passes the item id and the media path is resolved from
/// the LIBRARY MANIFEST, exactly the fence `serve_source` uses for project assets
/// (the served path is never caller-controlled).
///
/// The poster is cached under `~/.shellx-cut/library/posters` keyed by (resolved
/// path, mtime, kind, height), so an unchanged source is served from disk and a
/// replaced source re-renders. Render + fs run on a blocking thread (ffmpeg is
/// synchronous). An unreadable / non-media source → 404, and the UI falls back to
/// the kind glyph.
async fn serve_library_poster(Query(params): Query<HashMap<String, String>>) -> Response {
    use axum::http::StatusCode;
    let Some(id) = params.get("id").cloned() else {
        return (StatusCode::BAD_REQUEST, "id query param required").into_response();
    };
    // Bare-token id only (defence in depth; ids are 16-hex content digests).
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return (StatusCode::BAD_REQUEST, "invalid id").into_response();
    }
    let height = params
        .get("h")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(cut_media::poster::POSTER_HEIGHT);
    match tokio::task::spawn_blocking(move || build_library_poster(&id, height)).await {
        Ok(Ok((ct, bytes))) => ([(axum::http::header::CONTENT_TYPE, ct)], bytes).into_response(),
        Ok(Err(code)) => (code, "poster unavailable").into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "poster task failed").into_response(),
    }
}

/// Blocking worker for [`serve_library_poster`]: resolve the item's media path from
/// the library manifest, pick the recipe by kind, render-or-reuse the cached image,
/// and return `(content-type, bytes)`. Errors map to an HTTP status the handler
/// passes straight through.
fn build_library_poster(
    id: &str,
    height: u32,
) -> Result<(String, Vec<u8>), axum::http::StatusCode> {
    use axum::http::StatusCode;
    use cut_media::poster::PosterKind;
    use sha2::{Digest, Sha256};
    let manifest = crate::library::load().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let item = manifest
        .items
        .iter()
        .find(|i| i.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let kind = match item.kind.as_str() {
        "video" => PosterKind::Video,
        "image" => PosterKind::Image,
        "audio" => PosterKind::Audio,
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    // Path is resolved from the manifest (linked src_path or portable blob) — the
    // SAME fence serve_source uses; never from caller input.
    let path = crate::library::item_media_path(&manifest, id).ok_or(StatusCode::NOT_FOUND)?;
    let posters_dir = crate::userdata::library_posters_dir().ok_or(StatusCode::NOT_FOUND)?;
    // Cache key: source path + mtime (a replaced/edited source busts the cache) +
    // kind + height, hashed to a fixed-length filename-safe token.
    let mtime_ns = std::fs::metadata(&path)
        .and_then(|md| md.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let ext = if kind == PosterKind::Audio {
        "png"
    } else {
        "jpg"
    };
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update([0u8]);
    hasher.update(mtime_ns.to_le_bytes());
    hasher.update([0u8]);
    hasher.update(item.kind.as_bytes());
    hasher.update([0u8]);
    hasher.update(height.to_le_bytes());
    let digest = hex::encode(hasher.finalize());
    let out = posters_dir.join(format!("{}.{ext}", &digest[..32]));
    cut_media::poster::make_poster(&path, &out, kind, height).map_err(|_| StatusCode::NOT_FOUND)?;
    let bytes = std::fs::read(&out).map_err(|_| StatusCode::NOT_FOUND)?;
    let ct = if ext == "png" {
        "image/png"
    } else {
        "image/jpeg"
    };
    Ok((ct.to_string(), bytes))
}

/// Per-request body cap so a multi-GB source never loads whole into memory: each
/// range response covers at most this many bytes (the `<video>` simply requests the
/// next range). Bounds memory per request regardless of source size.
const SOURCE_CHUNK: u64 = 4 * 1024 * 1024;

/// GET /api/recording-rehearsal/{opaque_handle} — stream one server-owned,
/// disposable MP4.  This deliberately does not reuse an export/source path
/// parameter: only the rehearsal owner can turn the opaque handle into a
/// canonical regular file beneath its temporary root.
async fn serve_rehearsal_media(
    Path(handle): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    use axum::http::StatusCode;
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let Some(path) = crate::screen_record::rehearsal::playback_media(&handle) else {
        return (StatusCode::NOT_FOUND, "rehearsal playback unavailable").into_response();
    };
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(_) => return (StatusCode::NOT_FOUND, "rehearsal playback unavailable").into_response(),
    };
    let len = match file.metadata().await {
        Ok(metadata) if metadata.is_file() && metadata.len() > 0 => metadata.len(),
        _ => return (StatusCode::NOT_FOUND, "rehearsal playback unavailable").into_response(),
    };
    let Some(spec) = headers
        .get(axum::http::header::RANGE)
        .and_then(|value| value.to_str().ok())
    else {
        // A plain fetch needs the complete take and a 200 response. The shared
        // stream helper keeps large takes out of the server heap.
        return stream_file_response(&path, "video/mp4", true).await;
    };
    let (start, end) = match parse_single_range(spec, len) {
        Some(range) => range,
        None => {
            return (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [
                    (axum::http::header::ACCEPT_RANGES, "bytes".to_string()),
                    (axum::http::header::CONTENT_RANGE, format!("bytes */{len}")),
                ],
            )
                .into_response();
        }
    };
    let end = end.min(start + SOURCE_CHUNK - 1);
    let slice_len = (end - start + 1) as usize;
    if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "rehearsal playback seek failed",
        )
            .into_response();
    }
    let mut bytes = vec![0_u8; slice_len];
    if file.read_exact(&mut bytes).await.is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "rehearsal playback read failed",
        )
            .into_response();
    }
    (
        StatusCode::PARTIAL_CONTENT,
        [
            (axum::http::header::CONTENT_TYPE, "video/mp4".to_string()),
            (axum::http::header::ACCEPT_RANGES, "bytes".to_string()),
            (
                axum::http::header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{len}"),
            ),
        ],
        bytes,
    )
        .into_response()
}

/// GET /api/source/{asset} — stream a registered asset's ORIGINAL source media for
/// the preview `<video>` when no proxy exists yet, allowing editing while
/// the proxy builds, or when proxy generation is toggled off). FENCED: the asset id
/// must resolve to an asset in the CURRENT project — the served path comes from the
/// project's own asset registry, never from caller-controlled input. SEEK + capped
/// chunk (never reads the whole file) and honors a single HTTP byte-range so the
/// `<video>` can seek; always replies 206. A source whose codec the browser can't
/// decode just fires `<video>` onError → the UI falls back to the composed poster.
async fn serve_source(
    State(state): State<AppState>,
    Path(asset): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    use axum::http::StatusCode;
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    // Resolve the asset's source path from the OPEN project (registered id only).
    let path = {
        let guard = state.project.read().await;
        match guard.as_ref().and_then(|s| s.project.assets.get(&asset)) {
            Some(a) => std::path::PathBuf::from(&a.path),
            None => return (StatusCode::NOT_FOUND, "unknown asset").into_response(),
        }
    };
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return (StatusCode::NOT_FOUND, "source unavailable").into_response(),
    };
    let len = match file.metadata().await {
        Ok(m) => m.len(),
        Err(_) => return (StatusCode::NOT_FOUND, "source unavailable").into_response(),
    };
    if len == 0 {
        return (StatusCode::NOT_FOUND, "empty source").into_response();
    }
    let ct = match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "opus" | "ogg" => "audio/ogg",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "ogv" => "video/ogg",
        // mp4/m4v + best-effort default: an undecodable codec just errors in <video>.
        _ => "video/mp4",
    };
    // Resolve the requested range (synthesize `bytes=0-` when absent), then CAP it.
    let (start, end) = match headers
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok())
    {
        Some(spec) => match parse_single_range(spec, len) {
            Some(r) => r,
            None => {
                return (
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    [
                        (axum::http::header::ACCEPT_RANGES, "bytes".to_string()),
                        (axum::http::header::CONTENT_RANGE, format!("bytes */{len}")),
                    ],
                )
                    .into_response();
            }
        },
        None => (0, len - 1),
    };
    let end = end.min(start + SOURCE_CHUNK - 1);
    let slice_len = (end - start + 1) as usize;
    if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "seek failed").into_response();
    }
    let mut buf = vec![0u8; slice_len];
    if file.read_exact(&mut buf).await.is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "read failed").into_response();
    }
    (
        StatusCode::PARTIAL_CONTENT,
        [
            (axum::http::header::CONTENT_TYPE, ct.to_string()),
            (axum::http::header::ACCEPT_RANGES, "bytes".to_string()),
            (
                axum::http::header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{len}"),
            ),
        ],
        buf,
    )
        .into_response()
}

/// Parse a SINGLE HTTP byte-range (`bytes=A-B` | `bytes=A-` | `bytes=-N`) against
/// a known content length, returning the inclusive `(start, end)` it resolves to.
/// Returns `None` for: a non-`bytes=` unit, a syntactically bad spec, OR an
/// unsatisfiable range (start past EOF / start>end) — the caller maps `None`
/// (when a Range header WAS present) to 416. Multi-range (comma list) is not
/// honored (browsers never send it for media) — we take only the first range.
fn parse_single_range(spec: &str, len: u64) -> Option<(u64, u64)> {
    let rest = spec.trim().strip_prefix("bytes=")?;
    let first = rest.split(',').next()?.trim(); // first range only
    let (a, b) = first.split_once('-')?;
    let (start, end) = if a.is_empty() {
        // suffix range: last N bytes
        let n: u64 = b.parse().ok()?;
        if n == 0 || len == 0 {
            return None;
        }
        (len.saturating_sub(n), len - 1)
    } else {
        let start: u64 = a.parse().ok()?;
        let end: u64 = if b.is_empty() {
            len.saturating_sub(1)
        } else {
            b.parse().ok()?
        };
        (start, end.min(len.saturating_sub(1)))
    };
    if len == 0 || start > end || start >= len {
        return None; // unsatisfiable
    }
    Some((start, end))
}

/// Shared fenced file server for `<project>/<subdir>/<file>`. Honors a single
/// HTTP byte-range (the preview `<video>` sends these when scrubbing the proxy)
/// — returns 206 + Content-Range for a satisfiable range, 416 for an
/// unsatisfiable one, else the full 200 body. `Accept-Ranges: bytes` is set on
/// every response, now truthfully.
async fn serve_project_file(
    state: &AppState,
    subdir: &str,
    file: &str,
    headers: &axum::http::HeaderMap,
) -> Response {
    use axum::http::StatusCode;
    // Only a bare filename — reject separators / traversal before touching disk.
    if file.is_empty() || file.contains('/') || file.contains('\\') || file.contains("..") {
        return (StatusCode::BAD_REQUEST, "invalid file name").into_response();
    }
    let project_dir = {
        let guard = state.project.read().await;
        match guard.as_ref() {
            Some(store) => store.dir.clone(),
            None => return (StatusCode::NOT_FOUND, "no project open").into_response(),
        }
    };
    let dir = match crate::output_paths::existing_plain_project_relative_dir(
        &project_dir,
        std::path::Path::new(subdir),
    ) {
        Ok(dir) => dir,
        Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    // Canonicalize both and verify the file stays inside the subdir (defence in
    // depth on top of the bare-filename check).
    let (canon_project, canon_dir, canon_path) = match (
        project_dir.canonicalize(),
        dir.canonicalize(),
        dir.join(file).canonicalize(),
    ) {
        (Ok(project), Ok(d), Ok(p)) => (project, d, p),
        _ => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    if !canon_dir.starts_with(&canon_project) || !canon_path.starts_with(&canon_dir) {
        return (StatusCode::BAD_REQUEST, "path escapes project dir").into_response();
    }
    let ct = match canon_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
    {
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    };
    // RANGE present (the preview <video> scrubbing the proxy): SEEK + a capped
    // chunk so a large proxy never loads WHOLE into RAM on every seek (it used to
    // `tokio::fs::read` the entire file then slice — a per-scrub RAM spike +
    // latency on a multi-GB proxy). Mirrors serve_source. Honor it (206) or reject
    // (416); never a silent 200 (some WebView/WebKit stacks treat that as a
    // violation against Accept-Ranges). No-range requests (the small frame /
    // filmstrip <img>, which sends no Range) take the full-body path below.
    if let Some(spec) = headers
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok())
    {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let mut f = match tokio::fs::File::open(&canon_path).await {
            Ok(f) => f,
            Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
        };
        let len = match f.metadata().await {
            Ok(m) => m.len(),
            Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
        };
        match parse_single_range(spec, len) {
            Some((start, end)) => {
                let end = end.min(start + SOURCE_CHUNK - 1);
                let slice_len = (end - start + 1) as usize;
                if f.seek(std::io::SeekFrom::Start(start)).await.is_err() {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "seek failed").into_response();
                }
                let mut buf = vec![0u8; slice_len];
                if f.read_exact(&mut buf).await.is_err() {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "read failed").into_response();
                }
                return (
                    StatusCode::PARTIAL_CONTENT,
                    [
                        (axum::http::header::CONTENT_TYPE, ct.to_string()),
                        (axum::http::header::ACCEPT_RANGES, "bytes".to_string()),
                        (
                            axum::http::header::CONTENT_RANGE,
                            format!("bytes {start}-{end}/{len}"),
                        ),
                    ],
                    buf,
                )
                    .into_response();
            }
            None => {
                return (
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    [
                        (axum::http::header::ACCEPT_RANGES, "bytes".to_string()),
                        (axum::http::header::CONTENT_RANGE, format!("bytes */{len}")),
                    ],
                )
                    .into_response();
            }
        }
    }
    // No Range header still advertises range support, but streams from disk so
    // an unexpectedly large proxy/frame/filmstrip never becomes one heap buffer.
    stream_file_response(&canon_path, ct, true).await
}

/// GET /api/state — convenience alias of project.state (server contract).
async fn get_state(State(state): State<AppState>) -> Json<cut_core::VerbResult> {
    let actor = Actor {
        kind: ActorKind::Agent,
        name: "rest".into(),
        via: "rest".into(),
        request: None,
    };
    Json(
        dispatch(
            &state,
            "project.state",
            Value::Object(Default::default()),
            actor,
        )
        .await,
    )
}

/// GET /api/verbs — the verb registry (agent discovery + UI client check).
async fn get_verbs(State(state): State<AppState>) -> Response {
    // The shared mutation controls are expanded into each args schema so REST
    // discovery and MCP tools advertise the same executable contract.
    Json(state.registry.public_json().clone()).into_response()
}

/// GET /api/agent — concise discovery payload for a fresh-machine coding agent.
/// The live API is already self-describing via /api/verbs; this endpoint tells an
/// agent where the bundled ShellX Cut skill/reference docs live in an installed
/// desktop package, without needing the source repo.
async fn get_agent_info(State(state): State<AppState>) -> Response {
    let docs_available = agent_docs_root()
        .map(|p| p.join("skill/shellx-cut/SKILL.md").is_file())
        .unwrap_or(false);
    let executable = std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "cutd".to_string());
    let addr = state.addr.read().await.clone();
    Json(serde_json::json!({
        "schema": "shellx-cut/agent-docs/2",
        "product": "ShellX Cut",
        "version": env!("CARGO_PKG_VERSION"),
        "api": {
            "rest": "POST /api/verb/{name}",
            "verbs": "/api/verbs",
            "events": "/api/events",
            "mcp": "cutd mcp proxies the running cutd serve instance"
        },
        "runtime": {
            "addr": addr,
            "executable": executable,
            "mcp_proxy": {
                "command": executable,
                "args": ["mcp"],
                "mode": "proxy",
                "authority": "the running ShellX Cut engine"
            },
            "standalone": {
                "command": executable,
                "args": ["mcp", "--standalone"],
                "advanced_only": true,
                "warning": "refused while a Cut engine is running; standalone owns separate state"
            }
        },
        "mcp_client_config": {
            "mcpServers": {
                "shellx-cut": {
                    "command": executable,
                    "args": ["mcp"]
                }
            }
        },
        "self_test": {
            "verb": "system.mcp_test",
            "read_only": true,
            "checks": ["initialize", "ping", "tools/list", "system.doctor proxy"]
        },
        "docs_available": docs_available,
        "read_first": [
            {"id": "start-here", "path": "START_HERE_FOR_AGENT.txt", "url": "/api/agent-doc/START_HERE_FOR_AGENT.txt"},
            {"id": "readme", "path": "README.md", "url": "/api/agent-doc/README.md"},
            {"id": "skill", "path": "skill/shellx-cut/SKILL.md", "url": "/api/agent-doc/skill/shellx-cut/SKILL.md"},
            {"id": "reference", "path": "skill/shellx-cut/reference.md", "url": "/api/agent-doc/skill/shellx-cut/reference.md"},
            {"id": "craft-index", "path": "skill/shellx-cut/craft/INDEX.md", "url": "/api/agent-doc/skill/shellx-cut/craft/INDEX.md"},
            {"id": "verbs", "path": "schema/verbs.json", "url": "/api/agent-doc/schema/verbs.json"},
            {"id": "features", "path": "docs/public/FEATURES.md", "url": "/api/agent-doc/docs/public/FEATURES.md"},
            {"id": "debug-api", "path": "docs/public/DEBUG_API.md", "url": "/api/agent-doc/docs/public/DEBUG_API.md"},
            {"id": "local-trust", "path": "docs/public/shellx-cut-threat-model.md", "url": "/api/agent-doc/docs/public/shellx-cut-threat-model.md"},
            {"id": "feature-surfaces", "path": "docs/public/FEATURE_SURFACE_CONTRACT.md", "url": "/api/agent-doc/docs/public/FEATURE_SURFACE_CONTRACT.md"},
            {"id": "motion-boundary", "path": "docs/public/SHELLX_MOTION_BOUNDARY.md", "url": "/api/agent-doc/docs/public/SHELLX_MOTION_BOUNDARY.md"}
        ],
        "critical_verbs": [
            "ui.state",
            "ui.open",
            "ui.screenshot",
            "debug.screenshot",
            "system.mcp_test",
            "system.doctor"
        ]
    }))
    .into_response()
}

/// GET /api/agent-doc/*path — serve only the packaged agent docs bundle. The
/// root comes from SHELLX_CUT_AGENT_DOCS_DIR in desktop builds, with a dev-repo
/// fallback for local `cutd serve`.
async fn serve_agent_doc(Path(path): Path<String>) -> Response {
    let Some(root) = agent_docs_root() else {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok": false, "error": {
                "code": "not_found",
                "message": "agent docs are not bundled with this cutd build",
                "suggested_action": "read /api/verbs for the live verb registry, or install a ShellX Cut build that bundles agent-docs"
            }})),
        )
            .into_response();
    };
    let Some(rel) = clean_agent_doc_path(&path) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": {
                "code": "invalid_args",
                "message": "invalid agent doc path"
            }})),
        )
            .into_response();
    };
    let full = root.join(rel);
    match std::fs::read(&full) {
        Ok(bytes) => {
            let content_type = match full.extension().and_then(|e| e.to_str()) {
                Some("json") => "application/json",
                Some("txt") => "text/plain; charset=utf-8",
                _ => "text/markdown; charset=utf-8",
            };
            ([(axum::http::header::CONTENT_TYPE, content_type)], bytes).into_response()
        }
        Err(_) => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok": false, "error": {
                "code": "not_found",
                "message": "agent doc not found",
                "path": path
            }})),
        )
            .into_response(),
    }
}

fn agent_docs_root() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("SHELLX_CUT_AGENT_DOCS_DIR") {
        if !p.is_empty() {
            return Some(std::path::PathBuf::from(p));
        }
    }
    let dev = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../agent-docs");
    if dev.join("skill/shellx-cut/SKILL.md").is_file() {
        return Some(dev.components().collect());
    }
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    Some(repo.components().collect())
}

fn clean_agent_doc_path(path: &str) -> Option<std::path::PathBuf> {
    let allowed = path.starts_with("skill/shellx-cut/")
        || path == "START_HERE_FOR_AGENT.txt"
        || path == "README.md"
        || path == "schema/verbs.json"
        || path == "docs/public/FEATURES.md"
        || path == "docs/public/DEBUG_API.md"
        || path == "docs/public/shellx-cut-threat-model.md"
        || path == "docs/public/JUDGE_REVIEW.md"
        || path == "docs/public/FEATURE_SURFACE_CONTRACT.md"
        || path == "docs/public/SHELLX_MOTION_BOUNDARY.md";
    if !allowed || path.ends_with('/') {
        return None;
    }
    let mut out = std::path::PathBuf::new();
    for part in std::path::Path::new(path).components() {
        match part {
            std::path::Component::Normal(p) => out.push(p),
            _ => return None,
        }
    }
    Some(out)
}

/// GET /api/frame?at_ms=N[&h=540][&compose=1] — composed-frame JPEG, raw bytes
/// (server contract). Serves the FAST scrub frame (proxy seek, scaled to
/// `h`, default 540) for the human/UI; `compose=1` forces the EXACT composed
/// frame (captions + overlays — the agent's verify eyes). The served frame's
/// path/cache is shared with the render.frame verb (dispatch::scrub_frame_bytes).
/// An `X-Cut-Frame-Fast: true|false` header reports which path served it.
async fn get_frame(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(at_ms) = params.get("at_ms").and_then(|v| v.parse::<u64>().ok()) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "ok": false,
                "error": {"code": "invalid_args", "message": "at_ms query param required",
                           "cause": "GET /api/frame?at_ms=<milliseconds>[&h=<px>][&compose=1]"}
            })),
        )
            .into_response();
    };
    // Optional preview height (px) and exact-compose flag.
    let height = params
        .get("h")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(cut_media::render::SCRUB_DEFAULT_HEIGHT);
    let compose = params
        .get("compose")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    match crate::dispatch::scrub_frame_bytes(&state, at_ms, height, compose).await {
        Ok((bytes, fast)) => (
            [
                ("content-type", "image/jpeg"),
                ("x-cut-frame-fast", if fast { "true" } else { "false" }),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => {
            // Map error class to a transport status; body is the envelope.
            let status = match e.code.as_str() {
                "unimplemented" => axum::http::StatusCode::NOT_IMPLEMENTED,
                error_codes::GUARDRAIL => axum::http::StatusCode::TOO_MANY_REQUESTS,
                _ => axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            };
            (status, Json(serde_json::json!({"ok": false, "error": e}))).into_response()
        }
    }
}

/// GET /api/events — WS upgrade; streams every Event as one JSON text frame.
/// Also accepts client→server messages:
///   {"type":"ui_hello"}                       → marks this socket as a UI client
///   {"type":"ui_mounted"}                     → first real Cut app-root mount; warms Doctor once
///   {"type":"ui_state","state":{…}}           → updates AppState::ui_state + rebroadcast
///                                               (also marks the socket as UI)
///   {"type":"screenshot_result","request_id":N,…} → resolves ui.screenshot
///   {"type":"ui_command_result","request_id":N,…} → confirms a UI command
/// Server→UI messages (only to registered UI sockets): ui_command,
/// screenshot_request — see ui_bridge.rs.
async fn ws_events(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

/// Per-connection pump: fan out bus events; absorb UI pushes; deliver
/// relayed commands to sockets registered as UI clients.
async fn handle_ws(mut socket: WebSocket, state: AppState) {
    let mut rx = state.events.subscribe();
    // Outbound relay channel — only used once this socket says it is a UI.
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut ui_client_id: Option<u64> = None;
    loop {
        tokio::select! {
            // Bus → client
            ev = rx.recv() => {
                match ev {
                    Ok(ev) => {
                        let txt = serde_json::to_string(&crate::events::wire_event(&ev)).unwrap_or_default();
                        if socket.send(Message::Text(txt)).await.is_err() {
                            break; // client gone
                        }
                    }
                    // Lagged: client missed events; it must resync via project.ops.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            // Relay → UI client (ui_command / screenshot_request frames)
            Some(cmd) = cmd_rx.recv() => {
                if socket.send(Message::Text(cmd)).await.is_err() {
                    break;
                }
            }
            // Client → server (UI registration, state pushes, screenshot replies)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        let Ok(v) = serde_json::from_str::<Value>(&txt) else { continue };
                        match v.get("type").and_then(|t| t.as_str()) {
                            Some("ui_hello") if ui_client_id.is_none() => {
                                ui_client_id = Some(state.ui_bridge.register(cmd_tx.clone()));
                            }
                            // This internal message is accepted only after the
                            // existing loopback/origin-guarded socket has
                            // registered as a UI client. It deliberately adds
                            // no verb or HTTP route to the public contract.
                            Some("ui_mounted") if ui_client_id.is_some() => {
                                if state.ui_mount_readiness.notify_mounted() {
                                    tracing::debug!("first Cut app root mounted; warming doctor");
                                }
                            }
                            Some("ui_state") => {
                                // First state push doubles as UI registration.
                                if ui_client_id.is_none() {
                                    ui_client_id = Some(state.ui_bridge.register(cmd_tx.clone()));
                                }
                                let s = v.get("state").cloned().unwrap_or(Value::Null);
                                *state.ui_state.write().await = Some(s.clone());
                                state.events.publish(Event::UiState { state: s });
                            }
                            Some("screenshot_result" | "ui_command_result") => {
                                if let Some(id) = ui_client_id {
                                    state.ui_bridge.resolve(id, v);
                                }
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => continue,
                }
            }
        }
    }
    // Drop the UI registration when the socket closes.
    if let Some(id) = ui_client_id {
        state.ui_bridge.unregister(id);
        if state.ui_bridge.client_count() == 0 {
            *state.ui_state.write().await = None;
        }
    }
}

fn should_add_default_csp(response: &Response) -> bool {
    std::env::var("SHELLX_CUT_DISABLE_CSP").as_deref() != Ok("1")
        && !response
            .headers()
            .contains_key(axum::http::header::CONTENT_SECURITY_POLICY)
}
