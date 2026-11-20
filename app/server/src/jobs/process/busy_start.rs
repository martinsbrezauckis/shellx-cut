//! busy_start.rs — the async form of `cut_media::process_start`: start a
//! tokio child, retrying only a busy program file with the same bounded
//! waits, slept on the runtime timer instead of blocking a worker thread.

use cut_media::process_start::{is_program_busy, BUSY_RETRY_DELAYS};
use std::io;
use tokio::process::{Child, Command};

/// `command.spawn()`, retried while the program file is briefly busy (a
/// just-written program; see `cut_media::process_start`). Every other error,
/// and the busy error after the last wait, is returned unchanged.
pub(crate) async fn spawn_retrying_busy(command: &mut Command) -> io::Result<Child> {
    let mut waits = BUSY_RETRY_DELAYS.iter();
    loop {
        match command.spawn() {
            Err(error) if is_program_busy(&error) => match waits.next() {
                Some(wait) => tokio::time::sleep(*wait).await,
                None => return Err(error),
            },
            started => return started,
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn a_busy_program_starts_once_its_writer_closes_and_fails_plainly_if_never() {
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("tool");
        let mut writer = std::fs::File::create(&tool).unwrap();
        writer.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let closer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            drop(writer);
        });
        let mut child = spawn_retrying_busy(&mut Command::new(&tool)).await.unwrap();
        assert!(child.wait().await.unwrap().success());
        closer.join().unwrap();

        let _held = std::fs::OpenOptions::new().write(true).open(&tool).unwrap();
        let started = Instant::now();
        let error = spawn_retrying_busy(&mut Command::new(&tool))
            .await
            .unwrap_err();
        assert!(is_program_busy(&error), "{error}");
        assert!(started.elapsed() >= Duration::from_millis(600));

        let missing = spawn_retrying_busy(&mut Command::new(dir.path().join("absent")))
            .await
            .unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }
}
