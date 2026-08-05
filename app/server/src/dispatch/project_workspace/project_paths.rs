//! Project filesystem-root resolution.
//!
//! The project index/library state root and the user-visible projects directory
//! are deliberately separate. Tests and portable rigs need to isolate both,
//! so `SHELLX_CUT_PROJECTS_DIR` overrides only the latter without changing the
//! normal Documents/home behavior.

use std::ffi::OsString;
use std::path::PathBuf;

fn resolve_projects_dir(
    override_dir: Option<OsString>,
    user_profile: Option<OsString>,
    unix_home: Option<OsString>,
    current_dir: PathBuf,
) -> PathBuf {
    if let Some(path) = override_dir.filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(profile) = user_profile.filter(|path| !path.is_empty()) {
        let profile = PathBuf::from(profile);
        let documents = profile.join("Documents");
        return if documents.is_dir() {
            documents.join("ShellX Cut Projects")
        } else {
            profile.join("ShellX Cut Projects")
        };
    }
    unix_home
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join("ShellX Cut Projects"))
        .unwrap_or(current_dir)
}

pub(super) fn default_projects_dir() -> PathBuf {
    let directory = resolve_projects_dir(
        std::env::var_os("SHELLX_CUT_PROJECTS_DIR"),
        std::env::var_os("USERPROFILE"),
        std::env::var_os("HOME"),
        std::env::current_dir().unwrap_or_else(|_| ".".into()),
    );
    let _ = std::fs::create_dir_all(&directory);
    directory
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_projects_root_wins_over_user_homes() {
        let result = resolve_projects_dir(
            Some(OsString::from("/isolated/projects")),
            Some(OsString::from("/windows-profile")),
            Some(OsString::from("/unix-home")),
            PathBuf::from("/cwd"),
        );
        assert_eq!(result, PathBuf::from("/isolated/projects"));
    }

    #[test]
    fn unix_home_retains_the_visible_default_folder() {
        let result = resolve_projects_dir(
            None,
            None,
            Some(OsString::from("/unix-home")),
            PathBuf::from("/cwd"),
        );
        assert_eq!(result, PathBuf::from("/unix-home/ShellX Cut Projects"));
    }

    #[test]
    fn missing_homes_fall_back_to_the_process_directory() {
        let result = resolve_projects_dir(None, None, None, PathBuf::from("/cwd"));
        assert_eq!(result, PathBuf::from("/cwd"));
    }
}
