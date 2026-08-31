//! Canonical bounded JSONL replay parsing for the durable scene journal.

use record_core::reduce_scene_event;

use crate::scene_journal::{
    SceneJournalEntry, SceneJournalReplay, SceneJournalResult, MAX_SCENE_JOURNAL_EVENTS,
    SCENE_JOURNAL_SCHEMA,
};
use crate::scene_journal_io::{canonical_bytes, invalid};

pub(super) fn parse_entries(bytes: &[u8]) -> SceneJournalResult<SceneJournalReplay> {
    let mut replay = None;
    for (index, line) in bytes[..bytes.len().saturating_sub(1)]
        .split(|byte| *byte == b'\n')
        .enumerate()
    {
        let line_no = index + 1;
        if line.is_empty() {
            return Err(invalid(format!("scene journal line {line_no} is empty")));
        }
        let entry: SceneJournalEntry = serde_json::from_slice(line).map_err(|source| {
            invalid(format!(
                "scene journal line {line_no} is malformed: {source}"
            ))
        })?;
        if canonical_bytes(&entry)? != line {
            return Err(invalid(format!(
                "scene journal line {line_no} is not canonical"
            )));
        }
        match entry {
            SceneJournalEntry::Header { schema, snapshot }
                if replay.is_none() && schema == SCENE_JOURNAL_SCHEMA =>
            {
                replay = Some(SceneJournalReplay::initial(snapshot)?);
            }
            SceneJournalEntry::Header { .. } => {
                return Err(invalid("scene journal header is duplicate or unsupported"));
            }
            SceneJournalEntry::Event { event } => {
                let Some(replay) = replay.as_mut() else {
                    return Err(invalid("scene journal event precedes its snapshot header"));
                };
                if replay.events.len() >= MAX_SCENE_JOURNAL_EVENTS {
                    return Err(invalid("scene journal exceeds its event limit"));
                }
                replay.state = reduce_scene_event(&replay.snapshot, &replay.state, &event)?;
                replay.events.push(event);
            }
        }
    }
    replay.ok_or_else(|| invalid("scene journal is missing its snapshot header"))
}

/// A newline is the commit delimiter. Repair is allowed only for an
/// unfinished EOF JSON value, never a complete malformed/noncanonical line.
pub(super) fn torn_tail_is_unambiguously_incomplete(tail: &[u8]) -> bool {
    matches!(
        serde_json::from_slice::<SceneJournalEntry>(tail),
        Err(error) if error.is_eof()
    )
}
