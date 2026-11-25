use super::*;
use std::time::Duration;

struct KillOnDrop(Option<libc::pid_t>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            // Also runs on assertion failure. Check ownership without reaping
            // first, so a completed/reaped fixture cannot signal a reused PID.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let owned = unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if owned == 0 {
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    }
}

#[test]
fn stopped_child_reap_is_bounded_and_background_reaper_collects_it() {
    let mut command = Command::new("/bin/sleep");
    command.arg("30");
    let control = RenderProcessControl::bounded(Duration::from_secs(60), || false);
    let mut child = ManagedChild::spawn(&mut command, control, "reap fixture").unwrap();
    let pid = child.child.as_ref().unwrap().id() as libc::pid_t;
    let mut cleanup = KillOnDrop(Some(pid));
    child.skip_stop = true;

    let started = Instant::now();
    let error = child.stop_and_reap("reap fixture").unwrap_err();
    let elapsed = started.elapsed();
    assert!(
        elapsed >= REAP_GRACE,
        "returned before the reap grace: {elapsed:?}"
    );
    assert!(
        elapsed < STOP_GRACE * 2 + REAP_GRACE + Duration::from_secs(3),
        "cleanup blocked for {elapsed:?}"
    );
    assert!(error
        .to_string()
        .contains("did not finish exiting after it was stopped"));
    assert!(error.to_string().contains(&pid.to_string()));
    assert!(!child.reaped);
    assert!(child.child.is_none());
    // The seam suppressed both tree signals and the direct-child kill.
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0);

    let started = Instant::now();
    assert!(child.stop_and_reap("repeat cleanup").is_err());
    assert!(child.try_wait("repeat wait").is_err());
    drop(child);
    assert!(started.elapsed() < Duration::from_secs(1));

    assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        // WNOWAIT observes without stealing the background thread's reap.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == -1 {
            assert_eq!(
                io::Error::last_os_error().raw_os_error(),
                Some(libc::ECHILD)
            );
            cleanup.0 = None;
            break;
        }
        assert!(
            Instant::now() < until,
            "background reaper did not collect pid {pid}"
        );
        thread::sleep(PROCESS_POLL);
    }
}

#[test]
fn normally_stopped_child_is_reaped_without_handoff() {
    let mut command = Command::new("/bin/sleep");
    command.arg("30");
    let control = RenderProcessControl::bounded(Duration::from_secs(60), || false);
    let mut child = ManagedChild::spawn(&mut command, control, "normal stop fixture").unwrap();
    child.stop_and_reap("normal stop fixture").unwrap();
    assert!(child.reaped);
    assert!(child.tree_closed);
    assert!(child.child.as_mut().unwrap().try_wait().unwrap().is_some());
}
