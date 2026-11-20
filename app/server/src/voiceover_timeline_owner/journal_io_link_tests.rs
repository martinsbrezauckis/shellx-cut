//! Item 140 sweep: a voiceover take journal reopened for appending must have
//! one name; a second name (hard link) of a file elsewhere is refused.

use super::*;

#[test]
fn a_hard_linked_take_journal_is_refused_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(VOICEOVER_TAKE_JOURNAL_FILE);
    std::fs::write(&path, b"{}\n").unwrap();
    ensure_open_regular(&File::open(&path).unwrap()).expect("one name is accepted");
    std::fs::hard_link(&path, dir.path().join("outside-copy")).unwrap();
    let error = ensure_open_regular(&File::open(&path).unwrap()).unwrap_err();
    assert!(error.cause.contains("one name"), "{error:?}");
}
