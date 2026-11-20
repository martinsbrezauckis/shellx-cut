//! What to say when the captions runtime (the sidecar's Python) is not there.
//!
//! Captions, transcription and the output checks run on a Python that ShellX
//! Cut installs itself (`system.setup_perception`, the "Install captions"
//! button). A Python the person installed is never used by the installed app,
//! so "no usable Python" always means one thing for the person: captions are
//! not installed yet. This module is the one place that detects it and words it,
//! so the render receipt, `media.transcribe`, the background enrich job and the
//! other sidecar verbs cannot drift apart.
//!
//! The Windows `WindowsApps\python.exe` "app execution alias" is NOT a Python:
//! it is a shortcut that prints a Microsoft Store hint and exits with 9009.
//! Its text never reaches a person or an agent.

use cut_core::{error_codes, CutError};
use std::ffi::OsString;
use std::path::Path;

/// Stable message of the refusal. Callers match it through
/// [`is_captions_not_installed`], never by repeating the text.
pub const CAPTIONS_NOT_INSTALLED_MESSAGE: &str = "Captions and transcription are not installed";

/// Exit code the Windows Store python shortcut returns.
const WINDOWS_STORE_SHORTCUT_EXIT: i32 = 9009;

/// Why no Python could be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterpreterProblem {
    /// Nothing at the expected place: a path that does not exist, or a bare
    /// command that is not on PATH. `what` is a plain description for the cause.
    NotFound { what: String },
    /// The command exists but is the Microsoft Store shortcut, not a Python.
    WindowsStoreShortcut,
}

/// The next step for an agent (and what it should relay to the person).
pub fn captions_setup_action() -> String {
    "Ask the person to choose Install captions (Settings > Environment, or the Transcript \
     panel), or call system.setup_perception and wait for its job; it needs no administrator \
     rights and no Python of their own. Then retry. Core editing and export work without it."
        .to_string()
}

/// The refusal for a missing captions runtime. Code `sidecar`; nothing ran.
pub fn captions_not_installed_error(problem: &InterpreterProblem) -> CutError {
    let why = match problem {
        InterpreterProblem::NotFound { what } => what.clone(),
        InterpreterProblem::WindowsStoreShortcut => {
            "the 'python' command on this PC is only the Microsoft Store shortcut, not a Python"
                .to_string()
        }
    };
    CutError::new(
        error_codes::SIDECAR,
        CAPTIONS_NOT_INSTALLED_MESSAGE,
        format!(
            "ShellX Cut runs captions and transcription on its own Python, which Install \
             captions sets up; a Python you installed yourself is not used. In this case, {why}."
        ),
    )
    .with_suggested_action(captions_setup_action())
    .with_requirement("captions_runtime")
}

/// True for the refusal built by [`captions_not_installed_error`].
pub fn is_captions_not_installed(error: &CutError) -> bool {
    error.code == error_codes::SIDECAR && error.message == CAPTIONS_NOT_INSTALLED_MESSAGE
}

/// Does a finished run look like the Windows Store python shortcut? The text
/// is localized by Windows, so the exit code 9009 is the main evidence and the
/// English text only a second one.
pub fn is_windows_store_shortcut_run(exit_code: Option<i32>, output: &str) -> bool {
    exit_code == Some(WINDOWS_STORE_SHORTCUT_EXIT) || output.contains("Python was not found")
}

/// Check, without starting anything, that `python` can be started: an existing
/// file, or a bare command found on PATH. A bare command that resolves to the
/// Store shortcut passes here (it cannot be told from a real Store Python by
/// name) and is recognised after the run by [`is_windows_store_shortcut_run`].
pub fn check_interpreter(python: &Path) -> Result<(), CutError> {
    let bare = python.components().count() == 1;
    let found = if bare {
        on_path(python.as_os_str(), std::env::var_os("PATH"))
    } else {
        python.is_file()
    };
    if found {
        return Ok(());
    }
    let what = if bare {
        format!(
            "no '{}' command was found on this PC",
            python.to_string_lossy()
        )
    } else {
        format!("the Python expected at {} is missing", python.display())
    };
    Err(captions_not_installed_error(
        &InterpreterProblem::NotFound { what },
    ))
}

fn on_path(name: &std::ffi::OsStr, path_var: Option<OsString>) -> bool {
    let Some(path_var) = path_var else {
        return false;
    };
    let with_exe = cfg!(windows) && Path::new(name).extension().is_none();
    std::env::split_paths(&path_var).any(|dir| {
        dir.join(name).is_file() || (with_exe && dir.join(name).with_extension("exe").is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_path_is_captions_not_installed_with_the_real_action() {
        let dir = tempfile::tempdir().unwrap();
        let error = check_interpreter(&dir.path().join(".venv").join("python")).unwrap_err();
        assert!(is_captions_not_installed(&error));
        assert!(error.cause.contains("is missing"), "{}", error.cause);
        let action = error.suggested_action.unwrap();
        assert!(action.contains("Install captions"));
        assert!(action.contains("system.setup_perception"));
    }

    #[test]
    fn bare_command_not_on_path_is_missing_and_present_file_passes() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "py-fixture.exe"
        } else {
            "py-fixture"
        };
        assert!(!on_path(
            std::ffi::OsStr::new("py-fixture"),
            Some(dir.path().as_os_str().to_owned())
        ));
        std::fs::write(dir.path().join(name), b"").unwrap();
        assert!(on_path(
            std::ffi::OsStr::new("py-fixture"),
            Some(dir.path().as_os_str().to_owned())
        ));
        assert!(!on_path(std::ffi::OsStr::new("py-fixture"), None));
        let error = check_interpreter(Path::new("no-such-python-command-xyz")).unwrap_err();
        assert!(error
            .cause
            .contains("no 'no-such-python-command-xyz' command"));
    }

    #[test]
    fn store_shortcut_run_is_recognised_by_exit_code_or_text() {
        assert!(is_windows_store_shortcut_run(Some(9009), ""));
        assert!(is_windows_store_shortcut_run(
            Some(1),
            "Python was not found; run without arguments to install from the Microsoft Store"
        ));
        assert!(!is_windows_store_shortcut_run(
            Some(1),
            "ModuleNotFoundError"
        ));
    }

    #[test]
    fn store_shortcut_text_never_reaches_the_person() {
        let error = captions_not_installed_error(&InterpreterProblem::WindowsStoreShortcut);
        let all = format!(
            "{} {} {:?}",
            error.message, error.cause, error.suggested_action
        );
        assert!(!all.contains("Python was not found"), "{all}");
        assert!(!all.contains("App execution aliases"), "{all}");
        assert!(error.cause.contains("Microsoft Store shortcut"));
    }
}
