//! Test support for item 155: a perception report a test writes stands in for
//! one this engine's analysis wrote, so it is recorded as this engine's
//! (`cut_media::cache_trust`); a report written with plain `std::fs::write`
//! is treated like one that came with a received project.

use std::path::Path;

pub(super) fn write_engine_report(
    path: impl AsRef<Path>,
    contents: impl AsRef<[u8]>,
) -> std::io::Result<()> {
    std::fs::write(path.as_ref(), contents)?;
    cut_media::cache_trust::remember(path.as_ref());
    Ok(())
}
