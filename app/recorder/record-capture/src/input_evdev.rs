//! input_evdev.rs — global input capture via evdev (`/dev/input`), for WAYLAND.
//!
//! Wayland deliberately blocks passive global input hooks (anti-keylogger), so the
//! rdevin/X11 path doesn't work there. The proven cross-compositor approach — used by
//! showmethekey (libinput) and wshowkeys (evdev) — is to read the kernel input
//! devices directly. This is PASSIVE (does NOT grab input from the desktop, unlike
//! the InputCapture portal) but needs read access to `/dev/input/event*`: membership
//! in the `input` group (`sudo usermod -aG input <user>` + re-login), reported by
//! `doctor`. Captures clicks (BTN_*), keys (KEY_*, opt-in), and scroll wheels.
//!
//! CURSOR POSITION CAVEAT: relative mice report REL_X/REL_Y deltas, not an absolute
//! position. We accumulate from screen center and clamp — APPROXIMATE (constant
//! offset + slow edge drift). Wayland PipeWire metadata can later pair each click to
//! a fresh absolute cursor sample; until then this module marks every click
//! `Approximate` so no consumer silently treats the accumulator as exact.

use std::collections::HashSet;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use evdev::{EventSummary, EventType, InputEvent, KeyCode, RelativeAxisCode, SynchronizationCode};
use record_core::{
    error_codes, ClickPositionQuality, ClickSample, CursorSample, KeySample, MouseButton,
    RecordError, Result, ScrollSample,
};

use crate::cursor_correlation::{EvdevEvidence, ObservedMotion};
use crate::input::Input;

fn map_btn(k: KeyCode) -> Option<MouseButton> {
    match k {
        KeyCode::BTN_LEFT => Some(MouseButton::Left),
        KeyCode::BTN_RIGHT => Some(MouseButton::Right),
        KeyCode::BTN_MIDDLE => Some(MouseButton::Middle),
        _ => None,
    }
}

/// True if at least one `/dev/input/event*` device is readable (i.e. `input` group
/// or root). Used by `doctor` to report the Wayland input-capture prerequisite.
pub fn evdev_readable() -> bool {
    evdev::enumerate().next().is_some()
}

pub(crate) type EvdevSnapshot = (
    Vec<CursorSample>,
    Vec<ClickSample>,
    Vec<ScrollSample>,
    Vec<KeySample>,
);

fn event_paths() -> std::io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir("/dev/input")? {
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.strip_prefix("event").is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
                })
            })
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn uevent_sequence() -> std::io::Result<String> {
    std::fs::read_to_string("/sys/kernel/uevent_seqnum").map(|value| value.trim().to_owned())
}

fn set_raw_nonblocking_and_monotonic(dev: &evdev::raw_stream::RawDevice) -> (bool, bool) {
    let fd = dev.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return (false, false);
    }
    // Linux EVIOCSCLOCKID = _IOW('E', 0xa0, int). On an unsupported ABI the
    // request fails and held positions stay disabled; ordinary input still runs.
    let clock_id: libc::c_int = libc::CLOCK_MONOTONIC;
    let monotonic = (unsafe { libc::ioctl(fd, 0x4004_45a0_u64 as libc::c_ulong, &clock_id) }) == 0;
    (true, monotonic)
}

/// Sample state for every evdev reader. `accepting` lives under this mutex with
/// the vectors, so sealing cannot race a callback that already passed the gate.
struct EvdevState {
    accepting: bool,
    input: Input,
    evidence: EvdevEvidence,
}

/// Owns every Wayland evdev reader until a sealed snapshot has joined them.
///
/// The external capture stop remains a terminal input for the readers. The
/// private cancellation signal makes sealing self-contained, including callers
/// that need to close input before the outer capture stop is published.
pub(crate) struct EvdevListener {
    state: Arc<Mutex<EvdevState>>,
    cancel: Arc<AtomicBool>,
    external_stop: Arc<AtomicBool>,
    readers: Vec<JoinHandle<()>>,
    initial_paths: Option<Vec<PathBuf>>,
    initial_uevent: Option<String>,
}

