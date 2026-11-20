//! Where an ordinary-mode Python adapter script is taken from.
//!
//! A project, or the folder a person or agent starts `cutd` in, may come from
//! someone else. An adapter is therefore never looked up relative to the
//! current folder, a project folder, or a folder above the executable. It is
//! either the explicit override, which must be an absolute path to a file and
//! is authoritative (a missing one is not replaced by another file), or the
//! copy bundled beside the perception entrypoint that
//! `cut_perception::sidecar_paths` selects: the packaged payload, the app-data
//! sidecar folder, or the source checkout this build came from.

use super::adapter_python::bundled_adapter_path;
use std::path::{Path, PathBuf};

/// The adapter named by `override_env`, or the bundled `adapter_relative`.
pub(crate) fn configured_or_bundled_adapter(
    override_env: &str,
    adapter_relative: &str,
) -> Option<PathBuf> {
    let explicit = std::env::var_os(override_env)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let (_, entrypoint) = cut_perception::sidecar_paths();
    configured_or_bundled_adapter_from(explicit, &entrypoint, Path::new(adapter_relative))
}

pub(crate) fn configured_or_bundled_adapter_from(
    explicit: Option<PathBuf>,
    sidecar_entrypoint: &Path,
    adapter_relative: &Path,
) -> Option<PathBuf> {
    if let Some(path) = explicit {
        // A relative override would be resolved against the current folder.
        return (path.is_absolute() && path.is_file()).then_some(path);
    }
    bundled_adapter_path(sidecar_entrypoint, adapter_relative)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(root: &Path) -> PathBuf {
        let perception = root.join("perception");
        std::fs::create_dir_all(&perception).unwrap();
        std::fs::write(perception.join("instruments.py"), b"# fixture\n").unwrap();
        std::fs::write(
            perception.join("generate_prompt_adapter.py"),
            b"# bundled\n",
        )
        .unwrap();
        perception.join("instruments.py")
    }

    #[test]
    fn the_bundled_copy_is_used_without_an_override() {
        let dir = tempfile::tempdir().unwrap();
        let entrypoint = payload(dir.path());
        assert_eq!(
            configured_or_bundled_adapter_from(
                None,
                &entrypoint,
                Path::new("generate_prompt_adapter.py")
            ),
            Some(dir.path().join("perception/generate_prompt_adapter.py"))
        );
    }

    #[test]
    fn a_relative_or_missing_override_is_not_replaced_by_another_file() {
        let dir = tempfile::tempdir().unwrap();
        let entrypoint = payload(dir.path());
        for explicit in [
            PathBuf::from("generate_prompt_adapter.py"),
            PathBuf::from("perception/generate_prompt_adapter.py"),
            dir.path().join("missing.py"),
        ] {
            assert_eq!(
                configured_or_bundled_adapter_from(
                    Some(explicit.clone()),
                    &entrypoint,
                    Path::new("generate_prompt_adapter.py")
                ),
                None,
                "{explicit:?}"
            );
        }
        let absolute = dir.path().join("perception/generate_prompt_adapter.py");
        assert_eq!(
            configured_or_bundled_adapter_from(
                Some(absolute.clone()),
                &entrypoint,
                Path::new("generate_prompt_adapter.py")
            ),
            Some(absolute)
        );
    }
}
