use super::*;

#[test]
fn an_unsigned_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("vc_redist.exe");
    std::fs::write(&file, b"MZ not a signed program").unwrap();
    let reason = verify_microsoft_signature(&file).unwrap_err();
    assert!(
        reason.contains("Windows did not accept the file's signature"),
        "{reason}"
    );
}

#[test]
fn a_program_signed_by_someone_else_is_refused() {
    // Node.js is Authenticode-signed by its own publisher, not by Microsoft.
    let node = std::path::Path::new(r"C:\Program Files\nodejs\node.exe");
    if !node.is_file() {
        eprintln!("SKIP: no {} on this machine", node.display());
        return;
    }
    let reason = verify_microsoft_signature(node).unwrap_err();
    assert!(
        reason.contains("Microsoft root") || reason.contains("not Microsoft Corporation"),
        "{reason}"
    );
}

#[test]
fn microsofts_redistributable_passes() {
    // Microsoft's installer downloaded by hand from aka.ms/vc14 (never run here).
    let Some(path) = std::env::var_os("SHELLX_CUT_TEST_VC_REDIST") else {
        eprintln!("SKIP: SHELLX_CUT_TEST_VC_REDIST is not set");
        return;
    };
    assert_eq!(
        verify_microsoft_signature(Path::new(&path)).unwrap(),
        "Microsoft Corporation"
    );
}

#[test]
fn this_pc_state_is_read_without_error() {
    let state = status();
    assert!(state.applicable);
    assert_eq!(state.installed, state.missing_files.is_empty());
    // Either answer is a real one; the call itself must not need elevation.
    let _ = can_approve_admin_prompt();
}
