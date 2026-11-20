//! The automatic FFmpeg pick is the engine's own default: an engine started by
//! hand (`cutd serve`, how agents run it) picks the same FFmpeg as one started
//! by the desktop shell, and `system.doctor` says which one and why.
//!
//! The built `cutd` is copied out of the cargo target folder so it behaves like
//! an installed engine. PATH offers a plain FFmpeg first and a hardware-capable
//! one second (stand-ins: the plain one fails every test encode, the other
//! succeeds), so the fixed search order and the automatic pick disagree.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn free_loopback_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().to_string()
}

fn stand_in(dir: &Path, encodes_work: bool) {
    fs::create_dir_all(dir).unwrap();
    let encode = if encodes_work {
        "exit 0"
    } else {
        "echo 'Unknown encoder' >&2; exit 1"
    };
    let script = format!(
        "#!/bin/sh\ncase \"$*\" in\n  *-version*) echo 'ffmpeg version 9.9-stand-in Copyright (c) the FFmpeg developers'; exit 0;;\n  *-filters*|*-encoders*) exit 0;;\nesac\n{encode}\n"
    );
    for name in ["ffmpeg", "ffprobe"] {
        fs::write(dir.join(name), &script).unwrap();
        fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o755)).unwrap();
    }
}

struct Engine {
    child: Child,
    addr: String,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(exe: &Path, root: &Path, name: &str, auto: Option<&str>, path: &str) -> Engine {
    let home = root.join(name);
    fs::create_dir_all(&home).unwrap();
    let addr = free_loopback_addr();
    let mut command = Command::new(exe);
    command
        .args(["serve", "--headless", "--addr", &addr])
        .env("SHELLX_CUT_HOME", home.join("state"))
        .env("HOME", &home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("PATH", path)
        .env_remove("SHELLX_CUT_FFMPEG")
        .env_remove("SHELLX_CUT_FFPROBE")
        .env_remove("SHELLX_CUT_FFMPEG_DIR")
        .env_remove("SHELLX_CUT_NO_HWENC")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match auto {
        Some(value) => command.env("SHELLX_CUT_FFMPEG_AUTO", value),
        None => command.env_remove("SHELLX_CUT_FFMPEG_AUTO"),
    };
    let engine = Engine {
        child: command.spawn().expect("start cutd"),
        addr,
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while TcpStream::connect(&engine.addr).is_err() {
        assert!(Instant::now() < deadline, "{name}: engine did not listen");
        std::thread::sleep(Duration::from_millis(50));
    }
    engine
}

/// `system.doctor {"refresh":true}` → the ffmpeg card's details.
fn ffmpeg_details(engine: &Engine) -> serde_json::Value {
    let body = r#"{"refresh":true}"#;
    let mut stream = TcpStream::connect(&engine.addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(180)))
        .unwrap();
    write!(
        stream,
        "POST /api/verb/system.doctor HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n\r\n{body}",
        engine.addr,
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let json = response.split_once("\r\n\r\n").expect("http body").1;
    let doctor: serde_json::Value = serde_json::from_str(json).expect("doctor json");
    doctor["result"]["cards"]
        .as_array()
        .expect("cards")
        .iter()
        .find(|card| card["id"] == "ffmpeg")
        .expect("ffmpeg card")["details"]
        .clone()
}

#[test]
fn a_hand_started_engine_picks_the_same_ffmpeg_as_one_started_by_the_shell() {
    let root = tempfile::tempdir().unwrap();
    let plain = root.path().join("plain");
    let hardware = root.path().join("hardware");
    stand_in(&plain, false);
    stand_in(&hardware, true);
    let path = format!("{}:{}:/usr/bin:/bin", plain.display(), hardware.display());

    // An "installed" engine: the same program outside the cargo target folder.
    let installed = root.path().join("install").join("cutd");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_cutd"), &installed).unwrap();

    let by_hand = ffmpeg_details(&start(&installed, root.path(), "hand", None, &path));
    let by_shell = ffmpeg_details(&start(&installed, root.path(), "shell", Some("1"), &path));
    eprintln!("hand: {by_hand:#}\nshell: {by_shell:#}");
    assert_eq!(
        by_hand["resolved"], by_shell["resolved"],
        "a hand-started engine must pick the FFmpeg the shell-started one picks"
    );
    assert_ne!(
        by_hand["resolved"],
        serde_json::json!(plain.join("ffmpeg").display().to_string()),
        "the plain build first on PATH must not win over a hardware-capable one"
    );
    assert_eq!(by_hand["selection"]["reason"], "automatic_hardware");
    assert_eq!(by_hand["selection"]["automatic"], "on");
    assert_eq!(by_shell["selection"]["automatic"], "on_by_environment");
    assert!(by_hand["selection"]["found"].as_u64().unwrap() >= 2);
    let text = by_hand["selection"]["text"].as_str().unwrap();
    assert!(text.starts_with("Picked automatically"), "{text}");

    // An explicit opt-out keeps the fixed search order, and says so.
    let off = ffmpeg_details(&start(&installed, root.path(), "off", Some("0"), &path));
    assert_eq!(off["selection"]["reason"], "automatic_off");
    assert_eq!(off["selection"]["automatic"], "off_by_environment");
    assert_ne!(off["resolved"], by_hand["resolved"]);

    // A source build (the engine in the cargo target folder) stays deterministic.
    let source = Path::new(env!("CARGO_BIN_EXE_cutd"));
    let dev = ffmpeg_details(&start(source, root.path(), "source", None, &path));
    assert_eq!(dev["selection"]["automatic"], "off_source_build");
    assert_eq!(dev["resolved"], off["resolved"]);
}

cut_core::isolate_test_process!();
