//! system_programs.rs — the system programs the downloads use to unpack
//! (`tar`, and on Windows PowerShell), named by absolute path, and the PATH
//! their children get.
//!
//! `tar` does not decompress `.tar.xz` / `.tar.gz` itself: it starts `xz` or
//! `gzip` by NAME through its own PATH. An engine started by hand inside a
//! received project may have a relative PATH entry (`.`); tar inherits the
//! engine's working folder, so such an entry would run a planted `./xz`. tar
//! therefore gets a PATH of absolute folders only: just the folder of the
//! decompressor the preflight found, when there is one.

use std::path::{Path, PathBuf};

/// `%SystemRoot%\System32`, from an ABSOLUTE `SystemRoot` only (a relative value
/// would be read against the current folder), else `C:\Windows\System32`.
pub(super) fn windows_system32() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
}

/// The system's own archive tool by absolute path (bsdtar on macOS and
/// Windows reads zip; GNU tar on Linux reads tar.xz).
pub(super) fn system_tar() -> PathBuf {
    if cfg!(windows) {
        windows_system32().join("tar.exe")
    } else if Path::new("/usr/bin/tar").is_file() {
        PathBuf::from("/usr/bin/tar")
    } else {
        PathBuf::from("/bin/tar")
    }
}

/// Windows PowerShell by absolute path.
pub(super) fn windows_powershell() -> PathBuf {
    windows_system32()
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe")
}

/// A `tar` command whose own program search sees only absolute folders: the
/// folder of `helper` (the decompressor the preflight admitted) when given,
/// else the absolute entries of the engine's PATH.
pub(super) fn tar_command(helper: Option<&Path>) -> std::process::Command {
    let mut command = std::process::Command::new(system_tar());
    match helper.and_then(Path::parent) {
        Some(dir) => {
            command.env("PATH", dir);
        }
        None => match cut_media::toolpath::child_path_value() {
            Some(path) => {
                command.env("PATH", path);
            }
            None => {
                command.env_remove("PATH");
            }
        },
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_programs_are_named_by_absolute_path() {
        assert!(system_tar().is_absolute());
        assert!(windows_system32().is_absolute() || !cfg!(windows));
        assert!(windows_powershell().ends_with("powershell.exe"));
    }

    #[test]
    fn tar_searches_only_the_admitted_helper_folder() {
        let helper = if cfg!(windows) {
            PathBuf::from(r"C:\tools\xz.exe")
        } else {
            PathBuf::from("/opt/tools/xz")
        };
        let command = tar_command(Some(&helper));
        let path = command
            .get_envs()
            .find(|(key, _)| *key == "PATH")
            .and_then(|(_, value)| value)
            .expect("tar gets an explicit PATH");
        assert_eq!(Path::new(path), helper.parent().unwrap());
        // Without a helper: only absolute entries of ours, or none at all.
        let command = tar_command(None);
        if let Some((_, Some(path))) = command.get_envs().find(|(key, _)| *key == "PATH") {
            assert!(std::env::split_paths(path).all(|dir| dir.is_absolute()));
        }
    }
}
