//! Private, process-local ownership for native region-picker selections.
//!
//! The registry deliberately has no serde, logging, persistence, or public
//! verb dependency. A later native picker may hand it a validated exact target
//! and crop; a later capture coordinator may consume that value once after
//! revalidating the same opaque native monitor identity and physical crop
//! parent against the current display topology.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const TOKEN_PREFIX: &str = "region_cap_";
const TOKEN_BYTES: usize = 32;
const TOKEN_HEX_LEN: usize = TOKEN_BYTES * 2;
const ISSUE_ATTEMPTS: usize = 4;
const DEFAULT_CAPACITY: usize = 32;
const DEFAULT_TTL: Duration = Duration::from_secs(90);
const RETIRED_TTL: Duration = Duration::from_secs(90);

#[cfg(any(windows, test))]
pub(crate) use super::region_selection_value::NativeTopologyFingerprint;
#[cfg(windows)]
pub(crate) use super::region_selection_value::NativeWindowsTopologySnapshot;
pub(crate) use super::region_selection_value::{ConsumedRegionSelection, RegionSelectionValue};

/// Opaque stable native monitor identity. Display titles and ordinals must not
/// be substituted for this identity when a selection is consumed.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct NativeMonitorIdentity(String);

impl NativeMonitorIdentity {
    pub(crate) fn new(identity: impl Into<String>) -> Option<Self> {
        let identity = identity.into();
        (!identity.trim().is_empty() && identity.len() <= 1024).then_some(Self(identity))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Exact native geometry that remains private to the server/native boundary.
/// It is not serializable and has no display-oriented accessors.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeRegionCrop {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    parent_width: u32,
    parent_height: u32,
}

impl NativeRegionCrop {
    pub(crate) fn new(
        left: u32,
        top: u32,
        width: u32,
        height: u32,
        parent_width: u32,
        parent_height: u32,
    ) -> Option<Self> {
        let right = left.checked_add(width)?;
        let bottom = top.checked_add(height)?;
        (parent_width > 0
            && parent_height > 0
            && left.is_multiple_of(2)
            && top.is_multiple_of(2)
            && width >= 2
            && height >= 2
            && width.is_multiple_of(2)
            && height.is_multiple_of(2)
            && right <= parent_width
            && bottom <= parent_height)
            .then_some(Self {
                left,
                top,
                width,
                height,
                parent_width,
                parent_height,
            })
    }

