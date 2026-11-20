//! No render ffmpeg may sit for ever without working (item 170).
//!
//! A render's ffmpeg is STALLED when, for the stall time, its output time did
//! not move AND its whole process tree used less than 2 % of one processor
//! core of USER time (its own work; kernel reclaim time is not work, see
//! `activity`). Both conditions are needed:
//! - output alone is not a sign of life: a reversed clip, or a clip far into
//!   its source, keeps the output still for minutes while ffmpeg decodes at
//!   full speed (every source is decoded from its start);
//! - a slow machine is not a stall: a working ffmpeg uses processor time all
//!   the time, however slowly its output moves.
//!
//! Seen on Linux (item 170): a render window whose reversed clip needed more
//! memory than its cgroup MemoryHigh, on a machine with no swap. The kernel
//! held ffmpeg at the limit; it used 0.06-0.6 % of one core (and in one run
//! 2.7 %, nearly all of it kernel reclaim), wrote nothing for 10+ minutes, and
//! the job read "running" until cancelled. The cgroup's `high`
//! counter rising during the stall says the memory limit is the cause, and
//! the error says so.
//!
//! The stall time is 120 s (`SHELLX_CUT_RENDER_STALL_SEC` 10..=3600, `0` turns
//! the watch off); readings are taken every 5 s. A gap between readings much
//! longer than that (the computer slept, the engine was stopped) starts the
//! count again instead of counting the sleep as a stall.

use super::activity::{self, Usage};
use super::tree::ProcessTree;
use cut_core::{error_codes, CutError};
use std::time::{Duration, Instant};

pub(super) const DEFAULT_STALL_AFTER: Duration = Duration::from_secs(120);
const SAMPLE_EVERY: Duration = Duration::from_secs(5);
/// Processor time a working ffmpeg uses at the very least, as a share of the
/// time watched: 2 % of one core. The stalled window of item 170 used 0.6 %;
/// a deadlocked one would use less.
const WORK_FLOOR: f64 = 0.02;

impl super::RenderProcessControl {
    /// Stall time of renders run under this control (`None` turns the watch
    /// off). Default: 120 s, or `SHELLX_CUT_RENDER_STALL_SEC`.
    pub fn with_stall_after(mut self, stall_after: Option<Duration>) -> Self {
        self.stall_after = stall_after;
        self
    }
}

/// The stall time a render uses unless its control says otherwise.
pub(super) fn stall_after_from_env() -> Option<Duration> {
    match std::env::var("SHELLX_CUT_RENDER_STALL_SEC")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
    {
        Some(0) => None,
        Some(n) => Some(Duration::from_secs(n.clamp(10, 3600))),
        None => Some(DEFAULT_STALL_AFTER),
    }
}

/// What the watch saw when it ended a render.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Stalled {
    pub(super) idle: Duration,
    pub(super) cpu_used: Duration,
    /// Output time reached (ms), when ffmpeg had written any.
    pub(super) output_ms: Option<u64>,
    /// The kernel throttled the render for its memory limit during the stall.
    pub(super) memory_held: bool,
    pub(super) memory: Option<(u64, u64)>,
}

/// The decision, separated from the readings so it can be tested anywhere.
pub(super) struct Judge {
    stall_after: Duration,
    last_at: Option<Instant>,
    /// Start of the current quiet span: reading time and usage then.
    quiet_since: Option<(Instant, Usage)>,
    output_us: Option<u64>,
    output_moved: bool,
}

impl Judge {
    pub(super) fn new(stall_after: Duration) -> Self {
        Self {
            stall_after,
            last_at: None,
            quiet_since: None,
            output_us: None,
            output_moved: false,
        }
    }

    /// ffmpeg reported its output time (`out_time_us`).
    pub(super) fn output(&mut self, us: u64) {
        if self.output_us.is_none_or(|old| us > old) {
            self.output_us = Some(us);
            self.output_moved = true;
        }
    }

    /// One reading of the tree at `now`. Returns what was seen when the
    /// render has been quiet for the whole stall time.
    pub(super) fn reading(&mut self, now: Instant, usage: Option<Usage>) -> Option<Stalled> {
        let slept = self
            .last_at
            .is_some_and(|last| now.saturating_duration_since(last) > SAMPLE_EVERY * 3);
        self.last_at = Some(now);
        let moved = std::mem::take(&mut self.output_moved);
        let Some(usage) = usage else {
            self.quiet_since = None; // nothing to judge by: never stop on a guess
            return None;
        };
        let Some((since, at_start)) = self.quiet_since else {
            self.quiet_since = Some((now, usage));
            return None;
        };
        let span = now.saturating_duration_since(since);
        let cpu_used = usage.cpu.saturating_sub(at_start.cpu);
        let worked = cpu_used.as_secs_f64() >= span.as_secs_f64() * WORK_FLOOR;
        if moved || slept || (worked && span >= SAMPLE_EVERY) {
            self.quiet_since = Some((now, usage));
            return None;
        }
        if span < self.stall_after {
            return None;
        }
        Some(Stalled {
            idle: span,
            cpu_used,
            output_ms: self.output_us.map(|us| us / 1000),
            memory_held: at_start
                .memory_high_events
                .zip(usage.memory_high_events)
                .is_some_and(|(a, b)| b > a),
            memory: usage.memory,
        })
    }
}

