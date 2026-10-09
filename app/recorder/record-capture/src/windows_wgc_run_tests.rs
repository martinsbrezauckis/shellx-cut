use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use record_core::{error_codes, RecordError, Result, Settings};
use record_recovery::{Checkpoint, CheckpointFacts};

use super::windows_wgc_factory::TimedWgcFactory;
use super::windows_wgc_run::{
    SealedScreenRun, WgcAcceptedCapture, WgcCaptureRange, WgcCheckpointPublisher,
    WgcControlFactory, WgcNativeControl, WgcRunOwner, WgcStartObservation, WgcStartedControl,
};

type TestLog = Rc<RefCell<Vec<String>>>;
type TestOwner = WgcRunOwner<String, FakeFactory, FakePublisher>;

#[derive(Clone)]
struct FakeControl {
    log: TestLog,
    target: String,
}

impl WgcNativeControl for FakeControl {
    fn close(&mut self) -> Result<()> {
        self.log.borrow_mut().push(format!("close:{}", self.target));
        Ok(())
    }
}

struct FakeFactory {
    log: TestLog,
    targets: TestLog,
}

impl WgcControlFactory<String> for FakeFactory {
    type Control = FakeControl;

    fn start(
        &mut self,
        target: &String,
        staging: &Path,
    ) -> Result<WgcStartedControl<Self::Control>> {
        self.log
            .borrow_mut()
            .push(format!("start:{target}:{}", staging.display()));
        self.targets.borrow_mut().push(target.clone());
        Ok(WgcStartedControl::new(
            FakeControl {
                log: self.log.clone(),
                target: target.clone(),
            },
            accepted(),
        ))
    }
}

struct FakePublisher {
    log: TestLog,
    next: u64,
    reject_publication: bool,
    include_native_startup: bool,
    measured_media_start_ms: Option<u64>,
}

impl WgcCheckpointPublisher for FakePublisher {
    fn reserve(&mut self, _start_ms: u64) -> Result<(u64, PathBuf)> {
        let sequence = self.next;
        self.next += 1;
        let staging = PathBuf::from(format!("checkpoint-{sequence}.open.mp4"));
        self.log
            .borrow_mut()
            .push(format!("reserve:{sequence}:{}", staging.display()));
        Ok((sequence, staging))
    }

    fn capture_start_ms(&self, reserved_start_ms: u64, observed_start_ms: u64) -> u64 {
        if self.include_native_startup {
            reserved_start_ms
        } else {
            observed_start_ms
        }
    }

    fn sealed_media_start_ms(
        &self,
        _sequence: u64,
        reserved_start_ms: u64,
        _end_ms: u64,
    ) -> Result<u64> {
        Ok(self.measured_media_start_ms.unwrap_or(reserved_start_ms))
    }

    fn verify_and_publish_new(
        &mut self,
        sequence: u64,
        staging: &Path,
        facts: CheckpointFacts,
    ) -> Result<Checkpoint> {
        self.log.borrow_mut().push(format!(
            "verify-and-publish-new:{sequence}:{}:{}:{}",
            staging.display(),
            facts.start_ms,
            facts.end_ms
        ));
        if self.reject_publication {
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "fake checkpoint verification failed",
                "fake publisher refuses publication",
            ));
        }
        Ok(Checkpoint {
            sequence,
            file: format!("segment-{sequence:06}.mp4"),
            bytes: 7,
            sha256: format!("sealed-{sequence}"),
            media: None,
            held_last_frame_ms: None,
            facts,
        })
    }
}

fn accepted() -> WgcAcceptedCapture {
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

fn owner(reject_publication: bool) -> (TestOwner, TestLog, TestLog) {
    owner_with_startup(reject_publication, false)
}

fn owner_with_startup(
    reject_publication: bool,
    include_native_startup: bool,
) -> (TestOwner, TestLog, TestLog) {
    owner_with_media_start(reject_publication, include_native_startup, None)
}

fn owner_with_media_start(
    reject_publication: bool,
    include_native_startup: bool,
    measured_media_start_ms: Option<u64>,
) -> (TestOwner, TestLog, TestLog) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let targets = Rc::new(RefCell::new(Vec::new()));
    let factory = FakeFactory {
        log: log.clone(),
        targets: targets.clone(),
    };
    let publisher = FakePublisher {
        log: log.clone(),
        next: 0,
        reject_publication,
        include_native_startup,
        measured_media_start_ms,
    };
    (
        WgcRunOwner::new("monitor:exact-17".to_string(), factory, publisher),
        log,
        targets,
    )
}

#[test]
fn measured_first_frame_gap_keeps_input_clock_and_terminal_end() {
    let (mut owner, log, _) = owner_with_media_start(false, true, Some(560));
    let identity = owner.begin(0, || started(463)).unwrap();
    assert_eq!(identity.start_ms, 0); // Reservation and input clock stay at zero.
    let sealed = owner
        .stop(observed_after_close(log, 9063))
        .unwrap()
        .unwrap();
    assert_sealed(&sealed, 1, 560, 9063);
}

