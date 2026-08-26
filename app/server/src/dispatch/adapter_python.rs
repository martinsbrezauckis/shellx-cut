use std::path::PathBuf;

pub(crate) const ENV_ADAPTER_PYTHON: &str = "CUTD_ADAPTER_PYTHON";

pub(crate) fn configured_adapter_python() -> Option<PathBuf> {
    let explicit = std::env::var_os(ENV_ADAPTER_PYTHON)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    adapter_python_for_platform(
        explicit,
        cut_perception::configured_sidecar_python(),
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
