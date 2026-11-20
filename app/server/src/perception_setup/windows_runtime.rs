//! Windows-only parts of the captions runtime setup.
//!
//! WHY
//!   The captions environment is a uv-managed CPython 3.12 plus wheels from
//!   PyPI. Several of those wheels (onnxruntime, ctranslate2, torch and its
//!   family) import the Microsoft Visual C++ runtime (MSVCP140.dll and
//!   friends) and do not ship it. A clean Windows PC does not have it, so
//!   `import onnxruntime` fails with "DLL load failed". The managed Python
//!   ships only vcruntime140.dll and vcruntime140_1.dll.
//!
//! WHAT
//!   1. uv keeps everything it makes inside Cut's own perception folder: the
//!      managed Python, its download cache, no `~\.local\bin` shim and no
//!      registry entry. Then the uninstaller can remove exactly what Cut made
//!      and never touches a uv or Python the person had before.
//!   2. The runtime itself is Microsoft's Visual C++ Redistributable,
//!      installed system-wide by the person (vc_runtime.rs, the tools-list
//!      entry): ShellX Cut neither ships nor fetches these files from anyone
//!      else (owner decision 2026-11-20). Install captions refuses up front
//!      when it is missing.
//!   3. A gate reads the import tables of every native module in the
//!      environment and refuses, naming the DLL, when a Visual C++ runtime
//!      file it needs is in none of: the Python folder, the module's own
//!      folder, the Windows system folder.

use cut_core::{error_codes, CutError};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) use super::pe_imports::imported_dlls;

/// The environment that keeps uv inside Cut's perception folder. Empty off
/// Windows (those platforms keep uv's defaults for now).
pub(super) fn uv_environment() -> Vec<(&'static str, OsString)> {
    if !cfg!(windows) {
        return Vec::new();
    }
    match cut_perception::appdata_sidecar_dir() {
        Some(dir) => uv_environment_for(&dir),
        None => Vec::new(),
    }
}

pub(super) fn uv_environment_for(sidecar_dir: &Path) -> Vec<(&'static str, OsString)> {
    vec![
        (
            "UV_PYTHON_INSTALL_DIR",
            sidecar_dir.join("python").into_os_string(),
        ),
        (
            "UV_CACHE_DIR",
            sidecar_dir.join("uv-cache").into_os_string(),
        ),
        ("UV_PYTHON_INSTALL_BIN", OsString::from("0")),
        ("UV_PYTHON_INSTALL_REGISTRY", OsString::from("0")),
    ]
}

/// Apply [`uv_environment`] when `prog` is the uv binary.
pub(super) fn apply_uv_environment(command: &mut Command, prog: &Path) {
    let is_uv = prog
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("uv"));
    if is_uv {
        command.envs(uv_environment());
    }
}

/// A Visual C++ runtime DLL that a native module imports and that is in none
/// of the folders Windows will search for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MissingRuntime {
    pub dll: String,
    pub module: PathBuf,
}

/// True for the Visual C++ runtime DLL names (never part of Windows). Exact
/// names only: a wheel that bundles its own copy renames it with a hash
/// (`msvcp140-<hash>.dll`, delvewheel) and ships it in its `<name>.libs`
/// folder, so such a name is the wheel's own file, not the system runtime.
pub(super) fn is_vc_runtime_dll(name: &str) -> bool {
    const NAMES: &[&str] = &[
        "msvcp140.dll",
        "msvcp140_1.dll",
        "msvcp140_2.dll",
        "msvcp140_atomic_wait.dll",
        "msvcp140_codecvt_ids.dll",
        "vcruntime140.dll",
        "vcruntime140_1.dll",
        "vcruntime140_threads.dll",
        "concrt140.dll",
        "vcomp140.dll",
        "vccorlib140.dll",
        "vcamp140.dll",
    ];
    let lower = name.to_ascii_lowercase();
    NAMES.contains(&lower.as_str())
        || (lower.len() == 12
            && lower.starts_with("msvcr1")
            && lower.ends_with(".dll")
            && lower[6..8].bytes().all(|b| b.is_ascii_digit()))
}

