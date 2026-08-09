use super::*;
#[cfg(any(unix, windows))]
use std::fs;
use std::time::Duration;

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
async fn wait_for_pid(path: &std::path::Path) -> i32 {
    for _ in 0..100 {
        if let Ok(contents) = fs::read_to_string(path) {
            if let Ok(pid) = contents.trim().parse() {
                return pid;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("grandchild pid file was not written: {}", path.display());
}

#[cfg(unix)]
async fn assert_gone(pid: i32) {
    for _ in 0..100 {
        let result = unsafe { libc::kill(pid, 0) };
        if result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("grandchild {pid} survived owned-process cleanup");
}

#[cfg(unix)]
#[tokio::test]
async fn cancellation_stops_and_reaps_a_child_tree() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("grandchild.pid");
    let cancellation = JobCancellation::test_active();
    let control = ProcessControl::with_cancellation(Duration::from_secs(5), cancellation.clone());
    let mut command = tree_command(&pid_file);
    let task = tokio::spawn(async move { run_owned(&mut command, None, &control).await });
    let pid = wait_for_pid(&pid_file).await;

    cancellation.request_cancel_with(JobCancellationReason::CancelledByUser);
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(
        error.termination(),
        Some(ProcessTermination::Cancelled(
            JobCancellationReason::CancelledByUser
        ))
    );
    assert_gone(pid).await;
}

#[cfg(unix)]
#[tokio::test]
async fn job_cancel_waits_for_the_owned_tree_before_reporting_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("job-grandchild.pid");
    let jobs = crate::jobs::JobManager::new(crate::events::EventBus::new());
    let job = jobs.create("process-test");
    let job_id = job.job_id.clone();
    let mut command = tree_command(&pid_file);
    jobs.spawn(&job_id, async move {
        let _ = run_owned(
            &mut command,
            None,
            &ProcessControl::for_operation(Duration::from_secs(5)),
        )
        .await;
    });
    let pid = wait_for_pid(&pid_file).await;

    assert!(jobs.abort(&job_id).await.unwrap());
    assert_gone(pid).await;
    let record = jobs.get(&job_id).unwrap();
    assert_eq!(record.outcome, Some(crate::jobs::JobOutcome::Cancelled));
    assert_eq!(
        record.outcome_reason,
        Some(crate::jobs::JobOutcomeReason::UserCancelled)
    );
}

#[tokio::test]
async fn pending_job_cancel_does_not_publish_a_terminal_outcome() {
    use std::sync::Arc;
    use tokio::sync::Notify;

    let jobs = crate::jobs::JobManager::new(crate::events::EventBus::new());
    let job = jobs.create("process-test");
    let job_id = job.job_id.clone();
    let started = Arc::new(Notify::new());
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let signal = started.clone();
    jobs.spawn(&job_id, async move {
        let _ = crate::dispatch::run_blocking("test.blocking_worker", move || {
            signal.notify_one();
            release_rx.recv().expect("test releases worker");
            Ok(())
        })
        .await;
    });
    started.notified().await;

    let error = jobs.abort(&job_id).await.unwrap_err();
    assert_eq!(error.code, "job_cancel_pending");
    let record = jobs.get(&job_id).unwrap();
    assert_eq!(record.state, crate::jobs::JobState::Queued);
    assert_eq!(record.outcome, None);

    release_tx.send(()).unwrap();
    assert!(jobs.abort(&job_id).await.unwrap());
    let record = jobs.get(&job_id).unwrap();
    assert_eq!(record.outcome, Some(crate::jobs::JobOutcome::Cancelled));
}

#[cfg(unix)]
#[tokio::test]
async fn deadline_stops_and_reaps_a_child_tree() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("grandchild.pid");
    let control = ProcessControl::with_cancellation(
        Duration::from_millis(80),
        JobCancellation::test_active(),
    );
    let mut command = tree_command(&pid_file);
    let task = tokio::spawn(async move { run_owned(&mut command, None, &control).await });
    let pid = wait_for_pid(&pid_file).await;

    let error = task.await.unwrap().unwrap_err();
    assert_eq!(
        error.termination(),
        Some(ProcessTermination::DeadlineExceeded)
    );
    assert_gone(pid).await;
}

#[cfg(unix)]
#[tokio::test]
async fn deadline_interrupts_a_blocked_stdin_write_and_reaps() {
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 60"]);
    let started = tokio::time::Instant::now();
    let payload = vec![b'x'; 4 * 1024 * 1024];
    let error = run_owned(
        &mut command,
        Some(&payload),
        &ProcessControl::with_cancellation(
            Duration::from_millis(80),
            JobCancellation::test_active(),
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.termination(),
        Some(ProcessTermination::DeadlineExceeded)
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "blocked stdin write ignored the operation deadline"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn leader_exit_cannot_leave_an_owned_grandchild_holding_pipes_open() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("grandchild.pid");
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "sleep 60 & child=$!; printf '%s' \"$child\" > \"$1\"; exit 0",
        "sh",
        &pid_file.display().to_string(),
    ]);
    let output = run_owned(
        &mut command,
        None,
        &ProcessControl::with_cancellation(Duration::from_secs(5), JobCancellation::test_active()),
    )
    .await
    .unwrap();
    let pid = wait_for_pid(&pid_file).await;
    assert!(output.status.success());
    assert_gone(pid).await;
}

#[cfg(unix)]
#[tokio::test]
async fn tree_stop_failure_still_kills_and_reaps_the_direct_child() {
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 60"]);
    tree::configure(&mut command).unwrap();
    let mut child = command.spawn().unwrap();
    let mut tree = ProcessTree::failing_for_test(&child).unwrap();

    let error = cleanup::stop_and_wait(&mut child, &mut tree)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("tree stop failed"));
    assert!(child.try_wait().unwrap().is_some(), "child was not reaped");
}

#[cfg(windows)]
async fn wait_for_windows_pid(path: &std::path::Path) -> u32 {
    for _ in 0..100 {
        if let Ok(contents) = fs::read_to_string(path) {
            if let Ok(pid) = contents.trim().parse() {
                return pid;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "immediate grandchild pid file was not written: {}",
        path.display()
    );
}

#[cfg(windows)]
async fn assert_windows_process_gone(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    for _ in 0..100 {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return;
        }
        unsafe { CloseHandle(process) };
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("immediate grandchild {pid} escaped the Windows Job Object");
}

#[cfg(windows)]
#[tokio::test]
async fn suspended_job_claim_contains_an_immediate_grandchild() {
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
    let output = run_owned(
        &mut command,
        None,
        &ProcessControl::with_cancellation(Duration::from_secs(5), JobCancellation::test_active()),
    )
    .await
    .unwrap();
    let pid = wait_for_windows_pid(&pid_file).await;
    assert!(output.status.success());
    assert_windows_process_gone(pid).await;
}

#[tokio::test]
async fn caps_retained_diagnostics_while_draining_pipes() {
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "head -c 600000 /dev/zero; head -c 600000 /dev/zero >&2",
        ]);
        command
    };
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "for /L %i in (1,1,6000) do @echo 0123456789& for /L %i in (1,1,6000) do @echo 0123456789 1>&2"]);
        command
    };
    #[cfg(not(any(unix, windows)))]
    return;

    let output = run_owned(
        &mut command,
        None,
        &ProcessControl::with_cancellation(Duration::from_secs(5), JobCancellation::test_active()),
    )
    .await
    .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout.len(), OUTPUT_CAP_BYTES);
    assert_eq!(output.stderr.len(), OUTPUT_CAP_BYTES);
    assert!(output.stdout_truncated);
    assert!(output.stderr_truncated);
}
