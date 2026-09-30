//! A bounded diagnostic read, never a general imported-file read.
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAX_TAIL_BYTES: u64 = 64 * 1024;

pub(super) fn diagnostic(dir: &Path) -> String {
    match tail(dir) {
        Some(tail) if tail.is_empty() => "record.log is empty — the capture likely blocked before any output (most often an unanswered ScreenCast consent dialog)".into(),
        Some(tail) => format!("record.log tail: …{tail}"),
        None => "record.log diagnostic unavailable".into(),
    }
}

fn tail(dir: &Path) -> Option<String> {
    cut_core::matte_cache::require_plain_directory(dir).ok()?;
    let path = dir.join("record.log");
    if !cut_core::matte_cache::plain_file_exists(&path).ok()? {
        return None;
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(&path).ok()?;
    let len = file.metadata().ok()?.len();
    let offset = len.saturating_sub(MAX_TAIL_BYTES);
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_TAIL_BYTES).read_to_end(&mut bytes).ok()?;
    // A bounded seek may start inside a UTF-8 code point. Skip only that partial
    // prefix; other malformed UTF-8 still makes diagnostics unavailable.
    let skip = if offset > 0 {
        bytes
            .iter()
            .take(3)
            .take_while(|b| **b & 0xc0 == 0x80)
            .count()
    } else {
        0
    };
    let text = std::str::from_utf8(&bytes[skip..]).ok()?.trim();
    if offset > 0 && text.chars().count() < 600 {
        return None;
    }
    Some(
        text.chars()
            .rev()
            .take(600)
            .collect::<String>()
            .chars()
            .rev()
            .collect(),
    )
}

#[cfg(test)]
#[path = "stop_log_tests.rs"]
mod tests;
