use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use record_core::{error_codes, RecordError, Result, Settings};
use record_recovery::{Checkpoint, CheckpointFacts};

use super::*;
use crate::windows_pause_pilot::audio_run::DisabledWindowsPauseAudioFactory;
use crate::windows_pause_pilot::input_run::DisabledWindowsPauseInputFactory;
use crate::windows_pause_pilot::{WindowsPausePilotRequest, WindowsPausePilotStarted};
use crate::windows_wgc_run::{
    WgcAcceptedCapture, WgcCaptureRange, WgcCheckpointPublisher, WgcControlFactory,
    WgcNativeControl, WgcRunOwner, WgcStartObservation, WgcStartedControl,
};

pub(super) type Log = Rc<RefCell<Vec<String>>>;
pub(super) type Resolver = Box<dyn FnMut(&str) -> Option<String>>;
pub(super) type Worker = WindowsPausePilotWorker<String, Factory, Publisher, Resolver>;
pub(super) type DriftWorker = WindowsPausePilotWorker<String, DriftFactory, Publisher, Resolver>;

pub(super) struct Control(Log);

impl WgcNativeControl for Control {
    fn close(&mut self) -> Result<()> {
        self.0.borrow_mut().push("close".into());
        Ok(())
    }
}

pub(super) struct Factory(pub(super) Log);

impl WgcControlFactory<String> for Factory {
    type Control = Control;

    fn start(&mut self, target: &String, _: &Path) -> Result<WgcStartedControl<Self::Control>> {
        self.0.borrow_mut().push(format!("start:{target}"));
        Ok(WgcStartedControl::new(Control(self.0.clone()), accepted()))
    }
}

pub(super) struct DriftFactory {
    log: Log,
    accepted: Rc<RefCell<Vec<WgcAcceptedCapture>>>,
}

impl WgcControlFactory<String> for DriftFactory {
    type Control = Control;

    fn start(&mut self, target: &String, _: &Path) -> Result<WgcStartedControl<Self::Control>> {
        self.log.borrow_mut().push(format!("start:{target}"));
        let accepted = self.accepted.borrow_mut().remove(0);
        Ok(WgcStartedControl::new(Control(self.log.clone()), accepted))
    }
}

struct RejectStartFactory;

impl WgcControlFactory<String> for RejectStartFactory {
    type Control = Control;

    fn start(&mut self, _: &String, _: &Path) -> Result<WgcStartedControl<Self::Control>> {
        Err(RecordError::new(
            error_codes::CAPTURE,
            "test WGC start",
            "refuse native start",
        ))
    }
}

pub(super) struct Publisher {
    pub(super) log: Log,
    pub(super) next: u64,
    pub(super) reject: bool,
}

impl WgcCheckpointPublisher for Publisher {
    fn reserve(&mut self, _: u64) -> Result<(u64, PathBuf)> {
        let sequence = self.next;
        self.next += 1;
        self.log.borrow_mut().push("reserve".into());
        Ok((sequence, PathBuf::from(format!("{sequence}.open.mp4"))))
    }

    fn verify_and_publish_new(
        &mut self,
        sequence: u64,
        _: &Path,
        facts: CheckpointFacts,
    ) -> Result<Checkpoint> {
        self.log.borrow_mut().push("publish".into());
        if self.reject {
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "test publication",
                "reject",
            ));
        }
        Ok(Checkpoint {
            sequence,
            file: format!("checkpoints/segment-{sequence:06}.mp4"),
            bytes: 1,
            sha256: "a".repeat(64),
            media: None,
            held_last_frame_ms: None,
            facts,
        })
    }
}

pub(super) fn accepted() -> WgcAcceptedCapture {
    WgcAcceptedCapture::new(
        Settings {
            width: 640,
            height: 360,
            fps: 30.0,
            audio_rate: 48_000,
        },
        Some(WgcCaptureRange::new(-320, 0, 640, 360).unwrap()),
    )
    .unwrap()
}

pub(super) fn accepted_with_size(width: u32, height: u32) -> WgcAcceptedCapture {
    WgcAcceptedCapture::new(
        Settings {
            width,
            height,
            fps: 30.0,
            audio_rate: 48_000,
        },
        Some(WgcCaptureRange::new(-320, 0, width, height).unwrap()),
    )
    .unwrap()
}

pub(super) fn profile() -> WindowsPausePilotProfile {
    WindowsPausePilotProfile::admit(WindowsPausePilotRequest::screen_video_only(
        exact_monitor_id(),
        30.0,
    ))
    .unwrap()
}