/// The watch the render loop polls; it reads the tree every 5 s. `None`
/// (the watch turned off) never ends a render.
pub(super) struct Watch {
    judge: Option<Judge>,
    next_at: Instant,
}

impl Watch {
    pub(super) fn new(stall_after: Option<Duration>) -> Self {
        Self {
            judge: stall_after.map(Judge::new),
            next_at: Instant::now(),
        }
    }

    pub(super) fn output(&mut self, us: u64) {
        if let Some(judge) = self.judge.as_mut() {
            judge.output(us);
        }
    }

    pub(super) fn poll(&mut self, tree: &ProcessTree) -> Option<CutError> {
        let judge = self.judge.as_mut()?;
        let now = Instant::now();
        if now < self.next_at {
            return None;
        }
        self.next_at = now + SAMPLE_EVERY;
        judge
            .reading(now, activity::usage(tree))
            .map(|seen| stall_error(&seen))
    }
}

fn mb(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

/// The job error for a stalled render. The render layer adds which part of
/// the timeline and which clip it was making (`render::stall_context`).
pub(super) fn stall_error(seen: &Stalled) -> CutError {
    let secs = seen.idle.as_secs();
    let reached = seen
        .output_ms
        .map(|ms| format!("its output stopped at {:.1} s", ms as f64 / 1000.0))
        .unwrap_or_else(|| "it had written no picture yet".to_string());
    let memory = match seen.memory {
        Some((current, high)) => format!("; memory {} MB of a {} MB limit", mb(current), mb(high)),
        None => String::new(),
    };
    let cause = format!(
        "ffmpeg wrote no new output for {secs} s and did {:.1} s of its own processor work in that \
         time (under 2 % of one core); {reached}{memory}{}",
        seen.cpu_used.as_secs_f64(),
        if seen.memory_held {
            "; the system kept it at its memory limit (cgroup MemoryHigh) the whole time"
        } else {
            ""
        }
    );
    let (message, action) = if seen.memory_held {
        (
            format!(
                "The export stopped: it needed more memory than it may use and made no progress \
                 for {secs} s"
            ),
            "Free memory (close other programs), or make the export lighter: shorten long \
             reversed clips (a reversed clip is held in memory whole), or export at a lower \
             resolution. Then export again.",
        )
    } else {
        (
            format!("The export stopped: ffmpeg made no progress for {secs} s"),
            "Export again. If it stops at the same place, check that the source files of that \
             part can still be read (an unplugged drive or a network folder that stopped \
             answering also stops an export), then try again.",
        )
    };
    CutError::new(error_codes::RENDER_STALLED, message, cause).with_suggested_action(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(cpu_ms: u64, high: Option<u64>) -> Option<Usage> {
        Some(Usage {
            cpu: Duration::from_millis(cpu_ms),
            memory_high_events: high,
            memory: high.map(|_| (1400 << 20, 1300 << 20)),
        })
    }

    /// Feed readings every 5 s; `cpu` is cumulative ms at each reading.
    fn run(judge: &mut Judge, start: Instant, readings: &[(u64, Option<u64>)]) -> Option<Stalled> {
        let mut last = None;
        for (k, (cpu, high)) in readings.iter().enumerate() {
            last = judge.reading(start + SAMPLE_EVERY * k as u32, usage(*cpu, *high));
            if last.is_some() {
                return last;
            }
        }
        last
    }

    #[test]
    fn a_quiet_tree_is_stalled_after_the_stall_time() {
        let start = Instant::now();
        let mut judge = Judge::new(Duration::from_secs(120));
        judge.output(9_066_667);
        // 0.6 % of one core with the memory counter rising: item 170's window.
        let readings: Vec<_> = (0..=24u64)
            .map(|k| (11_000 + k * 30, Some(4000 + k * 300)))
            .collect();
        let seen = run(&mut judge, start, &readings).expect("stalled at 120 s");
        assert_eq!(seen.idle, Duration::from_secs(120));
        assert!(seen.memory_held);
        assert_eq!(seen.output_ms, Some(9066));
        let error = stall_error(&seen);
        assert_eq!(error.code, error_codes::RENDER_STALLED);
        assert!(error.message.contains("more memory"), "{}", error.message);
        assert!(error.cause.contains("stopped at 9.1 s"), "{}", error.cause);
    }

    #[test]
    fn one_reading_short_of_the_stall_time_is_not_a_stall() {
        let start = Instant::now();
        let mut judge = Judge::new(Duration::from_secs(120));
        let readings: Vec<_> = (0..24u64).map(|k| (k, None)).collect(); // 115 s
        assert_eq!(run(&mut judge, start, &readings), None);
    }

    #[test]
    fn processor_work_without_output_is_not_a_stall() {
        // A reversed clip being read ahead: no output, but one core busy.
        let start = Instant::now();
        let mut judge = Judge::new(Duration::from_secs(120));
        let readings: Vec<_> = (0..=60u64).map(|k| (k * 5_000, None)).collect();
        assert_eq!(run(&mut judge, start, &readings), None);
        // A very slow machine: 3 % of one core is still work.
        let mut judge = Judge::new(Duration::from_secs(120));
        let readings: Vec<_> = (0..=60u64).map(|k| (k * 150, None)).collect();
        assert_eq!(run(&mut judge, start, &readings), None);
    }

    #[test]
    fn output_that_moves_is_not_a_stall_even_without_processor_time() {
        // A hardware encoder doing the work: little processor time, output moving.
        let start = Instant::now();
        let mut judge = Judge::new(Duration::from_secs(30));
        for k in 0..=40u32 {
            judge.output(u64::from(k) * 1_000_000);
            assert_eq!(
                judge.reading(start + SAMPLE_EVERY * k, usage(0, None)),
                None
            );
        }
    }

    #[test]
    fn a_sleep_between_readings_starts_the_count_again() {
        let start = Instant::now();
        let mut judge = Judge::new(Duration::from_secs(120));
        assert_eq!(judge.reading(start, usage(0, None)), None);
        assert_eq!(judge.reading(start + SAMPLE_EVERY, usage(0, None)), None);
        // The computer slept for an hour: not a stall.
        let woke = start + Duration::from_secs(3600);
        assert_eq!(judge.reading(woke, usage(1, None)), None);
        assert_eq!(judge.reading(woke + SAMPLE_EVERY, usage(1, None)), None);
    }

    #[test]
    fn no_reading_never_stops_a_render() {
        let start = Instant::now();
        let mut judge = Judge::new(Duration::from_secs(10));
        for k in 0..100u32 {
            assert_eq!(judge.reading(start + SAMPLE_EVERY * k, None), None);
        }
    }

    /// A stand-in ffmpeg that writes one progress block and then does nothing
    /// (no output, no processor time) is ended with the stall error and reaped.
    #[cfg(unix)]
    #[test]
    fn a_stand_in_ffmpeg_that_stops_working_is_ended() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("stand-in.pid");
        let mut command = std::process::Command::new("sh");
        command.args([
            "-c",
            "printf '%s' $$ > \"$1\"; printf 'out_time_us=1500000\\nprogress=continue\\n'; exec sleep 600",
            "sh",
            &pid_file.display().to_string(),
        ]);
        let control =
            super::super::RenderProcessControl::bounded(Duration::from_secs(90), || false)
                .with_stall_after(Some(Duration::from_secs(10)));
        let started = Instant::now();
        let error = super::super::drive_with_control(&mut command, 10_000, &|_| {}, &control)
            .expect_err("a stand-in that stops working must not read running for ever");
        let took = started.elapsed();
        assert_eq!(error.code, error_codes::RENDER_STALLED, "{error:?}");
        assert!(
            took >= Duration::from_secs(10) && took < Duration::from_secs(30),
            "{took:?}"
        );
        assert!(error.cause.contains("stopped at 1.5 s"), "{}", error.cause);
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let gone = unsafe { libc::kill(pid, 0) } == -1;
        assert!(gone, "the stalled stand-in {pid} was not reaped");
    }

    /// A stand-in that keeps one core busy in the KERNEL only (dd copying
    /// /dev/zero with big blocks: almost no user time), the way a process held
    /// at MemoryHigh spends its time in memory reclaim, is a stall too.
    #[cfg(target_os = "linux")]
    #[test]
    fn kernel_time_without_own_work_is_a_stall() {
        let mut command = std::process::Command::new("sh");
        // GNU dd replaces stdout when opening its output; keep the progress pipe
        // open so this stand-in has the same pipe lifetime as a running FFmpeg.
        command.args([
            "-c",
            "exec 3>&1; exec dd if=/dev/zero of=/dev/null bs=16M 2>/dev/null",
        ]);
        let control =
            super::super::RenderProcessControl::bounded(Duration::from_secs(90), || false)
                .with_stall_after(Some(Duration::from_secs(10)));
        let error = super::super::drive_with_control(&mut command, 10_000, &|_| {}, &control)
            .expect_err("kernel time alone is not progress");
        assert_eq!(error.code, error_codes::RENDER_STALLED, "{error:?}");
    }

    /// A stand-in that writes nothing but keeps one core busy (a reversed clip
    /// being read ahead) runs to its end.
    #[cfg(unix)]
    #[test]
    fn a_busy_stand_in_without_output_is_not_stalled() {
        let mut command = std::process::Command::new("sh");
        command.args([
            "-c",
            "end=$(( $(date +%s) + 16 )); while :; do i=0; while [ $i -lt 100000 ]; do i=$((i+1)); \
             done; [ \"$(date +%s)\" -lt \"$end\" ] || break; done",
        ]);
        let control =
            super::super::RenderProcessControl::bounded(Duration::from_secs(90), || false)
                .with_stall_after(Some(Duration::from_secs(10)));
        super::super::drive_with_control(&mut command, 10_000, &|_| {}, &control)
            .expect("processor work is not a stall");
    }

    /// Windows: the same two stand-ins (cmd waiting on ping: no output, almost
    /// no processor time; PowerShell counting for 16 s: busy, no output).
    #[cfg(windows)]
    #[test]
    fn windows_stand_in_ffmpeg_that_stops_working_is_ended() {
        let mut command = std::process::Command::new("cmd");
        command.args([
            "/C",
            "echo out_time_us=1500000&& ping -n 600 127.0.0.1 >NUL",
        ]);
        let control =
            super::super::RenderProcessControl::bounded(Duration::from_secs(90), || false)
                .with_stall_after(Some(Duration::from_secs(10)));
        let started = Instant::now();
        let error = super::super::drive_with_control(&mut command, 10_000, &|_| {}, &control)
            .expect_err("a stand-in that stops working must not read running for ever");
        let took = started.elapsed();
        assert_eq!(error.code, error_codes::RENDER_STALLED, "{error:?}");
        assert!(
            took >= Duration::from_secs(10) && took < Duration::from_secs(40),
            "{took:?}"
        );
        assert!(error.cause.contains("stopped at 1.5 s"), "{}", error.cause);
    }

    #[cfg(windows)]
    #[test]
    fn windows_busy_stand_in_without_output_is_not_stalled() {
        let mut command = std::process::Command::new("powershell");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$end = (Get-Date).AddSeconds(16); while ((Get-Date) -lt $end) { $x = 0; for ($i = 0; $i -lt 20000; $i++) { $x += $i } }",
        ]);
        let control =
            super::super::RenderProcessControl::bounded(Duration::from_secs(90), || false)
                .with_stall_after(Some(Duration::from_secs(10)));
        super::super::drive_with_control(&mut command, 10_000, &|_| {}, &control)
            .expect("processor work is not a stall");
    }

    /// The reading itself, on every system: a busy child's tree is found and
    /// its own processor time grows. (On macOS this read nothing until the
    /// pid-count fix, so no stall was ever seen there.)
    #[cfg(any(unix, windows))]
    #[test]
    fn the_render_tree_reading_sees_a_busy_child() {
        #[cfg(unix)]
        let mut command = {
            let mut c = std::process::Command::new("sh");
            c.args([
                "-c",
                "end=$(( $(date +%s) + 5 )); while :; do i=0; while [ $i -lt 100000 ]; do i=$((i+1)); \
                 done; [ \"$(date +%s)\" -lt \"$end\" ] || break; done",
            ]);
            c
        };
        #[cfg(windows)]
        let mut command = {
            let mut c = std::process::Command::new("powershell");
            c.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$end = (Get-Date).AddSeconds(5); while ((Get-Date) -lt $end) { $x = 0; for ($i = 0; $i -lt 20000; $i++) { $x += $i } }",
            ]);
            c
        };
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let control =
            super::super::RenderProcessControl::bounded(Duration::from_secs(60), || false);
        let mut child =
            super::super::ManagedChild::spawn(&mut command, control, "reading test").unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        let first = activity::usage(&child.tree).expect("a reading of the running tree");
        std::thread::sleep(Duration::from_millis(1500));
        let second = activity::usage(&child.tree).expect("a second reading");
        assert!(
            second.cpu > first.cpu + Duration::from_millis(300),
            "user time must grow for a busy child: {first:?} -> {second:?}"
        );
        child.stop_and_reap("reading test").unwrap();
    }

    #[test]
    fn a_plain_stall_names_no_memory_cause() {
        let seen = Stalled {
            idle: Duration::from_secs(120),
            cpu_used: Duration::from_millis(100),
            output_ms: None,
            memory_held: false,
            memory: None,
        };
        let error = stall_error(&seen);
        assert!(error.message.contains("made no progress for 120 s"));
        assert!(error.cause.contains("no picture yet"));
        assert!(!error.cause.contains("memory limit"));
        assert!(error.suggested_action.unwrap().contains("Export again"));
    }
}
