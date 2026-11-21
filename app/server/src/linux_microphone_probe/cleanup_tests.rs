//! Process-table proof for the bounded probe child owner.

use super::*;
use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};

const ISOLATED_TEST: &str = "SHELLX_CUT_MIC_CLEANUP_ISOLATED_TEST";

fn run_in_own_process(test_name: &str) -> bool {
    if std::env::var(ISOLATED_TEST).as_deref() == Ok(test_name) {
        return false;
    }

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
        .env(ISOLATED_TEST, test_name)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("isolated {test_name} exceeded 180 seconds");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut output = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    assert!(
        status.success()
            && output.contains("running 1 test")
            && output.contains("test result: ok. 1 passed; 0 failed;"),
        "isolated {test_name} failed or did not run exactly one test:\n{output}"
    );
    true
}

fn open_fds() -> usize {
    fs::read_dir("/proc/self/fd").unwrap().count()
}

fn direct_children() -> Vec<(u32, char)> {
    let parent = std::process::id();
    let mut children = Vec::new();
    for entry in fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(status) = fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        let ppid = status
            .lines()
            .find_map(|line| line.strip_prefix("PPid:\t"))
            .and_then(|value| value.trim().parse::<u32>().ok());
        if ppid == Some(parent) {
            let state = status
                .lines()
                .find_map(|line| line.strip_prefix("State:\t"))
                .and_then(|value| value.chars().next())
                .unwrap_or('?');
            children.push((pid, state));
        }
    }
    children.sort_unstable();
    children
}

fn spawned_pids(marker: &Path) -> Vec<u32> {
    fs::read_to_string(marker)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.parse::<u32>().ok())
        .collect()
}

async fn wait_for_baseline(runner: &ProbeRunner, children: &[(u32, char)], fds: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if direct_children() == children
                && open_fds() == fds
                && runner.slots.available_permits() == MAX_PROBE_CHILDREN
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "probe child or descriptor remained: children={:?}, fds={}",
            direct_children(),
            open_fds()
        )
    });
}

#[tokio::test]
async fn repeated_deadlines_and_cancelled_caller_leave_no_children_or_fds() {
    if run_in_own_process(
        "linux_microphone_probe::cleanup_tests::repeated_deadlines_and_cancelled_caller_leave_no_children_or_fds",
    ) {
        return;
    }
    let runner = ProbeRunner::new();
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("probe-pids");
    let marker_arg = marker.to_str().unwrap();
    // The shell records its PID, then exec keeps that PID asleep past the deadline.
    let args = [
        "-c",
        "echo $$ >> \"$1\"; exec /bin/sleep 5",
        "probe",
        marker_arg,
    ];
    let before_children = direct_children();
    let before_fds = open_fds();

    let burst =
        futures_util::future::join_all((0..8).map(|_| runner.run(Path::new("/bin/sh"), &args)))
            .await;
    assert!(burst.iter().all(|answer| *answer == Availability::Unknown));
    let burst_pids = spawned_pids(&marker);
    wait_for_baseline(&runner, &before_children, before_fds).await;

    for _ in 0..8 {
        assert_eq!(
            runner.run(Path::new("/bin/sh"), &args).await,
            Availability::Unknown
        );
        wait_for_baseline(&runner, &before_children, before_fds).await;
    }

    let prior_count = spawned_pids(&marker).len();
    let marker_for_cancel = marker.clone();
    let cancelled_runner = runner.clone();
    let caller = tokio::spawn(async move {
        let marker_arg = marker_for_cancel.to_str().unwrap();
        cancelled_runner
            .run(
                Path::new("/bin/sh"),
                &[
                    "-c",
                    "echo $$ >> \"$1\"; exec /bin/sleep 5",
                    "probe",
                    marker_arg,
                ],
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while spawned_pids(&marker).len() == prior_count {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cancelled caller started a probe");
    caller.abort();
    let _ = caller.await;
    let spawned_before_reap = spawned_pids(&marker).len();
    assert_eq!(
        runner.run(Path::new("/bin/sh"), &args).await,
        Availability::Unknown
    );
    assert_eq!(
        spawned_pids(&marker).len(),
        spawned_before_reap,
        "a cancelled caller must not free the child slot before reap"
    );
    wait_for_baseline(&runner, &before_children, before_fds).await;
    eprintln!(
        "process table: children {} -> {}, open fds {} -> {}, spawned {} (burst {})",
        before_children.len(),
        direct_children().len(),
        before_fds,
        open_fds(),
        spawned_pids(&marker).len(),
        burst_pids.len()
    );

    // With a single owner/permit, the eight simultaneous callers launched at
    // most one child. The old implementation launched all eight.
    assert!(
        burst_pids.len() <= 1,
        "overlapping probe children: {burst_pids:?}"
    );
    assert!(
        spawned_pids(&marker).len() >= 9,
        "serial deadlines did not run"
    );
    for pid in spawned_pids(&marker) {
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "unreaped PID {pid}"
        );
    }
}

#[tokio::test]
async fn wait_errors_keep_the_only_slot_until_confirmed_reap() {
    if run_in_own_process(
        "linux_microphone_probe::cleanup_tests::wait_errors_keep_the_only_slot_until_confirmed_reap",
    ) {
        return;
    }
    for script in [
        "echo $$ >> \"$1\"; exec /bin/sleep 5",
        "echo $$ >> \"$1\"; exec 1>&-; exec /bin/sleep 5",
    ] {
        let runner = ProbeRunner::with_wait_errors(6);
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("probe-pids");
        let marker_arg = marker.to_str().unwrap();
        let before_children = direct_children();
        let before_fds = open_fds();

        assert_eq!(
            runner
                .run(Path::new("/bin/sh"), &["-c", script, "probe", marker_arg])
                .await,
            Availability::Unknown
        );
        assert_eq!(spawned_pids(&marker).len(), 1);
        assert!(runner.wait_errors.remaining() > 0);
        assert_eq!(runner.slots.available_permits(), 0);

        assert_eq!(
            runner
                .run(
                    Path::new("/bin/sh"),
                    &[
                        "-c",
                        "echo $$ >> \"$1\"; printf 'PRESENT\\n'",
                        "probe",
                        marker_arg,
                    ],
                )
                .await,
            Availability::Unknown
        );
        assert_eq!(
            spawned_pids(&marker).len(),
            1,
            "a second child spawned before the first had a confirmed reap"
        );

        wait_for_baseline(&runner, &before_children, before_fds).await;
        assert_eq!(runner.wait_errors.remaining(), 0);
        for pid in spawned_pids(&marker) {
            assert!(!Path::new(&format!("/proc/{pid}")).exists());
        }
        eprintln!(
            "wait errors: children {} -> {}, open fds {} -> {}, slot {}",
            before_children.len(),
            direct_children().len(),
            before_fds,
            open_fds(),
            runner.slots.available_permits()
        );
    }
}
