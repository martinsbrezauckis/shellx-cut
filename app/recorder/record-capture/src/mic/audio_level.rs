//! Fixed-capacity, callback-fed audio level measurements for live recording.
//!
//! A [`RollingAudioLevel`] belongs to one already-admitted native stream. It
//! observes the PCM delivered by that stream; it never selects or opens an
//! audio device itself. The fixed ring prevents an unattended recording from
//! accumulating level samples in memory.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

const BUCKET_COUNT: usize = 8;
const BUCKET_WIDTH: Duration = Duration::from_millis(100);
pub(super) const ROLLING_WINDOW: Duration = Duration::from_millis(800);
pub(super) const STALE_AFTER: Duration = Duration::from_millis(1_200);
const DECAY_HALF_LIFE: Duration = Duration::from_millis(350);
const MIN_DBFS: f32 = -96.0;
const CLIPPING_PEAK: u16 = 32_735;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioLevelLifecycle {
    Active,
    Stopped,
    DeviceLost,
}

/// A read-only level measurement from a [`RollingAudioLevel`].
///
/// Peak and RMS cover only real packets still inside the fixed rolling window.
/// A silent packet is reported as `-96 dBFS`; `None` means no applicable native
/// packet exists. `stale` tells a consumer that the stream has stopped feeding
/// the model and it must not present the retained decay as a live signal.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioLevelSnapshot {
    pub peak_dbfs: Option<f32>,
    pub rms_dbfs: Option<f32>,
    pub decayed_peak_dbfs: Option<f32>,
    pub sample_age_ms: Option<u64>,
    pub stale: bool,
    pub clipping: bool,
    pub lifecycle: AudioLevelLifecycle,
}

/// Fixed-capacity rolling peak/RMS for one callback-fed native stream.
#[derive(Debug)]
pub struct RollingAudioLevel {
    state: Mutex<LevelState>,
}

impl Default for RollingAudioLevel {
    fn default() -> Self {
        Self::new()
    }
}

impl RollingAudioLevel {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(LevelState::default()),
        }
    }

    /// Observe signed 16-bit PCM delivered by the stream's existing callback.
    pub fn observe_i16(&self, samples: &[i16]) {
        self.observe_i16_at(samples, Instant::now());
    }

    /// Observe normalized floating-point PCM delivered by the stream's existing
    /// callback. Non-finite samples are treated as silence so a malformed input
    /// cannot poison a live status projection.
    pub fn observe_f32(&self, samples: &[f32]) {
        self.observe_f32_at(samples, Instant::now());
    }

    /// Observe unsigned 16-bit PCM delivered by the stream's existing callback.
    pub fn observe_u16(&self, samples: &[u16]) {
        self.observe_u16_at(samples, Instant::now());
    }

    /// Observe S16LE PCM bytes delivered by an existing system-audio callback.
    /// A trailing incomplete byte is ignored: its enclosing backend owns packet
    /// validation and must decide whether that packet is writable.
    pub fn observe_s16le_bytes(&self, bytes: &[u8]) {
        self.observe_s16le_bytes_at(bytes, Instant::now());
    }

    /// Take a bounded rolling measurement at the current monotonic instant.
    pub fn snapshot(&self) -> AudioLevelSnapshot {
        self.snapshot_at(Instant::now())
    }

    /// Terminal capture; a known device loss remains more specific.
    pub fn mark_stopped(&self) {
        let mut state = self.lock_state();
        if state.lifecycle == AudioLevelLifecycle::Active {
            state.lifecycle = AudioLevelLifecycle::Stopped;
        }
    }

    /// Record a native device failure; a later Stop cannot overwrite it.
    pub fn mark_device_lost(&self) {
        let mut state = self.lock_state();
        if state.lifecycle == AudioLevelLifecycle::Active {
            state.lifecycle = AudioLevelLifecycle::DeviceLost;
        }
    }

    pub(super) fn observe_i16_at(&self, samples: &[i16], observed_at: Instant) {
        self.observe_block(BlockStats::from_i16(samples), observed_at);
    }

    pub(super) fn observe_f32_at(&self, samples: &[f32], observed_at: Instant) {
        self.observe_block(BlockStats::from_f32(samples), observed_at);
    }

    pub(super) fn observe_u16_at(&self, samples: &[u16], observed_at: Instant) {
        self.observe_block(BlockStats::from_u16(samples), observed_at);
    }

    pub(super) fn observe_s16le_bytes_at(&self, bytes: &[u8], observed_at: Instant) {
        self.observe_block(BlockStats::from_s16le_bytes(bytes), observed_at);
    }

    pub(super) fn snapshot_at(&self, observed_at: Instant) -> AudioLevelSnapshot {
        self.snapshot_inner(observed_at)
    }

    fn observe_block(&self, block: BlockStats, observed_at: Instant) {
        if block.samples == 0 {
            return;
        }
        // Metering is diagnostic and must never make a real-time native audio
        // callback wait behind a status snapshot. Dropping one meter update is
        // preferable to delaying the capture stream that owns the samples.
        let Ok(mut state) = self.state.try_lock() else {
            return;
        };
        if state.lifecycle != AudioLevelLifecycle::Active {
            return;
        }
        state.observe(block, observed_at);
    }

    fn snapshot_inner(&self, observed_at: Instant) -> AudioLevelSnapshot {
        self.lock_state().snapshot(observed_at)
    }

    fn lock_state(&self) -> MutexGuard<'_, LevelState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct LevelBucket {
    last_observed_at: Option<Instant>,
    samples: u64,
    sum_squares: f64,
    peak: u16,
}

