//! Reading render receipts from a project.
//!
//! A receipt is a small JSON file the engine writes into `receipts/`. A
//! project may come from someone else, so a file at a receipt name can be a
//! link, a file shared with another path, a multi-gigabyte file, or one of
//! millions of names in the folder. Every consumer of a render receipt reads
//! it through [`render_receipt_bytes`]: an ordinary single-link file directly
//! inside the project's own receipts folder, opened without following a link
//! and never read past [`MAX_RENDER_RECEIPT_BYTES`]. Selecting the latest
//! receipt walks the folder once, keeps only the best name, and stops at
//! [`MAX_RECEIPT_FOLDER_ENTRIES`] entries.

use cut_core::store::{read_bounded_cache, BoundedCacheRead};
use cut_core::{error_codes, CutError};
use std::path::Path;

/// Largest render receipt the engine reads or writes. Receipts the engine
/// writes are a few kilobytes; earlier receipts use the same limit.
pub(crate) const MAX_RENDER_RECEIPT_BYTES: u64 = 16 * 1024 * 1024;
const LIMIT_TEXT: &str = "16 MiB";
/// Most entries the receipts folder may hold when the latest receipt is
/// chosen by name. A project with this many renders does not exist in
/// practice; a folder past it is refused instead of being walked further.
pub(crate) const MAX_RECEIPT_FOLDER_ENTRIES: usize = 100_000;
/// Most text read from earlier receipts for one review's edit intent.
pub(crate) const MAX_EARLIER_RECEIPTS_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

fn shown(receipts: &Path, path: &Path) -> String {
    let base = receipts.parent().unwrap_or(receipts);
    path.strip_prefix(base)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

fn delete_or_render_again(name: &str) -> String {
    format!(
        "delete {name} from the project folder or render again to create a new receipt, then run this again"
    )
}

/// The bytes of the render receipt at `path`, which must name a file directly
/// inside `receipts` (the caller has admitted `receipts` as the project's
/// plain receipts folder). Refused without reading when it is larger than
/// [`MAX_RENDER_RECEIPT_BYTES`], a link, or not an ordinary single-link file.
pub(crate) fn render_receipt_bytes(receipts: &Path, path: &Path) -> Result<Vec<u8>, CutError> {
    let name = shown(receipts, path);
    if path.parent() != Some(receipts) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("{name} is not in the project's receipts folder, so it was not read"),
            "a render receipt is read only from the open project's receipts folder",
        )
        .with_suggested_action("use a render id from the current project's receipts folder"));
    }
    match read_bounded_cache(path, MAX_RENDER_RECEIPT_BYTES) {
        BoundedCacheRead::Bytes(bytes) => Ok(bytes),
        BoundedCacheRead::Oversized => Err(CutError::new(
            error_codes::CONFLICT,
            format!("render receipt {name} is larger than {LIMIT_TEXT}, so it was not read"),
            format!(
                "render receipts are small files the engine writes; the limit is {LIMIT_TEXT} ({MAX_RENDER_RECEIPT_BYTES} bytes). A file this large did not come from a render."
            ),
        )
        .with_suggested_action(delete_or_render_again(&name))),
        BoundedCacheRead::Unavailable => Err(CutError::new(
            error_codes::CONFLICT,
            format!(
                "render receipt {name} is a link, a file shared with another path, or not an ordinary file, so it was not read"
            ),
            "a render receipt is used only when it is an ordinary file of this project",
        )
        .with_suggested_action(delete_or_render_again(&name))),
    }
}

/// Refuse to write a receipt the engine could not read back.
pub(crate) fn ensure_receipt_fits(
    receipts: &Path,
    path: &Path,
    len: usize,
) -> Result<(), CutError> {
    if len as u64 <= MAX_RENDER_RECEIPT_BYTES {
        return Ok(());
    }
    let name = shown(receipts, path);
    Err(CutError::new(
        error_codes::CONFLICT,
        format!("the review is too large to keep on render receipt {name}"),
        format!(
            "the receipt with the review attached would hold {len} bytes, more than the {LIMIT_TEXT} receipt limit"
        ),
    )
    .with_suggested_action("run the review again; if it repeats, review a shorter render"))
}

/// Text of an earlier receipt, or `None` when it is larger than the limit,
/// not valid text, or not an ordinary single-link file.
pub(crate) fn earlier_receipt_text(path: &Path) -> Option<String> {
    match read_bounded_cache(path, MAX_RENDER_RECEIPT_BYTES) {
        BoundedCacheRead::Bytes(bytes) => String::from_utf8(bytes).ok(),
        BoundedCacheRead::Oversized | BoundedCacheRead::Unavailable => None,
    }
}

/// Whether a folder entry name is a render receipt (`render_<id>.json`, not
/// the `render_<id>.output.*` files written beside it).
pub(crate) fn is_render_receipt_name(name: &str) -> bool {
    name.starts_with("render_") && name.ends_with(".json") && !name.contains(".output")
}

