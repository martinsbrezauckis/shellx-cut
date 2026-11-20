//! unpack_tools.rs — the programs the FFmpeg download needs to unpack its
//! archive, checked BEFORE the 100+ MB download starts.
//!
//! The pinned Linux archive is a `.tar.xz`. GNU `tar -J` does not decompress by
//! itself: it starts the separate `xz` program. A minimal Debian or Ubuntu image
//! has `tar` but not `xz`, so the download used to finish and then fail with
//! `tar (child): xz: Cannot exec`, naming nothing the person could act on. The
//! Linux packages now declare `xz` (see tauri.conf.json), and this check covers
//! the machines where it was removed anyway: it says which program is missing
//! and the exact command for this distribution.

use super::*;
use cut_media::tool_setup::ArchiveKind;

/// Directories `tar` searches for `xz`: its own PATH, or the C library's default
/// when PATH is unset (a bare environment, as an agent may start the engine in).
pub(super) fn search_dirs(path: Option<OsString>) -> Vec<PathBuf> {
    let path = path.unwrap_or_else(|| OsString::from("/usr/bin:/bin"));
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .collect()
}

fn found_in(dirs: &[PathBuf], program: &str) -> bool {
    dirs.iter().any(|dir| dir.join(program).is_file())
}

/// The package manager command that installs `program` on the system whose
/// `/etc/os-release` text is given. Unknown systems get the package name.
pub(super) fn install_command(os_release: &str, program: &str) -> String {
    let mut ids: Vec<String> = Vec::new();
    for line in os_release.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key == "ID" || key == "ID_LIKE" {
            ids.extend(
                value
                    .trim_matches(|c| c == '"' || c == '\'')
                    .split_whitespace()
                    .map(str::to_ascii_lowercase),
            );
        }
    }
    let has = |names: &[&str]| ids.iter().any(|id| names.contains(&id.as_str()));
    let package = match (program, has(&["debian", "ubuntu"])) {
        ("xz", true) => "xz-utils",
        _ => program,
    };
    if has(&["debian", "ubuntu"]) {
        format!("sudo apt install {package}")
    } else if has(&["fedora", "rhel", "centos"]) {
        format!("sudo dnf install {package}")
    } else if has(&["suse", "opensuse", "sles"]) {
        format!("sudo zypper install {package}")
    } else if has(&["arch"]) {
        format!("sudo pacman -S {package}")
    } else if has(&["alpine"]) {
        format!("sudo apk add {package}")
    } else {
        format!("install the '{package}' package with your distribution's package manager")
    }
}

/// The refusal naming a missing unpack program and how to get it.
pub(super) fn missing_program(program: &str, os_release: &str, downloaded: bool) -> CutError {
    let command = install_command(os_release, program);
    let state = if downloaded {
        "the download was not used"
    } else {
        "nothing was downloaded"
    };
    CutError::new(
        error_codes::JOB_FAILED,
        format!(
            "FFmpeg was not installed: the '{program}' program is needed to unpack the download \
             and this computer does not have it"
        ),
        format!(
            "the FFmpeg archive is a .tar.xz and `tar` starts '{program}' to read it; \
             '{program}' was not found, {state}"
        ),
    )
    .with_suggested_action(format!(
        "Install it ({command}), then choose Download again. Or install your distribution's \
         FFmpeg and click Check again."
    ))
}

fn os_release_text() -> String {
    std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default()
}

/// The program missing for this kind of archive, if any: `tar` (given as a
/// path check) and, for `.tar.xz`, `xz`.
fn missing_for(kinds: &[ArchiveKind], dirs: &[PathBuf], tar_present: bool) -> Option<&'static str> {
    if kinds.contains(&ArchiveKind::TarXz) {
        if !tar_present {
            return Some("tar");
        }
        if !found_in(dirs, "xz") {
            return Some("xz");
        }
    }
    None
}

/// Check every program the archives of this release need. Cheap; no network.
/// Returns the absolute `xz` found when a `.tar.xz` is among them: the unpack
/// lets tar search only that program's folder, so tar starts exactly the `xz`
/// checked here.
pub(super) fn preflight(kinds: &[ArchiveKind], tar: &Path) -> Result<Option<PathBuf>, CutError> {
    let dirs = search_dirs(std::env::var_os("PATH"));
    match missing_for(kinds, &dirs, tar.is_file()) {
        Some(program) => Err(missing_program(program, &os_release_text(), false)),
        None => Ok(kinds
            .contains(&ArchiveKind::TarXz)
            .then(|| {
                dirs.iter()
                    .map(|dir| dir.join("xz"))
                    .find(|xz| xz.is_file())
            })
            .flatten()),
    }
}

