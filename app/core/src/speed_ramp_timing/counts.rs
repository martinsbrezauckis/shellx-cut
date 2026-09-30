//! Shared persisted speed-ramp work bounds.

use serde::{Deserialize, Deserializer};

pub(super) fn supported_segments(count: usize) -> bool {
    (crate::types::MIN_RAMP_SEGMENTS..=crate::types::MAX_RAMP_SEGMENTS).contains(&count)
}

fn validate_segments<E: serde::de::Error>(count: usize) -> Result<usize, E> {
    if supported_segments(count) {
        Ok(count)
    } else {
        Err(E::custom(
            "speed ramp segment count must be between 2 and 120",
        ))
    }
}

// Validate at the shared persisted type, including nested sequences and caches.
// Imported counts must be rejected, not clamped into a different duration.
pub(crate) fn deserialize_segments<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<usize, D::Error> {
    validate_segments(usize::deserialize(deserializer)?)
}

pub(crate) fn deserialize_preferred_segments<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<usize>, D::Error> {
    Option::<usize>::deserialize(deserializer)?
        .map(validate_segments)
        .transpose()
}
