use cut_core::{store::apply_edit_verb, Clip, Project, ProjectSettings};
use serde_json::{json, Value};

fn project() -> Project {
    let mut project = Project::new("redaction limits", ProjectSettings::default());
    project.tracks[0]
        .clips
        .push(Clip::Media(cut_core::edit::make_media_clip(
            "clip", "asset", 0, 3000,
        )));
    project
}

fn track(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| json!({"t_ms":i*120,"cx":0.5,"cy":0.5}))
        .collect()
}

fn redact(boxes: Vec<Value>, primary_track: Option<Vec<Value>>) -> Value {
    json!({"clip":"clip","shape":"rect","points":[[0.1,0.1],[0.2,0.2]],
        "track":primary_track,"boxes":boxes})
}

#[test]
fn new_redaction_over_256_regions_is_refused() {
    let boxes = (0..256)
        .map(|_| json!({"shape":"rect","points":[[0.1,0.1],[0.2,0.2]]}))
        .collect();
    let error = apply_edit_verb(&mut project(), "edit.redact", &redact(boxes, None))
        .expect_err("257 regions must be refused");
    assert!(error.to_string().contains("257"));
    assert!(error.to_string().contains("256"));
    assert!(error.suggested_action.unwrap().contains("remove"));
}

#[test]
fn new_redaction_over_200000_total_motion_points_is_refused() {
    let error = apply_edit_verb(
        &mut project(),
        "edit.redact",
        &redact(vec![], Some(track(200_001))),
    )
    .expect_err("200001 total points must be refused");
    assert!(error.to_string().contains("200001"));
    assert!(error.to_string().contains("200000"));
    assert!(error.suggested_action.unwrap().contains("reduce"));
}

#[test]
fn eight_regions_with_300_motion_points_each_are_accepted() {
    let boxes = (0..7)
        .map(|_| json!({"shape":"rect","points":[[0.1,0.1],[0.2,0.2]],"track":track(300)}))
        .collect();
    apply_edit_verb(
        &mut project(),
        "edit.redact",
        &redact(boxes, Some(track(300))),
    )
    .expect("ordinary tracked face redaction must remain accepted");
}
