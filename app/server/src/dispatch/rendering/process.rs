//! Server-to-media bridge for one cancellable final-render operation.

use crate::jobs::JobCancellation;
use cut_core::CutError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

// Long-form exports are legitimate, but no render may wait indefinitely. Every
// ffmpeg phase in one render shares this single deadline.
const RENDER_OPERATION_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
const FOREGROUND_FRAME_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Direct preview-frame work is outside a durable render job, but still owns
/// an ffmpeg process tree. The server shutdown signal makes the existing media
/// owner cancel and reap that tree before `cutd` exits on SIGTERM.
pub(super) fn run_server_owned_foreground_render<T>(
    shutdown: Arc<AtomicBool>,
    render: impl FnOnce() -> Result<T, CutError>,
) -> Result<T, CutError> {
    let control =
        cut_media::ffmpeg::RenderProcessControl::bounded(FOREGROUND_FRAME_TIMEOUT, move || {
            shutdown.load(Ordering::Acquire)
        });
    cut_media::ffmpeg::with_render_process_control(&control, render)
}

pub(super) fn run_owned_render<T>(
    cancellation: JobCancellation,
    render: impl FnOnce() -> Result<T, CutError>,
) -> Result<T, CutError> {
    let probe = cancellation.clone();
    let control =
        cut_media::ffmpeg::RenderProcessControl::bounded(RENDER_OPERATION_TIMEOUT, move || {
            probe.is_cancelled()
        });
    cut_media::ffmpeg::with_render_process_control(&control, render)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use cut_core::error_codes;
    use std::process::Command;
    use std::time::Duration;

    fn wait_for_pid(path: &std::path::Path) -> i32 {
        for _ in 0..100 {
            if let Ok(pid) = std::fs::read_to_string(path)
                .and_then(|text| text.trim().parse().map_err(std::io::Error::other))
            {
                return pid;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("fixture pid was not written: {}", path.display());
    }

    fn assert_gone(pid: i32) {
        for _ in 0..100 {
            let result = unsafe { libc::kill(pid, 0) };
            if result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("foreground render child {pid} survived server shutdown");
    }

    #[test]
    fn server_shutdown_reaps_direct_foreground_render_tree() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("foreground-render.pid");
        let shutdown = Arc::new(AtomicBool::new(false));
        let task = std::thread::spawn({
            let shutdown = shutdown.clone();
            let pid_file = pid_file.clone();
            move || {
                run_server_owned_foreground_render(shutdown, || {
                    let mut command = Command::new("sh");
                    command.env("CUT_TEST_PID_FILE", &pid_file);
                    command.args([
                        "-c",
                        "sleep 60 & child=$!; printf '%s' \"$child\" > \"$CUT_TEST_PID_FILE\"; wait \"$child\"",
                    ]);
                    cut_media::ffmpeg::run_bounded_command(
                        &mut command,
                        "foreground render fixture",
                    )
                    .map(|_| ())
                })
            }
        });
        let pid = wait_for_pid(&pid_file);
        shutdown.store(true, Ordering::Release);
        let error = task.join().unwrap().unwrap_err();
        assert_eq!(error.code, error_codes::RENDER_CANCELLED);
        assert_gone(pid);
    }
}
