//! cache.rs — media-tool caches that follow the installed tools.
//!
//! Several answers about ffmpeg are expensive (hardware test encodes, the
//! candidate scan) or are read once (the manual choice). They used to live in
//! `OnceLock`s for the whole process, so installing or choosing an ffmpeg only
//! took effect after the app restarted. Each cache here remembers the tools
//! generation it was computed for; [`refresh`] advances the generation when
//! the set of discoverable ffmpeg/ffprobe files actually changes, and the next
//! read recomputes. A doctor re-scan ("Check again") calls [`refresh`].
//!
//! Invariant: the generation only moves when the fingerprint of the tools
//! changes, so a routine re-scan never repeats the hardware probes.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The current tools generation. Caches compare against it on every read.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// A value derived from the media tools present at one generation.
pub(crate) struct ToolCache<T> {
    slot: Mutex<Option<(u64, T)>>,
}

impl<T: Clone> ToolCache<T> {
    pub(crate) const fn new() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }

    /// The cached value for the current generation, computing it when the
    /// tools changed since it was stored. `init` must not read this same cache.
    pub(crate) fn get_or_init(&self, init: impl FnOnce() -> T) -> T {
        let current = generation();
        let mut slot = self.slot.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((stored, value)) = slot.as_ref() {
            if *stored == current {
                return value.clone();
            }
        }
        let value = init();
        *slot = Some((current, value.clone()));
        value
    }

    /// The cached value only when it belongs to the current generation.
    pub(crate) fn get(&self) -> Option<T> {
        let current = generation();
        let slot = self.slot.lock().unwrap_or_else(|p| p.into_inner());
        slot.as_ref()
            .filter(|(stored, _)| *stored == current)
            .map(|(_, value)| value.clone())
    }

    /// Store a value for the current generation.
    pub(crate) fn set(&self, value: T) {
        let mut slot = self.slot.lock().unwrap_or_else(|p| p.into_inner());
        *slot = Some((generation(), value));
    }
}

/// One discoverable tool file: path plus size and modification time, so an
/// in-place upgrade (same path, new build) also counts as a change.
type FileStamp = (PathBuf, Option<(u64, SystemTime)>);

fn stamp(path: PathBuf) -> FileStamp {
    let meta = std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok().map(|t| (m.len(), t)));
    (path, meta)
}

#[derive(PartialEq, Eq)]
struct Fingerprint {
    manual_choice: Option<String>,
    ffmpeg: Option<FileStamp>,
    ffprobe: Option<FileStamp>,
    candidates: Vec<FileStamp>,
}

static FINGERPRINT: Mutex<Option<Fingerprint>> = Mutex::new(None);

/// Re-read what media tools exist. Returns true when they changed since the
/// previous call, in which case every [`ToolCache`] recomputes on next read.
/// The first call only records the baseline: the caches were filled from the
/// same tools the process started with.
pub fn refresh() -> bool {
    let current = Fingerprint {
        manual_choice: super::read_override_setting(),
        ffmpeg: super::locate_media_tool_uncached("ffmpeg").map(stamp),
        ffprobe: super::locate_media_tool_uncached("ffprobe").map(stamp),
        candidates: super::ffmpeg_candidates().into_iter().map(stamp).collect(),
    };
    let mut stored = FINGERPRINT.lock().unwrap_or_else(|p| p.into_inner());
    let changed = stored.as_ref().is_some_and(|prev| *prev != current);
    *stored = Some(current);
    if changed {
        GENERATION.fetch_add(1, Ordering::AcqRel);
    }
    changed
}

pub(super) fn bump_generation() {
    GENERATION.fetch_add(1, Ordering::AcqRel);
}

#[cfg(test)]
pub(crate) fn advance_for_test() {
    bump_generation();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_recomputes_after_the_generation_moves() {
        static CACHE: ToolCache<u32> = ToolCache::new();
        let mut calls = 0;
        let first = CACHE.get_or_init(|| {
            calls += 1;
            7
        });
        assert_eq!(first, 7);
        assert_eq!(
            CACHE.get_or_init(|| 99),
            7,
            "same generation keeps the value"
        );
        advance_for_test();
        assert_eq!(
            CACHE.get(),
            None,
            "a value from an older generation is stale"
        );
        assert_eq!(CACHE.get_or_init(|| 8), 8);
        assert_eq!(calls, 1);
    }
}
