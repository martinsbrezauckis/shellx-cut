//! Volatile, bounded readable-picture verdicts for `media.check`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// 100 fixture decodes took 4.963 s in the release review: 64 ordinary
/// pictures take about 3.2 s. Twelve seconds preserves the existing 10 s
/// single-picture allowance and caps a pass with slow inputs.
pub(crate) const MAX_PICTURES_PER_PASS: usize = 64;
pub(crate) const PASS_BUDGET: Duration = Duration::from_secs(12);
/// Enough for several large projects without unbounded process-lifetime growth.
pub(crate) const CACHE_LIMIT: usize = 4096;

#[derive(Clone, Eq, Hash, PartialEq)]
pub(crate) struct SourceKey(PathBuf, u64, Option<SystemTime>);

impl SourceKey {
    /// Same source identity as the measured reverse-source facts cache.
    pub(crate) fn capture(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        meta.is_file()
            .then(|| Self(path.to_path_buf(), meta.len(), meta.modified().ok()))
    }
}

#[derive(Default)]
struct Record {
    verdicts: HashMap<SourceKey, bool>,
    order: VecDeque<SourceKey>,
}

#[derive(Default)]
pub(crate) struct PictureChecks {
    pass: std::sync::Arc<tokio::sync::Mutex<()>>,
    record: Mutex<Record>,
}

/// Retained by the blocking worker when its awaiting request is cancelled.
pub(crate) type PassToken = std::sync::Arc<tokio::sync::OwnedMutexGuard<()>>;

impl PictureChecks {
    pub(crate) fn try_begin(&self) -> Option<PassToken> {
        self.pass
            .clone()
            .try_lock_owned()
            .ok()
            .map(std::sync::Arc::new)
    }

    pub(crate) fn get(&self, key: &SourceKey) -> Option<bool> {
        self.record
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .verdicts
            .get(key)
            .copied()
    }

    pub(crate) fn insert(&self, key: SourceKey, readable: bool) {
        let mut record = self
            .record
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if record.verdicts.contains_key(&key) {
            return;
        }
        if record.order.len() == CACHE_LIMIT {
            if let Some(oldest) = record.order.pop_front() {
                record.verdicts.remove(&oldest);
            }
        }
        record.order.push_back(key.clone());
        record.verdicts.insert(key, readable);
    }
}
