use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use record_core::{error_codes, RecordError, Result, Settings};
use record_recovery::{Checkpoint, CheckpointFacts};

use super::super::audio_run::DisabledWindowsPauseAudioFactory;
use super::super::input_run::DisabledWindowsPauseInputFactory;
use super::*;
use crate::windows_pause_pilot::{WindowsPausePilotEvent, WindowsPausePilotRequest};
use crate::windows_wgc_run::{
    WgcAcceptedCapture, WgcCaptureRange, WgcCheckpointPublisher, WgcControlFactory,
    WgcNativeControl, WgcRunOwner, WgcStartObservation, WgcStartedControl,
};

type Log = Arc<Mutex<Vec<&'static str>>>;

struct Control(Log);

impl WgcNativeControl for Control {
    fn close(&mut self) -> Result<()> {
        self.0.lock().unwrap().push("close");
        Ok(())
    }
}

struct Factory(Log);

impl WgcControlFactory<String> for Factory {
    type Control = Control;

    fn start(&mut self, _: &String, _: &Path) -> Result<WgcStartedControl<Self::Control>> {
        self.0.lock().unwrap().push("start");
        Ok(WgcStartedControl::new(Control(self.0.clone()), accepted()))
    }
}

struct RejectFactory;

impl WgcControlFactory<String> for RejectFactory {
    type Control = Control;

    fn start(&mut self, _: &String, _: &Path) -> Result<WgcStartedControl<Self::Control>> {
        Err(RecordError::new(
            error_codes::CAPTURE,
            "test WGC start",
            "native refusal",
        ))
    }
}

struct Publisher {
    log: Log,
    next: u64,
}

impl WgcCheckpointPublisher for Publisher {
    fn reserve(&mut self, _: u64) -> Result<(u64, PathBuf)> {
        self.log.lock().unwrap().push("reserve");
        let sequence = self.next;
        self.next += 1;
        Ok((sequence, PathBuf::from("checkpoint.open.mp4")))
    }

    fn verify_and_publish_new(
        &mut self,
        sequence: u64,
        _: &Path,
        facts: CheckpointFacts,
    ) -> Result<Checkpoint> {
        self.log.lock().unwrap().push("publish");
        Ok(Checkpoint {
            sequence,
            file: "checkpoints/segment-000000.mp4".into(),
            bytes: 1,
            sha256: "a".repeat(64),
            media: None,
            facts,
        })
    }
}

fn profile() -> WindowsPausePilotProfile {
    WindowsPausePilotProfile::admit(WindowsPausePilotRequest::screen_video_only(
        format!("shellx-monitor-v1:windows:{}", "a".repeat(64)),
        30.0,
    ))
    .unwrap()
}

fn accepted() -> WgcAcceptedCapture {
    WgcAcceptedCapture::new(
        Settings {
            width: 640,
            height: 360,
            fps: 30.0,
            audio_rate: 48_000,
        },
        Some(WgcCaptureRange::new(0, 0, 640, 360).unwrap()),
    )
    .unwrap()
}

fn observation(start_ms: u64) -> Result<WgcStartObservation> {
    Ok(WgcStartObservation::for_test(
        start_ms,
        Instant::now(),
        1_700_000_000_000 + start_ms,
    ))
}

fn started_lifecycle() -> (WindowsPausePilotThread, Log) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let factory_log = log.clone();
    let publisher_log = log.clone();
    let lifecycle = spawn_owned(
        profile(),
        |_| Some("exact-monitor".to_string()),
        move |target| {
            Ok(WgcRunOwner::new(
                target,
                Factory(factory_log),
                Publisher {
                    log: publisher_log,
                    next: 0,
                },
            ))
        },
        Box::new(DisabledWindowsPauseInputFactory),
        Box::new(DisabledWindowsPauseAudioFactory),
        || 0,
        || observation(10),
        || (60, Instant::now()),
        Duration::from_secs(60),
    )
    .unwrap();
    (lifecycle, log)
}

fn next_event_within(
    events: &WindowsPausePilotEventReceiver,
    timeout: Duration,
) -> WindowsPausePilotEvent {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(event) = events.try_recv().unwrap() {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "worker did not emit its event in time"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn startup_refusal_joins_the_worker_before_no_handle_is_returned() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let result = spawn_owned(
        profile(),
        |_| Some("exact-monitor".to_string()),
        move |target| {
            Ok(WgcRunOwner::new(
                target,
                RejectFactory,
                Publisher { log, next: 0 },
            ))
        },
        Box::new(DisabledWindowsPauseInputFactory),
        Box::new(DisabledWindowsPauseAudioFactory),
        || 0,
        || observation(10),
        || (60, Instant::now()),
        Duration::from_millis(10),
    );

    assert!(matches!(
        result,
        Err(WindowsPausePilotStartError::NativeStartFailed)
    ));
}

#[test]
fn zero_rollover_cadence_is_refused_before_a_worker_thread_starts() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let factory_log = log.clone();
    let publisher_log = log.clone();
    let result = spawn_owned(
        profile(),
        |_| Some("exact-monitor".to_string()),
        move |target| {
            Ok(WgcRunOwner::new(
                target,
                Factory(factory_log),
                Publisher {
                    log: publisher_log,
                    next: 0,
                },
            ))
        },
        Box::new(DisabledWindowsPauseInputFactory),
        Box::new(DisabledWindowsPauseAudioFactory),
        || 0,
        || observation(10),
        || (60, Instant::now()),
        Duration::ZERO,
    );

    assert!(matches!(
        result,
        Err(WindowsPausePilotStartError::NativeStartFailed)
    ));
    assert!(
        log.lock().unwrap().is_empty(),
        "zero cadence must fail before target, WGC, or checkpoint work starts"
    );
}

