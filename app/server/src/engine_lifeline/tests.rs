use super::*;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};

#[cfg(target_os = "linux")]
fn marked_sleeper(mark: &str) -> std::process::Child {
    Command::new("sleep")
        .arg("60")
        .env(ENV_ENGINE_RUN, mark)
        .stdout(Stdio::null())
        .spawn()
        .unwrap()
}

// Uses /bin/sleep as the helper: macOS hides the environment of Apple's own
// tools from other processes, so this runs on Linux; on macOS the same path is
// covered by tests/helper_lifeline.rs with a non-Apple helper.
#[cfg(target_os = "linux")]
#[test]
fn a_helper_is_found_by_the_runs_mark_and_stopped() {
    let mark = format!("{}-test-{:x}", std::process::id(), 0xa11ce_u32);
    let mut helper = marked_sleeper(&mark);
    let pid = helper.id() as i32;
    // An unmarked process with the same program is never touched.
    let mut bystander = Command::new("sleep").arg("60").spawn().unwrap();
    let mut found = Vec::new();
    for _ in 0..50 {
        found = scan::marked(ENV_ENGINE_RUN, &mark);
        if found.contains(&pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        found.contains(&pid),
        "the marked helper is found: {found:?}"
    );
    assert!(!found.contains(&(bystander.id() as i32)));
    assert_eq!(stop_marked(&mark), 1);
    let status = helper.wait().unwrap();
    assert!(!status.success(), "the helper was stopped");
    assert!(
        bystander.try_wait().unwrap().is_none(),
        "the bystander still runs"
    );
    let _ = bystander.kill();
    let _ = bystander.wait();
}

#[test]
fn a_mark_is_never_a_bare_pid_or_a_path() {
    assert!(valid_mark("4411-123:456-0000abcd"));
    assert!(!valid_mark(""));
    assert!(!valid_mark("../x"));
    assert!(!valid_mark("a b"));
    assert_eq!(stop_marked("bad mark"), 0);
}

#[test]
fn the_macos_process_arguments_yield_the_environment() {
    let mut buffer = 2i32.to_ne_bytes().to_vec();
    buffer.extend_from_slice(
        b"/usr/bin/ffmpeg\0\0\0\0ffmpeg\0-i\0A=1\0SHELLX_CUT_ENGINE_RUN=9-x\0\0\0",
    );
    let environment = scan::procargs_environment(&buffer);
    assert_eq!(
        environment,
        vec![b"A=1".to_vec(), b"SHELLX_CUT_ENGINE_RUN=9-x".to_vec()]
    );
}

/// Both the engine and its watcher were killed: the next start finds the dead
/// run's record and stops what still carries its mark, then forgets it.
// Uses /bin/sleep as the helper: macOS hides the environment of Apple's own
// tools from other processes, so this runs on Linux; on macOS the same path is
// covered by tests/helper_lifeline.rs with a non-Apple helper.
#[cfg(target_os = "linux")]
#[test]
fn the_next_start_stops_what_a_dead_run_left_running() {
    let mark = format!("{}-deadrun-{:x}", std::process::id(), 0xdead_u32);
    let mut helper = marked_sleeper(&mark);
    let mut gone = Command::new("true").spawn().unwrap();
    let dead_pid = gone.id();
    gone.wait().unwrap();
    let dir = crate::userdata::shellx_cut_home()
        .unwrap()
        .join("jobs")
        .join("running");
    std::fs::create_dir_all(&dir).unwrap();
    let record = dir.join(format!("run-{dead_pid}.json"));
    std::fs::write(
        &record,
        serde_json::json!({
            "schema": "shellx-cut/engine-run/1",
            "mark": mark,
            "owner_pid": dead_pid,
            "owner_identity": "1:2",
        })
        .to_string(),
    )
    .unwrap();
    assert!(stop_dead_runs() >= 1);
    assert!(
        !helper.wait().unwrap().success(),
        "the leftover helper was stopped"
    );
    assert!(!record.exists(), "the dead run is forgotten");
}
