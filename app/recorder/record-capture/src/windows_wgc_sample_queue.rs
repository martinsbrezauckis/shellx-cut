//! Bound retained GPU sample ownership without blocking the WGC callback.

use std::sync::mpsc::{self, Receiver, SyncSender};

fn capacity(width: u32, height: u32) -> Result<usize, &'static str> {
    const GPU_BUDGET_BYTES: u64 = 256 * 1024 * 1024;
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes > 0)
        .ok_or("WGC video sample geometry exceeds the GPU queue budget")?;
    if bytes > GPU_BUDGET_BYTES {
        return Err("WGC video sample exceeds the 256 MiB GPU queue budget");
    }
    Ok((GPU_BUDGET_BYTES / bytes).min(16) as usize)
}

pub(crate) fn sample_queue<T>(
    width: u32,
    height: u32,
) -> Result<(SyncSender<T>, Receiver<T>), &'static str> {
    // Up to 16 samples absorb transient startup/consumer delay. Queued BGRA
    // pixels stay within 256 MiB (16 at 1080p, 8 at 4K), excluding native
    // resource overhead and the current producer/consumer surfaces.
    Ok(mpsc::sync_channel(capacity(width, height)?))
}

pub(crate) fn offer<T>(sender: &SyncSender<T>, sample: T) -> Result<(), &'static str> {
    sender.try_send(sample).map_err(|error| match error {
        mpsc::TrySendError::Full(_) => {
            "WGC video encoder backlog exceeded the bounded GPU sample queue"
        }
        mpsc::TrySendError::Disconnected(_) => "WGC video sample receiver closed",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalled_consumer_fails_promptly_without_dropping_or_retiming_admitted_samples() {
        let (sender, receiver) = sample_queue(7680, 4320).unwrap();
        for timestamp in [0, 30_333_334] {
            offer(&sender, timestamp).unwrap();
        }
        assert!(offer(&sender, 30_500_001).unwrap_err().contains("backlog"));
        drop(sender);
        assert_eq!(receiver.into_iter().collect::<Vec<_>>(), [0, 30_333_334]);
    }

    #[test]
    fn terminated_consumer_fails_without_waiting_for_another_callback() {
        let (sender, receiver) = sample_queue(1920, 1080).unwrap();
        drop(receiver);
        assert!(offer(&sender, 0).unwrap_err().contains("closed"));
    }

    #[test]
    fn queue_budget_scales_with_admitted_gpu_geometry() {
        assert_eq!(capacity(1920, 1080), Ok(16));
        assert_eq!(capacity(3840, 2160), Ok(8));
        assert_eq!(capacity(7680, 4320), Ok(2));
        assert!(capacity(0, 1080).is_err());
        assert!(capacity(u32::MAX, u32::MAX).is_err());
    }
}
