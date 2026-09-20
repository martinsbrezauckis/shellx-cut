use cut_core::{error_codes, CutError};
use std::path::{Component, Path, PathBuf};

pub(crate) const ENV_ADAPTER_PYTHON: &str = "CUTD_ADAPTER_PYTHON";

/// Interpreter and script chosen for a local adapter invocation.
///
/// In prepared-runtime mode both values are derived from the admitted
/// interpreter and Cut's bundled perception payload. Ordinary installs retain
/// their existing adapter and Python ladders.
#[derive(Debug, Clone)]
pub(crate) struct AdapterRuntime {
    pub(crate) python: Option<PathBuf>,
    pub(crate) script: Option<PathBuf>,
    pub(crate) native_context: bool,
}

/// Resolve an adapter under the supplied Runner context when one is present.
///
/// `ordinary_adapter` is deliberately lazy: an admitted context must not even
/// inspect adapter overrides, the current directory, or their ancestor probes.
/// The SidecarRuntime already verifies the context interpreter/import inventory
/// and selects the application-owned `instruments.py`; adapter scripts are
/// constrained to regular files below that same bundled directory.
pub(crate) fn resolve_adapter_runtime(
    adapter_relative: &str,
    ordinary_adapter: impl FnOnce() -> Option<PathBuf>,
) -> Result<AdapterRuntime, CutError> {
    let sidecar = cut_perception::sidecar_runtime()?;
    resolve_adapter_runtime_from_parts(
        sidecar.python,
        sidecar.script,
        sidecar.native_context.is_some(),
        Path::new(adapter_relative),
        ordinary_configured_adapter_python,
        ordinary_adapter,
    )
    .map_err(|_| {
        CutError::new(
            error_codes::SIDECAR,
            format!("prepared native runtime requires bundled {adapter_relative}"),
            "the installed application payload is missing its adapter script",
        )
    })
}

/// Apply the shared no-bytecode policy before a Python adapter script is
/// appended. In prepared mode this is `-E -s -B`, which ignores host
/// `PYTHON*` configuration and the user site directory.
pub(crate) fn apply_adapter_python_policy(
    command: &mut tokio::process::Command,
    native_context: bool,
) {
    command
        .args(cut_perception::sidecar::python_command_args(native_context))
        .env(cut_perception::sidecar::PYTHONDONTWRITEBYTECODE_ENV, "1");
}

fn resolve_adapter_runtime_from_parts(
    sidecar_python: PathBuf,
    sidecar_script: PathBuf,
    native_context: bool,
    adapter_relative: &Path,
    ordinary_python: impl FnOnce() -> Option<PathBuf>,
    ordinary_adapter: impl FnOnce() -> Option<PathBuf>,
) -> Result<AdapterRuntime, String> {
    if native_context {
        let script = bundled_adapter_path(&sidecar_script, adapter_relative).ok_or_else(|| {
            format!(
                "bundled adapter {} is unavailable",
                adapter_relative.display()
            )
        })?;
        return Ok(AdapterRuntime {
            python: Some(sidecar_python),
            script: Some(script),
            native_context: true,
        });
    }

    Ok(AdapterRuntime {
        python: ordinary_python(),
        script: ordinary_adapter(),
        native_context: false,
    })
}

fn bundled_adapter_path(sidecar_entrypoint: &Path, adapter_relative: &Path) -> Option<PathBuf> {
    if adapter_relative.as_os_str().is_empty()
        || adapter_relative.is_absolute()
        || !adapter_relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return None;
    }
    let adapter = sidecar_entrypoint.parent()?.join(adapter_relative);
    adapter.is_file().then_some(adapter)
}

fn ordinary_configured_adapter_python() -> Option<PathBuf> {
    let explicit = std::env::var_os(ENV_ADAPTER_PYTHON)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let managed = cut_perception::configured_sidecar_python().ok()?;
    adapter_python_for_platform(
        explicit,
        managed,
        find_python_on_path(),
        cfg!(target_os = "macos"),
    )
}

pub(super) fn adapter_python_for_platform(
    explicit: Option<PathBuf>,
    managed: Option<PathBuf>,
    path_python: Option<PathBuf>,
    is_macos: bool,
) -> Option<PathBuf> {
    if explicit.is_some() {
        return explicit;
    }
    if managed.is_some() {
        return managed;
    }
    if is_macos {
        return None;
    }
    path_python
}