impl LevelBucket {
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn record(&mut self, block: BlockStats, observed_at: Instant) {
        self.last_observed_at = Some(observed_at);
        self.samples = self.samples.saturating_add(block.samples);
        self.sum_squares += block.sum_squares;
        self.peak = self.peak.max(block.peak);
    }
}

#[derive(Debug)]
struct LevelState {
    buckets: [LevelBucket; BUCKET_COUNT],
    current_bucket: usize,
    bucket_started_at: Option<Instant>,
    last_observed_at: Option<Instant>,
    decayed_peak: f32,
    decay_anchor: Option<Instant>,
    lifecycle: AudioLevelLifecycle,
}

impl Default for LevelState {
    fn default() -> Self {
        Self {
            buckets: [LevelBucket::default(); BUCKET_COUNT],
            current_bucket: 0,
            bucket_started_at: None,
            last_observed_at: None,
            decayed_peak: 0.0,
            decay_anchor: None,
            lifecycle: AudioLevelLifecycle::Active,
        }
    }
}

impl LevelState {
    fn observe(&mut self, block: BlockStats, observed_at: Instant) {
        self.advance_bucket(observed_at);
        let prior_peak = self.decayed_at(observed_at);
        self.decayed_peak = prior_peak.max(block.peak_ratio());
        self.decay_anchor = Some(observed_at);
        self.last_observed_at = Some(observed_at);
        self.buckets[self.current_bucket].record(block, observed_at);
    }

    fn advance_bucket(&mut self, observed_at: Instant) {
        let Some(started_at) = self.bucket_started_at else {
            self.bucket_started_at = Some(observed_at);
            return;
        };
        let Some(elapsed) = observed_at.checked_duration_since(started_at) else {
            return;
        };
        let width_ns = BUCKET_WIDTH.as_nanos();
        let elapsed_buckets = elapsed.as_nanos() / width_ns;
        if elapsed_buckets == 0 {
            return;
        }
        let steps = usize::try_from(elapsed_buckets).unwrap_or(usize::MAX);
        if steps >= BUCKET_COUNT {
            self.buckets = [LevelBucket::default(); BUCKET_COUNT];
            self.current_bucket = 0;
        } else {
            for _ in 0..steps {
                self.current_bucket = (self.current_bucket + 1) % BUCKET_COUNT;
                self.buckets[self.current_bucket].clear();
            }
        }
        self.bucket_started_at = Some(observed_at);
    }

