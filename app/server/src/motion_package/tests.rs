use super::*;
use std::fs;

fn write_package(root: &Path, motion: &str) {
    fs::write(
        root.join("manifest.json"),
        r#"{"id":"pkg_demo","motion":"motion.json"}"#,
    )
    .expect("manifest");
    fs::write(root.join("motion.json"), motion).expect("motion document");
}

#[test]
fn reads_identity_document_duration_and_stable_revision() {
    let root = tempfile::tempdir().expect("package root");
    write_package(
        root.path(),
        r#"{"schema":"shellx-motion/motion@1","id":"motion_demo","durationMs":2400,"layers":[]}"#,
    );
    assert_eq!(
        identity(root.path()).expect("identity"),
        ("pkg_demo".into(), "motion_demo".into())
    );
    assert_eq!(duration_ms(root.path()), Some(2400));
    assert_eq!(
        document(root.path()).unwrap()["layers"],
        serde_json::json!([])
    );
    let first = revision(root.path()).expect("revision");
    assert_eq!(first.len(), 64);
    assert_eq!(revision(root.path()).unwrap(), first);
    fs::write(
        root.path().join("motion.json"),
        r#"{"id":"motion_demo","durationMs":2500,"layers":[]}"#,
    )
    .unwrap();
    assert_ne!(revision(root.path()).unwrap(), first);
}

#[test]
fn refuses_package_relative_escape_and_missing_identity() {
    let package = tempfile::tempdir().expect("package root");
    let outside = package.path().parent().unwrap().join("outside-motion.json");
    fs::write(&outside, r#"{"id":"escaped"}"#).unwrap();
    fs::write(
        package.path().join("manifest.json"),
        r#"{"id":"pkg","motion":"../outside-motion.json"}"#,
    )
    .unwrap();
    let error = document(package.path()).expect_err("escape rejected");
    assert_eq!(error.code, error_codes::GUARDRAIL);

    write_package(package.path(), r#"{"layers":[]}"#);
    let error = identity(package.path()).expect_err("identity required");
    assert_eq!(error.code, error_codes::INVALID_ARGS);
}

#[cfg(unix)]
#[test]
fn refuses_symlinked_control_plane_file() {
    use std::os::unix::fs::symlink;

    let package = tempfile::tempdir().expect("package root");
    let backing = package.path().join("manifest-backing.json");
    fs::write(&backing, r#"{"id":"pkg","motion":"motion.json"}"#).unwrap();
    symlink(&backing, package.path().join("manifest.json")).unwrap();
    fs::write(package.path().join("motion.json"), r#"{"id":"motion"}"#).unwrap();
    let error = document(package.path()).expect_err("symlink rejected");
    assert_eq!(error.code, error_codes::INVALID_ARGS);
}
