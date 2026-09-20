//! Prepared native runtime selection for Python sidecars.
//!
//! The release runner's locator pins the interpreter and import origins. The
//! application continues to select its bundled scripts beside `instruments.py`.
//! A present locator is authoritative: errors never fall through to the legacy
//! Python ladder.

use super::{
    dev_checkout_sidecar_dir_for_exe, sidecar_base_dirs, sidecar_paths, sidecar_python_from_bases,
    ENV_PYTHON,
};
use cut_core::{error_codes, CutError};
use cut_native_runtime_context::RuntimeContext;
use std::path::PathBuf;
use std::process::Command;

/// Interpreter and script chosen for a sidecar invocation.
#[derive(Debug, Clone)]
pub struct SidecarRuntime {
    pub python: PathBuf,
    pub script: PathBuf,
    pub native_context: Option<RuntimeContext>,
}

/// Resolve the native interpreter when the Runner supplied a context, otherwise
/// retain the established sidecar ladder. Both paths keep the application-owned
/// `instruments.py` selection.
pub fn sidecar_runtime() -> Result<SidecarRuntime, CutError> {
    match RuntimeContext::from_env().map_err(native_runtime_error)? {
        Some(context) => {
            context.verify_interpreter().map_err(native_runtime_error)?;
            context.verify_imports().map_err(native_runtime_error)?;
            Ok(SidecarRuntime {
                python: context.interpreter.path.clone(),
                script: native_bundled_sidecar_script()?,
                native_context: Some(context),
            })
        }
        None => {
            let (python, script) = sidecar_paths();
            Ok(SidecarRuntime {
                python,
                script,
                native_context: None,
            })
        }
    }
}

/// Load the supplied context for consumers that need a product-specific asset.
/// A malformed supplied locator remains an error rather than a legacy fallback.
pub fn native_runtime_context() -> Result<Option<RuntimeContext>, CutError> {
    RuntimeContext::from_env().map_err(native_runtime_error)
}

/// Resolve only an app-managed or explicitly configured sidecar Python.
///
/// `system.doctor` runs on launch. On clean macOS, spawning bare `python3`
/// opens Apple's Command Line Tools installer prompt, so passive environment
/// scans must not use the PATH fallback or the build machine's checkout unless
/// this process is actually a repo-launched dev binary. A supplied native
/// context instead yields its re-verified interpreter.
pub fn configured_sidecar_python() -> Result<Option<PathBuf>, CutError> {
    if let Some(context) = native_runtime_context()? {
        context
            .verify_interpreter()
            .and_then(|()| context.verify_imports())
            .map_err(native_runtime_error)?;
        return Ok(Some(context.interpreter.path));
    }
    if let Some(python) = std::env::var_os(ENV_PYTHON) {
        if !python.is_empty() {
            return Ok(Some(PathBuf::from(python)));
        }
    }
    Ok(sidecar_python_from_bases(&sidecar_base_dirs()))
}

/// Apply the process policy before a Python script or `-c` snippet is appended.
/// A supplied native context also ignores host `PYTHON*` variables and the user
/// site directory, so declared import hashes describe the modules Python loads.
pub fn apply_python_command_policy(command: &mut Command, native_context: bool) {
    command.args(python_command_args(native_context));
    command.env(PYTHONDONTWRITEBYTECODE_ENV, "1");
}

/// Python flags shared by std and Tokio sidecar commands. Callers append the
/// script or `-c` snippet after these flags.
pub fn python_command_args(native_context: bool) -> &'static [&'static str] {
    if native_context {
        &["-E", "-s", "-B"]
    } else {
        &["-B"]
    }
}

/// Environment key used with [`python_command_args`] to avoid bytecode writes.
pub const PYTHONDONTWRITEBYTECODE_ENV: &str = "PYTHONDONTWRITEBYTECODE";

fn native_runtime_error(reason: String) -> CutError {
    CutError::new(
        error_codes::SIDECAR,
        format!("prepared native runtime rejected: {reason}"),
        "repair or remove RELEASE_RUNNER_NATIVE_RUNTIME_CONTEXT before using Python tools",
    )
}

fn native_bundled_sidecar_script() -> Result<PathBuf, CutError> {
    let current_exe = std::env::current_exe().ok();
    native_bundled_sidecar_script_from_bases(&native_bundled_sidecar_bases(current_exe.as_deref()))
        .ok_or_else(|| {
            CutError::new(
                error_codes::SIDECAR,
                "prepared native runtime requires bundled instruments.py",
                "the installed application payload is missing its sidecar script",
            )
        })
}