#[test]
fn measured_first_frame_cannot_cross_its_reserved_or_terminal_boundary() {
    for invalid_start in [9_064, 0] {
        let (mut owner, log, _) = owner_with_media_start(false, true, Some(invalid_start));
        owner.begin(100, || started(463)).unwrap();
        assert!(owner.stop(observed_after_close(log, 9_063)).is_err());
        assert!(owner.is_stopped());
    }
}

#[test]
fn ordinary_factory_receives_each_exact_checkpoint_reservation() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let reservations = Rc::new(RefCell::new(Vec::new()));
    let observed = reservations.clone();
    let control_log = log.clone();
    let factory = TimedWgcFactory(move |target: &String, _staging: &Path, reserved_ms| {
        observed.borrow_mut().push(reserved_ms);
        Ok(WgcStartedControl::new(
            FakeControl {
                log: control_log.clone(),
                target: target.clone(),
            },
            accepted(),
        ))
    });
    let publisher = FakePublisher {
        log: log.clone(),
        next: 0,
        reject_publication: false,
        include_native_startup: true,
        measured_media_start_ms: None,
    };
    let mut owner = WgcRunOwner::new("monitor:exact-17".to_string(), factory, publisher);
    owner.begin(0, || started(10)).unwrap();
    owner
        .rollover_checkpoint(observed_after_close(log, 100), || 125, || started(130))
        .unwrap();
    assert_eq!(*reservations.borrow(), [0, 125]);
}

#[test]
fn ordinary_wgc_checkpoint_covers_frames_encoded_during_native_startup() {
    // Retained PC4 native evidence: reserve=0, post-open observation=463,
    // close=7373, encoded media=7533 ms. The media exceeds a post-open span
    // by 623 ms, but fits the reserved span with 160 ms of encoder drain.
    let (mut owner, log, _) = owner_with_startup(false, true);
    let identity = owner.begin(0, || started(463)).unwrap();
    assert_eq!(identity.start_ms, 0);
    assert_eq!(identity.started.start_ms, 463);
    let sealed = owner
        .stop(observed_after_close(log, 7373))
        .unwrap()
        .unwrap();
    assert_sealed(&sealed, 1, 0, 7373);
    assert!(7533 > 7373 - identity.started.start_ms + 200);
    assert!(7533 <= sealed.boundary.end_ms - sealed.boundary.start_ms + 200);
}

fn started(start_ms: u64) -> Result<WgcStartObservation> {
    Ok(WgcStartObservation::for_test(
        start_ms,
        Instant::now(),
        1_700_000_000_000 + start_ms,
    ))
}

fn assert_sealed(run: &SealedScreenRun, physical_generation: u64, start_ms: u64, end_ms: u64) {
    assert_eq!(run.boundary.physical_generation, physical_generation);
    assert_eq!(run.boundary.start_ms, start_ms);
    assert_eq!(run.boundary.end_ms, end_ms);
    assert_eq!(run.boundary.event_offset_ms, start_ms);
    assert_eq!(run.checkpoint.sequence, physical_generation - 1);
    assert_eq!(run.checkpoint.facts.start_ms, start_ms);
    assert_eq!(run.checkpoint.facts.event_offset_ms, start_ms);
    assert_eq!(run.checkpoint.facts.end_ms, end_ms);
    assert_eq!(run.accepted, accepted());
}

fn observed_after_close(log: TestLog, end_ms: u64) -> impl FnOnce() -> u64 {
    move || {
        log.borrow_mut().push(format!("observe-closed:{end_ms}"));
        end_ms
    }
}

#[test]
fn checkpoint_rollover_keeps_contiguous_physical_segments_inside_one_active_run() {
    let (mut owner, log, targets) = owner(false);
    let first = owner.begin(90, || started(100)).unwrap();
    assert_eq!(first.physical_generation, 1);
    assert_eq!(first.started.unix_ms, 1_700_000_000_100);

    let (sealed, second) = owner
        .rollover_checkpoint(
            observed_after_close(log.clone(), 160),
            || 160,
            || started(160),
        )
        .unwrap();
    assert_sealed(&sealed, 1, 100, 160);
    assert_eq!(second.physical_generation, 2);
    assert_eq!(second.start_ms, 160);

    let final_run = owner
        .stop(observed_after_close(log.clone(), 220))
        .unwrap()
        .unwrap();
    assert_sealed(&final_run, 2, 160, 220);
    assert_eq!(
        targets.borrow().as_slice(),
        ["monitor:exact-17", "monitor:exact-17"]
    );
    assert_eq!(
        log.borrow().as_slice(),
        [
            "reserve:0:checkpoint-0.open.mp4",
            "start:monitor:exact-17:checkpoint-0.open.mp4",
            "close:monitor:exact-17",
            "observe-closed:160",
            "verify-and-publish-new:0:checkpoint-0.open.mp4:100:160",
            "reserve:1:checkpoint-1.open.mp4",
            "start:monitor:exact-17:checkpoint-1.open.mp4",
            "close:monitor:exact-17",
            "observe-closed:220",
            "verify-and-publish-new:1:checkpoint-1.open.mp4:160:220",
        ]
    );
}