#[test]
fn idle_recovery_rolls_only_at_the_configured_checkpoint_cadence() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let factory_log = log.clone();
    let publisher_log = log.clone();
    let clock = Arc::new(AtomicU64::new(0));
    let reserve_clock = clock.clone();
    let start_clock = clock.clone();
    let boundary_clock = clock.clone();
    let mut lifecycle = spawn_owned(
        profile(),
        |_| Some("exact-monitor".to_string()),
        move |target| {
            Ok(WgcRunOwner::new(
                target,
                Factory(factory_log),
                Publisher {
                    log: publisher_log,
                    next: 0,
                },
            ))
        },
        Box::new(DisabledWindowsPauseInputFactory),
        Box::new(DisabledWindowsPauseAudioFactory),
        move || reserve_clock.fetch_add(100, Ordering::AcqRel),
        move || observation(start_clock.load(Ordering::Acquire)),
        move || {
            let previous = boundary_clock.fetch_add(100, Ordering::AcqRel);
            (previous.saturating_add(100), Instant::now())
        },
        Duration::from_millis(20),
    )
    .unwrap();
    let events = lifecycle.take_event_receiver().unwrap();
    assert!(matches!(
        events.try_recv().unwrap(),
        Some(WindowsPausePilotEvent::Started { .. })
    ));

    let deadline = Instant::now() + Duration::from_millis(150);
    while log
        .lock()
        .unwrap()
        .iter()
        .filter(|entry| **entry == "start")
        .count()
        < 2
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .filter(|entry| **entry == "start")
            .count()
            >= 2,
        "the supplied 20 ms checkpoint cadence must rotate before a hypothetical 250 ms polling loop"
    );

    lifecycle
        .command_sender()
        .send(WindowsPausePilotCommand::Stop { epoch: 12 })
        .unwrap();
    lifecycle.shutdown_and_join().unwrap();
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .filter(|entry| **entry == "publish")
            .count()
            >= 2,
        "every cadence rollover seals and publishes before reopening"
    );
}

#[test]
fn started_owner_sends_terminal_stop_and_joins_the_owned_thread() {
    let (mut lifecycle, log) = started_lifecycle();
    let done = lifecycle.completion_probe();
    let events = lifecycle.take_event_receiver().unwrap();
    assert!(matches!(
        events.try_recv().unwrap(),
        Some(WindowsPausePilotEvent::Started { .. })
    ));

    lifecycle
        .command_sender()
        .send(WindowsPausePilotCommand::Stop { epoch: 7 })
        .unwrap();
    lifecycle.shutdown_and_join().unwrap();

    assert!(
        done.load(Ordering::Acquire),
        "join must leave no worker alive"
    );
    assert!(matches!(
        next_event_within(&events, Duration::from_millis(100)),
        WindowsPausePilotEvent::StopSealed { epoch: 7, .. }
    ));
    assert_eq!(
        log.lock().unwrap().as_slice(),
        ["reserve", "start", "close", "publish"]
    );
}

#[test]
fn join_after_real_stop_sealed_joins_without_injecting_another_command() {
    let (mut lifecycle, log) = started_lifecycle();
    let done = lifecycle.completion_probe();
    let events = lifecycle.take_event_receiver().unwrap();
    assert!(matches!(
        events.try_recv().unwrap(),
        Some(WindowsPausePilotEvent::Started { .. })
    ));

    lifecycle
        .command_sender()
        .send(WindowsPausePilotCommand::Stop { epoch: 7 })
        .unwrap();
    assert!(matches!(
        next_event_within(&events, Duration::from_millis(100)),
        WindowsPausePilotEvent::StopSealed { epoch: 7, .. }
    ));
    lifecycle.join_after_terminal().unwrap();

    assert!(
        done.load(Ordering::Acquire),
        "normal terminal join must leave no worker alive"
    );
    assert!(
        matches!(
            events.try_recv(),
            Ok(None) | Err(WindowsPausePilotChannelError::Disconnected)
        ),
        "normal terminal join must not inject a second terminal event"
    );
    assert_eq!(
        log.lock().unwrap().as_slice(),
        ["reserve", "start", "close", "publish"]
    );
}

#[test]
fn dropping_the_controller_stops_and_joins_without_a_detached_wgc_thread() {
    let (lifecycle, log) = started_lifecycle();
    let done = lifecycle.completion_probe();
    drop(lifecycle);

    assert!(
        done.load(Ordering::Acquire),
        "owner drop must join the worker"
    );
    assert_eq!(
        log.lock().unwrap().as_slice(),
        ["reserve", "start", "close", "publish"]
    );
}

#[test]
fn dropped_event_channel_still_reaps_before_owner_drop_returns() {
    let (mut lifecycle, log) = started_lifecycle();
    let done = lifecycle.completion_probe();
    drop(lifecycle.take_event_receiver());
    drop(lifecycle);

    assert!(
        done.load(Ordering::Acquire),
        "event-channel loss cannot detach the worker"
    );
    assert_eq!(
        log.lock().unwrap().as_slice(),
        ["reserve", "start", "close", "publish"]
    );
}