/// `tar` ran but could not start `xz` (it was removed after the check, or the
/// check could not see it): the same plain refusal instead of a raw tar error.
pub(super) fn from_tar_failure(stderr: &str) -> Option<CutError> {
    let lower = stderr.to_ascii_lowercase();
    let xz_cannot_start = lower.contains("xz: cannot exec")
        || lower.contains("xz: not found")
        || lower.contains("xz: no such file");
    xz_cannot_start.then(|| missing_program("xz", &os_release_text(), true))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UBUNTU: &str = "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n";
    const DEBIAN: &str = "PRETTY_NAME=\"Debian GNU/Linux 13\"\nID=debian\n";
    const FEDORA: &str = "NAME=\"Fedora Linux\"\nID=fedora\n";
    const SUSE: &str = "ID=\"opensuse-leap\"\nID_LIKE=\"suse opensuse\"\n";
    const ARCH: &str = "ID=arch\n";
    const MINT: &str = "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n";

    #[test]
    fn the_command_is_for_the_distribution_and_xz_has_its_debian_package_name() {
        assert_eq!(install_command(UBUNTU, "xz"), "sudo apt install xz-utils");
        assert_eq!(install_command(DEBIAN, "xz"), "sudo apt install xz-utils");
        assert_eq!(install_command(MINT, "xz"), "sudo apt install xz-utils");
        assert_eq!(install_command(FEDORA, "xz"), "sudo dnf install xz");
        assert_eq!(install_command(SUSE, "xz"), "sudo zypper install xz");
        assert_eq!(install_command(ARCH, "xz"), "sudo pacman -S xz");
        assert_eq!(install_command(DEBIAN, "tar"), "sudo apt install tar");
        assert!(install_command("", "xz").contains("'xz' package"));
    }

    #[test]
    fn a_missing_xz_is_named_before_any_download_with_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = vec![dir.path().to_path_buf()];
        assert_eq!(missing_for(&[ArchiveKind::TarXz], &dirs, true), Some("xz"));
        assert_eq!(
            missing_for(&[ArchiveKind::TarXz], &dirs, false),
            Some("tar")
        );
        assert_eq!(missing_for(&[ArchiveKind::Zip], &dirs, true), None);
        std::fs::write(dir.path().join("xz"), b"").unwrap();
        assert_eq!(missing_for(&[ArchiveKind::TarXz], &dirs, true), None);

        let error = missing_program("xz", UBUNTU, false);
        let text = format!(
            "{} | {} | {:?}",
            error.message, error.cause, error.suggested_action
        );
        assert!(text.contains("'xz' program"), "{text}");
        assert!(text.contains("sudo apt install xz-utils"), "{text}");
        assert!(text.contains("nothing was downloaded"), "{text}");
        assert!(!text.contains("Cannot exec"), "{text}");
    }

    #[test]
    fn the_search_follows_tar_and_skips_relative_entries() {
        // Platform-true absolute folders (a Unix path is not absolute on Windows).
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([
            a.path().to_path_buf(),
            PathBuf::from("relative"),
            PathBuf::new(),
            b.path().to_path_buf(),
        ])
        .unwrap();
        assert_eq!(
            search_dirs(Some(path)),
            vec![a.path().to_path_buf(), b.path().to_path_buf()]
        );
    }

    // The preflight only matters for the Linux `.tar.xz`; Windows and macOS
    // unpack a `.zip` with the system tar, which needs no xz.
    #[cfg(unix)]
    #[test]
    fn a_bare_environment_searches_the_c_library_default_folders() {
        assert_eq!(
            search_dirs(None),
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]
        );
    }

    #[test]
    fn a_zip_release_needs_no_unpack_program_check() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            missing_for(&[ArchiveKind::Zip], &[dir.path().to_path_buf()], false),
            None
        );
    }

    #[test]
    fn the_raw_tar_error_becomes_the_plain_refusal() {
        let raw = "tar (child): xz: Cannot exec: No such file or directory\n\
                   tar (child): Error is not recoverable: exiting now\n\
                   tar: Child returned status 2";
        let error = from_tar_failure(raw).expect("xz failure recognised");
        assert!(error.message.contains("'xz' program"));
        assert!(from_tar_failure("tar: member not found").is_none());
        assert!(from_tar_failure("tar: Unexpected EOF in archive").is_none());
    }
}
