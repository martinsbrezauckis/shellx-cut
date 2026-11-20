//! Windows counterpart of `text_busy_stress.rs`: can a just-written `.exe`
//! fail to start with ERROR_SHARING_VIOLATION (32), for example while an
//! antivirus scan holds it? Plain `Command` starts (no retry) are counted, so
//! the result is the raw rate. Two forms:
//! - written in-process with `fs::write`, then started at once, from many
//!   threads while other threads start `cmd /c exit` without pause;
//! - extracted by the system `tar.exe` (as the built-in FFmpeg download does),
//!   then started at once.
//!
//! Run by name on Windows:
//! `cargo test -p cut-media --test sharing_violation_stress -- --ignored --nocapture --test-threads=1`
#![cfg(windows)]

cut_core::isolate_test_process!();

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const SHARING_VIOLATION: i32 = 32;

fn program_bytes() -> Vec<u8> {
    std::fs::read(system32().join("whoami.exe")).expect("whoami.exe")
}

fn system32() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
        .join("System32")
}

struct Counts {
    sharing: AtomicU64,
    other: Mutex<Vec<String>>,
}

impl Counts {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            sharing: AtomicU64::new(0),
            other: Mutex::new(Vec::new()),
        })
    }
    fn start(&self, program: &Path) {
        match Command::new(program).output() {
            Ok(out) if out.status.success() => {}
            Ok(out) => self
                .other
                .lock()
                .unwrap()
                .push(format!("exit {:?}", out.status)),
            Err(e) if e.raw_os_error() == Some(SHARING_VIOLATION) => {
                self.sharing.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => self.other.lock().unwrap().push(e.to_string()),
        }
    }
    fn report(&self, label: &str, starts: u64) {
        let other = self.other.lock().unwrap();
        eprintln!(
            "STRESS {label} starts={starts} sharing_violation={} other_failures={} first_other={:?}",
            self.sharing.load(Ordering::Relaxed),
            other.len(),
            other.first()
        );
    }
}

fn background_spawners(stop: &Arc<AtomicBool>) -> Vec<std::thread::JoinHandle<()>> {
    (0..4)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = Command::new(system32().join("cmd.exe"))
                        .args(["/c", "exit"])
                        .status();
                }
            })
        })
        .collect()
}

#[test]
#[ignore = "stress run: thousands of process starts"]
fn freshly_written_exes_start_while_other_threads_spawn() {
    let bytes = Arc::new(program_bytes());
    let dir = tempfile::tempdir().unwrap();
    let counts = Counts::new();
    let stop = Arc::new(AtomicBool::new(false));
    let background = background_spawners(&stop);
    let (writers, starts) = (8u64, 250u64);
    let workers: Vec<_> = (0..writers)
        .map(|w| {
            let (root, bytes, counts) = (dir.path().to_path_buf(), bytes.clone(), counts.clone());
            std::thread::spawn(move || {
                for i in 0..starts {
                    let exe = root.join(format!("tool-{w}-{i}.exe"));
                    std::fs::write(&exe, &*bytes).unwrap();
                    counts.start(&exe);
                    let _ = std::fs::remove_file(&exe);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    background.into_iter().for_each(|t| t.join().unwrap());
    counts.report("written-in-process", writers * starts);
}

#[test]
#[ignore = "stress run: hundreds of extractions and starts"]
fn exes_extracted_by_tar_start_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("ffmpeg.exe"), program_bytes()).unwrap();
    let archive = dir.path().join("fixture.zip");
    let tar = system32().join("tar.exe");
    let made = Command::new(&tar)
        .arg("-a")
        .arg("-c")
        .arg("-f")
        .arg(&archive)
        .arg("-C")
        .arg(&src)
        .arg("ffmpeg.exe")
        .status()
        .unwrap();
    assert!(made.success());
    let counts = Counts::new();
    let stop = Arc::new(AtomicBool::new(false));
    let background = background_spawners(&stop);
    let rounds = 300u64;
    for round in 0..rounds {
        let dest = dir.path().join(format!("x{round}"));
        std::fs::create_dir(&dest).unwrap();
        let unpacked = Command::new(&tar)
            .arg("-x")
            .arg("-f")
            .arg(&archive)
            .arg("-C")
            .arg(&dest)
            .status()
            .unwrap();
        assert!(unpacked.success());
        counts.start(&dest.join("ffmpeg.exe"));
    }
    stop.store(true, Ordering::Relaxed);
    background.into_iter().for_each(|t| t.join().unwrap());
    counts.report("extracted-by-tar", rounds);
}
