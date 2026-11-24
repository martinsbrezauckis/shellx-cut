use super::*;

#[test]
fn configured_but_damaged_captions_are_not_a_missing_optional_install() {
    let card = missing_perception_card(
        json!("/configured/python"),
        Some(true),
        "The configured captions interpreter could not run: loader error.",
        "selected-model".into(),
        false,
        None,
    );
    assert_eq!(card.status, CardStatus::Degraded);
    assert!(card.hint.unwrap().contains("loader error"));
}

#[test]
fn no_captions_installation_remains_an_optional_missing_card() {
    let card = missing_perception_card(
        Value::Null,
        Some(false),
        "Captions tools are not installed.",
        "selected-model".into(),
        false,
        None,
    );
    assert_eq!(card.status, CardStatus::Missing);
}