    fn snapshot(&self, observed_at: Instant) -> AudioLevelSnapshot {
        let age = self
            .last_observed_at
            .and_then(|last| observed_at.checked_duration_since(last));
        let mut peak = 0_u16;
        let mut samples = 0_u64;
        let mut sum_squares = 0.0_f64;
        for bucket in &self.buckets {
            let Some(last_observed_at) = bucket.last_observed_at else {
                continue;
            };
            let Some(bucket_age) = observed_at.checked_duration_since(last_observed_at) else {
                continue;
            };
            if bucket_age > ROLLING_WINDOW {
                continue;
            }
            peak = peak.max(bucket.peak);
            samples = samples.saturating_add(bucket.samples);
            sum_squares += bucket.sum_squares;
        }
        let stale = self.lifecycle != AudioLevelLifecycle::Active
            || age.is_none_or(|age| age > STALE_AFTER);
        AudioLevelSnapshot {
            peak_dbfs: (samples > 0).then(|| dbfs(f32::from(peak) / 32768.0)),
            rms_dbfs: (samples > 0).then(|| dbfs((sum_squares / samples as f64).sqrt() as f32)),
            decayed_peak_dbfs: self
                .last_observed_at
                .map(|_| dbfs(self.decayed_at(observed_at))),
            sample_age_ms: age.map(|age| u64::try_from(age.as_millis()).unwrap_or(u64::MAX)),
            stale,
            clipping: samples > 0 && peak >= CLIPPING_PEAK,
            lifecycle: self.lifecycle,
        }
    }

    fn decayed_at(&self, observed_at: Instant) -> f32 {
        let elapsed = self
            .decay_anchor
            .and_then(|anchor| observed_at.checked_duration_since(anchor))
            .unwrap_or_default();
        let half_lives = elapsed.as_secs_f32() / DECAY_HALF_LIFE.as_secs_f32();
        self.decayed_peak * 0.5_f32.powf(half_lives)
    }
}

#[derive(Clone, Copy)]
struct BlockStats {
    samples: u64,
    sum_squares: f64,
    peak: u16,
}

impl BlockStats {
    fn from_i16(samples: &[i16]) -> Self {
        Self::from_normalized(
            samples
                .iter()
                .copied()
                .map(|sample| sample as f32 / 32768.0),
        )
    }

    fn from_f32(samples: &[f32]) -> Self {
        Self::from_normalized(
            samples
                .iter()
                .copied()
                .map(|sample| (if sample.is_finite() { sample } else { 0.0 }).clamp(-1.0, 1.0)),
        )
    }

    fn from_u16(samples: &[u16]) -> Self {
        Self::from_normalized(
            samples
                .iter()
                .copied()
                .map(|sample| (i32::from(sample) - 32768) as f32 / 32768.0),
        )
    }

    fn from_s16le_bytes(bytes: &[u8]) -> Self {
        Self::from_normalized(
            bytes
                .chunks_exact(2)
                .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32768.0),
        )
    }

    fn from_normalized(samples: impl Iterator<Item = f32>) -> Self {
        let mut result = Self {
            samples: 0,
            sum_squares: 0.0,
            peak: 0,
        };
        for sample in samples {
            let sample = sample.clamp(-1.0, 1.0);
            result.samples = result.samples.saturating_add(1);
            result.sum_squares += f64::from(sample) * f64::from(sample);
            let peak = (sample.abs() * 32768.0).round().clamp(0.0, 32768.0) as u16;
            result.peak = result.peak.max(peak);
        }
        result
    }

    fn peak_ratio(self) -> f32 {
        f32::from(self.peak) / 32768.0
    }
}

fn dbfs(ratio: f32) -> f32 {
    if ratio <= 0.0 || !ratio.is_finite() {
        return MIN_DBFS;
    }
    (20.0 * ratio.log10()).clamp(MIN_DBFS, 0.0)
}
