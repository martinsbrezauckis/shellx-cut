//! Bounded, shared Linux microphone inventory from a disposable cutd child.

use std::future::Future;
use std::path::Path;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, Mutex, Semaphore};
use tokio::{io::AsyncReadExt, process::Command};

mod reap;

const DEADLINE: Duration = Duration::from_secs(1);
const SHARED_PROBE_DEADLINE: Duration = Duration::from_millis(1150);
const CALL_DEADLINE: Duration = Duration::from_millis(1300);
const CACHE_TTL: Duration = Duration::from_secs(2);
const MAX_OUTPUT: u64 = 4096;
const MAX_PROBE_CHILDREN: usize = 1;
pub(crate) const PROBE_ARG: &str = "--cut-private-microphone-probe";

/// The engine's own binary is the only shipped probe helper.
pub(crate) fn run_if_requested() {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new(PROBE_ARG))
        && std::env::args_os().nth(2).is_none()
    {
        use std::io::Write;
        let answer = record_capture::linux_microphone_probe_answer();
        let mut stdout = std::io::stdout().lock();
        let written = stdout
            .write_all(answer.as_bytes())
            .and_then(|_| stdout.flush());
        std::process::exit(if written.is_ok() { 0 } else { 1 });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Availability {
    Present,
    Absent,
    Unknown,
}

struct Cache {
    value: Option<(Instant, Availability)>,
}

struct SharedProbe {
    cache: Mutex<Cache>,
    ttl: Duration,
}

impl SharedProbe {
    fn new(ttl: Duration) -> Self {
        Self {
            cache: Mutex::new(Cache { value: None }),
            ttl,
        }
    }

    async fn query<F, Fut>(&self, run: F) -> Availability
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Availability>,
    {
        tokio::time::timeout(CALL_DEADLINE, async {
            let mut cache = self.cache.lock().await;
            if let Some((at, answer)) = cache.value {
                if at.elapsed() < self.ttl {
                    return answer;
                }
            }
            let answer = tokio::time::timeout(SHARED_PROBE_DEADLINE, run())
                .await
                .unwrap_or(Availability::Unknown);
            cache.value = Some((Instant::now(), answer));
            answer
        })
        .await
        .unwrap_or(Availability::Unknown)
    }
}

pub(crate) async fn availability() -> Availability {
    static PROBE: OnceLock<SharedProbe> = OnceLock::new();
    PROBE
        .get_or_init(|| SharedProbe::new(CACHE_TTL))
        .query(|| async {
            let Ok(exe) = std::env::current_exe() else {
                return Availability::Unknown;
            };
            run_program(&exe, &[PROBE_ARG]).await
        })
        .await
}

fn parse_answer(output: &[u8]) -> Availability {
    match output {
        b"PRESENT\n" => Availability::Present,
        b"ABSENT\n" => Availability::Absent,
        _ => Availability::Unknown,
    }
}

#[derive(Clone)]
struct ProbeRunner {
    slots: std::sync::Arc<Semaphore>,
    wait_errors: reap::WaitErrors,
}

impl ProbeRunner {
    fn new() -> Self {
        Self {
            slots: std::sync::Arc::new(Semaphore::new(MAX_PROBE_CHILDREN)),
            wait_errors: reap::WaitErrors::default(),
        }
    }

    #[cfg(test)]
    fn with_wait_errors(count: usize) -> Self {
        Self {
            slots: std::sync::Arc::new(Semaphore::new(MAX_PROBE_CHILDREN)),
            wait_errors: reap::WaitErrors::with_count(count),
        }
    }

    async fn run(&self, exe: &Path, args: &[&str]) -> Availability {
        // The permit follows Child into its owner task. A stalled reap holds the
        // only slot, so later callers answer UNKNOWN without spawning.
        let Ok(permit) = self.slots.clone().try_acquire_owned() else {
            return Availability::Unknown;
        };
        let child = match Command::new(exe)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(_) => return Availability::Unknown,
        };
        let deadline = tokio::time::Instant::now() + DEADLINE;
        let (answer_tx, answer_rx) = oneshot::channel();
        let wait_errors = self.wait_errors.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let mut child = child;
            let Some(stdout) = child.stdout.take() else {
                let _ = answer_tx.send(Availability::Unknown);
                reap::confirmed_reap(&mut child, &wait_errors).await;
                return;
            };
            let result = tokio::time::timeout_at(deadline, async {
                let mut output = Vec::with_capacity(32);
                stdout
                    .take(MAX_OUTPUT + 1)
                    .read_to_end(&mut output)
                    .await
                    .ok()?;
                if output.len() > MAX_OUTPUT as usize {
                    return None;
                }
                let status = reap::wait_once(&mut child, &wait_errors).await.ok()?;
                status.success().then(|| parse_answer(&output))
            })
            .await;
            if let Ok(Some(answer)) = result {
                // The child is already reaped; make the slot available before
                // the caller can start a subsequent successful probe.
                drop(_permit);
                let _ = answer_tx.send(answer);
                return;
            }
            let _ = answer_tx.send(Availability::Unknown);
            // An unconfirmed reap keeps the sole slot for process lifetime.
            // UNKNOWN is safer than spawning more potentially unreaped children.
            reap::confirmed_reap(&mut child, &wait_errors).await;
        });
        answer_rx.await.unwrap_or(Availability::Unknown)
    }
}

