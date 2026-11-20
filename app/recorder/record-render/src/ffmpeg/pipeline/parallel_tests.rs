use super::*;
use std::time::Duration;

fn control() -> ProcessControl {
    ProcessControl::bounded(Duration::from_secs(60), || false)
}

fn items(count: u64) -> impl FnMut() -> Result<Option<(u64, u64)>> {
    let mut next = 0;
    move || {
        let item = (next < count).then_some((next, next * 100));
        next += 1;
        Ok(item)
    }
}

#[test]
fn frames_come_back_in_order_whatever_finishes_first() {
    let mut written = Vec::new();
    // Early frames take longest, so they finish after later ones.
    let compose = |index: u64, _| {
        std::thread::sleep(Duration::from_millis(30 - (index % 6) * 5));
        Ok(vec![index as u8])
    };
    let count = stream(6, &control(), items(40), &compose, &mut |frame: Vec<u8>| {
        written.push(frame[0]);
        Ok(true)
    })
    .unwrap();
    assert_eq!(count, 40);
    assert_eq!(written, (0..40).collect::<Vec<u8>>());
}

#[test]
fn several_frames_are_composed_at_the_same_time() {
    let running = std::sync::atomic::AtomicUsize::new(0);
    let most = std::sync::atomic::AtomicUsize::new(0);
    let compose = |index: u64, _| {
        let now = running.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        most.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(20));
        running.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        Ok(vec![index as u8])
    };
    stream(4, &control(), items(24), &compose, &mut |_| Ok(true)).unwrap();
    assert!(most.into_inner() >= 3, "the workers overlap");
}

#[test]
fn a_compositor_panic_is_an_error_and_no_later_frame_is_written() {
    let mut written = Vec::new();
    let compose = |index: u64, _| {
        assert!(index != 5, "compositor panicked");
        Ok(vec![index as u8])
    };
    let error = stream(3, &control(), items(30), &compose, &mut |frame: Vec<u8>| {
        written.push(frame[0]);
        Ok(true)
    })
    .unwrap_err();
    assert!(
        error.cause.contains("compositor thread stopped"),
        "{error:?}"
    );
    assert_eq!(written, vec![0, 1, 2, 3, 4]);
}

#[test]
fn a_compose_error_stops_the_run_with_that_error() {
    let compose = |index: u64, _| {
        if index == 2 {
            return Err(record_core::RecordError::new(
                "camera_sentinel",
                "camera failed",
                "original",
            ));
        }
        Ok(vec![index as u8])
    };
    let error = stream(4, &control(), items(50), &compose, &mut |_| Ok(true)).unwrap_err();
    assert_eq!(error.code, "camera_sentinel");
}

#[test]
fn a_closed_encoder_ends_the_run_without_an_error() {
    let mut taken = 0;
    let count = stream(
        2,
        &control(),
        items(100),
        &|index: u64, _| Ok(vec![index as u8]),
        &mut |_| {
            taken += 1;
            Ok(taken < 10)
        },
    )
    .unwrap();
    assert_eq!(count, 9);
}

#[test]
fn cancellation_stops_the_run() {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stop.clone();
    let control = ProcessControl::bounded(Duration::from_secs(60), move || {
        flag.load(std::sync::atomic::Ordering::SeqCst)
    });
    let mut written = 0;
    let error = stream(
        2,
        &control,
        items(10_000),
        &|index: u64, _| Ok(vec![index as u8]),
        &mut |_| {
            written += 1;
            if written == 20 {
                stop.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(true)
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "render_cancelled");
    assert!(written < 30);
}

#[test]
fn compose_workers_leave_room_for_the_coders_and_bound_memory() {
    assert!((1..=8).contains(&compose_workers(1920 * 1080 * 4)));
    // An 8K frame is 133 MB: at most two in flight within the 1 GiB budget.
    assert!(compose_workers(7680 * 4320 * 4) <= 2);
    assert_eq!(compose_workers(usize::MAX), 1);
}

/// One early frame that takes long must not let the later finished frames pile
/// up: frames in flight (read and not yet written) stay within a fixed bound
/// however many later frames finish meanwhile.
#[test]
fn a_slow_early_frame_does_not_let_finished_frames_pile_up() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let read = AtomicU64::new(0);
    let written = AtomicU64::new(0);
    let most_in_flight = AtomicU64::new(0);
    let workers = 4;
    let mut next = items(400);
    let compose = |index: u64, _| {
        if index == 0 {
            // The early frame finishes long after many later ones could.
            std::thread::sleep(Duration::from_millis(400));
        }
        Ok(vec![index as u8])
    };
    let count = stream(
        workers,
        &control(),
        || {
            let item = next()?;
            if item.is_some() {
                let now = read.fetch_add(1, Ordering::SeqCst) + 1;
                let in_flight = now - written.load(Ordering::SeqCst);
                most_in_flight.fetch_max(in_flight, Ordering::SeqCst);
            }
            Ok(item)
        },
        &compose,
        &mut |_| {
            written.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        },
    )
    .unwrap();
    assert_eq!(count, 400);
    let most = most_in_flight.into_inner();
    assert!(
        most <= in_flight_limit(workers) as u64 + 1,
        "{most} frames were in flight with {workers} workers"
    );
}
