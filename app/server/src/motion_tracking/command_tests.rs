use super::command::{tracking_command, validate_mutation};
use serde_json::Value;
use std::fs;
use std::path::Path;

#[test]
fn command_uses_fixed_argv_without_shell_interpolation() {
    let spec = tracking_command(
        "tracking-inspect",
        "read_motion",
        Path::new("/tmp/pkg; touch pwned"),
        Path::new("/tmp/workspace.cutproj"),
        vec!["--analysis-id".into(), "track_1".into()],
    );
    assert!(spec.args.iter().any(|arg| arg == "/tmp/pkg; touch pwned"));
    assert!(spec.args.iter().any(|arg| arg == "--trusted-local-tier"));
    assert!(spec
        .args
        .windows(2)
        .any(|args| { args[0] == "--caller-id" && args[1].starts_with("cut:") }));
    assert!(!spec.args.iter().any(|arg| arg == "sh" || arg == "-c"));
}

#[test]
fn mutation_validation_binds_output_identity_and_receipt() {
    let root = tempfile::tempdir().expect("temp root");
    let output = root.path().join("output");
    fs::create_dir(&output).expect("output");
    fs::write(
        output.join("manifest.json"),
        r#"{"id":"pkg_tracking","motion":"motion.json"}"#,
    )
    .expect("manifest");
    fs::write(output.join("motion.json"), r#"{"id":"motion_tracking"}"#).expect("motion");
    let envelope = serde_json::json!({
        "ok": true,
        "result": {
            "packageRoot": output,
            "packageId": "pkg_tracking",
            "receipt": {
                "id": "receipt_tracking_apply",
                "operation": "analysis.tracking.apply",
                "status": "passed",
                "warnings": []
            }
        }
    });
    let valid = validate_mutation(
        envelope.clone(),
        &output,
        ("pkg_tracking".into(), "motion_tracking".into()),
        "analysis.tracking.apply",
    )
    .expect("validated mutation");
    assert_eq!(valid.receipt_id, "receipt_tracking_apply");

    let mut wrong = envelope;
    wrong["result"]["packageId"] = Value::String("pkg_other".into());
    assert!(validate_mutation(
        wrong,
        &output,
        ("pkg_tracking".into(), "motion_tracking".into()),
        "analysis.tracking.apply",
    )
    .is_err());
}
