use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

// Tee panic diagnostics to `$HOME/.cutd-panic.log` (or temp) because a
// Tauri-piped stderr otherwise hides a handler panic from the caller.
pub(super) fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if append_panic_log(
            std::env::var_os("HOME").as_deref(),
            &std::env::temp_dir(),
            &format!("[cutd panic] {info}"),
        )
        .is_err()
        {
            eprintln!("[cutd panic] {info}");
        }
        prev(info);
    }));
}

fn append_panic_log(home: Option<&OsStr>, temp_dir: &Path, message: &str) -> io::Result<()> {
    let path = home
        .map(|h| PathBuf::from(h).join(".cutd-panic.log"))
        .unwrap_or_else(|| temp_dir.join("cutd-panic.log"));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    if home.is_none() {
        use std::os::unix::fs::OpenOptionsExt;
        // The shared temp directory may contain another user's planted entry.
        options
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    if home.is_none() {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unsafe temporary panic log file",
            ));
        }
    }
    writeln!(file, "{message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn no_home_does_not_append_through_temp_symlink() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let victim = temp.path().join("victim");
        std::fs::write(&victim, b"unchanged").unwrap();
        symlink(&victim, temp.path().join("cutd-panic.log")).unwrap();

        assert!(append_panic_log(None, temp.path(), "[cutd panic] test").is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"unchanged");
    }

    #[cfg(unix)]
    #[test]
    fn no_home_appends_to_regular_temp_log() {
        let temp = tempfile::tempdir().unwrap();
        append_panic_log(None, temp.path(), "[cutd panic] test").unwrap();
        assert_eq!(
            std::fs::read(temp.path().join("cutd-panic.log")).unwrap(),
            b"[cutd panic] test\n"
        );
    }

    #[test]
    fn home_log_stays_in_home() {
        let home = tempfile::tempdir().unwrap();
        append_panic_log(
            Some(home.path().as_os_str()),
            home.path(),
            "[cutd panic] test",
        )
        .unwrap();
        assert_eq!(
            std::fs::read(home.path().join(".cutd-panic.log")).unwrap(),
            b"[cutd panic] test\n"
        );
        assert!(!home.path().join("cutd-panic.log").exists());
    }
}