pub(super) fn find_python_on_path() -> Option<PathBuf> {
    find_executable_on_path(&["python3", "python"])
}

fn find_executable_on_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
            #[cfg(windows)]
            {
                let executable = directory.join(format!("{name}.exe"));
                if executable.is_file() {
                    return Some(executable);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn prepared_adapter_uses_only_the_bundled_script_and_interpreter() {
        let temp = tempfile::tempdir().unwrap();
        let payload = temp.path().join("perception");
        let instruments = payload.join("instruments.py");
        let bundled = payload.join("judge/adapters/ladder_judge.py");
        let native_python = temp.path().join("native-python");
        let ordinary_python = temp.path().join("ordinary-python");
        let ordinary_adapter = temp.path().join("ordinary-adapter.py");
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        for path in [
            &instruments,
            &bundled,
            &native_python,
            &ordinary_python,
            &ordinary_adapter,
        ] {
            std::fs::write(path, b"fixture").unwrap();
        }
        let fallbacks = Cell::new(0);

        let runtime = resolve_adapter_runtime_from_parts(
            native_python.clone(),
            instruments,
            true,
            Path::new("judge/adapters/ladder_judge.py"),
            || {
                fallbacks.set(fallbacks.get() + 1);
                Some(ordinary_python)
            },
            || {
                fallbacks.set(fallbacks.get() + 1);
                Some(ordinary_adapter)
            },
        )
        .unwrap();
        assert_eq!(runtime.python, Some(native_python));
        assert_eq!(runtime.script, Some(bundled));
        assert!(runtime.native_context);
        assert_eq!(
            fallbacks.get(),
            0,
            "native selection must not inspect fallbacks"
        );
    }

    #[test]
    fn prepared_adapter_refuses_missing_or_escaping_bundled_script_without_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let payload = temp.path().join("perception");
        let instruments = payload.join("instruments.py");
        std::fs::create_dir_all(&payload).unwrap();
        std::fs::write(&instruments, b"fixture").unwrap();
        let fallbacks = Cell::new(0);

        assert!(resolve_adapter_runtime_from_parts(
            temp.path().join("native-python"),
            instruments.clone(),
            true,
            Path::new("comment-draft-adapter.py"),
            || {
                fallbacks.set(fallbacks.get() + 1);
                Some(temp.path().join("ordinary-python"))
            },
            || {
                fallbacks.set(fallbacks.get() + 1);
                Some(temp.path().join("ordinary-adapter.py"))
            },
        )
        .is_err());
        assert_eq!(
            fallbacks.get(),
            0,
            "missing bundled script must not fall back"
        );
        assert!(bundled_adapter_path(&instruments, Path::new("../ordinary-adapter.py")).is_none());
        assert!(bundled_adapter_path(&instruments, Path::new("/absolute-adapter.py")).is_none());
    }

    #[test]
    fn ordinary_adapter_keeps_its_existing_python_and_script_selection() {
        let temp = tempfile::tempdir().unwrap();
        let ordinary_python = temp.path().join("ordinary-python");
        let ordinary_adapter = temp.path().join("ordinary-adapter.py");
        let calls = Cell::new(0);
        let runtime = resolve_adapter_runtime_from_parts(
            temp.path().join("unused-native-python"),
            temp.path().join("instruments.py"),
            false,
            Path::new("generate_prompt_adapter.py"),
            || {
                calls.set(calls.get() + 1);
                Some(ordinary_python.clone())
            },
            || {
                calls.set(calls.get() + 1);
                Some(ordinary_adapter.clone())
            },
        )
        .unwrap();
        assert_eq!(runtime.python, Some(ordinary_python));
        assert_eq!(runtime.script, Some(ordinary_adapter));
        assert!(!runtime.native_context);
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn adapter_python_policy_uses_the_shared_native_flags() {
        assert_eq!(
            cut_perception::sidecar::python_command_args(true),
            ["-E", "-s", "-B"]
        );
        assert_eq!(cut_perception::sidecar::python_command_args(false), ["-B"]);
    }
}
