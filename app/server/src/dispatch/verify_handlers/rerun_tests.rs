use super::*;

fn receipt(checks: Vec<cut_core::CheckResult>) -> cut_core::RenderReceipt {
    cut_core::RenderReceipt {
        render_id: "render_001".into(),
        ts: "now".into(),
        output_path: "exports/render_001.mp4".into(),
        output_hash: "sha256:abc".into(),
        duration_ms: 1,
        preset: "test".into(),
        at_op: "op_000001".into(),
        checks,
        pass: true,
        judge: None,
        fix_actions: Vec::new(),
    }
}

#[test]
fn full_hash_is_complete_and_receipt_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("render.mp4");
    std::fs::write(&path, [b'a'; 128 * 1024]).unwrap();
    let expected = full_sha256(&path).unwrap();
    assert_receipt_hash(&path, &expected).unwrap();
    std::fs::write(&path, [b'b'; 128 * 1024]).unwrap();
    assert_eq!(
        assert_receipt_hash(&path, &expected).unwrap_err().code,
        error_codes::CONFLICT
    );
}

#[test]
fn legacy_profile_defaults_but_malformed_profile_fails_closed() {
    assert_eq!(
        profile_from_receipt(&receipt(Vec::new())).unwrap(),
        cut_perception::FootageProfile::TalkingHead
    );
    let malformed = receipt(vec![cut_core::CheckResult {
        name: cut_core::check_names::FOOTAGE_PROFILE.into(),
        pass: true,
        details: json!({"active_profile": "future_profile"}),
        evidence: json!({}),
    }]);
    assert_eq!(
        profile_from_receipt(&malformed).unwrap_err().code,
        error_codes::CONFLICT
    );
}

#[test]
fn verification_receipt_is_separate_and_atomically_published() {
    let dir = tempfile::tempdir().unwrap();
    let render = dir.path().join("render_001.json");
    std::fs::write(&render, "original render receipt").unwrap();
    let name = verification_receipt_name("job_001");
    let result = json!({
        "render_id": "render_001",
        "source_receipt_id": "render_001",
        "verification_receipt": format!("receipts/{name}"),
        "output_hash": "sha256:abc",
        "checked_at": "now",
        "scope": "rendered_output",
        "profile": "talking_head",
        "checks": [],
        "pass": true,
    });

    write_verification_receipt(dir.path(), &name, &result).unwrap();
    assert_eq!(
        std::fs::read_to_string(render).unwrap(),
        "original render receipt"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(dir.path().join(name)).unwrap()).unwrap()
            ["source_receipt_id"],
        "render_001"
    );
}

#[test]
fn source_render_receipt_names_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let error = write_verification_receipt(dir.path(), "render_001.json", &json!({})).unwrap_err();
    assert_eq!(error.code, error_codes::IO);
}

#[test]
fn selected_receipt_embedded_id_must_match_requested_identity() {
    let dir = tempfile::tempdir().unwrap();
    let receipts = dir.path().join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    let mut forged = receipt(Vec::new());
    forged.render_id = "render_other".into();
    std::fs::write(
        receipts.join("render_001.json"),
        serde_json::to_vec(&forged).unwrap(),
    )
    .unwrap();

    let error = selected_render_receipt(&receipts, "render_001").unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(error.message.contains("id does not match"));
}

#[cfg(unix)]
#[test]
fn selected_receipt_refuses_a_linked_leaf() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let receipts = dir.path().join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    let outside = dir.path().join("outside.json");
    std::fs::write(&outside, serde_json::to_vec(&receipt(Vec::new())).unwrap()).unwrap();
    symlink(&outside, receipts.join("render_001.json")).unwrap();

    let error = selected_render_receipt(&receipts, "render_001").unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(error.message.contains("local regular file"));
}

#[cfg(unix)]
#[test]
fn selected_receipt_refuses_a_linked_receipts_directory() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside-receipts");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        outside.join("render_001.json"),
        serde_json::to_vec(&receipt(Vec::new())).unwrap(),
    )
    .unwrap();
    let receipts = dir.path().join("receipts");
    symlink(&outside, &receipts).unwrap();

    let error = selected_render_receipt(&receipts, "render_001").unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(error.message.contains("not a local directory"));
}

#[cfg(unix)]
#[test]
fn receipt_bound_output_refuses_a_linked_leaf() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project.cutproj");
    let exports = project.join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    let output = exports.join("render.mp4");
    let linked = exports.join("linked.mp4");
    std::fs::write(&output, b"render bytes").unwrap();
    symlink(&output, &linked).unwrap();

    let error = fenced_output_for_receipt(&project, linked.to_str().unwrap(), None).unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(error.message.contains("not a local regular file"));
}