/// Gate: every Visual C++ runtime DLL imported by a native module under
/// `site_packages` must exist beside the Python program (`app_dir`), in the
/// module's own folder or in `system_dir`. Links are not followed.
pub(super) fn missing_runtime_dlls(
    site_packages: &Path,
    app_dir: &Path,
    system_dir: Option<&Path>,
) -> Vec<MissingRuntime> {
    let mut modules = Vec::new();
    collect_native_modules(site_packages, &mut modules, 0);
    let mut missing = Vec::new();
    for module in modules {
        let Ok(bytes) = std::fs::read(&module) else {
            continue;
        };
        let Some(imports) = imported_dlls(&bytes) else {
            continue;
        };
        let own_dir = module.parent().unwrap_or(site_packages);
        for dll in imports.into_iter().filter(|dll| is_vc_runtime_dll(dll)) {
            let present = [Some(app_dir), Some(own_dir), system_dir]
                .into_iter()
                .flatten()
                .any(|dir| has_file_ignoring_case(dir, &dll));
            if !present {
                missing.push(MissingRuntime {
                    dll,
                    module: module.clone(),
                });
            }
        }
    }
    missing
}

fn collect_native_modules(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 24 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            // Windows junctions report as directories with a reparse tag;
            // symlink_metadata keeps the walk inside the environment.
            if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.file_type().is_symlink()) {
                collect_native_modules(&path, out, depth + 1);
            }
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("pyd") || e.eq_ignore_ascii_case("dll"))
        {
            out.push(path);
        }
    }
}

fn has_file_ignoring_case(dir: &Path, name: &str) -> bool {
    if dir.join(name).is_file() {
        return true;
    }
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
    })
}

/// The refusal for a gate failure: which file is missing, which module needs
/// it, and the way forward.
pub(super) fn missing_runtime_error(missing: &[MissingRuntime], stage: &str) -> CutError {
    let mut names: Vec<String> = missing.iter().map(|m| m.dll.to_ascii_uppercase()).collect();
    names.sort();
    names.dedup();
    let modules: Vec<String> = missing
        .iter()
        .take(4)
        .map(|m| format!("{} needs {}", m.module.display(), m.dll))
        .collect();
    CutError::new(
        error_codes::SIDECAR,
        format!(
            "Captions cannot start on this PC: the {stage} environment needs {} (Microsoft Visual C++ runtime), which is missing",
            names.join(", ")
        ),
        modules.join("; "),
    )
    .with_requirement("vc_runtime")
    .with_suggested_action(format!(
        "Install Microsoft's Visual C++ runtime in Settings > AI & transcription > {title} (Windows asks \
         for administrator permission; agents: system.fetch_tool {{\"tool\":\"{id}\"}}), or from {page}; then choose Install captions again",
        title = crate::vc_runtime::TITLE,
        id = crate::vc_runtime::TOOL_ID,
        page = crate::vc_runtime::OFFICIAL_PAGE,
    ))
}

/// Run the gate for an environment. `managed_python` is the real interpreter
/// every venv on it runs. No-op off Windows.
pub(super) fn check_environment(
    venv_dir: &Path,
    managed_python: &Path,
    stage: &str,
) -> Result<(), CutError> {
    if !cfg!(windows) {
        return Ok(());
    }
    let Some(app_dir) = managed_python.parent() else {
        return Ok(());
    };
    let system_dir =
        std::env::var_os("SystemRoot").map(|root| PathBuf::from(root).join("System32"));
    let site_packages = venv_dir.join("Lib").join("site-packages");
    let missing = missing_runtime_dlls(&site_packages, app_dir, system_dir.as_deref());
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing_runtime_error(&missing, stage))
    }
}

#[cfg(test)]
#[path = "windows_runtime_tests.rs"]
mod tests;
