//! Bounded concat list for verified, constant-rate recording parts.

use super::{cfr, plan::SegmentPlan};
use crate::ManifestError;

/// Seven days at the recorder's normal 15-second checkpoint interval is
/// 40,320 main entries. A wider hold unit keeps the typical second entry per
/// checkpoint below this budget, even for a mostly still take.
pub(super) const MAX_CONCAT_ENTRIES: u64 = 100_000;
const MAX_CONCAT_LIST_BYTES: usize = 32 * 1024 * 1024;
const ROW_SUFFIX: &str = "\noption format_whitelist mov\n";

pub(super) fn rows(
    parts: &[(SegmentPlan, cfr::Made)],
    max_entries: u64,
) -> Result<(String, u64, u64), ManifestError> {
    let mut rows = String::new();
    let (mut entries, mut duration_ms, mut frames) = (0u64, 0u64, 0u64);
    for (_, part) in parts {
        duration_ms = duration_ms.saturating_add(part.facts.duration_ms);
        frames = frames.saturating_add(part.facts.decoded_video_frames);
        let repeats = part
            .unit
            .iter()
            .flat_map(|unit| std::iter::repeat_n(unit, part.units as usize));
        for path in std::iter::once(&part.main).chain(repeats) {
            // Quoted; a line break in the project folder name is refused.
            let line = cut_media::ffmpeg::concat_demuxer_file_line(path).map_err(|e| {
                let action = e.suggested_action.unwrap_or_default();
                ManifestError::Invalid(format!("{}: {}. {action}", e.message, e.cause))
            })?;
            let entry_bytes = line.len() + ROW_SUFFIX.len();
            if entries >= max_entries
                || entry_bytes > MAX_CONCAT_LIST_BYTES.saturating_sub(rows.len())
            {
                return Err(ManifestError::StitchLimit(
                    "the join list is too large to process safely; keep the original capture folder and contact support for recovery",
                ));
            }
            rows.push_str(&line);
            rows.push_str(ROW_SUFFIX);
            entries += 1;
        }
    }
    Ok((rows, duration_ms, frames))
}