pub(super) fn exact_monitor_id() -> String {
    format!("shellx-monitor-v1:windows:{}", "a".repeat(64))
}

pub(super) fn observation(start_ms: u64) -> Result<WgcStartObservation> {
    Ok(WgcStartObservation::for_test(
        start_ms,
        Instant::now(),
        1_700_000_000_000 + start_ms,
    ))
}

pub(super) fn start_worker(resolutions: Vec<Option<&'static str>>, reject: bool) -> (Worker, Log) {
    let (worker, log, _) = start_worker_with_started(resolutions, reject);
    (worker, log)
}

pub(super) fn start_worker_with_started(
    resolutions: Vec<Option<&'static str>>,
    reject: bool,
) -> (Worker, Log, WindowsPausePilotStarted) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let resolutions = Rc::new(RefCell::new(resolutions));
    let resolver: Resolver = Box::new({
        let resolutions = resolutions.clone();
        move |_id: &str| resolutions.borrow_mut().remove(0).map(str::to_owned)
    });
    let log_for_owner = log.clone();
    let log_for_publisher = log.clone();
    let (worker, started) = WindowsPausePilotWorker::start(
        profile(),
        resolver,
        move |target| {
            Ok(WgcRunOwner::new(
                target,
                Factory(log_for_owner),
                Publisher {
                    log: log_for_publisher.clone(),
                    next: 0,
                    reject,
                },
            ))
        },
        Box::new(DisabledWindowsPauseInputFactory),
        Box::new(DisabledWindowsPauseAudioFactory),
        || 0,
        || observation(10),
    )
    .unwrap();
    let WindowsPausePilotEvent::Started { started } = started else {
        panic!("accepted WGC start must emit native evidence")
    };
    assert_eq!(started.physical_generation, 1);
    assert_eq!(started.observed_start_ms, 10);
    assert_eq!(started.unix_ms, 1_700_000_000_010);
    assert_eq!(started.accepted.settings.width, 640);
    assert_eq!(started.accepted.range.origin_x, -320);
    (worker, log, started)
}

pub(super) fn start_drift_worker(accepted: Vec<WgcAcceptedCapture>) -> (DriftWorker, Log) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let resolver: Resolver = Box::new(|_: &str| Some("monitor:first".to_string()));
    let accepted = Rc::new(RefCell::new(accepted));
    let log_for_owner = log.clone();
    let log_for_publisher = log.clone();
    let accepted_for_owner = accepted.clone();
    let (worker, started) = WindowsPausePilotWorker::start(
        profile(),
        resolver,
        move |target| {
            Ok(WgcRunOwner::new(
                target,
                DriftFactory {
                    log: log_for_owner,
                    accepted: accepted_for_owner,
                },
                Publisher {
                    log: log_for_publisher,
                    next: 0,
                    reject: false,
                },
            ))
        },
        Box::new(DisabledWindowsPauseInputFactory),
        Box::new(DisabledWindowsPauseAudioFactory),
        || 0,
        || observation(10),
    )
    .unwrap();
    assert!(matches!(started, WindowsPausePilotEvent::Started { .. }));
    (worker, log)
}

pub(super) fn process(
    worker: &mut Worker,
    receiver: &WindowsPausePilotCommandReceiver,
    sender: &WindowsPausePilotEventSender,
    start_ms: u64,
    end_ms: u64,
) {
    worker
        .process_available(
            receiver,
            sender,
            || start_ms,
            || observation(start_ms),
            || (end_ms, Instant::now()),
        )
        .unwrap();
}

#[test]
fn native_start_failure_is_not_misclassified_as_an_exact_target_refusal() {
    let resolver: Resolver = Box::new(|_: &str| Some("monitor:first".to_string()));
    let log = Rc::new(RefCell::new(Vec::new()));
    let outcome = WindowsPausePilotWorker::start(
        profile(),
        resolver,
        move |target| {
            Ok(WgcRunOwner::new(
                target,
                RejectStartFactory,
                Publisher {
                    log,
                    next: 0,
                    reject: false,
                },
            ))
        },
        Box::new(DisabledWindowsPauseInputFactory),
        Box::new(DisabledWindowsPauseAudioFactory),
        || 0,
        || observation(10),
    );
    assert!(matches!(
        outcome,
        Err(WindowsPausePilotStartError::NativeStartFailed)
    ));
}