    pub(crate) fn native_parts(self) -> (u32, u32, u32, u32, u32, u32) {
        (
            self.left,
            self.top,
            self.width,
            self.height,
            self.parent_width,
            self.parent_height,
        )
    }
}

/// Short-lived opaque handle that may be returned by the later human-only
/// picker boundary. It deliberately cannot serialize a region value.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct RegionSelectionTicket(String);

impl RegionSelectionTicket {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegionSelectionIssueError {
    CapacityExhausted,
    EntropyUnavailable,
    TokenCollision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegionSelectionConsumeError {
    MalformedSelectionId,
    ExpiredSelection,
    ConsumedSelection,
    SelectionNotFound,
    MonitorNotFound,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RetiredState {
    Expired,
    Consumed,
}

struct PendingSelection {
    value: RegionSelectionValue,
    expires_at: Instant,
}

struct RetiredSelection {
    state: RetiredState,
    expires_at: Instant,
}

/// Bounded state owner. Embed it behind a mutex when sharing it across server
/// tasks; every mutation, including one-use consumption, is then atomic.
pub(crate) struct RegionSelectionRegistry {
    capacity: NonZeroUsize,
    ttl: Duration,
    entries: HashMap<String, PendingSelection>,
    retired: HashMap<String, RetiredSelection>,
}

impl RegionSelectionRegistry {
    pub(crate) fn new(capacity: NonZeroUsize, ttl: Duration) -> Self {
        Self {
            capacity,
            ttl,
            entries: HashMap::new(),
            retired: HashMap::new(),
        }
    }

    pub(crate) fn issue(
        &mut self,
        value: RegionSelectionValue,
    ) -> Result<RegionSelectionTicket, RegionSelectionIssueError> {
        self.issue_at_with_random(value, Instant::now(), secure_random)
    }

    pub(crate) fn consume(
        &mut self,
        selection_id: &str,
        selection_is_current: impl FnOnce(&RegionSelectionValue) -> bool,
    ) -> Result<ConsumedRegionSelection, RegionSelectionConsumeError> {
        self.consume_at(selection_id, Instant::now(), selection_is_current)
    }

    pub(super) fn issue_at_with_random(
        &mut self,
        value: RegionSelectionValue,
        now: Instant,
        mut fill_random: impl FnMut(&mut [u8; TOKEN_BYTES]) -> Result<(), RegionSelectionIssueError>,
    ) -> Result<RegionSelectionTicket, RegionSelectionIssueError> {
        self.purge_expired(now);
        if self.entries.len() >= self.capacity.get() {
            return Err(RegionSelectionIssueError::CapacityExhausted);
        }
        for _ in 0..ISSUE_ATTEMPTS {
            let mut bytes = [0_u8; TOKEN_BYTES];
            fill_random(&mut bytes)?;
            let selection_id = token_from_bytes(bytes);
            if self.entries.contains_key(&selection_id) || self.retired.contains_key(&selection_id)
            {
                continue;
            }
            self.entries.insert(
                selection_id.clone(),
                PendingSelection {
                    value,
                    expires_at: now + self.ttl,
                },
            );
            return Ok(RegionSelectionTicket(selection_id));
        }
        Err(RegionSelectionIssueError::TokenCollision)
    }

    pub(super) fn consume_at(
        &mut self,
        selection_id: &str,
        now: Instant,
        selection_is_current: impl FnOnce(&RegionSelectionValue) -> bool,
    ) -> Result<ConsumedRegionSelection, RegionSelectionConsumeError> {
        if !is_well_formed_token(selection_id) {
            return Err(RegionSelectionConsumeError::MalformedSelectionId);
        }
        self.purge_expired(now);
        if let Some(pending) = self.entries.remove(selection_id) {
            self.remember_retired(selection_id.to_owned(), RetiredState::Consumed, now);
            if !selection_is_current(&pending.value) {
                return Err(RegionSelectionConsumeError::MonitorNotFound);
            }
            return Ok(ConsumedRegionSelection(pending.value));
        }
        match self.retired.get(selection_id).map(|retired| retired.state) {
            Some(RetiredState::Expired) => Err(RegionSelectionConsumeError::ExpiredSelection),
            Some(RetiredState::Consumed) => Err(RegionSelectionConsumeError::ConsumedSelection),
            None => Err(RegionSelectionConsumeError::SelectionNotFound),
        }
    }

    pub(super) fn active_len(&self) -> usize {
        self.entries.len()
    }

    fn purge_expired(&mut self, now: Instant) {
        let expired = self
            .entries
            .iter()
            .filter_map(|(id, entry)| (entry.expires_at <= now).then_some(id.clone()))
            .collect::<Vec<_>>();
        for selection_id in expired {
            self.entries.remove(&selection_id);
            self.remember_retired(selection_id, RetiredState::Expired, now);
        }
        self.retired.retain(|_, retired| retired.expires_at > now);
    }

    fn remember_retired(&mut self, selection_id: String, state: RetiredState, now: Instant) {
        if self.retired.len() >= self.capacity.get() && !self.retired.contains_key(&selection_id) {
            if let Some(oldest) = self
                .retired
                .iter()
                .min_by_key(|(_, retired)| retired.expires_at)
                .map(|(id, _)| id.clone())
            {
                self.retired.remove(&oldest);
            }
        }
        self.retired.insert(
            selection_id,
            RetiredSelection {
                state,
                expires_at: now + RETIRED_TTL,
            },
        );
    }
}

impl Default for RegionSelectionRegistry {
    fn default() -> Self {
        Self::new(
            NonZeroUsize::new(DEFAULT_CAPACITY).expect("region selection capacity is non-zero"),
            DEFAULT_TTL,
        )
    }
}

fn registry() -> &'static Mutex<RegionSelectionRegistry> {
    static REGION_SELECTIONS: OnceLock<Mutex<RegionSelectionRegistry>> = OnceLock::new();
    REGION_SELECTIONS.get_or_init(|| Mutex::new(RegionSelectionRegistry::default()))
}

pub(super) fn issue(
    value: RegionSelectionValue,
) -> Result<RegionSelectionTicket, RegionSelectionIssueError> {
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .issue(value)
}

pub(super) fn consume(
    selection_id: &str,
    selection_is_current: impl FnOnce(&RegionSelectionValue) -> bool,
) -> Result<ConsumedRegionSelection, RegionSelectionConsumeError> {
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .consume(selection_id, selection_is_current)
}

fn secure_random(bytes: &mut [u8; TOKEN_BYTES]) -> Result<(), RegionSelectionIssueError> {
    getrandom::fill(bytes).map_err(|_| RegionSelectionIssueError::EntropyUnavailable)
}

fn token_from_bytes(bytes: [u8; TOKEN_BYTES]) -> String {
    format!("{TOKEN_PREFIX}{}", hex::encode(bytes))
}

fn is_well_formed_token(selection_id: &str) -> bool {
    selection_id
        .strip_prefix(TOKEN_PREFIX)
        .filter(|encoded| encoded.len() == TOKEN_HEX_LEN)
        .is_some_and(|encoded| encoded.bytes().all(|byte| byte.is_ascii_hexdigit()))
}
