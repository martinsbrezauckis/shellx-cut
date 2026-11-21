use super::*;

#[test]
fn a_closed_private_writer_moves_once_into_the_journal_slot() {
    let root = tempfile::tempdir().unwrap();
    let mut stage = WriterStage::Private(PrivateStaging::create_windows_wgc(root.path()).unwrap());
    let source = match &stage {
        WriterStage::Private(stage) => stage.path().to_path_buf(),
        _ => unreachable!(),
    };
    std::fs::write(&source, b"closed writer bytes").unwrap();
    let logical = root.path().join("logical.mp4");
    install_closed_stage(&mut stage, &logical).unwrap();
    assert_eq!(std::fs::read(&logical).unwrap(), b"closed writer bytes");
    assert!(!source.exists());
    assert!(install_closed_stage(&mut stage, &logical).is_err());
}

#[test]
fn a_private_writer_cannot_replace_an_existing_journal_slot() {
    let root = tempfile::tempdir().unwrap();
    let mut stage = WriterStage::Private(PrivateStaging::create_windows_wgc(root.path()).unwrap());
    let source = match &stage {
        WriterStage::Private(stage) => stage.path().to_path_buf(),
        _ => unreachable!(),
    };
    std::fs::write(&source, b"new").unwrap();
    let logical = root.path().join("logical.mp4");
    std::fs::write(&logical, b"kept").unwrap();
    assert!(install_closed_stage(&mut stage, &logical).is_err());
    assert_eq!(std::fs::read(&logical).unwrap(), b"kept");
}

#[test]
fn no_future_writer_can_overlap_or_leave_a_zero_length_checkpoint() {
    assert!(valid_boundaries(0, 15_000, Some(15_000)));
    assert!(valid_boundaries(15_000, 20_000, None));
    assert!(!valid_boundaries(0, 15_000, Some(14_999)));
    assert!(!valid_boundaries(15_000, 15_000, None));
}

#[test]
fn a_first_writer_without_a_real_picture_cannot_publish() {
    let root = tempfile::tempdir().unwrap();
    let timing = WgcTimingRecorder::new(Instant::now(), 30, &root.path().join("._writer.d/s.mp4"));
    assert!(placed_media_start_ms(None, 0, 0, &timing).is_err());
    assert_eq!(
        placed_media_start_ms(Some(15_020), 15_000, 1, &timing).unwrap(),
        15_020
    );
    assert!(placed_media_start_ms(Some(14_999), 15_000, 1, &timing).is_err());
}

#[test]
fn only_a_successfully_sealed_later_empty_writer_can_hold_the_prior_picture() {
    let root = tempfile::tempdir().unwrap();
    let timing = WgcTimingRecorder::new(Instant::now(), 30, &root.path().join("._empty.d/s.mp4"));
    assert!(placed_media_start_ms(None, 15_000, 1, &timing).is_err());
    timing.writer_sealed(Instant::now(), true);
    assert_eq!(
        placed_media_start_ms(None, 15_000, 1, &timing).unwrap(),
        15_000
    );
    assert!(placed_media_start_ms(None, 0, 0, &timing).is_err());
    timing.writer_sealed(Instant::now(), false);
    assert!(placed_media_start_ms(None, 15_000, 1, &timing).is_err());
}

#[test]
fn preparation_reads_the_next_real_frame_after_publication() {
    use std::sync::atomic::{AtomicU64, Ordering};

    let first_picture = Arc::new(AtomicU64::new(0));
    let observed = first_picture.clone();
    first_picture.store(15_034, Ordering::Release);
    assert_eq!(
        preparation_start(
            15_000,
            Some(Box::new(move || Some(observed.load(Ordering::Acquire))))
        ),
        15_034,
    );
    assert_eq!(preparation_start(15_000, Some(Box::new(|| None))), 15_000);
    assert_eq!(
        preparation_start(15_000, Some(Box::new(|| Some(14_999)))),
        15_000
    );
}

#[test]
fn full_publication_queue_returns_the_unclosed_writer_to_its_owner() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let root = tempfile::tempdir().unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let owner = BackgroundWindowsPublisher {
        sender: Some(sender),
        worker: None,
        first_failure: Arc::new(Mutex::new(None)),
        preparer: None,
    };
    let closed = Arc::new(AtomicUsize::new(0));
    let make = |sequence: u64, closed: Arc<AtomicUsize>| RetiredPart {
        stage: WriterStage::Journal,
        reserved_start_ms: sequence * 15_000,
        media_start_ms: Some(sequence * 15_000),
        end_ms: (sequence + 1) * 15_000,
        boundary_at: Instant::now(),
        next_start_ms: Some((sequence + 1) * 15_000),
        next_media_start: None,
        timing: WgcTimingRecorder::new(
            Instant::now(),
            30,
            &root.path().join(format!("._{sequence}.d/s.mp4")),
        ),
        close_writer: Box::new(move || {
            closed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
    };
    assert!(owner.try_retire(make(0, closed.clone())).is_ok());
    let (failure, returned) = owner.try_retire(make(1, closed.clone())).unwrap_err();
    assert!(failure.cause.contains("queue is full"));
    assert_eq!(closed.load(Ordering::SeqCst), 0);
    (returned.close_writer)().unwrap();
    let queued = receiver.try_recv().unwrap();
    (queued.close_writer)().unwrap();
    assert_eq!(closed.load(Ordering::SeqCst), 2);
}
