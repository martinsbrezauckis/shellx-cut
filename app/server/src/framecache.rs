//! framecache.rs — bounded LRU of recently served scrub frames.
//!
//! Role: the human scrubs back and forth over the same few positions; the agent
//! pulls the same frame twice (once to look, once to verify). Caching the
//! encoded JPEG bytes turns a repeat scrub into a memcpy instead of a second
//! ffmpeg spawn. Small + in-memory + bounded — derived state, dropped freely.
//!
//! KEY: (edl_rev, at_ms, height, mode). `edl_rev` is a hash of the PROJECT dir +
//! the current op id (dispatch::edl_rev) — project-scoped so frames never leak
//! between projects, and op-sensitive so ANY edit invalidates every cached
//! frame for the old timeline by simply changing the key — no explicit
//! invalidation pass, and never a stale frame served after an edit. `mode`
//! separates the fast scrub frame from the exact composed (compose=1) frame at
//! the same position (they can differ — captions/overlays).
//!
//! Dependencies: std only. Primary caller: dispatch.rs (scrub_frame_bytes).

use std::collections::HashMap;
use std::sync::Mutex;

/// Which frame path produced the bytes — keeps the fast and the exact composed
/// frame at the same (rev, at_ms, h) from colliding in the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameMode {
    /// Fast scrub frame (proxy seek, no captions/overlay).
    Scrub,
    /// Exact composed frame (full graph, captions + overlays).
    Compose,
    /// Past-the-timeline black frame. Distinct because it is cached but never
    /// reports the fast-proxy header.
    Black,
}

/// Cache key: timeline revision + position + height + mode.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    edl_rev: u64,
    at_ms: u64,
    height: u32,
    mode: FrameMode,
}

/// A tiny LRU (insertion-order eviction by a monotonic tick). Bounded by both
/// `cap` entries and `byte_cap` retained JPEG bytes, so a few high-resolution
/// previews cannot turn a count-only cache into an unbounded memory sink.
/// Thread-safe via one Mutex — the hot path is a hashmap get.
pub struct FrameCache {
    inner: Mutex<Inner>,
    cap: usize,
    byte_cap: usize,
}

struct Inner {
    map: HashMap<Key, (u64, Vec<u8>)>, // key -> (last_used_tick, bytes)
    tick: u64,
    bytes: usize,
}

impl FrameCache {
    /// New cache holding at most `cap` frames.
    pub fn new(cap: usize, byte_cap: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                tick: 0,
                bytes: 0,
            }),
            cap,
            byte_cap,
        }
    }

    /// Fetch cached bytes for this key, bumping its recency. None on a miss.
    pub fn get(&self, edl_rev: u64, at_ms: u64, height: u32, mode: FrameMode) -> Option<Vec<u8>> {
        let key = Key {
            edl_rev,
            at_ms,
            height,
            mode,
        };
        let mut inner = self.inner.lock().ok()?;
        inner.tick += 1;
        let tick = inner.tick;
        if let Some(slot) = inner.map.get_mut(&key) {
            slot.0 = tick;
            return Some(slot.1.clone());
        }
        None
    }

    /// Insert bytes for this key, evicting the least-recently-used entry when
    /// over the entry or byte capacity. A single oversized JPEG is not cached.
    pub fn put(&self, edl_rev: u64, at_ms: u64, height: u32, mode: FrameMode, bytes: Vec<u8>) {
        let byte_len = bytes.len();
        if self.cap == 0 || byte_len > self.byte_cap {
            return;
        }
        let key = Key {
            edl_rev,
            at_ms,
            height,
            mode,
        };
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        inner.tick += 1;
        let tick = inner.tick;
        if let Some((_, old)) = inner.map.insert(key, (tick, bytes)) {
            inner.bytes = inner.bytes.saturating_sub(old.len());
        }
        inner.bytes = inner.bytes.saturating_add(byte_len);
        // Evict oldest while over either capacity.
        while inner.map.len() > self.cap || inner.bytes > self.byte_cap {
            if let Some(oldest) = inner
                .map
                .iter()
                .min_by_key(|(_, (t, _))| *t)
                .map(|(k, _)| k.clone())
            {
                if let Some((_, old)) = inner.map.remove(&oldest) {
                    inner.bytes = inner.bytes.saturating_sub(old.len());
                }
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A put then get returns the bytes; a different edl_rev (an edit happened)
    /// misses — proving an edit never serves a stale frame.
    #[test]
    fn cache_hit_and_rev_invalidation() {
        let c = FrameCache::new(4, 16);
        c.put(1, 1000, 540, FrameMode::Scrub, vec![1, 2, 3]);
        assert_eq!(c.get(1, 1000, 540, FrameMode::Scrub), Some(vec![1, 2, 3]));
        // Same position, NEW revision (timeline edited) → miss.
        assert_eq!(c.get(2, 1000, 540, FrameMode::Scrub), None);
        // Different height → miss (a 540 frame is not a 720 frame).
        assert_eq!(c.get(1, 1000, 720, FrameMode::Scrub), None);
        // Different mode → miss (scrub vs compose can differ).
        assert_eq!(c.get(1, 1000, 540, FrameMode::Compose), None);
    }

    /// Capacity is enforced: a 2-slot cache holding 3 frames keeps the two most
    /// recently used and evicts the least recent.
    #[test]
    fn lru_eviction_keeps_recent() {
        let c = FrameCache::new(2, 16);
        c.put(1, 0, 540, FrameMode::Scrub, vec![0]);
        c.put(1, 1, 540, FrameMode::Scrub, vec![1]);
        // Touch frame 0 so it is more-recently-used than frame 1.
        assert!(c.get(1, 0, 540, FrameMode::Scrub).is_some());
        // Insert a third → frame 1 (LRU) is evicted, frame 0 survives.
        c.put(1, 2, 540, FrameMode::Scrub, vec![2]);
        assert!(
            c.get(1, 0, 540, FrameMode::Scrub).is_some(),
            "recently-touched survives"
        );
        assert!(
            c.get(1, 2, 540, FrameMode::Scrub).is_some(),
            "newest survives"
        );
        assert!(c.get(1, 1, 540, FrameMode::Scrub).is_none(), "LRU evicted");
    }

    #[test]
    fn byte_budget_evicts_old_entries_and_skips_oversized_jpegs() {
        let c = FrameCache::new(4, 4);
        c.put(1, 0, 540, FrameMode::Scrub, vec![1, 2, 3]);
        c.put(1, 1, 540, FrameMode::Scrub, vec![4, 5]);
        assert!(c.get(1, 0, 540, FrameMode::Scrub).is_none());
        assert_eq!(c.get(1, 1, 540, FrameMode::Scrub), Some(vec![4, 5]));

        c.put(1, 2, 540, FrameMode::Scrub, vec![6, 7, 8, 9, 10]);
        assert!(c.get(1, 2, 540, FrameMode::Scrub).is_none());
    }
}