/// Candidate script directories supplied by the installed application layout.
/// This intentionally excludes sidecar environment overrides and per-user
/// app-data: neither identifies the package inventory that owns the script.
fn native_bundled_sidecar_bases(current_exe: Option<&std::path::Path>) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(exe_dir) = current_exe.and_then(|path| path.parent()) {
        bases.push(exe_dir.join("perception"));
        if let Some(contents) = exe_dir.parent() {
            bases.push(contents.join("Resources").join("perception"));
            #[cfg(target_os = "linux")]
            bases.push(contents.join("lib").join("ShellX Cut").join("perception"));
        }
    }
    if let Some(dev_checkout) = dev_checkout_sidecar_dir_for_exe(current_exe) {
        bases.push(dev_checkout);
    }
    bases
}

fn native_bundled_sidecar_script_from_bases(bases: &[PathBuf]) -> Option<PathBuf> {
    bases
        .iter()
        .map(|base| base.join("instruments.py"))
        .find(|script| script.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_runtime_absent_keeps_the_existing_sidecar_ladder() {
        let _guard = super::super::NATIVE_RUNTIME_ENV_LOCK.lock().unwrap();
        let prior = std::env::var_os(cut_native_runtime_context::CONTEXT_ENV);
        std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV);
        let runtime = sidecar_runtime().unwrap();
        assert!(runtime.native_context.is_none());
        match prior {
            Some(value) => std::env::set_var(cut_native_runtime_context::CONTEXT_ENV, value),
            None => std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV),
        }
    }

    #[test]
    fn invalid_native_runtime_never_falls_through_to_the_legacy_python() {
        let _guard = super::super::NATIVE_RUNTIME_ENV_LOCK.lock().unwrap();
        let prior = std::env::var_os(cut_native_runtime_context::CONTEXT_ENV);
        let temp = tempfile::tempdir().unwrap();
        let invalid = temp.path().join("native-runtime-context.json");
        std::fs::write(&invalid, b"not json").unwrap();
        std::env::set_var(cut_native_runtime_context::CONTEXT_ENV, &invalid);
        assert!(sidecar_runtime().is_err());
        match prior {
            Some(value) => std::env::set_var(cut_native_runtime_context::CONTEXT_ENV, value),
            None => std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV),
        }
    }

    #[test]
    fn native_python_command_policy_isolated_and_normal_compatible() {
        let mut normal = Command::new("python");
        apply_python_command_policy(&mut normal, false);
        assert_eq!(
            normal
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["-B"]
        );

        let mut command = Command::new("python");
        apply_python_command_policy(&mut command, true);
        assert_eq!(
            command
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["-E", "-s", "-B"]
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == "PYTHONDONTWRITEBYTECODE")
                .and_then(|(_, value)| value)
                .and_then(|value| value.to_str()),
            Some("1")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_python_command_ignores_hostile_pythonpath() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("sitecustomize.py"),
            "print('hostile-pythonpath')\n",
        )
        .unwrap();
        let mut command = Command::new("python3");
        apply_python_command_policy(&mut command, true);
        command
            .args(["-c", "print('native-safe')"])
            .env("PYTHONPATH", temp.path());
        let output = command
            .output()
            .expect("Linux source check requires python3 for the hostile-PYTHONPATH proof");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "native-safe\n",
            "-E must prevent PYTHONPATH from importing hostile sitecustomize"
        );
    }

    #[test]
    fn native_script_resolution_uses_only_installed_resource_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let macos = temp.path().join("Contents").join("MacOS");
        let resources = temp
            .path()
            .join("Contents")
            .join("Resources")
            .join("perception");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::create_dir_all(&resources).unwrap();
        let script = resources.join("instruments.py");
        std::fs::write(&script, b"fixture").unwrap();

        let bases = native_bundled_sidecar_bases(Some(&macos.join("cutd")));
        assert_eq!(
            native_bundled_sidecar_script_from_bases(&bases),
            Some(script),
            "native selection must resolve the application resource, not app-data or an env override"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_script_resolution_supports_the_deb_lib_payload() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("usr").join("bin");
        let resources = temp
            .path()
            .join("usr")
            .join("lib")
            .join("ShellX Cut")
            .join("perception");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&resources).unwrap();
        let script = resources.join("instruments.py");
        std::fs::write(&script, b"fixture").unwrap();

        let bases = native_bundled_sidecar_bases(Some(&bin.join("cutd")));
        assert_eq!(
            native_bundled_sidecar_script_from_bases(&bases),
            Some(script),
            "the native DEB route must bind its packaged /usr/lib payload"
        );
    }
}
