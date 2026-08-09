use super::*;
use std::time::Duration;

#[cfg(any(unix, windows))]
use std::fs;

#[cfg(unix)]
fn tree_command(pid_file: &std::path::Path) -> Command {
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "sleep 60 & child=$!; printf '%s' \"$child\" > \"$1\"; wait \"$child\"",
        "sh",
        &pid_file.display().to_string(),
    ]);
    command
}

#[cfg(unix)]
fn wait_for_pid(path: &std::path::Path) -> i32 {
    for _ in 0..100 {
        if let Ok(pid) = fs::read_to_string(path)
            .and_then(|text| text.trim().parse().map_err(std::io::Error::other))
        {
            return pid;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("grandchild pid file was not written: {}", path.display());
}

#[cfg(unix)]
fn assert_gone(pid: i32) {
    for _ in 0..100 {
        let result = unsafe { libc::kill(pid, 0) };
        if result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("grandchild {pid} survived render process cleanup");
}

#[cfg(unix)]
#[test]
fn cancellation_reaps_the_ffmpeg_child_tree() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("grandchild.pid");
    let cancelled = Arc::new(AtomicBool::new(false));
    let probe = cancelled.clone();
    let control = RenderProcessControl::bounded(Duration::from_secs(5), move || {
        probe.load(Ordering::Acquire)
    });
    let task = std::thread::spawn({
        let mut command = tree_command(&pid_file);
        move || {
            with_render_process_control(&control, || {
                command_output_with_current_control(&mut command, "test render")
            })
        }
    });
    let pid = wait_for_pid(&pid_file);
    cancelled.store(true, Ordering::Release);
    let error = task.join().unwrap().unwrap_err();
    assert_eq!(error.code, error_codes::RENDER_CANCELLED);
    assert_gone(pid);
}

#[cfg(unix)]
#[test]
fn deadline_reaps_the_ffmpeg_child_tree() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("deadline-grandchild.pid");
    let task = std::thread::spawn({
        let mut command = tree_command(&pid_file);
        move || {
            command_output_with_control(
                &mut command,
                &RenderProcessControl::bounded(Duration::from_millis(80), || false),
                "test render",
            )
        }
    });
    let pid = wait_for_pid(&pid_file);
    let error = task.join().unwrap().unwrap_err();
    assert_eq!(error.code, error_codes::FFMPEG);
    assert_eq!(error.message, "operation timed out");
    assert_gone(pid);
}

#[cfg(unix)]
#[test]
fn bounded_probe_uses_the_active_render_deadline() {
    let started = std::time::Instant::now();
    let error = with_render_process_control(
        &RenderProcessControl::bounded(Duration::from_millis(80), || false),
        || {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 60"]);
            run_bounded_command(&mut command, "test owned probe")
        },
    )
    .unwrap_err();
    assert_eq!(error.message, "operation timed out");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
fn leader_exit_cannot_leave_a_render_grandchild_holding_pipes() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("grandchild.pid");
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "sleep 60 & child=$!; printf '%s' \"$child\" > \"$1\"; exit 0",
        "sh",
        &pid_file.display().to_string(),
    ]);
    let started = std::time::Instant::now();
    let output = command_output_with_control(
        &mut command,
        &RenderProcessControl::bounded(Duration::from_secs(5), || false),
        "test render",
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
    let output = command_output_with_control(
        &mut command,
        &RenderProcessControl::bounded(Duration::from_secs(5), || false),
        "test render",
    )
    .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout.len(), OUTPUT_CAP_BYTES);
    assert_eq!(output.stderr.len(), OUTPUT_CAP_BYTES);
}

#[cfg(unix)]
#[test]
fn unread_stdin_cannot_bypass_the_owned_deadline() {
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 60"]);
    let input = vec![b'x'; 512 * 1024];
    let started = std::time::Instant::now();
    let error = run_owned_command_with_input(
        &mut command,
        &input,
        &RenderProcessControl::bounded(Duration::from_millis(80), || false),
        "test unread stdin",
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, error_codes::FFMPEG);
    assert_eq!(error.message, "operation timed out");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
fn owned_stdin_command_streams_bounded_stderr_lines() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let progress = seen.clone();
    let observer: ProcessStderrLineObserver = Arc::new(move |line| {
        progress.lock().unwrap().push(line.to_string());
    });
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "cat >/dev/null; printf '[instruments] PROGRESS 0.5 analysis\\n' >&2; printf ok",
    ]);
    let output = run_owned_command_with_input(
        &mut command,
        br#"{\"instrument\":\"test\"}"#,
        &RenderProcessControl::bounded(Duration::from_secs(5), || false),
        "test streamed stderr",
        Some(observer),
    )
    .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"ok");
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["[instruments] PROGRESS 0.5 analysis"]
    );
}

#[cfg(windows)]
fn wait_for_windows_pid(path: &std::path::Path) -> u32 {
    for _ in 0..100 {
        if let Ok(pid) = fs::read_to_string(path)
            .and_then(|text| text.trim().parse().map_err(std::io::Error::other))
        {
            return pid;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "immediate grandchild pid file was not written: {}",
        path.display()
    );
}

#[cfg(windows)]
fn assert_windows_process_gone(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    for _ in 0..100 {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return;
        }
        unsafe { CloseHandle(process) };
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("immediate grandchild {pid} escaped the render Job Object");
}

#[cfg(windows)]
#[test]
fn suspended_job_claim_contains_an_immediate_render_grandchild() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("immediate-grandchild.pid");
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
    let output = command_output_with_control(
        &mut command,
        &RenderProcessControl::bounded(Duration::from_secs(5), || false),
        "test render",
    )
    .unwrap();
    assert!(output.status.success());
    assert_windows_process_gone(wait_for_windows_pid(&pid_file));
}
