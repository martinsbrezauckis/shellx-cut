//! GStreamer capture segment plus a private first-screen-frame observer.
//!
//! The normal X11 path uses `gst-launch-1.0`, which cannot surface a Rust frame
//! callback. A tee therefore reduces each delivered PipeWire raw frame to one
//! gray pixel and writes it to a private stdout pipe. Reading a byte proves a
//! native screen buffer reached this active pipeline; process start and output
//! file growth are deliberately not used as readiness evidence.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use record_core::Result;
use tokio::io::AsyncReadExt;

use crate::linux_runtime::{cap_err, gst_bin};
use crate::CaptureReadiness;

/// Keep portal remotes away from stdin/stdout/stderr, which belong to the
/// recorder protocol. `pipewiresrc` accepts any inherited descriptor number.
const FIRST_SAFE_CHILD_FD: RawFd = 3;

/// A close-on-exec duplicate of a portal-owned PipeWire remote.
///
/// D-Bus-delivered descriptors are normally close-on-exec. That is correct for
/// Cut itself, but a separate `gst-launch-1.0` process needs one explicit
/// exception. The original and duplicate both remain close-on-exec in Cut. The
/// child flips the duplicate immediately before its own `exec`, eliminating a
/// parent-process window in which an unrelated concurrent spawn could inherit
/// the remote. It is dropped in the parent immediately after a successful
/// spawn; the child retains its own descriptor through `exec`.
struct GStreamerPortalRemote {
    fd: OwnedFd,
}

impl GStreamerPortalRemote {
    fn raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

fn descriptor_flags(fd: RawFd) -> io::Result<i32> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(flags)
    }
}

fn set_descriptor_flags(fd: RawFd, flags: i32) -> io::Result<()> {
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn inherit_portal_remote(remote: &OwnedFd) -> io::Result<GStreamerPortalRemote> {
    // Do not leak the portal's original descriptor into the exec'd process. It
    // is still owned by Cut and is only the source for the dedicated duplicate.
    let original_flags = descriptor_flags(remote.as_raw_fd())?;
    set_descriptor_flags(remote.as_raw_fd(), original_flags | libc::FD_CLOEXEC)?;

    // F_DUPFD_CLOEXEC gives the parent a private duplicate with a predictable
    // lower bound and no race with another thread reusing a closed descriptor.
    let duplicated = unsafe {
        libc::fcntl(
            remote.as_raw_fd(),
            libc::F_DUPFD_CLOEXEC,
            FIRST_SAFE_CHILD_FD,
        )
    };
    if duplicated < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(duplicated) };

    Ok(GStreamerPortalRemote { fd })
}

/// Runs in the just-forked GStreamer child, immediately before `exec`.
///
/// On the success path it only performs `fcntl` syscalls: no allocation, no
/// locks, and no access to parent-owned runtime state. That makes the one
/// intentional descriptor inheritance safe even while Cut has other threads.
fn expose_remote_in_child(fd: RawFd) -> io::Result<()> {
    let flags = descriptor_flags(fd)?;
    set_descriptor_flags(fd, flags & !libc::FD_CLOEXEC)
}

fn bind_portal_remote_to_child(command: &mut tokio::process::Command, remote_fd: RawFd) {
    // SAFETY: the closure captures only a raw fd and only invokes the two
    // async-signal-safe `fcntl` syscalls on the successful path. It touches no
    // Rust allocation or shared state after `fork`.
    unsafe {
        command.pre_exec(move || expose_remote_in_child(remote_fd));
    }
}

fn command_args(node: u32, remote_fd: RawFd, segment_path: &str) -> Vec<String> {
    vec![
        "pipewiresrc".into(),
        format!("path={node}"),
        format!("fd={remote_fd}"),
        "!".into(),
        "videoconvert".into(),
        "!".into(),
        "tee".into(),
        "name=screen_frames".into(),
        // Main encoder/output branch: unchanged capture artifact semantics.
        "screen_frames.".into(),
        "!".into(),
        "queue".into(),
        "x264enc".into(),
        "speed-preset=ultrafast".into(),
        "tune=zerolatency".into(),
        "!".into(),
        "mp4mux".into(),
        "!".into(),
        "filesink".into(),
        format!("location={segment_path}"),
        // Observer branch: a raw frame has no container/header, so one stdout
        // byte is evidence of a delivered native video buffer, not startup.
        "screen_frames.".into(),
        "!".into(),
        "queue".into(),
        "leaky=downstream".into(),
        "max-size-buffers=1".into(),
        "!".into(),
        "videoscale".into(),
        "!".into(),
        "videoconvert".into(),
        "!".into(),
        "video/x-raw,format=GRAY8,width=1,height=1".into(),
        "!".into(),
        "fdsink".into(),
        "fd=1".into(),
        "sync=false".into(),
    ]
}

