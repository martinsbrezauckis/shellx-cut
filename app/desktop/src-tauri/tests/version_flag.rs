// ─────────────────────────────────────────────────────────────────────────────
// ShellX Cut desktop shell — `--version` contract test.
//
// ROLE
//   Pins the ONE behaviour a version probe needs from the GUI binary: print the
//   version and exit 0, without initialising a GUI toolkit, opening a window,
//   or starting the bundled cutd engine.
//
// WHY AN INTEGRATION TEST AND NOT ONLY A UNIT TEST
//   The unit tests in `src/lib.rs` pin the argument matcher; only spawning the
//   real binary proves the answer happens BEFORE `tauri::Builder::run` touches
//   GTK. The defect this guards is exactly that ordering: with `--version`
//   falling through to the builder, a headless Linux host aborted with SIGABRT
//   (`gtk::init()` fails → tao panics → `[profile.release] panic = "abort"`),
//   and a host WITH a display launched the whole editor plus an engine and
//   never returned. Evidence: `/var/crash/_usr_bin_shellx-cut.1000.crash`,
//   Ubuntu 24.04, `ProcCmdline: /usr/bin/shellx-cut --version`, `Signal: 6`.
//
// TEST DESIGN
//   * DISPLAY / WAYLAND_DISPLAY are cleared so a regression cannot quietly
//     succeed by opening a window on a developer's desktop — headless is the
//     harshest environment for the old code path and the one that aborted.
//   * The run is bounded by a wall clock, because the pre-fix behaviour on a
//     machine with a display was to hang forever rather than exit.
//
// CALLERS: `cargo test --manifest-path app/desktop/src-tauri/Cargo.toml`.
// ─────────────────────────────────────────────────────────────────────────────

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Upper bound for answering `--version`. Printing a string needs milliseconds;
/// anything approaching this means the shell started doing GUI or engine work.
const VERSION_ANSWER_BUDGET: Duration = Duration::from_secs(20);

/// Runs the shipped binary with the given arguments and returns
/// (exit status code, stdout, stderr), failing the test if it outlives the
/// budget instead of blocking the suite forever.
fn run_shell(args: &[&str]) -> (Option<i32>, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_shellx-cut"))
        .args(args)
        // Headless: no X11 display and no Wayland socket to fall back to.
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the desktop shell binary must be spawnable");

    let started = Instant::now();
    let status = loop {
        match child.try_wait().expect("waiting on the shell must not fail") {
            Some(status) => break status,
            None if started.elapsed() >= VERSION_ANSWER_BUDGET => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "shellx-cut {args:?} did not exit within {VERSION_ANSWER_BUDGET:?} — it is \
                     starting the app instead of answering the flag"
                );
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(pipe) = child.stdout.as_mut() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(pipe) = child.stderr.as_mut() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (status.code(), stdout, stderr)
}

#[test]
fn version_flag_prints_the_version_and_exits_zero_headless() {
    for flag in ["--version", "-V"] {
        let (code, stdout, stderr) = run_shell(&[flag]);

        // `CARGO_PKG_VERSION` and tauri.conf.json's `version` are the same
        // shipping number by contract; if they ever drift this assertion is the
        // tripwire, which is the correct outcome.
        assert_eq!(
            stdout,
            format!("shellx-cut {}\n", env!("CARGO_PKG_VERSION")),
            "{flag} must print exactly one version line (stderr: {stderr})",
        );
        assert_eq!(
            code,
            Some(0),
            "{flag} must exit 0, never abort (stderr: {stderr})",
        );
        // The engine is the expensive side effect a version probe must not
        // cause; the shell prints this marker whenever it wires cutd up.
        assert!(
            !stderr.contains("engine ready at"),
            "{flag} must not start the cutd engine (stderr: {stderr})",
        );
        assert!(
            !stderr.contains("Failed to initialize gtk backend"),
            "{flag} must not reach the GTK event loop (stderr: {stderr})",
        );
    }
}