/// Order of receipt names: render ids are `render_NNN` with at least three
/// digits, so `render_1000` follows `render_999`; other names sort after
/// numbered ones by text.
fn receipt_order(name: &str) -> (bool, usize, &str, &str) {
    let stem = name
        .strip_prefix("render_")
        .and_then(|rest| rest.strip_suffix(".json"))
        .unwrap_or(name);
    let numbered = !stem.is_empty() && stem.bytes().all(|byte| byte.is_ascii_digit());
    let number = if numbered {
        stem.trim_start_matches('0')
    } else {
        stem
    };
    (!numbered, number.len(), number, name)
}

/// The latest render receipt's file name in `receipts`, or `None` when the
/// folder holds none. Walks the folder once without collecting names.
pub(crate) fn latest_render_receipt_name(receipts: &Path) -> Result<Option<String>, CutError> {
    latest_render_receipt_name_within(receipts, MAX_RECEIPT_FOLDER_ENTRIES)
}

pub(crate) fn latest_render_receipt_name_within(
    receipts: &Path,
    max_entries: usize,
) -> Result<Option<String>, CutError> {
    let mut best: Option<String> = None;
    for (seen, entry) in std::fs::read_dir(receipts)?.enumerate() {
        if seen >= max_entries {
            return Err(CutError::new(
                error_codes::CONFLICT,
                format!(
                    "the project's receipts folder holds more than {max_entries} entries, so the latest render receipt was not looked up"
                ),
                "a project's receipts folder holds a few files per render; this one was not made by renders",
            )
            .with_suggested_action(
                "pass render_id to name the receipt, or remove the extra files from the receipts folder",
            ));
        }
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_render_receipt_name(name) {
            continue;
        }
        if best
            .as_deref()
            .is_none_or(|current| receipt_order(name) > receipt_order(current))
        {
            best = Some(name.to_owned());
        }
    }
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_receipts_order_by_number_then_other_names() {
        let mut names = vec![
            "render_999.json",
            "render_1000.json",
            "render_001.json",
            "render_x.json",
        ];
        names.sort_by_key(|name| receipt_order(name));
        assert_eq!(
            names,
            [
                "render_001.json",
                "render_999.json",
                "render_1000.json",
                "render_x.json"
            ]
        );
    }

    #[test]
    fn latest_name_skips_output_files_and_picks_the_highest_number() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "render_999.json",
            "render_1000.json",
            "render_1000.output.perception.json",
            "notes.txt",
        ] {
            std::fs::write(dir.path().join(name), b"{}").unwrap();
        }
        assert_eq!(
            latest_render_receipt_name(dir.path()).unwrap().as_deref(),
            Some("render_1000.json")
        );
    }

    #[test]
    fn a_crowded_receipts_folder_is_refused_without_collecting_names() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..12 {
            std::fs::write(dir.path().join(format!("render_{index:03}.json")), b"{}").unwrap();
        }
        let error = latest_render_receipt_name_within(dir.path(), 10).unwrap_err();
        assert!(error.message.contains("more than 10 entries"), "{error:?}");
        assert!(
            error
                .suggested_action
                .as_deref()
                .is_some_and(|action| action.contains("render_id")),
            "{error:?}"
        );
        assert_eq!(
            latest_render_receipt_name_within(dir.path(), 12)
                .unwrap()
                .as_deref(),
            Some("render_011.json")
        );
    }

    #[test]
    fn an_oversized_receipt_is_refused_by_size_without_reading() {
        let dir = tempfile::tempdir().unwrap();
        let receipts = dir.path().join("receipts");
        std::fs::create_dir(&receipts).unwrap();
        let path = receipts.join("render_001.json");
        // On Unix a sparse 64 GiB file: no disk space is used and reading it
        // would take minutes. NTFS would reserve the space, so Windows uses
        // one byte over the limit.
        let len = if cfg!(unix) {
            64 * 1024 * 1024 * 1024
        } else {
            MAX_RENDER_RECEIPT_BYTES + 1
        };
        std::fs::File::create(&path).unwrap().set_len(len).unwrap();
        let started = std::time::Instant::now();
        let error = render_receipt_bytes(&receipts, &path).unwrap_err();
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert!(
            error.message.contains("receipts/render_001.json") && error.message.contains("16 MiB"),
            "{error:?}"
        );
        assert!(error.suggested_action.is_some());
    }

    #[test]
    fn a_receipt_outside_the_receipts_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let receipts = dir.path().join("receipts");
        std::fs::create_dir(&receipts).unwrap();
        let elsewhere = dir.path().join("render_001.json");
        std::fs::write(&elsewhere, b"{}").unwrap();
        assert!(render_receipt_bytes(&receipts, &elsewhere).is_err());
        assert!(ensure_receipt_fits(&receipts, &elsewhere, 10).is_ok());
        assert!(ensure_receipt_fits(&receipts, &elsewhere, 17 * 1024 * 1024).is_err());
    }
}