impl EvdevListener {
    fn new(external_stop: Arc<AtomicBool>, screen_w: u32, screen_h: u32) -> Self {
        Self {
            state: Arc::new(Mutex::new(EvdevState {
                accepting: true,
                input: Input {
                    last: (screen_w as f64 / 2.0, screen_h as f64 / 2.0),
                    ..Input::default()
                },
                evidence: EvdevEvidence::default(),
            })),
            cancel: Arc::new(AtomicBool::new(false)),
            external_stop,
            readers: Vec::new(),
            initial_paths: None,
            initial_uevent: None,
        }
    }

    /// Close the acceptance gate, stop all readers, then return a half-open
    /// snapshot only after every reader has exited.
    pub(crate) fn seal(self, duration_ms: u64) -> Result<EvdevSnapshot> {
        self.seal_with_evidence(duration_ms)
            .map(|(snapshot, _)| snapshot)
    }

    pub(crate) fn seal_with_evidence(
        mut self,
        duration_ms: u64,
    ) -> Result<(EvdevSnapshot, EvdevEvidence)> {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.accepting = false;
        }
        self.cancel.store(true, Ordering::Release);

        let reader_panicked = self.join_readers();
        if reader_panicked {
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "evdev input reader panicked while sealing capture",
                "a Wayland evdev reader did not exit cleanly",
            )
            .with_action("retry the recording; if this repeats, collect the Linux capture logs"));
        }

        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let snapshot = snapshot_before(&state.input, duration_ms);
        let mut evidence = state.evidence.clone();
        let paths_unchanged = self
            .initial_paths
            .as_ref()
            .zip(event_paths().ok().as_ref())
            .is_some_and(|(before, after)| before == after);
        let topology_unchanged = self
            .initial_uevent
            .as_ref()
            .zip(uevent_sequence().ok().as_ref())
            .is_some_and(|(before, after)| before == after);
        evidence.complete &= paths_unchanged
            && topology_unchanged
            && evidence.button_times.len() == state.input.clicks.len();
        // Keep motion read after the video boundary: its kernel timestamp may
        // precede an included button, and dropping it could falsely validate a
        // held or fresh SPA position. The proof compares kernel event order.
        evidence.button_times = state
            .input
            .clicks
            .iter()
            .zip(&evidence.button_times)
            .filter(|(click, _)| click.t_ms < duration_ms)
            .map(|(_, &time)| time)
            .collect();
        Ok((snapshot, evidence))
    }

    fn join_readers(&mut self) -> bool {
        let mut reader_panicked = false;
        for reader in self.readers.drain(..) {
            if reader.join().is_err() {
                reader_panicked = true;
            }
        }
        reader_panicked
    }
}

impl Drop for EvdevListener {
    fn drop(&mut self) {
        // A capture error before `seal` must still not leave evdev readers alive.
        // Normal sealing drains this list first and reports a reader panic through
        // its Result; Drop has no error channel, so it can only make cleanup best effort.
        self.cancel.store(true, Ordering::Release);
        if self.join_readers() {
            eprintln!("evdev: reader panicked during cleanup");
        }
    }
}

fn readers_should_stop(cancel: &AtomicBool, external_stop: &AtomicBool) -> bool {
    cancel.load(Ordering::Acquire) || external_stop.load(Ordering::Acquire)
}

/// Runs an append only while the shared sample gate is open. The gate check and
/// vector mutation share the same lock, so a callback queued before sealing cannot
/// append after the seal operation acquires that lock.
#[cfg(test)]
fn append_if_accepting(state: &Mutex<EvdevState>, append: impl FnOnce(&mut Input)) -> bool {
    let mut state = state.lock().unwrap();
    if !state.accepting {
        return false;
    }
    append(&mut state.input);
    true
}

fn snapshot_before(input: &Input, duration_ms: u64) -> EvdevSnapshot {
    let (mut cursor, mut clicks, mut scrolls, mut keys) = input.snapshot();
    cursor.retain(|sample| sample.t_ms < duration_ms);
    clicks.retain(|sample| sample.t_ms < duration_ms);
    scrolls.retain(|sample| sample.t_ms < duration_ms);
    keys.retain(|sample| sample.t_ms < duration_ms);
    (cursor, clicks, scrolls, keys)
}