#[test]
fn checkpoint_rollover_reserves_after_native_close_and_publication() {
    // PC4 r10 had already passed the 15,003 ms cadence deadline when close
    // finished at 15,112 ms. Using that old deadline for the next native
    // startup made the two published checkpoint facts overlap by 109 ms.
    let (mut owner, log, _) = owner_with_startup(false, true);
    owner.begin(0, || started(100)).unwrap();
    let (first, second) = owner
        .rollover_checkpoint(
            observed_after_close(log.clone(), 15_112),
            || {
                log.borrow_mut().push("reserve-clock:15113".into());
                15_113
            },
            || started(15_300),
        )
        .unwrap();
    assert_sealed(&first, 1, 0, 15_112);
    assert_eq!(second.start_ms, 15_113);
    let (second_sealed, third) = owner
        .rollover_checkpoint(
            observed_after_close(log.clone(), 30_116),
            || {
                log.borrow_mut().push("reserve-clock:30117".into());
                30_117
            },
            || started(30_300),
        )
        .unwrap();
    assert_sealed(&second_sealed, 2, 15_113, 30_116);
    assert_eq!(third.start_ms, 30_117);
    let last = owner
        .stop(observed_after_close(log.clone(), 32_747))
        .unwrap()
        .unwrap();
    assert_sealed(&last, 3, 30_117, 32_747);

    let events = log.borrow();
    for (checkpoint, clock) in [(0, "reserve-clock:15113"), (1, "reserve-clock:30117")] {
        let published = events
            .iter()
            .position(|event| event.starts_with(&format!("verify-and-publish-new:{checkpoint}:")))
            .unwrap();
        let reserved = events.iter().position(|event| event == clock).unwrap();
        assert!(
            published < reserved,
            "next clock must follow prior publication"
        );
    }
}

#[test]
fn checkpoint_rollover_rejects_a_clock_that_precedes_the_closed_boundary() {
    let (mut owner, log, _) = owner_with_startup(false, true);
    owner.begin(0, || started(100)).unwrap();
    let error = owner
        .rollover_checkpoint(
            observed_after_close(log.clone(), 15_112),
            || 15_003,
            || started(15_300),
        )
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("next WGC checkpoint starts before"));
    assert!(owner.is_stopped());
    assert_eq!(
        log.borrow()
            .iter()
            .filter(|event| event.starts_with("start:"))
            .count(),
        1,
        "a stale reservation must not start another native encoder"
    );
}

#[test]
fn accepted_settings_and_native_range_must_agree_before_start() {
    let mismatch = WgcAcceptedCapture::new(
        Settings {
            width: 640,
            height: 360,
            fps: 30.0,
            audio_rate: 48_000,
        },
        Some(WgcCaptureRange::new(0, 0, 1280, 720).unwrap()),
    );
    assert!(mismatch.is_err());
}

#[test]
fn stop_is_terminal_before_close_and_rejects_every_later_command() {
    let (mut owner, log, _) = owner(false);
    owner.begin(0, || started(10)).unwrap();
    assert_sealed(
        &owner
            .stop(observed_after_close(log.clone(), 60))
            .unwrap()
            .unwrap(),
        1,
        10,
        60,
    );
    let commands_after_stop = log.borrow().clone();

    assert!(owner.is_stopped());
    assert!(owner.begin(61, || started(62)).is_err());
    assert!(owner
        .seal_active_checkpoint(observed_after_close(log.clone(), 62))
        .is_err());
    assert!(owner
        .stop(observed_after_close(log.clone(), 62))
        .unwrap()
        .is_none());
    assert_eq!(*log.borrow(), commands_after_stop);
}

#[test]
fn a_failed_publication_never_returns_a_sealed_run_or_allows_resume() {
    let (mut owner, log, _) = owner(true);
    owner.begin(0, || started(10)).unwrap();

    assert!(owner
        .seal_active_checkpoint(observed_after_close(log.clone(), 60))
        .is_err());
    assert!(owner.is_stopped());
    assert!(owner.begin(61, || started(62)).is_err());
    assert_eq!(
        log.borrow().as_slice(),
        [
            "reserve:0:checkpoint-0.open.mp4",
            "start:monitor:exact-17:checkpoint-0.open.mp4",
            "close:monitor:exact-17",
            "observe-closed:60",
            "verify-and-publish-new:0:checkpoint-0.open.mp4:10:60",
        ]
    );
}
