use super::*;

#[cfg(unix)]
use std::fs;

fn slow_command() -> Command {
    #[cfg(unix)]
    {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 60"]);
        command
    }
    #[cfg(windows)]
    {
        let mut command = Command::new("cmd");
        command.args(["/C", "ping -n 60 127.0.0.1 >NUL"]);
        command
    }
}

#[test]
fn cancellation_kills_and_reaps_a_blocked_child() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let probe = cancelled.clone();
    let control = ProcessControl::bounded(Duration::from_secs(2), move || {
        probe.load(Ordering::Acquire)
    });
    let mut command = slow_command();
    let mut child = ManagedChild::spawn(&mut command, control, "test child").unwrap();

    thread::sleep(Duration::from_millis(40));
    cancelled.store(true, Ordering::Release);
    let error = child.wait().unwrap_err();

    assert_eq!(error.code, "render_cancelled");
    assert!(child.reaped, "cancelled child must be waited before return");
}

#[cfg(unix)]
fn wait_for_unreaped_exit(child: &ManagedChild) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !child.exit_ready().unwrap() {
        assert!(Instant::now() < deadline, "leader did not exit in time");
        thread::sleep(Duration::from_millis(5));
    }
    // WNOWAIT must leave the exit status available and the PID reserved until
    // the owned group closes. A reaping probe would produce ECHILD here.
    assert!(!child.reaped);
    assert!(child.exit_ready().unwrap());
}

#[cfg(unix)]
#[test]
fn natural_exit_is_unreaped_until_stop_closes_the_tree() {
    let mut command = Command::new("sh");
    command.args(["-c", "exit 0"]);
    let mut child = ManagedChild::spawn(
        &mut command,
        ProcessControl::bounded(Duration::from_secs(5), || false),
        "natural exit fixture",
    )
    .unwrap();
    wait_for_unreaped_exit(&child);
    child.stop_and_reap().unwrap();
    assert!(child.tree_closed);
    assert!(child.reaped);
    assert!(child.child.lock().unwrap().wait().unwrap().success());
    child.stop_and_reap().unwrap();
}

#[cfg(unix)]
#[test]
fn natural_wait_joins_the_watcher_before_releasing_the_leader_pid() {
    let mut command = Command::new("sh");
    command.args(["-c", "exit 0"]);
    let mut child = ManagedChild::spawn(
        &mut command,
        ProcessControl::bounded(Duration::from_secs(5), || false),
        "watcher join fixture",
    )
    .unwrap();
    child.stop_watcher();
    wait_for_unreaped_exit(&child);
    let pid = child.child.lock().unwrap().id();
    let stopped = child.stopped.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    child.stopped.store(false, Ordering::Release);
    child.watcher = Some(thread::spawn(move || {
        while !stopped.load(Ordering::Acquire) {
            thread::yield_now();
        }
        // An in-flight watcher still owns the numeric group until joined.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        tx.send(result == 0 && unsafe { info.si_pid() } == pid as i32)
            .unwrap();
    }));
    assert!(child.wait().unwrap().success());
    assert!(
        rx.recv().unwrap(),
        "watcher ran after the leader was reaped"
    );
    assert!(child.reaped && child.tree_closed);
}

#[cfg(unix)]
#[test]
fn stop_closes_and_reaps_a_live_child() {
    let mut command = slow_command();
    let mut child = ManagedChild::spawn(
        &mut command,
        ProcessControl::bounded(Duration::from_secs(5), || false),
        "live stop fixture",
    )
    .unwrap();
    assert!(!child.exit_ready().unwrap());
    child.stop_and_reap().unwrap();
    assert!(child.tree_closed);
    assert!(child.reaped);
    assert!(!child.child.lock().unwrap().wait().unwrap().success());
}

#[cfg(unix)]
#[test]
fn stop_after_leader_exit_closes_a_live_descendant() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("exited-leader-grandchild.pid");
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "sleep 60 & child=$!; printf '%s' \"$child\" > \"$1\"; exit 0",
        "sh",
        &pid_file.display().to_string(),
    ]);
    let mut child = ManagedChild::spawn(
        &mut command,
        ProcessControl::bounded(Duration::from_secs(5), || false),
        "exited leader stop fixture",
    )
    .unwrap();
    let pid = wait_for_pid(&pid_file);
    wait_for_unreaped_exit(&child);
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "descendant must be live");
    child.stop_and_reap().unwrap();
    assert!(child.tree_closed);
    assert!(child.reaped);
    assert!(child.child.lock().unwrap().wait().unwrap().success());
    assert_gone(pid);
}

#[cfg(unix)]
fn wait_for_pid(path: &std::path::Path) -> i32 {
    for _ in 0..100 {
        if let Ok(pid) =
            fs::read_to_string(path).and_then(|text| text.trim().parse().map_err(io::Error::other))
        {
            return pid;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("grandchild pid file was not written: {}", path.display());
}

#[cfg(unix)]
fn assert_gone(pid: i32) {
    for _ in 0..100 {
        let result = unsafe { libc::kill(pid, 0) };
        if result == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("record-render grandchild {pid} survived process cleanup");
}

#[cfg(unix)]
#[test]
fn cancellation_reaps_an_ffmpeg_descendant_tree() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("grandchild.pid");
    let cancelled = Arc::new(AtomicBool::new(false));
    let probe = cancelled.clone();
    let control = ProcessControl::bounded(Duration::from_secs(5), move || {
        probe.load(Ordering::Acquire)
    });
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "sleep 60 & child=$!; printf '%s' \"$child\" > \"$1\"; wait \"$child\"",
        "sh",
        &pid_file.display().to_string(),
    ]);
    let wait = thread::spawn(move || {
        let mut child = ManagedChild::spawn(&mut command, control, "tree fixture").unwrap();
        let result = child.wait();
        (result, child.reaped)
    });
    let pid = wait_for_pid(&pid_file);
    cancelled.store(true, Ordering::Release);
    let (error, reaped) = wait.join().unwrap();
    assert_eq!(error.unwrap_err().code, "render_cancelled");
    assert!(reaped, "direct process must be waited before returning");
    assert_gone(pid);
}

