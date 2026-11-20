//! The last step of the Python ladder: a Python found on PATH, accepted only
//! when it really runs as Python.
//!
//! A command that is merely named `python` is not a Python: on Windows it can be
//! the Microsoft Store shortcut (`WindowsApps\python.exe`, a zero-length
//! app-execution alias that answers with a Store hint and exit 9009), and on a
//! Mac `/usr/bin/python3` may be Apple's installer stub. Each candidate is
//! therefore asked one cheap question, `python -c "import sys; print(sys.version_info[:2])"`,
//! with a short timeout; anything that does not answer with a version of Python 3
//! is skipped and the next PATH entry is tried. The answer is cached per PATH
//! value for the life of the process.
//!
//! This step is for source builds and test runs only. An installed app (its
//! `perception` payload beside the executable) never uses a Python from PATH:
//! Cut runs captions on the Python that Install captions provisions.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

fn candidate_name() -> &'static str {
    if cfg!(windows) {
        "python.exe"
    } else {
        "python3"
    }
}

/// Does `output` (stdout of the probe) name a Python 3 version, like `(3, 12)`?
fn is_python3_version(output: &str) -> bool {
    let text = output.trim();
    let Some(inner) = text.strip_prefix('(').and_then(|t| t.strip_suffix(')')) else {
        return false;
    };
    let mut parts = inner.split(',').map(str::trim);
    matches!(
        (parts.next().and_then(|m| m.parse::<u32>().ok()), parts.next().and_then(|m| m.parse::<u32>().ok())),
        (Some(major), Some(_)) if major >= 3
    )
}

/// Run the probe against one candidate. False for a command that cannot start,
/// exits non-zero (the Store shortcut exits 9009), answers anything else, or
/// does not answer within the timeout.
pub fn runs_as_python(candidate: &Path) -> bool {
    let Ok(mut child) = Command::new(candidate)
        .args(["-c", "import sys; print(sys.version_info[:2])"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return false;
                }
                let mut text = String::new();
                if let Some(mut out) = child.stdout.take() {
                    use std::io::Read;
                    let _ = out.read_to_string(&mut text);
                }
                return is_python3_version(&text);
            }
            Ok(None) if started.elapsed() < PROBE_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// On a Mac, `/usr/bin/python3` without the Command Line Tools is Apple's stub:
/// running it opens an installer dialog. Never start it as a probe.
fn is_apple_installer_stub(candidate: &Path) -> bool {
    cfg!(target_os = "macos")
        && candidate.starts_with("/usr/bin")
        && !Path::new("/Library/Developer/CommandLineTools/usr/bin/python3").exists()
}

/// First PATH entry whose `candidate_name` really runs as Python, tested with
/// `probe` (the real one in production).
pub(super) fn first_real_python(
    path_var: &OsStr,
    name: &str,
    probe: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    std::env::split_paths(path_var)
        // A relative entry would be searched against the current folder.
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        // symlink_metadata also sees app-execution aliases, which `is_file` hides.
        .filter(|path| path.symlink_metadata().is_ok())
        .filter(|path| !is_apple_installer_stub(path))
        .find(|path| probe(path))
}

static CACHE: Mutex<Vec<(OsString, Option<PathBuf>)>> = Mutex::new(Vec::new());

/// The Python in `path_var` that really runs, or None. Cached per PATH value.
/// Taking the PATH as a value (not reading the process environment) lets tests
/// choose a PATH without changing it for every other test.
pub(super) fn real_python_in(path_var: &OsStr) -> Option<PathBuf> {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, found)) = cache.iter().find(|(key, _)| key == path_var) {
        return found.clone();
    }
    let found = first_real_python(path_var, candidate_name(), &runs_as_python);
    cache.push((path_var.to_os_string(), found.clone()));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_python_three_version_answer_counts() {
        assert!(is_python3_version("(3, 13)\n"));
        assert!(is_python3_version("(3, 9)"));
        assert!(!is_python3_version("(2, 7)"));
        assert!(!is_python3_version(
            "Python was not found; run without arguments to install from the Microsoft Store"
        ));
        assert!(!is_python3_version(""));
    }

    #[test]
    fn a_candidate_that_is_not_python_is_skipped_for_the_next_path_entry() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("store");
        let real = dir.path().join("real");
        std::fs::create_dir_all(&fake).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        for d in [&fake, &real] {
            std::fs::write(d.join(candidate_name()), b"x").unwrap();
        }
        let path = std::env::join_paths([&fake, &real]).unwrap();
        let probe = |p: &Path| p.starts_with(&real);
        assert_eq!(
            first_real_python(&path, candidate_name(), &probe),
            Some(real.join(candidate_name()))
        );
        assert_eq!(first_real_python(&path, candidate_name(), &|_| false), None);
        let empty = std::env::join_paths([dir.path().join("nothing")]).unwrap();
        assert_eq!(first_real_python(&empty, candidate_name(), &|_| true), None);
    }

    /// Write a small command script: a batch file on Windows, a shell script elsewhere.
    fn script(dir: &Path, name: &str, unix_body: &str, windows_body: &str) -> PathBuf {
        #[cfg(windows)]
        {
            let p = dir.join(format!("{name}.cmd"));
            std::fs::write(&p, windows_body.replace('\n', "\r\n")).unwrap();
            let _ = unix_body;
            p
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = dir.join(format!("{name}.sh"));
            std::fs::write(&p, unix_body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            let _ = windows_body;
            p
        }
    }

    /// A command that behaves like the Windows Store shortcut (prints its hint,
    /// exits 9009) is rejected by the real probe; one that answers like Python
    /// is accepted; one that never answers is stopped at the timeout.
    #[test]
    fn the_real_probe_rejects_the_store_shortcut_and_accepts_python() {
        let dir = tempfile::tempdir().unwrap();
        let stub = script(
            dir.path(),
            "stub",
            "#!/bin/sh\necho 'Python was not found; run without arguments to install from the Microsoft Store'\nexit 9009\n",
            "@echo Python was not found; run without arguments to install from the Microsoft Store\n@exit /b 9009\n",
        );
        let real = script(
            dir.path(),
            "real",
            "#!/bin/sh\necho '(3, 12)'\n",
            "@echo (3, 12)\n",
        );
        assert!(!runs_as_python(&stub));
        assert!(runs_as_python(&real));
        assert!(!runs_as_python(&dir.path().join("missing")));
        #[cfg(not(windows))]
        {
            let slow = script(
                dir.path(),
                "slow",
                "#!/bin/sh\nsleep 30\necho '(3, 12)'\n",
                "",
            );
            let started = Instant::now();
            assert!(!runs_as_python(&slow));
            assert!(started.elapsed() < Duration::from_secs(15));
        }
    }
}
