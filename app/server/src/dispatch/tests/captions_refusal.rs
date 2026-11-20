//! Every verb that runs on the captions Python (the perception sidecar) gives
//! the one shared refusal when that Python is not installed: the message
//! "Captions and transcription are not installed" with the Install captions /
//! `system.setup_perception` action. The runner-specific "not installed"
//! errors of the same verbs only fire when a Python exists but a script does
//! not, which is a damaged install, not a missing runtime.

use super::lock_agent_cli_env;
use cut_core::CutError;
use std::ffi::OsString;

pub(super) struct SidecarEnv(Vec<(&'static str, Option<OsString>)>);

impl SidecarEnv {
    pub(super) fn without_python(dir: &std::path::Path) -> Self {
        let keys = [
            "SHELLX_CUT_PYTHON",
            "SHELLX_CUT_SIDECAR_DIR",
            cut_native_runtime_context::CONTEXT_ENV,
        ];
        let saved = keys.iter().map(|k| (*k, std::env::var_os(k))).collect();
        std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV);
        std::env::set_var("SHELLX_CUT_PYTHON", dir.join("no-python-here"));
        std::env::set_var("SHELLX_CUT_SIDECAR_DIR", dir);
        Self(saved)
    }
}

#[cfg(unix)]
impl SidecarEnv {
    /// Captions count as installed: the sidecar Python is a file that exists.
    pub(super) fn with_python(dir: &std::path::Path, python: &std::path::Path) -> Self {
        let env = Self::without_python(dir);
        std::env::set_var("SHELLX_CUT_PYTHON", python);
        env
    }
}

impl Drop for SidecarEnv {
    fn drop(&mut self) {
        for (key, value) in self.0.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn assert_shared_refusal(verb: &str, error: &CutError) {
    assert!(
        cut_perception::is_captions_not_installed(error),
        "{verb} must give the shared captions refusal, got {error:?}"
    );
    let action = error.suggested_action.as_deref().unwrap_or_default();
    assert!(action.contains("Install captions"), "{verb}: {action}");
    assert!(
        action.contains("system.setup_perception"),
        "{verb}: {action}"
    );
}

#[test]
fn track_faces_ocr_and_index_give_the_shared_refusal_without_captions() {
    let _guard = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("instruments.py"), "").unwrap();
    let _env = SidecarEnv::without_python(dir.path());

    assert_shared_refusal(
        "edit.track",
        &crate::track::runtime().expect_err("no Python, no tracker"),
    );
    assert_shared_refusal(
        "edit.redact faces",
        &crate::faces::runtime().expect_err("no Python, no face detector"),
    );
    assert_shared_refusal(
        "edit.redact ocr_auto",
        &crate::ocr::runtime().expect_err("no Python, no OCR"),
    );
    assert_shared_refusal(
        "media.index",
        &crate::vissearch::runtime().expect_err("no Python, no indexer"),
    );
}

/// With no captions Python the review (judge) cards must name that, not a
/// "missing bundled adapter" (the adapters ship inside the app).
#[test]
fn judge_cards_name_missing_captions_not_a_missing_adapter() {
    let _guard = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("instruments.py"), "").unwrap();
    let _env = SidecarEnv::without_python(dir.path());

    let cards = crate::doctor::judge_cards();
    assert!(!cards.is_empty());
    for card in &cards {
        let hint = card.hint.as_deref().unwrap_or_default();
        assert!(!hint.contains("adapter is missing"), "{}: {hint}", card.id);
        assert!(!hint.contains("Reinstall"), "{}: {hint}", card.id);
        assert_eq!(card.details["adapter_runtime_ready"], false, "{}", card.id);
        if card.details["found"] == true {
            // An installed CLI is held back by the captions runtime, and says so.
            assert!(hint.contains("Install captions"), "{}: {hint}", card.id);
            assert!(
                hint.contains("system.setup_perception"),
                "{}: {hint}",
                card.id
            );
            let reason = card.details["availability_reason"]
                .as_str()
                .unwrap_or_default();
            assert!(reason.contains("not installed"), "{}: {reason}", card.id);
        }
    }
}
