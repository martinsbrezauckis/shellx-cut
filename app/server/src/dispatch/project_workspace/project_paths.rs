//! Project filesystem-root resolution.
//!
//! The project index/library state root and the user-visible projects directory
//! are deliberately separate. Tests and portable rigs need to isolate both,
//! so `SHELLX_CUT_PROJECTS_DIR` overrides only the latter. An isolated
//! `SHELLX_CUT_HOME` also keeps default projects inside its attempt state;
//! ordinary launches retain the normal Documents/home behavior.

use cut_core::{error_codes, CutError};
use std::ffi::OsString;
use std::path::PathBuf;

fn resolve_projects_dir(
    override_dir: Option<OsString>,
    cut_home: Option<OsString>,
    user_profile: Option<OsString>,
    unix_home: Option<OsString>,
    current_dir: PathBuf,
) -> Result<PathBuf, &'static str> {
    if let Some(path) = override_dir.filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    if let Some(home) = cut_home.filter(|path| !path.is_empty()) {
        let root = PathBuf::from(home);
        if !root.is_absolute()
            || !root.is_dir()
            || root
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(
                "SHELLX_CUT_HOME must be an existing absolute directory for isolated projects",
            );
        }
        return Ok(root.join("Projects"));
    }
    if let Some(profile) = user_profile.filter(|path| !path.is_empty()) {
        let profile = PathBuf::from(profile);
        let documents = profile.join("Documents");
        return Ok(if documents.is_dir() {
            documents.join("ShellX Cut Projects")
        } else {
            profile.join("ShellX Cut Projects")
        });
    }
    Ok(unix_home
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join("ShellX Cut Projects"))
        .unwrap_or(current_dir))
}

fn default_projects_dir_for(
    override_dir: Option<OsString>,
    cut_home: Option<OsString>,
    user_profile: Option<OsString>,
    unix_home: Option<OsString>,
    current_dir: PathBuf,
) -> Result<PathBuf, CutError> {
    let directory =
        resolve_projects_dir(override_dir, cut_home, user_profile, unix_home, current_dir)
            .map_err(|reason| {
                CutError::new(error_codes::IO, "cannot resolve projects directory", reason)
            })?;
    let _ = std::fs::create_dir_all(&directory);
    Ok(directory)
}

pub(super) fn default_projects_dir() -> Result<PathBuf, CutError> {
    default_projects_dir_for(
        std::env::var_os("SHELLX_CUT_PROJECTS_DIR"),
        std::env::var_os("SHELLX_CUT_HOME"),
        std::env::var_os("USERPROFILE"),
        std::env::var_os("HOME"),
        std::env::current_dir().unwrap_or_else(|_| ".".into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_projects_root_wins_over_user_homes() {
        let result = resolve_projects_dir(
            Some(OsString::from("/isolated/projects")),
            None,
            Some(OsString::from("/windows-profile")),
            Some(OsString::from("/unix-home")),
            PathBuf::from("/cwd"),
        )
        .unwrap();
        assert_eq!(result, PathBuf::from("/isolated/projects"));
    }

    #[test]
    fn unix_home_retains_the_visible_default_folder() {
        let result = resolve_projects_dir(
            None,
            None,
            None,
            Some(OsString::from("/unix-home")),
            PathBuf::from("/cwd"),
        )
        .unwrap();
        assert_eq!(result, PathBuf::from("/unix-home/ShellX Cut Projects"));
    }

    #[test]
    fn missing_homes_fall_back_to_the_process_directory() {
        let result = resolve_projects_dir(None, None, None, None, PathBuf::from("/cwd")).unwrap();
        assert_eq!(result, PathBuf::from("/cwd"));
    }

    #[test]
    fn attempt_home_contains_project_list_creation_before_user_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let attempt = tmp.path().join("attempt");
        let profile = tmp.path().join("user-profile");
        std::fs::create_dir(&attempt).unwrap();
        std::fs::create_dir(&profile).unwrap();

        let selected = default_projects_dir_for(
            None,
            Some(attempt.clone().into_os_string()),
            Some(profile.clone().into_os_string()),
            None,
            tmp.path().to_path_buf(),
        )
        .unwrap();
        assert_eq!(selected, attempt.join("Projects"));
        assert!(selected.is_dir());
        assert!(!profile.join("ShellX Cut Projects").exists());

        let explicit = tmp.path().join("explicit-projects");
        let selected = default_projects_dir_for(
            Some(explicit.clone().into_os_string()),
            Some(attempt.into_os_string()),
            Some(profile.into_os_string()),
            None,
            tmp.path().to_path_buf(),
        )
        .unwrap();
        assert_eq!(selected, explicit);
        assert!(selected.is_dir());
    }

    #[test]
    fn invalid_attempt_home_does_not_create_user_projects_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let profile = tmp.path().join("user-profile");
        std::fs::create_dir(&profile).unwrap();
        for invalid in [
            PathBuf::from("relative-home"),
            tmp.path().join("missing-home"),
            tmp.path().join("..").join("bad-home"),
        ] {
            assert!(default_projects_dir_for(
                None,
                Some(invalid.into_os_string()),
                Some(profile.clone().into_os_string()),
                None,
                tmp.path().to_path_buf(),
            )
            .is_err());
            assert!(!profile.join("ShellX Cut Projects").exists());
        }
    }
}