fn append_raw_event(
    state: &Mutex<EvdevState>,
    event: InputEvent,
    t: u64,
    capture_keys: bool,
    sw: f64,
    sh: f64,
) -> bool {
    let mut state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !state.accepting {
        state.evidence.complete = false;
        return false;
    }
    let kernel_time = event.timestamp();
    match event.destructure() {
        EventSummary::Synchronization(_, SynchronizationCode::SYN_DROPPED, _) => {
            state.evidence.complete = false;
        }
        EventSummary::RelativeAxis(_, RelativeAxisCode::REL_X, v) => {
            if v != 0 {
                state.evidence.motion.push(ObservedMotion {
                    observed_ms: t,
                    kernel_time,
                });
            }
            state.input.last.0 = (state.input.last.0 + v as f64).clamp(0.0, sw);
            let (x, y) = state.input.last;
            state.input.cursor.push(CursorSample { t_ms: t, x, y });
        }
        EventSummary::RelativeAxis(_, RelativeAxisCode::REL_Y, v) => {
            if v != 0 {
                state.evidence.motion.push(ObservedMotion {
                    observed_ms: t,
                    kernel_time,
                });
            }
            state.input.last.1 = (state.input.last.1 + v as f64).clamp(0.0, sh);
            let (x, y) = state.input.last;
            state.input.cursor.push(CursorSample { t_ms: t, x, y });
        }
        EventSummary::AbsoluteAxis(_, _, _) => {
            // This route has no absolute frame mapping; its presence invalidates
            // a held SPA point without making the relative fallback exact.
            state.evidence.motion.push(ObservedMotion {
                observed_ms: t,
                kernel_time,
            });
        }
        EventSummary::RelativeAxis(_, RelativeAxisCode::REL_WHEEL, v) => {
            let (x, y) = state.input.last;
            state.input.scrolls.push(ScrollSample {
                t_ms: t,
                x,
                y,
                dx: 0.0,
                dy: v as f64,
            });
        }
        EventSummary::RelativeAxis(_, RelativeAxisCode::REL_HWHEEL, v) => {
            let (x, y) = state.input.last;
            state.input.scrolls.push(ScrollSample {
                t_ms: t,
                x,
                y,
                dx: v as f64,
                dy: 0.0,
            });
        }
        EventSummary::Key(_, code, value) => {
            if value == 2 {
                return true;
            }
            let down = value == 1;
            if let Some(button) = map_btn(code) {
                let (x, y) = state.input.last;
                state.input.clicks.push(ClickSample {
                    t_ms: t,
                    x,
                    y,
                    button,
                    down,
                    position_quality: ClickPositionQuality::Approximate,
                });
                state.evidence.button_times.push(kernel_time);
            } else if capture_keys {
                state.input.keys.push(KeySample {
                    t_ms: t,
                    key: format!("{code:?}"),
                    down,
                });
            }
        }
        _ => {}
    }
    true
}

#[derive(Default)]
struct DeviceRecovery {
    dropping: bool,
    recovered: bool,
    pressed: HashSet<KeyCode>,
}

enum RawDisposition {
    Ordinary,
    Dropped,
    Recover,
    Discard,
}

impl DeviceRecovery {
    fn classify(&mut self, event: InputEvent) -> RawDisposition {
        match event.destructure() {
            EventSummary::Synchronization(_, SynchronizationCode::SYN_DROPPED, _) => {
                self.dropping = true;
                RawDisposition::Dropped
            }
            EventSummary::Synchronization(_, SynchronizationCode::SYN_REPORT, _)
                if self.dropping =>
            {
                RawDisposition::Recover
            }
            _ if self.dropping => RawDisposition::Discard,
            _ => RawDisposition::Ordinary,
        }
    }