/// Run exactly one GStreamer segment and retain only the first-frame admission
/// proof in memory. The caller owns checkpoint publication and all terminal
/// lifecycle decisions.
pub(crate) async fn capture_segment(
    portal_remote: OwnedFd,
    node: u32,
    segment_path: &str,
    interval_end: u64,
    start: Instant,
    stop: Arc<AtomicBool>,
    readiness: Option<CaptureReadiness>,
) -> Result<(u64, u64)> {
    let child_remote = inherit_portal_remote(&portal_remote)
        .map_err(|error| cap_err("prepare portal PipeWire remote for GStreamer", error))?;
    let remote_fd = child_remote.raw_fd();
    let mut command = tokio::process::Command::new(gst_bin());
    command
        .arg("-e")
        .args(command_args(node, remote_fd, segment_path))
        .stdout(Stdio::piped());
    bind_portal_remote_to_child(&mut command, remote_fd);
    let mut child = command
        .spawn()
        .map_err(|error| cap_err("spawn gst-launch-1.0", error))?;
    // `spawn` has completed the fork/exec handoff. The child holds the only
    // intentionally inheritable duplicate; retaining either parent descriptor
    // would keep a stale remote alive across a segment rotation or error path.
    drop(child_remote);
    drop(portal_remote);
    let Some(mut frame_bytes) = child.stdout.take() else {
        let _ = child.start_kill();
        let _ = child.wait().await;
        return Err(cap_err(
            "observe gst screen frames",
            "gst stdout was not configured for the frame observer",
        ));
    };
    let frame_observer = tokio::spawn(async move {
        let mut buffer = [0u8; 256];
        loop {
            match frame_bytes.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    if let Some(readiness) = readiness.as_ref() {
                        readiness.mark_first_screen_frame_delivered();
                    }
                }
            }
        }
    });

    // The shared clock remains a timeline origin only. It does not satisfy the
    // separate first-frame readiness contract.
    let capture_start_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    while !stop.load(Ordering::Relaxed) && start.elapsed() < Duration::from_millis(interval_end) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let capture_end_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    if let Some(pid) = child.id() {
        let _ = Command::new("kill")
            .args(["-INT", &pid.to_string()])
            .status();
    }
    let status = child
        .wait()
        .await
        .map_err(|error| cap_err("wait gst", error))?;
    let _ = frame_observer.await;
    if !status.success() {
        return Err(cap_err(
            "gst screen capture failed",
            format!("gst exit {status}"),
        ));
    }
    Ok((capture_start_ms, capture_end_ms))
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::os::fd::{AsRawFd, OwnedFd};

    use super::{
        bind_portal_remote_to_child, command_args, descriptor_flags, expose_remote_in_child,
        inherit_portal_remote,
    };

    #[test]
    fn observer_branch_requires_a_raw_screen_buffer_before_it_can_emit_a_byte() {
        let owned = command_args(17, 41, "/tmp/capture/staging.mp4");
        let args: Vec<&str> = owned.iter().map(String::as_str).collect();
        assert!(args.contains(&"path=17"));
        assert!(args.contains(&"fd=41"));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["tee", "name=screen_frames"]));
        assert!(args.windows(11).any(|branch| {
            branch
                == [
                    "screen_frames.",
                    "!",
                    "queue",
                    "leaky=downstream",
                    "max-size-buffers=1",
                    "!",
                    "videoscale",
                    "!",
                    "videoconvert",
                    "!",
                    "video/x-raw,format=GRAY8,width=1,height=1",
                ]
        }));
        assert!(args.contains(&"fdsink"));
        assert!(args.contains(&"fd=1"));
        assert!(args.contains(&"sync=false"));
        assert!(args.contains(&"location=/tmp/capture/staging.mp4"));
    }

    #[test]
    fn portal_remote_handoff_keeps_both_parent_descriptors_close_on_exec() {
        let file = File::open("/dev/null").expect("open harmless descriptor");
        let source: OwnedFd = file.into();
        let child = inherit_portal_remote(&source).expect("duplicate portal remote");

        assert_ne!(source.as_raw_fd(), child.raw_fd());
        assert_ne!(
            descriptor_flags(source.as_raw_fd()).expect("source flags") & libc::FD_CLOEXEC,
            0,
            "Cut's portal descriptor must not leak into gst-launch"
        );
        assert_eq!(
            descriptor_flags(child.raw_fd()).expect("child flags") & libc::FD_CLOEXEC,
            libc::FD_CLOEXEC,
            "the portal remote remains non-inheritable in Cut until the child handoff"
        );
    }

    #[test]
    fn child_handoff_exposes_only_the_explicit_pipewire_remote_before_exec() {
        let file = File::open("/dev/null").expect("open harmless descriptor");
        let source: OwnedFd = file.into();
        let child = inherit_portal_remote(&source).expect("duplicate portal remote");

        expose_remote_in_child(child.raw_fd()).expect("clear close-on-exec for child handoff");
        assert_eq!(
            descriptor_flags(child.raw_fd()).expect("child flags") & libc::FD_CLOEXEC,
            0,
            "pipewiresrc fd must survive the imminent gst-launch exec"
        );
        assert_ne!(
            descriptor_flags(source.as_raw_fd()).expect("source flags") & libc::FD_CLOEXEC,
            0,
            "Cut's portal descriptor must not leak into gst-launch"
        );
    }

    #[tokio::test]
    async fn registered_child_handoff_keeps_the_remote_open_across_exec() {
        let file = File::open("/dev/null").expect("open harmless descriptor");
        let source: OwnedFd = file.into();
        let child = inherit_portal_remote(&source).expect("duplicate portal remote");
        let fd = child.raw_fd();
        let mut command = tokio::process::Command::new("sh");
        command.args([
            "-c",
            "test -e /proc/self/fd/$1",
            "pipewire-fd-probe",
            &fd.to_string(),
        ]);
        bind_portal_remote_to_child(&mut command, fd);

        let status = command.status().await.expect("spawn child fd probe");
        assert!(status.success(), "the child must retain pipewiresrc's fd");
    }
}
