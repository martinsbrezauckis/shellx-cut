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
use cut_native_runtime_context::{
    context_schema_from_env, RuntimeContext, RuntimeContextSet, SET_CONTEXT_CONTRACT,
};
use std::path::PathBuf;
use std::process::Command;
/// Interpreter and script chosen for a sidecar invocation.
#[derive(Debug, Clone)]
pub struct SidecarRuntime {
    pub python: PathBuf,
    pub script: PathBuf,
    pub native_context: Option<RuntimeContext>,
}
const PYTHON_RUNTIME_MEMBER_ID: &str = "python";
const PREMIUM_RUNTIME_MEMBER_ID: &str = "premium";
fn selected_native_runtime_context(prefer_premium: bool) -> Result<Option<RuntimeContext>, String> {
    match context_schema_from_env()? {
        Some(schema) if schema == SET_CONTEXT_CONTRACT => match RuntimeContextSet::from_env()? {
            Some(set) => {
                let ordinary = set.python_context(PYTHON_RUNTIME_MEMBER_ID)?;
                let selected = if prefer_premium {
                    set.optional_python_context(PREMIUM_RUNTIME_MEMBER_ID)?
                        .unwrap_or(ordinary)
                } else {
                    ordinary
                };
                Ok(Some(selected.clone()))
            }
            None => Ok(None),
        },
        Some(_) | None => RuntimeContext::from_env(),
    }
}
/// Resolve the native interpreter when the Runner supplied a context, otherwise
/// retain the established sidecar ladder. Both paths keep the application-owned
/// `instruments.py` selection.
pub fn sidecar_runtime() -> Result<SidecarRuntime, CutError> {
    sidecar_runtime_for_member(false)
}

/// Select a separately sealed premium Python when Runner supplied one. Older
/// single-Python catalogs retain their existing premium matte behavior.
pub fn premium_sidecar_runtime() -> Result<SidecarRuntime, CutError> {
    sidecar_runtime_for_member(true)
}

fn sidecar_runtime_for_member(prefer_premium: bool) -> Result<SidecarRuntime, CutError> {
    match selected_native_runtime_context(prefer_premium).map_err(native_runtime_error)? {
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
    selected_native_runtime_context(false).map_err(native_runtime_error)
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
#[path = "native_runtime_tests.rs"]
mod tests;
