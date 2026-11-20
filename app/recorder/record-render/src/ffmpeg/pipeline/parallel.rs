//! Composing decoded frames on several threads while keeping their order.
//!
//! Composing a 4K frame takes about 60-70 ms on one core. Done one frame at a
//! time between reading and writing, a 62 s 4K polish took two minutes; done
//! in batches (compose several, then write them, then read the next ones) the
//! threads still waited for the encoder pipe and it took about a minute. Here
//! the three stages overlap: the calling thread reads and prepares frames in
//! order, `workers` threads compose them, and a writer thread puts them back
//! in order and feeds the encoder.
//!
//! Frames in flight are bounded by permits, not only by the queues: the reader
//! takes a permit before it reads a frame, and the writer gives it back only
//! when that frame is written. One early frame that takes long therefore
//! stops the reading after `in_flight_limit` frames instead of letting every
//! later finished frame wait in memory for its turn.

use std::collections::BTreeMap;
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::{Arc, Mutex};

use record_core::Result;

use super::super::{ff_err, ProcessControl};

/// How many frames to compose at once for frames of `frame_bytes`: leave two
/// cores for decoding and encoding, and keep the frames in flight (queued,
/// being composed and waiting for their turn, three per worker) under about
/// 1 GiB, so an 8K take does not exhaust memory.
pub fn compose_workers(frame_bytes: usize) -> usize {
    const IN_FLIGHT_BUDGET: usize = 1 << 30;
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let by_memory = IN_FLIGHT_BUDGET / frame_bytes.saturating_mul(3).max(1);
    cores.saturating_sub(2).min(by_memory).clamp(1, 8)
}

/// Read the next whole frame, or `None` at the end of the stream.
pub(super) fn read_frame(stdout: &mut impl Read, frame_bytes: usize) -> Result<Option<Vec<u8>>> {
    let mut frame = vec![0; frame_bytes];
    let mut filled = 0;
    while filled < frame.len() {
        match stdout.read(&mut frame[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(error) => return Err(ff_err("read decode frame", error)),
        }
    }
    Ok((filled == frame.len() && filled > 0).then_some(frame))
}

/// Frames that may be read and not yet written at once: one composing and
/// one queued per worker. `compose_workers` sizes `workers` so this many
/// frames fit the memory budget.
pub(super) fn in_flight_limit(workers: usize) -> usize {
    workers.max(1) * 2
}

/// What the writer does with a composed frame: `Ok(true)` to go on,
/// `Ok(false)` when the encoder closed its input (the render ends there).
pub(super) type Sink<'a> = dyn FnMut(Vec<u8>) -> Result<bool> + Send + 'a;

/// Compose every item `next` yields on `workers` threads and hand the results
/// to `sink` in item order. Returns how many frames `sink` took. The first
/// error (reading, composing, a compositor panic, writing, cancellation) ends
/// the run; frames after it are not written.
pub(super) fn stream<P, C>(
    workers: usize,
    control: &ProcessControl,
    mut next: impl FnMut() -> Result<Option<(P, u64)>>,
    compose: &C,
    sink: &mut Sink<'_>,
) -> Result<u64>
where
    P: Send,
    C: Fn(P, u64) -> Result<Vec<u8>> + Sync,
{
    let workers = workers.max(1);
    // A permit is a token in this queue: sending takes one (blocking at the
    // limit), the writer receiving gives one back.
    let (permit_tx, permit_rx) = sync_channel::<()>(in_flight_limit(workers));
    let (work_tx, work_rx) = sync_channel::<(u64, P, u64)>(workers);
    let (done_tx, done_rx) = sync_channel::<(u64, Result<Vec<u8>>)>(workers);
    let work_rx = Arc::new(Mutex::new(work_rx));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let work_rx = Arc::clone(&work_rx);
            let done_tx = done_tx.clone();
            scope.spawn(move || compose_worker(&work_rx, &done_tx, compose));
        }
        // Only the workers hold the queue's receiver and the results' sender:
        // when they stop, sending work fails and the writer's input ends.
        drop(work_rx);
        drop(done_tx);
        let writer = scope.spawn(move || write_in_order(done_rx, permit_rx, control, sink));
        let mut index = 0_u64;
        let read = (|| {
            loop {
                // No permit: the writer stopped (an error or a closed encoder).
                if permit_tx.send(()).is_err() {
                    break;
                }
                let Some((prepared, time_ms)) = next()? else {
                    break;
                };
                control.check("render recorder frames")?;
                if work_tx.send((index, prepared, time_ms)).is_err() {
                    // The writer stopped (an error or a closed encoder).
                    break;
                }
                index += 1;
            }
            Ok(())
        })();
        drop(work_tx);
        let written = writer
            .join()
            .unwrap_or_else(|_| Err(ff_err("write encode frame", "the writer thread stopped")));
        read.and(written)
    })
}

fn compose_worker<P, C>(
    work_rx: &Mutex<Receiver<(u64, P, u64)>>,
    done_tx: &std::sync::mpsc::SyncSender<(u64, Result<Vec<u8>>)>,
    compose: &C,
) where
    C: Fn(P, u64) -> Result<Vec<u8>>,
{
    loop {
        let item = work_rx.lock().map(|rx| rx.recv());
        let Ok(Ok((index, prepared, time_ms))) = item else {
            return;
        };
        let composed = catch_unwind(AssertUnwindSafe(|| compose(prepared, time_ms)))
            .unwrap_or_else(|_| {
                Err(ff_err(
                    "compose recorder frame",
                    "a compositor thread stopped unexpectedly",
                ))
            });
        if done_tx.send((index, composed)).is_err() {
            return;
        }
    }
}

fn write_in_order(
    done_rx: Receiver<(u64, Result<Vec<u8>>)>,
    permits: Receiver<()>,
    control: &ProcessControl,
    sink: &mut Sink<'_>,
) -> Result<u64> {
    let mut waiting = BTreeMap::new();
    let mut written = 0_u64;
    for (index, composed) in done_rx {
        waiting.insert(index, composed);
        while let Some(composed) = waiting.remove(&written) {
            let frame = composed?;
            control.check("compose recorder frame")?;
            if !sink(frame)? {
                return Ok(written);
            }
            written += 1;
            // This frame is written: its permit lets the reader take the next.
            let _ = permits.recv();
        }
    }
    Ok(written)
}

#[cfg(test)]
#[path = "parallel_tests.rs"]
mod tests;
