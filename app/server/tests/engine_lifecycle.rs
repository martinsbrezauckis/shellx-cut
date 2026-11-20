//! Engine lifecycle: the `engine.addr` discovery file is gone once the engine is
//! gone, whichever way the engine was told to stop, and an engine started by the
//! desktop shell does not outlive that shell.
//!
//! Drives the real `cutd` binary. Unix only: it relies on `sh`, signals and
//! reparenting; the Windows rule is checked on a Windows machine.

#![cfg(unix)]

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn free_loopback_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a temporary loopback port");
    listener.local_addr().expect("local addr").to_string()
}

fn listening(addr: &str) -> bool {
    TcpStream::connect_timeout(&addr.parse().expect("addr"), Duration::from_millis(100)).is_ok()
}

fn wait_until(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

fn discovery_text(state: &Path) -> Option<String> {
    // First line only: the address. The second line is `instance=<id>`.
    fs::read_to_string(state.join("engine.addr"))
        .ok()
        .map(|text| text.lines().next().unwrap_or("").trim().to_string())
}

fn signal(pid: i32, sig: i32) {
    // SAFETY: plain signal delivery to a process this test started.
    unsafe { libc::kill(pid, sig) };
}

/// An engine started by `sh`, which stands in for the desktop shell. When
/// `name_shell` is set the engine is told (as the real shell does) to follow it.
/// Returns the stand-in shell and the engine's pid.
fn start_engine_under_a_shell(
    root: &Path,
    addr: &str,
    name_shell: bool,
) -> (std::process::Child, i32) {
    let state = root.join("state");
    fs::create_dir_all(&state).expect("state dir");
    let pid_file = root.join("engine.pid");
    let script = format!(
        "{}\"{}\" serve --headless --addr {addr} >/dev/null 2>&1 & echo $! > \"{}\"; wait",
        if name_shell {
            "SHELLX_CUT_PARENT_PID=$$ "
        } else {
            ""
        },
        env!("CARGO_BIN_EXE_cutd"),
        pid_file.display(),
    );
    let shell = Command::new("sh")
        .arg("-c")
        .arg(script)
        .env("SHELLX_CUT_HOME", &state)
        .env("XDG_DATA_HOME", root.join("data"))
        .env_remove("SHELLX_CUT_PARENT_PID")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn stand-in shell");
    wait_until("the engine to listen", Duration::from_secs(30), || {
        listening(addr)
    });
    let pid = fs::read_to_string(&pid_file)
        .expect("engine pid file")
        .trim()
        .parse()
        .expect("engine pid");
    (shell, pid)
}

#[test]
fn the_discovery_file_is_removed_when_the_engine_is_terminated() {
    let root = tempfile::tempdir().expect("temp root");
    let addr = free_loopback_addr();
    let (mut shell, engine) = start_engine_under_a_shell(root.path(), &addr, false);
    let state = root.path().join("state");
    wait_until("engine.addr to be written", Duration::from_secs(10), || {
        discovery_text(&state).as_deref() == Some(addr.as_str())
    });

    signal(engine, libc::SIGTERM);

    wait_until(
        "the engine to stop listening",
        Duration::from_secs(15),
        || !listening(&addr),
    );
    wait_until("engine.addr to be removed", Duration::from_secs(5), || {
        discovery_text(&state).is_none()
    });
    let _ = shell.wait();
}

#[test]
fn an_engine_that_follows_its_shell_stops_and_cleans_up_when_the_shell_is_killed() {
    let root = tempfile::tempdir().expect("temp root");
    let addr = free_loopback_addr();
    let (mut shell, engine) = start_engine_under_a_shell(root.path(), &addr, true);
    let state = root.path().join("state");
    wait_until("engine.addr to be written", Duration::from_secs(10), || {
        discovery_text(&state).as_deref() == Some(addr.as_str())
    });

    // The shell dies with no chance to stop its engine (a termination signal, a
    // crash or a forced kill).
    let _ = shell.kill();
    let _ = shell.wait();

    let outcome = std::panic::catch_unwind(|| {
        wait_until(
            "the orphaned engine to stop listening",
            Duration::from_secs(15),
            || !listening(&addr),
        );
        wait_until("engine.addr to be removed", Duration::from_secs(5), || {
            discovery_text(&state).is_none()
        });
    });
    signal(engine, libc::SIGKILL);
    outcome.expect("the engine must follow its shell");
}

#[test]
fn an_engine_not_started_by_a_shell_keeps_running_when_its_parent_goes() {
    let root = tempfile::tempdir().expect("temp root");
    let addr = free_loopback_addr();
    let (mut shell, engine) = start_engine_under_a_shell(root.path(), &addr, false);
    let state = root.path().join("state");
    wait_until("engine.addr to be written", Duration::from_secs(10), || {
        discovery_text(&state).as_deref() == Some(addr.as_str())
    });

    let _ = shell.kill();
    let _ = shell.wait();
    // Longer than several watchdog polls: a standalone engine must stay up.
    std::thread::sleep(Duration::from_millis(1500));
    let still_listening = listening(&addr);
    let file_kept = discovery_text(&state).as_deref() == Some(addr.as_str());
    signal(engine, libc::SIGKILL);
    assert!(still_listening, "a standalone engine must not stop itself");
    assert!(file_kept, "its discovery file must stay while it runs");
}

/// The file names the engine instance, and the running engine reports the same
/// value, so a client can tell this engine from another program or an older
/// engine that reused the port.
#[test]
fn the_discovery_file_names_the_instance_the_engine_reports() {
    use std::io::{Read, Write};
    let root = tempfile::tempdir().expect("temp root");
    let addr = free_loopback_addr();
    let (mut shell, engine) = start_engine_under_a_shell(root.path(), &addr, false);
    let state = root.path().join("state");
    wait_until("engine.addr to be written", Duration::from_secs(10), || {
        discovery_text(&state).as_deref() == Some(addr.as_str())
    });
    let file = fs::read_to_string(state.join("engine.addr")).expect("engine.addr");
    let instance = file
        .lines()
        .find_map(|line| line.trim().strip_prefix("instance="))
        .expect("engine.addr has an instance line")
        .to_string();
    assert!(!instance.is_empty());

    let mut stream = TcpStream::connect(&addr).expect("connect");
    stream
        .write_all(
            format!("GET /api/agent HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .expect("request");
    let mut raw = String::new();
    stream.read_to_string(&mut raw).expect("response");
    signal(engine, libc::SIGTERM);
    let _ = shell.wait();
    assert!(raw.contains("\"product\":\"ShellX Cut\""), "{raw}");
    assert!(
        raw.contains(&format!("\"instance\":\"{instance}\"")),
        "/api/agent must report the instance engine.addr names: {raw}"
    );
}

cut_core::isolate_test_process!();