async fn run_program(exe: &Path, args: &[&str]) -> Availability {
    static RUNNER: OnceLock<ProbeRunner> = OnceLock::new();
    RUNNER.get_or_init(ProbeRunner::new).run(exe, args).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn parser_accepts_only_three_exact_small_answers() {
        assert_eq!(parse_answer(b"PRESENT\n"), Availability::Present);
        assert_eq!(parse_answer(b"ABSENT\n"), Availability::Absent);
        assert_eq!(parse_answer(b"UNKNOWN\n"), Availability::Unknown);
        for bad in [
            b"".as_slice(),
            b"PRESENT",
            b"PRESENT\nextra",
            b"absent\n",
            b"ABSENT\0\n",
        ] {
            assert_eq!(parse_answer(bad), Availability::Unknown);
        }
    }

    #[tokio::test]
    async fn valid_child_answer_is_accepted() {
        let runner = ProbeRunner::new();
        assert_eq!(
            runner
                .run(Path::new("/bin/sh"), &["-c", "printf 'PRESENT\\n'"])
                .await,
            Availability::Present
        );
        assert_eq!(
            runner
                .run(Path::new("/bin/sh"), &["-c", "printf 'ABSENT\\n'"])
                .await,
            Availability::Absent
        );
    }

    #[tokio::test]
    async fn sleeping_child_hits_the_real_deadline() {
        let runner = ProbeRunner::new();
        let started = Instant::now();
        assert_eq!(
            runner.run(Path::new("/bin/sleep"), &["5"]).await,
            Availability::Unknown
        );
        assert!(
            (Duration::from_millis(900)..Duration::from_millis(1500)).contains(&started.elapsed())
        );
    }

    async fn fake_with_marker(script: &str) -> (Availability, Duration, bool) {
        let runner = ProbeRunner::new();
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("ran");
        let marker_arg = marker.to_str().unwrap();
        let started = Instant::now();
        let answer = runner
            .run(Path::new("/bin/sh"), &["-c", script, "probe", marker_arg])
            .await;
        (answer, started.elapsed(), marker.exists())
    }

    #[tokio::test]
    async fn flooding_child_is_killed_at_the_output_cap() {
        let (answer, elapsed, ran) = fake_with_marker(
            "printf x > \"$1\"; printf 'PRESENT\\n'; while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done",
        )
        .await;
        assert!(ran);
        assert_eq!(answer, Availability::Unknown);
        assert!(elapsed < Duration::from_millis(900), "{elapsed:?}");
    }

    #[tokio::test]
    async fn signalled_child_cannot_trust_its_partial_answer() {
        let (answer, elapsed, ran) =
            fake_with_marker("printf x > \"$1\"; printf 'PRESENT\\n'; kill -TERM $$").await;
        assert!(ran);
        assert_eq!(answer, Availability::Unknown);
        assert!(elapsed < Duration::from_millis(900), "{elapsed:?}");
    }

    #[tokio::test]
    async fn nonzero_child_cannot_trust_its_partial_answer() {
        let (answer, _, ran) =
            fake_with_marker("printf x > \"$1\"; printf 'ABSENT\\n'; exit 9").await;
        assert!(ran);
        assert_eq!(answer, Availability::Unknown);
    }

    #[tokio::test]
    async fn concurrent_calls_share_one_probe_and_cache_expires() {
        let cache = SharedProbe::new(Duration::from_millis(60));
        let runs = AtomicUsize::new(0);
        let calls = (0..10).map(|_| {
            cache.query(|| async {
                runs.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                Availability::Absent
            })
        });
        for answer in futures_util::future::join_all(calls).await {
            assert_eq!(answer, Availability::Absent);
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert_eq!(
            cache
                .query(|| async {
                    runs.fetch_add(1, Ordering::SeqCst);
                    Availability::Present
                })
                .await,
            Availability::Present
        );
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn stalled_shared_probe_bounds_every_waiter() {
        let cache = SharedProbe::new(Duration::from_secs(2));
        let started = Instant::now();
        let first = cache.query(|| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Availability::Present
        });
        let second = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            cache
                .query(|| async { panic!("a concurrent waiter must reuse the first result") })
                .await
        };
        let (first, second) = tokio::join!(first, second);
        assert_eq!(first, Availability::Unknown);
        assert_eq!(second, Availability::Unknown);
        assert!(started.elapsed() < Duration::from_millis(1500));
    }

    #[tokio::test]
    async fn lock_contention_cannot_outlive_the_call_deadline() {
        let cache = SharedProbe::new(Duration::from_secs(2));
        let guard = cache.cache.lock().await;
        let started = Instant::now();
        let answer = cache.query(|| async { Availability::Present }).await;
        assert_eq!(answer, Availability::Unknown);
        assert!(started.elapsed() < Duration::from_millis(1500));
        drop(guard);
    }
}

#[cfg(test)]
mod cleanup_tests;
