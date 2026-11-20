//! A helper never outlives a killed engine (Linux and macOS).
//!
//! 1. The watcher (`cutd __lifeline`) stops every process that carries the
//!    engine's mark once the engine's end of its pipe closes.
//! 2. A real serving engine starts that watcher, and a `kill -9` of the engine
//!    closes the pipe: the watcher runs and ends within seconds.
//!
//! Drives the real `cutd` binary. The Windows rule (the engine's kill-on-close
//! Job Object) is checked on a Windows machine.

#![cfg(unix)]

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MARK_VAR: &str = "SHELLX_CUT_ENGINE_RUN";

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    done()
}

#[test]
fn the_watcher_stops_marked_helpers_when_the_engine_end_closes() {
    let mark = format!("{}-it-{:x}", std::process::id(), 0xbee5_u32);
    let mut watcher = Command::new(env!("CARGO_BIN_EXE_cutd"))
        .arg("__lifeline")
        .env(MARK_VAR, &mark)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the watcher");
    // The helper is a non-Apple program that just waits (another watcher,
    // whose own pipe this test keeps open): macOS hides the environment of
    // Apple's own tools such as /bin/sleep, as it would an engine helper's
    // environment only for Apple's tools.
    let mut helper = Command::new(env!("CARGO_BIN_EXE_cutd"))
        .arg("__lifeline")
        .env(MARK_VAR, &mark)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start a marked helper");
    let mut bystander = Command::new("sleep")
        .arg("120")
        .spawn()
        .expect("start a bystander");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        watcher.try_wait().unwrap().is_none(),
        "the watcher waits while the engine lives"
    );

    // What the kernel does when the engine dies: its write end closes.
    drop(watcher.stdin.take());

    let helper_pid = helper.id() as i32;
    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            helper.try_wait(),
            Ok(Some(_))
        )),
        "the marked helper {helper_pid} was stopped"
    );
    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            watcher.try_wait(),
            Ok(Some(_))
        )),
        "the watcher ends after its work"
    );
    assert!(
        bystander.try_wait().unwrap().is_none(),
        "an unmarked process is never touched"
    );
    let _ = bystander.kill();
    let _ = bystander.wait();
}

fn free_loopback_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a temporary loopback port");
    listener.local_addr().expect("local addr").to_string()
}

fn listening(addr: &str) -> bool {
    TcpStream::connect_timeout(&addr.parse().expect("addr"), Duration::from_millis(100)).is_ok()
}

/// The pid of the engine's watcher: its child running `__lifeline`.
fn watcher_of(engine: i32) -> Option<i32> {
    let out = Command::new("pgrep")
        .args(["-P", &engine.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .filter_map(|pid| pid.parse::<i32>().ok())
        .find(|pid| {
            Command::new("ps")
                .args(["-o", "command=", "-p", &pid.to_string()])
                .output()
                .map(|out| String::from_utf8_lossy(&out.stdout).contains("__lifeline"))
                .unwrap_or(false)
        })
}

fn start_engine(root: &Path, addr: &str) -> std::process::Child {
    let state = root.join("state");
    fs::create_dir_all(&state).expect("state dir");
    let engine = Command::new(env!("CARGO_BIN_EXE_cutd"))
        .args(["serve", "--headless", "--addr", addr])
        .env("SHELLX_CUT_HOME", &state)
        .env("XDG_DATA_HOME", root.join("data"))
        .env_remove("SHELLX_CUT_PARENT_PID")
        .env_remove(MARK_VAR)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the engine");
    assert!(
        wait_until(Duration::from_secs(30), || listening(addr)),
        "the engine listens"
    );
    engine
}

#[test]
fn a_killed_engine_leaves_its_watcher_to_stop_the_helpers() {
    let root = tempfile::tempdir().expect("temp root");
    let addr = free_loopback_addr();
    let mut engine = start_engine(root.path(), &addr);
    let engine_pid = engine.id() as i32;
    let mut watcher = None;
    assert!(
        wait_until(Duration::from_secs(5), || {
            watcher = watcher_of(engine_pid);
            watcher.is_some()
        }),
        "a serving engine starts its watcher"
    );
    let watcher = watcher.unwrap();
    // A session leader of its own: a terminal hangup or a kill of the engine's
    // session or group does not reach it.
    #[cfg(target_os = "linux")]
    {
        // SAFETY: getsid only reads the session id of a pid.
        assert_eq!(
            unsafe { libc::getsid(watcher) },
            watcher,
            "the watcher leads its own session"
        );
        assert_ne!(unsafe { libc::getsid(engine_pid) }, watcher);
    }

    // SAFETY: SIGKILL to the engine this test started.
    unsafe { libc::kill(engine_pid, libc::SIGKILL) };
    let _ = engine.wait();

    assert!(
        wait_until(Duration::from_secs(8), || !alive(watcher)),
        "the watcher noticed the engine's death and finished its work"
    );
}

cut_core::isolate_test_process!();
