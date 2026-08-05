//! Motion product-template discovery for Cut Generate.

use crate::motion_runtime::{find_motion_root, ENV_MOTION_ROOT};
use cut_core::{error_codes, CutError};
use std::path::{Path, PathBuf};

pub(crate) const ENV_MOTION_TEMPLATE_ROOT: &str = "SHELLX_MOTION_TEMPLATE_ROOT";

pub(crate) fn resolve_motion_template_package(template: &str) -> Result<PathBuf, CutError> {
    let candidate = PathBuf::from(template);
    if candidate.is_absolute() || template.contains(std::path::MAIN_SEPARATOR) {
        return path_if_package(candidate).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                "Motion template package was not found",
                template,
            )
        });
    }

    if let Ok(root) = std::env::var(ENV_MOTION_TEMPLATE_ROOT) {
        if let Some(path) = resolve_from_template_root(Path::new(root.trim()), template) {
            return Ok(path);
        }
    }
    if let Some(root) = find_motion_root() {
        if let Some(path) = resolve_from_motion_root(&root, template) {
            return Ok(path);
        }
    }

    Err(CutError::new(
        error_codes::NOT_FOUND,
        format!("Motion template alias '{template}' could not be resolved"),
        format!("set {ENV_MOTION_TEMPLATE_ROOT} or {ENV_MOTION_ROOT}"),
    )
    .with_suggested_action(
        "Install or checkout ShellX Motion and expose its product-template root",
    ))
}

fn resolve_from_template_root(root: &Path, template: &str) -> Option<PathBuf> {
    if root.as_os_str().is_empty() {
        return None;
    }
    path_if_package(root.join(template)).or_else(|| path_if_package(root.to_path_buf()))
}

fn resolve_from_motion_root(root: &Path, template: &str) -> Option<PathBuf> {
    [
        root.join("templates")
            .join("shellx-product-pack")
            .join(template),
        root.join("templates").join(template),
        root.join("fixtures").join("packages").join(template),
    ]
    .into_iter()
    .find_map(path_if_package)
}

fn path_if_package(path: PathBuf) -> Option<PathBuf> {
    path.join("manifest.json").is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, write};

    #[test]
    fn discovers_promoted_product_templates_before_legacy_fixtures() {
        let root = tempfile::tempdir().expect("motion root");
        let promoted = package(
            root.path()
                .join("templates/shellx-product-pack/cinematic-fog-title"),
        );
        let fixture = package(root.path().join("fixtures/packages/cinematic-fog-title"));

        assert_eq!(
            resolve_from_motion_root(root.path(), "cinematic-fog-title"),
            Some(promoted)
        );
        assert_ne!(
            resolve_from_motion_root(root.path(), "cinematic-fog-title"),
            Some(fixture)
        );
    }

    #[test]
    fn explicit_template_root_accepts_a_catalog_or_one_package() {
        let root = tempfile::tempdir().expect("template root");
        let package = package(root.path().join("tracked-callout-overlay"));
        assert_eq!(
            resolve_from_template_root(root.path(), "tracked-callout-overlay"),
            Some(package.clone())
        );
        assert_eq!(
            resolve_from_template_root(&package, "ignored-alias"),
            Some(package)
        );
    }

    fn package(path: PathBuf) -> PathBuf {
        create_dir_all(&path).expect("package directory");
        write(path.join("manifest.json"), b"{}\n").expect("manifest");
        path
    }
}
