//! Unit tests of title_remake.rs: the words a person reads (the path rules are
//! tested in title_movie_path_tests.rs).
use super::*;

#[test]
fn a_person_reads_the_title_text_or_the_shape() {
    assert_eq!(
        overlay_name(&json!({"text": "TITLE 147"})),
        "the title 'TITLE 147'"
    );
    assert_eq!(overlay_name(&json!({"shape": "arrow"})), "the arrow shape");
    assert_eq!(overlay_name(&json!({})), "a title");
    let error = not_drawn("the title 'TITLE 147'", "x", "y");
    assert!(error
        .message
        .starts_with("the title 'TITLE 147' could not be drawn: its rendered file is missing"));
    assert!(!error.message.contains("asset"), "never an internal id");
}
