//! Completion grace is distinct from process-stop grace.
use super::*;
use std::time::Duration;

fn exited_child() -> ManagedChild {
    #[cfg(unix)]
    let mut command = std::process::Command::new("sh");
    #[cfg(unix)]
    command.args(["-c", "exit 0"]);
    #[cfg(windows)]
    let mut command = std::process::Command::new("cmd");
    #[cfg(windows)]
    command.args(["/c", "exit 0"]);
    let control = super::super::OwnedProcessControl::bounded(Duration::from_secs(30), || false);
    let mut child = ManagedChild::spawn(&mut command, control, "late reader").unwrap();
    assert!(child.wait("late reader").unwrap().success());
    child
}

// Test hook: delay the reader's EOF/result longer than both old 250 ms waits,
// without relying on load. The child is already gone before we start this.
struct LateEof(Receiver<()>);
impl Read for LateEof {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        self.0.recv().unwrap();
        thread::sleep(Duration::from_millis(750));
        Ok(0)
    }
}

#[test]
fn exited_child_allows_late_pipe_reader_results() {
    let mut child = exited_child();
    for lines in [false, true] {
        let (tx, rx) = mpsc::sync_channel(1);
        let reader = if lines {
            read_capped_lines(LateEof(rx), 32, None)
        } else {
            read_capped(LateEof(rx), 32)
        };
        tx.send(()).unwrap();
        assert_eq!(
            finish_reader(reader, &mut child, "late reader").unwrap(),
            b""
        );
    }
}

#[test]
fn exited_child_allows_late_progress_reader_completion() {
    let mut child = exited_child();
    let (tx, rx) = mpsc::sync_channel(1);
    let progress = read_progress(LateEof(rx));
    tx.send(()).unwrap();
    finish_progress(progress, &mut child, 0, &|_| {}).unwrap();
}

#[cfg(unix)]
#[test]
fn outside_process_holding_a_pipe_is_still_a_bounded_error() {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::process::{Command, Stdio};

    let mut child = exited_child();
    for progress in [false, true] {
        let (input, output) = UnixStream::pair().unwrap();
        // Spawned independently, not in ManagedChild's process group. Eight
        // seconds bounds even a regression to an unbounded completion wait.
        let mut holder = Command::new("sleep")
            .arg("8")
            .stdout(Stdio::from(OwnedFd::from(output)))
            .spawn()
            .unwrap();
        let started = Instant::now();
        let result = if progress {
            finish_progress(read_progress(input), &mut child, 0, &|_| {})
        } else {
            finish_reader(read_capped(input, 32), &mut child, "held pipe").map(|_| ())
        };
        let elapsed = started.elapsed();
        let _ = holder.kill();
        holder.wait().unwrap();
        let error = result.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("did not close"));
        assert!(elapsed < Duration::from_secs(7), "elapsed: {elapsed:?}");
    }
}
