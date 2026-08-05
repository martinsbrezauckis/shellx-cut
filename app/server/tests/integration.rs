//! integration.rs — end-to-end server test (build brief: "start server temp
//! project, POST create+edit verbs, assert ops + WS event + /api/state").
//!
//! Drives the REAL `cutd` binary (CARGO_BIN_EXE_cutd) over real TCP:
//!   1. spawn `cutd serve --headless` on a test port
//!   2. open a WS client on /api/events (tokio-tungstenite)
//!   3. POST project.create + media.import (an op) + an edit verb
//!   4. assert: op_applied arrives on WS, /api/state shows the asset,
//!      project.ops lists the op, todo-backed edit verbs return STRUCTURED
//!      errors (server stays alive), unknown verbs are rejected.
//! HTTP client is a minimal std TcpStream POST/GET (Connection: close) — no
//! client crate needed for loopback.

use futures_util::StreamExt;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Test port — NOT 6161 so a dev server can stay running while tests run.
const ADDR: &str = "127.0.0.1:16161";

/// Minimal blocking HTTP request against the test server; returns the body.
/// `content_type` None = send NO Content-Type header (the bare `curl -d`
/// shape of finding the JSON-body compatibility contract — curl then sends x-www-form-urlencoded; omitting
/// the header entirely is the stricter probe).
fn http_with_ct(
    method: &str,
    path: &str,
    body: Option<&str>,
    content_type: Option<&str>,
) -> serde_json::Value {
    let payload = body.unwrap_or_default();
    let ct = content_type
        .map(|c| format!("Content-Type: {c}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {ADDR}\r\n{ct}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let mut s = TcpStream::connect(ADDR).expect("connect test server");
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let text = String::from_utf8_lossy(&buf);
    let (_head, body) = text.split_once("\r\n\r\n").expect("http response shape");
    serde_json::from_str(body.trim()).unwrap_or_else(|e| panic!("non-JSON body ({e}): {body}"))
}

fn http(method: &str, path: &str, body: Option<&serde_json::Value>) -> serde_json::Value {
    let payload = body.map(|b| b.to_string());
    http_with_ct(method, path, payload.as_deref(), Some("application/json"))
}

fn post_verb(name: &str, args: serde_json::Value) -> serde_json::Value {
    http("POST", &format!("/api/verb/{name}"), Some(&args))
}

#[tokio::test(flavor = "multi_thread")]
async fn server_end_to_end() {
    // --- 1. spawn the real binary, wait for the port -----------------------
    // SHELLX_CUT_HOME isolates the spawned engine's app-state root: this test
    // runs the REAL cutd (no cfg(test) in that process), so without the
    // override its project.create would append a dead entry to the
    // developer's real ~/.shellx-cut/projects.json on every run (the 3.7k-
    // ghost leak.
    let state_home = tempfile::tempdir().expect("state home tempdir");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_cutd"))
        .args(["serve", "--headless", "--addr", ADDR])
        .env("SHELLX_CUT_HOME", state_home.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn cutd");
    let up = (0..50).any(|_| {
        std::thread::sleep(Duration::from_millis(100));
        TcpStream::connect(ADDR).is_ok()
    });
    assert!(up, "cutd never opened {ADDR}");

    // Everything in a closure so the child is killed even on panic.
    let result = std::panic::AssertUnwindSafe(async {
        // --- 2. WS client on /api/events (the real upgrade path) ----------
        // Plain (non-TLS) websocket scheme is correct here: the server binds
        // 127.0.0.1 only (server contract) and this is a loopback test — no TLS in
        // the path, nothing crosses a network boundary.
        let ws_url = format!("{scheme}://{ADDR}/api/events", scheme = "ws");
        let (ws, _resp) = tokio_tungstenite::connect_async(ws_url)
            .await
            .expect("ws connect");
        let (_w, mut ws_read) = ws.split();

        // --- 3. drive verbs over REST --------------------------------------
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("e2e.cutproj");
        let r = post_verb(
            "project.create",
            serde_json::json!({"name":"e2e","dir": proj}),
        );
        assert_eq!(r["ok"], true, "create: {r}");

        // media.import IS an op (the append-only operation-log contract) → op_applied must hit the WS.
        let media = tmp.path().join("clip.mp4");
        std::fs::write(&media, b"fake-bytes-for-hashing").unwrap();
        let r = post_verb(
            "media.import",
            serde_json::json!({"path": media, "rationale": "integration import"}),
        );
        assert_eq!(r["ok"], true, "import: {r}");
        assert_eq!(r["result"]["asset_id"], "a1"); // verbs.json: {asset_id, job_id}
        assert!(r["result"]["job_id"].as_str().unwrap().starts_with("job_"));

        // --- 4a. WS event assert: op_applied for media.import --------------
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut saw_op_applied = false;
        while tokio::time::Instant::now() < deadline && !saw_op_applied {
            let next = tokio::time::timeout_at(deadline, ws_read.next()).await;
            let Ok(Some(Ok(msg))) = next else { break };
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&msg.to_string()) {
                if v["type"] == "op_applied" && v["op"]["verb"] == "media.import" {
                    assert_eq!(v["op"]["rationale"], "integration import");
                    saw_op_applied = true;
                }
            }
        }
        assert!(saw_op_applied, "no op_applied event for media.import on WS");

        // --- 4b. edit verb: structured envelope, never a dead server -------
        let r = post_verb("edit.split", serde_json::json!({"track":"v1","at_ms":100}));
        if r["ok"] != true {
            // Core may still be todo!() — the contract is a STRUCTURED error.
            let code = r["error"]["code"].as_str().unwrap_or("");
            assert!(
                code == "unimplemented" || code == "not_found" || code == "invalid_args",
                "unexpected edit.split error: {r}"
            );
        }

        // --- 4c. /api/state shows the asset --------------------------------
        let s = http("GET", "/api/state", None);
        assert_eq!(s["ok"], true, "state: {s}");
        assert!(
            s["result"]["assets"]["a1"].is_object(),
            "asset missing from state: {s}"
        );

        // --- 4d. ops list shows the import op -------------------------------
        let ops = post_verb("project.ops", serde_json::json!({}));
        let arr = ops["result"]["ops"].as_array().unwrap();
        assert!(
            arr.iter().any(|o| o["verb"] == "media.import"),
            "import op not in log: {ops}"
        );

        // --- 4e. contract edges ---------------------------------------------
        // the JSON-body compatibility contract: a JSON body WITHOUT Content-Type is parsed as JSON anyway —
        // the old extractor dropped it and dispatch reported a phantom
        // "missing field 'name'" (misleading: the field WAS in the body).
        let r = http_with_ct(
            "POST",
            "/api/verb/project.checkpoint",
            Some(r#"{"name":"bare-curl"}"#),
            None,
        );
        assert_eq!(r["ok"], true, "bare body must reach the verb: {r}");
        assert_eq!(r["result"]["checkpoint"]["name"], "bare-curl");
        // x-www-form-urlencoded (what bare `curl -d` actually sends) too.
        let r = http_with_ct(
            "POST",
            "/api/verb/project.checkpoint",
            Some(r#"{"name":"curl-default-ct"}"#),
            Some("application/x-www-form-urlencoded"),
        );
        assert_eq!(r["ok"], true, "curl-default content type must work: {r}");
        // A body that ISN'T JSON gets an error naming the BODY, not a field.
        let r = http_with_ct("POST", "/api/verb/project.checkpoint", Some("name=x"), None);
        assert_eq!(r["error"]["code"], "invalid_args", "{r}");
        assert!(
            r["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("not valid JSON"),
            "error must name the body-parse problem: {r}"
        );
        // Unknown verb → not_found (registry gate).
        let r = post_verb("nope.nothing", serde_json::json!({}));
        assert_eq!(r["error"]["code"], "not_found");
        // ui.screenshot without a UI client → actionable no_ui_client.
        let r = post_verb("ui.screenshot", serde_json::json!({}));
        assert_eq!(r["error"]["code"], "no_ui_client", "{r}");
        // the output-fencing contract: render.final refuses an escaping output path.
        let r = post_verb("render.final", serde_json::json!({"path": "/tmp/evil.mp4"}));
        assert_eq!(r["error"]["code"], "invalid_args", "{r}");
        // the required-argument contract: remove_silences without aggressiveness → invalid_args.
        let r = post_verb("transcript.remove_silences", serde_json::json!({}));
        assert_eq!(r["error"]["code"], "invalid_args", "{r}");

        // --- 4f. edit.speed dispatch gates (fire before the timeline) -------
        // Out-of-range factor → invalid_args (range gate, before clip lookup).
        let r = post_verb("edit.speed", serde_json::json!({"clip":"c1","factor":10}));
        assert_eq!(
            r["error"]["code"], "invalid_args",
            "factor 10 must reject: {r}"
        );
        let r = post_verb("edit.speed", serde_json::json!({"clip":"c1","factor":0}));
        assert_eq!(
            r["error"]["code"], "invalid_args",
            "factor 0 must reject: {r}"
        );
        let r = post_verb("edit.speed", serde_json::json!({"clip":"c1","factor":-2}));
        assert_eq!(
            r["error"]["code"], "invalid_args",
            "negative factor must reject: {r}"
        );
        // Varispeed (preserve_pitch:false) is a reserved v2 effect → rejected,
        // not silently ignored.
        let r = post_verb(
            "edit.speed",
            serde_json::json!({"clip":"c1","factor":2,"preserve_pitch":false}),
        );
        assert_eq!(
            r["error"]["code"], "invalid_args",
            "varispeed must reject: {r}"
        );
        assert!(
            r["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("varispeed"),
            "pitch-gate message must name varispeed: {r}"
        );
        // H1 reject_unknown_args (additionalProperties:false): a bogus arg is
        // rejected by the schema gate before dispatch.
        let r = post_verb(
            "edit.speed",
            serde_json::json!({"clip":"c1","factor":2,"bogus":1}),
        );
        assert_eq!(
            r["error"]["code"], "invalid_args",
            "unknown arg must reject: {r}"
        );
        // A valid in-range request whose clip doesn't exist → clip not_found
        // (passed the gates, failed at the core clip lookup).
        let r = post_verb("edit.speed", serde_json::json!({"clip":"nope","factor":2}));
        assert_eq!(
            r["error"]["code"], "not_found",
            "missing clip must be not_found: {r}"
        );
    });

    // futures_util gives catch_unwind on the future via AssertUnwindSafe.
    let outcome = futures_util::FutureExt::catch_unwind(result).await;
    let _ = child.kill();
    let _ = child.wait();
    if let Err(p) = outcome {
        std::panic::resume_unwind(p);
    }
}