    fn observe_key(&mut self, event: InputEvent, capture_keys: bool) -> bool {
        let EventSummary::Key(_, code, value) = event.destructure() else {
            return true;
        };
        if map_btn(code).is_none() && !capture_keys {
            return false;
        }
        match value {
            0 => {
                let changed = self.pressed.remove(&code);
                !self.recovered || changed
            }
            1 => {
                let changed = self.pressed.insert(code);
                !self.recovered || changed
            }
            _ => false,
        }
    }

    fn reconcile(&mut self, current: HashSet<KeyCode>) -> Vec<InputEvent> {
        let mut changed: Vec<_> = self
            .pressed
            .symmetric_difference(&current)
            .copied()
            .collect();
        changed.sort_unstable_by_key(|code| code.0);
        let edges = changed
            .into_iter()
            .map(|code| {
                InputEvent::new(EventType::KEY.0, code.0, i32::from(current.contains(&code)))
            })
            .collect();
        self.pressed = current;
        self.dropping = false;
        self.recovered = true;
        edges
    }
}

/// Spawn evdev reader threads (one per key/rel device) in an owned listener.
/// Cursor is seeded at the screen center and accumulated from relative motion
/// (see caveat above). A listener with zero readable devices is still valid and
/// seals to an empty snapshot.
pub(crate) fn spawn_evdev_listener(
    start: Instant,
    external_stop: Arc<AtomicBool>,
    capture_keys: bool,
    screen_w: u32,
    screen_h: u32,
) -> EvdevListener {
    let mut listener = EvdevListener::new(external_stop, screen_w, screen_h);
    let (sw, sh) = (screen_w as f64, screen_h as f64);
    listener.initial_uevent = uevent_sequence().ok();
    listener.initial_paths = event_paths().ok();
    let mut opened = 0usize;
    let mut inventory_complete =
        listener.initial_uevent.is_some() && listener.initial_paths.is_some();
    listener
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .evidence
        .complete = true;
    for path in listener.initial_paths.clone().unwrap_or_default() {
        let mut dev = match evdev::raw_stream::RawDevice::open(&path) {
            Ok(dev) => dev,
            Err(error) => {
                inventory_complete = false;
                eprintln!("evdev: cannot observe {}: {error}", path.display());
                continue;
            }
        };
        // Only devices that emit keys/buttons or relative motion are interesting.
        let evs = dev.supported_events();
        if !(evs.contains(EventType::KEY)
            || evs.contains(EventType::RELATIVE)
            || evs.contains(EventType::ABSOLUTE))
        {
            continue;
        }
        // `fetch_events` blocks by default. Readers must poll so the owned
        // listener can observe either its private cancellation or external stop
        // and join deterministically during sealing.
        let (nonblocking, monotonic) = set_raw_nonblocking_and_monotonic(&dev);
        if !nonblocking {
            eprintln!("evdev: cannot poll {} without blocking", path.display());
            inventory_complete = false;
            continue;
        }
        if !monotonic {
            eprintln!(
                "evdev: cannot establish monotonic timestamps for {}",
                path.display()
            );
            inventory_complete = false;
        }
        if std::env::var("SHELLX_RECORD_DEBUG").is_ok() {
            eprintln!(
                "evdev: reading {} ({})",
                path.display(),
                dev.name().unwrap_or("?")
            );
        }
        opened += 1;
        let state = listener.state.clone();
        let cancel = listener.cancel.clone();
        let external_stop = listener.external_stop.clone();
        listener.readers.push(thread::spawn(move || {
            let mut recovery = DeviceRecovery::default();
            match dev.get_key_state() {
                Ok(keys) => {
                    recovery.pressed = keys
                        .iter()
                        .filter(|code| map_btn(*code).is_some() || capture_keys)
                        .collect();
                }
                Err(error) => {
                    state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .evidence
                        .complete = false;
                    eprintln!(
                        "evdev: cannot inspect initial key state on {}: {error}",
                        path.display()
                    );
                }
            }
            loop {
                let stopping = readers_should_stop(&cancel, &external_stop);
                let events = match dev.fetch_events() {
                    Ok(e) => e.collect::<Vec<_>>(),
                    // Non-blocking device → poll; only a real error ends the reader.
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if stopping {
                            break;
                        }
                        thread::sleep(std::time::Duration::from_millis(4));
                        continue;
                    }
                    Err(error) => {
                        state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .evidence
                            .complete = false;
                        eprintln!("evdev: reader lost {}: {error}", path.display());
                        break;
                    }
                };
                for event in events {
                    if stopping {
                        state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .evidence
                            .complete = false;
                        continue;
                    }
                    let t = start.elapsed().as_millis() as u64;
                    match recovery.classify(event) {
                        RawDisposition::Dropped => {
                            append_raw_event(&state, event, t, capture_keys, sw, sh);
                        }
                        RawDisposition::Discard => {}
                        RawDisposition::Recover => {
                            // The raw stream offers no compensation after an overrun.
                            // Discard the undefined span, then reconcile button/key
                            // state from the same kernel device. Relative deltas lost
                            // in that span cannot be recovered; Held stays disabled.
                            let current = match dev.get_key_state() {
                                Ok(keys) => keys
                                    .iter()
                                    .filter(|code| map_btn(*code).is_some() || capture_keys)
                                    .collect(),
                                Err(error) => {
                                    eprintln!(
                                        "evdev: cannot recover key state on {}: {error}",
                                        path.display()
                                    );
                                    return;
                                }
                            };
                            for edge in recovery.reconcile(current) {
                                append_raw_event(&state, edge, t, capture_keys, sw, sh);
                            }
                        }
                        RawDisposition::Ordinary if recovery.observe_key(event, capture_keys) => {
                            append_raw_event(&state, event, t, capture_keys, sw, sh);
                        }
                        RawDisposition::Ordinary => {}
                    }
                }
            }
        }));
    }
    if listener.initial_uevent.as_ref() != uevent_sequence().ok().as_ref()
        || listener.initial_paths.as_ref() != event_paths().ok().as_ref()
    {
        inventory_complete = false;
    }
    listener
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .evidence
        .complete &= inventory_complete && opened != 0;
    if opened == 0 {
        eprintln!("evdev: NO readable input devices — need `input` group or root");
    }
    listener
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn test_listener() -> EvdevListener {
        EvdevListener::new(Arc::new(AtomicBool::new(false)), 1920, 1080)
    }

    #[test]
    fn dropped_raw_span_is_discarded_then_kernel_button_state_is_reconciled() {
        let listener = test_listener();
        listener.state.lock().unwrap().evidence.complete = true;
        let mut recovery = DeviceRecovery::default();
        let preexisting_release = InputEvent::new(EventType::KEY.0, KeyCode::BTN_RIGHT.0, 0);
        assert!(recovery.observe_key(preexisting_release, false));
        let down = InputEvent::new(EventType::KEY.0, KeyCode::BTN_LEFT.0, 1);
        assert!(matches!(recovery.classify(down), RawDisposition::Ordinary));
        assert!(recovery.observe_key(down, false));
        append_raw_event(&listener.state, down, 100, false, 1920.0, 1080.0);
        let dropped = InputEvent::new(
            EventType::SYNCHRONIZATION.0,
            SynchronizationCode::SYN_DROPPED.0,
            0,
        );
        assert!(matches!(
            recovery.classify(dropped),
            RawDisposition::Dropped
        ));
        append_raw_event(&listener.state, dropped, 200, false, 1920.0, 1080.0);
        let undefined_release = InputEvent::new(EventType::KEY.0, KeyCode::BTN_LEFT.0, 0);
        assert!(matches!(
            recovery.classify(undefined_release),
            RawDisposition::Discard
        ));
        let report = InputEvent::new(
            EventType::SYNCHRONIZATION.0,
            SynchronizationCode::SYN_REPORT.0,
            0,
        );
        assert!(matches!(recovery.classify(report), RawDisposition::Recover));
        let edges = recovery.reconcile(HashSet::new());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].code(), KeyCode::BTN_LEFT.0);
        assert_eq!(edges[0].value(), 0);
        append_raw_event(&listener.state, edges[0], 300, false, 1920.0, 1080.0);
        assert!(matches!(recovery.classify(down), RawDisposition::Ordinary));
        assert!(recovery.observe_key(down, false));
        append_raw_event(&listener.state, down, 400, false, 1920.0, 1080.0);
        assert!(!recovery.observe_key(down, false));
        let state = listener.state.lock().unwrap();
        assert!(!state.evidence.complete);
        assert_eq!(
            state
                .input
                .clicks
                .iter()
                .map(|click| (click.t_ms, click.down))
                .collect::<Vec<_>>(),
            [(100, true), (300, false), (400, true)]
        );
    }

    #[test]
    fn raw_reader_records_pointer_motion_and_disables_held_after_syn_drop() {
        let listener = test_listener();
        {
            let mut state = listener.state.lock().unwrap();
            state.evidence.complete = true;
        }
        assert!(append_raw_event(
            &listener.state,
            InputEvent::new(EventType::RELATIVE.0, RelativeAxisCode::REL_X.0, 3),
            500,
            false,
            1920.0,
            1080.0,
        ));
        assert!(append_raw_event(
            &listener.state,
            InputEvent::new(EventType::KEY.0, KeyCode::BTN_LEFT.0, 1),
            810,
            false,
            1920.0,
            1080.0,
        ));
        {
            let state = listener.state.lock().unwrap();
            assert_eq!(state.evidence.motion[0].observed_ms, 500);
            assert_eq!(state.evidence.button_times.len(), 1);
            assert_eq!(state.input.clicks.len(), 1);
        }
        assert!(append_raw_event(
            &listener.state,
            InputEvent::new(
                EventType::SYNCHRONIZATION.0,
                SynchronizationCode::SYN_DROPPED.0,
                0
            ),
            811,
            false,
            1920.0,
            1080.0,
        ));
        assert!(!listener.state.lock().unwrap().evidence.complete);
    }

    #[test]
    fn seal_retains_late_read_motion_with_earlier_kernel_time() {
        let listener = test_listener();
        listener
            .state
            .lock()
            .unwrap()
            .evidence
            .motion
            .push(ObservedMotion {
                observed_ms: 12,
                kernel_time: std::time::SystemTime::UNIX_EPOCH
                    + std::time::Duration::from_millis(8),
            });
        let (_, evidence) = listener.seal_with_evidence(10).unwrap();
        assert_eq!(evidence.motion.len(), 1);
        assert_eq!(evidence.motion[0].observed_ms, 12);
    }

    #[test]
    fn seal_filters_button_kernel_times_with_the_same_clicks() {
        let listener = test_listener();
        {
            let mut state = listener.state.lock().unwrap();
            for t_ms in [11, 9] {
                state.input.clicks.push(ClickSample {
                    t_ms,
                    x: 1.0,
                    y: 2.0,
                    button: MouseButton::Left,
                    down: true,
                    position_quality: ClickPositionQuality::Approximate,
                });
                state.evidence.button_times.push(
                    std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(t_ms),
                );
            }
        }
        let (snapshot, evidence) = listener.seal_with_evidence(10).unwrap();
        assert_eq!(
            snapshot
                .1
                .iter()
                .map(|click| click.t_ms)
                .collect::<Vec<_>>(),
            [9]
        );
        assert_eq!(
            evidence.button_times,
            [std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(9)]
        );
    }

    #[test]
    fn sealed_snapshot_is_half_open_for_every_sample_type() {
        let listener = test_listener();
        let state = listener.state.clone();
        assert!(append_if_accepting(&state, |input| {
            for t_ms in [9, 10] {
                input.cursor.push(CursorSample {
                    t_ms,
                    x: 1.0,
                    y: 2.0,
                });
                input.clicks.push(ClickSample {
                    t_ms,
                    x: 1.0,
                    y: 2.0,
                    button: MouseButton::Left,
                    down: true,
                    position_quality: ClickPositionQuality::Approximate,
                });
                input.scrolls.push(ScrollSample {
                    t_ms,
                    x: 1.0,
                    y: 2.0,
                    dx: 3.0,
                    dy: 4.0,
                });
                input.keys.push(KeySample {
                    t_ms,
                    key: "KEY_A".into(),
                    down: true,
                });
            }
        }));

        let (cursor, clicks, scrolls, keys) = listener.seal(10).unwrap();
        assert_eq!(
            cursor.iter().map(|sample| sample.t_ms).collect::<Vec<_>>(),
            [9]
        );
        assert_eq!(
            clicks.iter().map(|sample| sample.t_ms).collect::<Vec<_>>(),
            [9]
        );
        assert_eq!(
            scrolls.iter().map(|sample| sample.t_ms).collect::<Vec<_>>(),
            [9]
        );
        assert_eq!(
            keys.iter().map(|sample| sample.t_ms).collect::<Vec<_>>(),
            [9]
        );
    }

    #[test]
    fn queued_callback_cannot_append_after_the_seal_gate_wins_the_mutex() {
        let listener = test_listener();
        let state = listener.state.clone();
        let mut gate = state.lock().unwrap();
        let queued_state = state.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let callback = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            append_if_accepting(&queued_state, |input| {
                input.cursor.push(CursorSample {
                    t_ms: 1,
                    x: 1.0,
                    y: 1.0,
                });
            })
        });
        ready_rx.recv().unwrap();

        // This is the first operation performed by `seal`, while the queued
        // callback remains blocked on the same mutex.
        gate.accepting = false;
        drop(gate);

        assert!(!callback.join().unwrap());
        assert!(listener.seal(10).unwrap().0.is_empty());
    }

    #[test]
    fn seal_cancels_and_joins_controlled_readers() {
        let mut listener = test_listener();
        let cancel = listener.cancel.clone();
        let joined = Arc::new(AtomicBool::new(false));
        let joined_by_reader = joined.clone();
        listener.readers.push(thread::spawn(move || {
            while !cancel.load(Ordering::Acquire) {
                thread::yield_now();
            }
            joined_by_reader.store(true, Ordering::Release);
        }));

        listener.seal(10).unwrap();
        assert!(joined.load(Ordering::Acquire));
    }

    #[test]
    fn reader_observes_external_stop_before_seal() {
        let external_stop = Arc::new(AtomicBool::new(false));
        let mut listener = EvdevListener::new(external_stop.clone(), 1920, 1080);
        let cancel = listener.cancel.clone();
        let observed_external_stop = Arc::new(AtomicBool::new(false));
        let observed_by_reader = observed_external_stop.clone();
        let external_for_reader = external_stop.clone();
        listener.readers.push(thread::spawn(move || {
            while !readers_should_stop(&cancel, &external_for_reader) {
                thread::yield_now();
            }
            observed_by_reader.store(
                external_for_reader.load(Ordering::Acquire),
                Ordering::Release,
            );
        }));

        external_stop.store(true, Ordering::Release);
        listener.seal(10).unwrap();
        assert!(observed_external_stop.load(Ordering::Acquire));
    }

    #[test]
    fn reader_panic_is_an_explicit_seal_error_after_other_readers_join() {
        let mut listener = test_listener();
        let cancel = listener.cancel.clone();
        let joined = Arc::new(AtomicBool::new(false));
        let joined_by_reader = joined.clone();
        listener.readers.push(thread::spawn(move || {
            while !cancel.load(Ordering::Acquire) {
                thread::yield_now();
            }
            joined_by_reader.store(true, Ordering::Release);
        }));
        listener
            .readers
            .push(thread::spawn(|| panic!("controlled reader panic")));

        let error = listener.seal(10).unwrap_err();
        assert_eq!(error.code, error_codes::CAPTURE);
        assert!(error.message.contains("reader panicked"));
        assert!(joined.load(Ordering::Acquire));
    }

    #[test]
    fn zero_device_listener_seals_to_an_empty_snapshot() {
        let (cursor, clicks, scrolls, keys) = test_listener().seal(10).unwrap();
        assert!(cursor.is_empty());
        assert!(clicks.is_empty());
        assert!(scrolls.is_empty());
        assert!(keys.is_empty());
    }
}
