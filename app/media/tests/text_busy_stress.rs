//! Stress reproduction of "Text file busy" (ETXTBSY) when a program is
//! written and then started at once in a multi-threaded process, the way test
//! stand-ins (and any tool Cut writes itself) are run.
//!
//! Writer threads write a small shell program, close it, and start it through
//! the shared owned-process runner; spawner threads start `/bin/true` without
//! pause, so some child is always between fork and exec holding a copy of
//! whatever descriptors were open. Run by name on Unix:
//! `cargo test -p cut-media --test text_busy_stress -- --ignored --nocapture`
//! (sizes: `CUT_STRESS_WRITERS`, `CUT_STRESS_STARTS`, `CUT_STRESS_SPAWNERS`).
#![cfg(unix)]

cut_core::isolate_test_process!();

use cut_media::ffmpeg::{run_owned_command, OwnedProcessControl};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn env_or(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "stress run: thousands of process starts"]
fn freshly_written_programs_start_while_other_threads_spawn() {
    let writers = env_or("CUT_STRESS_WRITERS", 8);
    let starts = env_or("CUT_STRESS_STARTS", 2000);
    let spawners = env_or("CUT_STRESS_SPAWNERS", 4);
    let dir = tempfile::tempdir().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let churn = Arc::new(AtomicU64::new(0));
    let background: Vec<_> = (0..spawners)
        .map(|_| {
            let stop = stop.clone();
            let churn = churn.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = Command::new("/bin/true").status();
                    churn.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .collect();

    let busy = Arc::new(AtomicU64::new(0));
    let other = Arc::new(AtomicU64::new(0));
    let first_other = Arc::new(std::sync::Mutex::new(None::<String>));
    let started = Instant::now();
    let workers: Vec<_> = (0..writers)
        .map(|w| {
            let root = dir.path().to_path_buf();
            let (busy, other, first_other) = (busy.clone(), other.clone(), first_other.clone());
            std::thread::spawn(move || {
                for i in 0..starts {
                    let tool = root.join(format!("tool-{w}-{i}"));
                    std::fs::write(&tool, "#!/bin/sh\nexit 0\n").unwrap();
                    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                    let control = OwnedProcessControl::bounded(Duration::from_secs(30), || false);
                    match run_owned_command(&mut Command::new(&tool), &control, "stand-in") {
                        Ok(out) if out.status.success() => {}
                        Ok(out) => {
                            other.fetch_add(1, Ordering::Relaxed);
                            first_other
                                .lock()
                                .unwrap()
                                .get_or_insert(format!("exit {:?}", out.status));
                        }
                        Err(error) => {
                            let text = format!("{} ({})", error.message, error.cause);
                            if text.contains("os error 26") || text.contains("Text file busy") {
                                busy.fetch_add(1, Ordering::Relaxed);
                            } else {
                                other.fetch_add(1, Ordering::Relaxed);
                                first_other.lock().unwrap().get_or_insert(text);
                            }
                        }
                    }
                    let _ = std::fs::remove_file(&tool);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    for thread in background {
        thread.join().unwrap();
    }
    let total = writers * starts;
    let busy = busy.load(Ordering::Relaxed);
    let other = other.load(Ordering::Relaxed);
    eprintln!(
        "STRESS starts={total} text_file_busy={busy} other_failures={other} \
         background_spawns={} seconds={:.1} first_other={:?}",
        churn.load(Ordering::Relaxed),
        started.elapsed().as_secs_f64(),
        first_other.lock().unwrap()
    );
    assert_eq!(
        busy, 0,
        "{busy} of {total} starts failed with Text file busy"
    );
    assert_eq!(other, 0, "{other} of {total} starts failed otherwise");
}
