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
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("immediate-grandchild.pid");
    let path = pid_file.display().to_string().replace('\'', "''");
    let script = format!(
        "$child = Start-Process cmd.exe -ArgumentList '/C ping -n 60 127.0.0.1 >NUL' -PassThru; Set-Content -NoNewline -Path '{path}' -Value $child.Id"
    );
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &script,
    ]);
    let output = super::super::command_output_with_control(
        &mut command,
        &ProcessControl::bounded(Duration::from_secs(5), || false),
        "immediate descendant fixture",
    )
    .unwrap();
    assert!(output.status.success());
    let pid = wait_for_windows_pid(&pid_file);
    for _ in 0..100 {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return;
        }
        unsafe { CloseHandle(process) };
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