#[cfg(unix)]
#[test]
fn deadline_reaps_an_ffmpeg_descendant_tree() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("deadline-grandchild.pid");
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "sleep 60 & child=$!; printf '%s' \"$child\" > \"$1\"; wait \"$child\"",
        "sh",
        &pid_file.display().to_string(),
    ]);
    let wait = thread::spawn(move || {
        let mut child = ManagedChild::spawn(
            &mut command,
            ProcessControl::bounded(Duration::from_millis(80), || false),
            "deadline fixture",
        )
        .unwrap();
        let result = child.wait();
        (result, child.reaped)
    });
    let pid = wait_for_pid(&pid_file);
    let (error, reaped) = wait.join().unwrap();
    assert_eq!(error.unwrap_err().code, error_codes::FFMPEG);
    assert!(reaped, "direct process must be waited before returning");
    assert_gone(pid);
}

#[cfg(unix)]
#[test]
fn leader_exit_cannot_leave_a_descendant_holding_output_pipes() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("pipe-grandchild.pid");
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "sleep 60 & child=$!; printf '%s' \"$child\" > \"$1\"; exit 0",
        "sh",
        &pid_file.display().to_string(),
    ]);
    let started = Instant::now();
    let output = super::super::command_output_with_control(
        &mut command,
        &ProcessControl::bounded(Duration::from_secs(5), || false),
        "pipe fixture",
    )
    .unwrap();
    assert!(output.status.success());
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_gone(wait_for_pid(&pid_file));
}

#[cfg(unix)]
#[test]
fn output_is_capped_while_the_process_is_drained() {
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "head -c 600000 /dev/zero; head -c 600000 /dev/zero >&2",
    ]);
    let output = super::super::command_output_with_control(
        &mut command,
        &ProcessControl::bounded(Duration::from_secs(5), || false),
        "output cap fixture",
    )
    .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout.len(), OUTPUT_CAP_BYTES);
    assert_eq!(output.stderr.len(), OUTPUT_CAP_BYTES);
}

#[cfg(windows)]
#[test]
fn suspended_job_claim_contains_an_immediate_descendant() {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    const PID_FILE: &str = "CUT_RECORD_RENDER_JOB_FIXTURE_PID_FILE";
    const DESCENDANT: &str = "CUT_RECORD_RENDER_JOB_FIXTURE_DESCENDANT";
    const TEST: &str =
        "ffmpeg::process::tests::suspended_job_claim_contains_an_immediate_descendant";

    if std::env::var_os(DESCENDANT).is_some() {
        thread::sleep(Duration::from_secs(60));
        return;
    }
    if let Some(pid_file) = std::env::var_os(PID_FILE) {
        // Keep inherited output pipes open until the owning Job closes this live child.
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(DESCENDANT, "1")
            .spawn()
            .unwrap();
        assert!(child.try_wait().unwrap().is_none());
        std::fs::write(pid_file, child.id().to_string()).unwrap();
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("immediate-grandchild.pid");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", TEST, "--nocapture"])
        .env(PID_FILE, &pid_file);
    let started = Instant::now();
    let output = super::super::command_output_with_control(
        &mut command,
        &ProcessControl::bounded(Duration::from_secs(5), || false),
        "immediate descendant fixture",
    )
    .unwrap_or_else(|error| {
        panic!(
            "record-render Job fixture failed after {:?}; descendant PID={:?}; original error={error:?}",
            started.elapsed(),
            std::fs::read_to_string(&pid_file)
        )
    });
    assert!(
        output.status.success(),
        "record-render Job fixture exited {:?} after {:?}; descendant PID={:?}; stdout={:?}; stderr={:?}",
        output.status,
        started.elapsed(),
        std::fs::read_to_string(&pid_file),
        output.stdout,
        output.stderr
    );
    let pid = wait_for_windows_pid(&pid_file);
    for _ in 0..100 {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            eprintln!(
                "record-render Job fixture: descendant PID={pid} was live before parent exit and is now gone; elapsed={:?}",
                started.elapsed()
            );
            return;
        }
        let mut exit_code = 0;
        let exited = unsafe { GetExitCodeProcess(process, &mut exit_code) } != 0
            && exit_code != STILL_ACTIVE as u32;
        unsafe { CloseHandle(process) };
        if exited {
            eprintln!(
                "record-render Job fixture: descendant PID={pid} was live before parent exit and is now exited; elapsed={:?}",
                started.elapsed()
            );
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("immediate record-render descendant {pid} escaped its Job Object");
}

#[cfg(windows)]
fn wait_for_windows_pid(path: &std::path::Path) -> u32 {
    for _ in 0..100 {
        if let Ok(pid) = std::fs::read_to_string(path)
            .and_then(|text| text.trim().parse().map_err(io::Error::other))
        {
            return pid;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "immediate descendant pid file was not written: {}",
        path.display()
    );
}
