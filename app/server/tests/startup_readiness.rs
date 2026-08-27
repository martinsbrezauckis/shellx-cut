//! Startup regression: automatic FFmpeg selection must not hold the loopback
//! listener behind a slow capability probe, nor begin on a headless/API-only
//! server. The first real Cut app-root mount is the only startup warm trigger.

#![cfg(unix)]

use futures_util::SinkExt;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message};

fn free_loopback_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a temporary loopback port");
    listener
        .local_addr()
        .expect("read temporary loopback port")
        .to_string()
}

fn listener_ready(addr: &str) -> bool {
    TcpStream::connect_timeout(
        &addr.parse().expect("test address"),
        Duration::from_millis(100),
    )
    .is_ok()
}

fn spawn_headless_server_with_slow_ffmpeg(
    root: &std::path::Path,
    addr: &str,
    fake_pid: &std::path::Path,
) -> std::process::Child {
    let ffmpeg_dir = root.join("ffmpeg");
    fs::create_dir(&ffmpeg_dir).expect("create fake ffmpeg directory");
    let fake_ffmpeg = ffmpeg_dir.join("ffmpeg");
    fs::write(
        &fake_ffmpeg,
        "#!/bin/sh\nprintf '%s\\n' \"$$\" >> \"$SHELLX_CUT_TEST_FAKE_FFMPEG_PID\"\nexec sleep 8\n",
    )
    .expect("write deliberately slow fake ffmpeg");
    fs::set_permissions(&fake_ffmpeg, fs::Permissions::from_mode(0o755))
        .expect("make fake ffmpeg executable");

    Command::new(env!("CARGO_BIN_EXE_cutd"))
        .args(["serve", "--headless", "--addr", addr])
        .env("SHELLX_CUT_HOME", root.join("state"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("SHELLX_CUT_FFMPEG_DIR", &ffmpeg_dir)
        .env("SHELLX_CUT_FFMPEG_AUTO", "1")
        .env("SHELLX_CUT_TEST_FAKE_FFMPEG_PID", fake_pid)
        .env_remove("SHELLX_CUT_FFMPEG")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cutd")
}

fn stop_server(mut child: std::process::Child, fake_pid: &std::path::Path) -> String {
    let _ = child.kill();
    let _ = child.wait();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    if let Ok(pids) = fs::read_to_string(fake_pid) {
        for pid in pids.lines().filter(|pid| !pid.trim().is_empty()) {
            let _ = Command::new("kill").arg("-9").arg(pid.trim()).status();
        }
    }
    stderr
}

#[tokio::test]
async fn automatic_ffmpeg_selection_waits_for_the_first_real_ui_mount() {
    let root = tempfile::tempdir().expect("temporary startup test root");
    let fake_pid = root.path().join("fake-ffmpeg.pid");
    let addr = free_loopback_addr();
    let started = Instant::now();
    let child = spawn_headless_server_with_slow_ffmpeg(root.path(), &addr, &fake_pid);

    let deadline = Instant::now() + Duration::from_secs(3);
    let ready = loop {
        if listener_ready(&addr) {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let ready_elapsed = started.elapsed();

    // This exceeds the previous 500 ms fallback. A headless/API-only server
    // must stay idle until a real app root reports its mount over the existing
    // loopback UI WebSocket.
    tokio::time::sleep(Duration::from_millis(750)).await;
    let probe_was_deferred = !fake_pid.exists();

    let (mut ui_socket, _) = connect_async(format!("ws://{addr}/api/events"))
        .await
        .expect("connect the loopback UI WebSocket");
    ui_socket
        .send(Message::Text(r#"{"type":"ui_mounted"}"#.into()))
        .await
        .expect("send unregistered app-root message");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let probe_was_deferred_before_ui_registration = !fake_pid.exists();
    ui_socket
        .send(Message::Text(r#"{"type":"ui_hello"}"#.into()))
        .await
        .expect("register test UI socket");
    // A registered socket without its real-root event is still not enough.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let probe_was_deferred_until_mount = !fake_pid.exists();

    let app_root_mounted_at = Instant::now();
    ui_socket
        .send(Message::Text(r#"{"type":"ui_mounted"}"#.into()))
        .await
        .expect("send first app-root mount");
    ui_socket
        .send(Message::Text(r#"{"type":"ui_mounted"}"#.into()))
        .await
        .expect("send duplicate app-root mount");

    let probe_deadline = Instant::now() + Duration::from_secs(3);
    let probe_started_at = loop {
        if fake_pid.exists() {
            break Some(Instant::now());
        }
        if Instant::now() >= probe_deadline {
            break None;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let stderr = stop_server(child, &fake_pid);

    assert!(
        ready,
        "cutd did not open its loopback listener within 3 seconds: {stderr}"
    );
    assert!(
        ready_elapsed < Duration::from_secs(3),
        "automatic hardware probing blocked listener readiness",
    );
    assert!(
        probe_was_deferred,
        "a fixed delay probed before a headless/API-only server mounted Cut",
    );
    assert!(
        probe_was_deferred_before_ui_registration,
        "an unregistered WebSocket must not trigger the startup doctor",
    );
    assert!(
        probe_was_deferred_until_mount,
        "a registered UI socket without the real app-root event must not probe",
    );
    assert!(
        probe_started_at.is_some(),
        "automatic hardware probing never started after the real app-root event: {stderr}",
    );
    assert!(
        probe_started_at.expect("checked above") >= app_root_mounted_at,
        "slow FFmpeg marker must follow the real app-root event",
    );
}

#[tokio::test]
async fn explicit_doctor_request_keeps_headless_lazy_discovery_available() {
    let root = tempfile::tempdir().expect("temporary startup test root");
    let fake_pid = root.path().join("fake-ffmpeg.pid");
    let addr = free_loopback_addr();
    let child = spawn_headless_server_with_slow_ffmpeg(root.path(), &addr, &fake_pid);

    let deadline = Instant::now() + Duration::from_secs(3);
    while !listener_ready(&addr) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let listener_is_ready = listener_ready(&addr);
    tokio::time::sleep(Duration::from_millis(750)).await;
    let no_eager_probe = !fake_pid.exists();

    let mut request = TcpStream::connect(&addr).expect("connect the loopback API");
    request
        .write_all(
            b"POST /api/verb/system.doctor HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
        )
        .expect("request the lazy Doctor scan");
    let probe_deadline = Instant::now() + Duration::from_secs(3);
    let probe_started = loop {
        if fake_pid.exists() {
            break true;
        }
        if Instant::now() >= probe_deadline {
            break false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let stderr = stop_server(child, &fake_pid);

    assert!(
        listener_is_ready,
        "cutd did not open its loopback listener within 3 seconds: {stderr}"
    );
    assert!(
        no_eager_probe,
        "headless startup must not probe before an explicit Doctor request",
    );
    assert!(
        probe_started,
        "an explicit system.doctor request must retain lazy discovery: {stderr}",
    );
}
