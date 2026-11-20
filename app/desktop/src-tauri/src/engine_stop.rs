//! Stopping the engine this shell started, and leaving no stale address behind.
//!
//! The engine records where it listens in `engine.addr` and removes the file when
//! it shuts down by itself. A hard kill gives it no chance to, so a normal close
//! used to leave a file that named a dead address. This module asks the engine to
//! stop first (a termination signal where the system has one), waits a short
//! while, kills it only if it does not stop, and then removes the discovery file
//! if it still names this engine.

use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::OnceLock;
use std::time::Duration;

/// How long a polite stop gets before the engine is killed.
const GRACE: Duration = Duration::from_secs(3);

/// The address the engine this shell started listens on, kept so its discovery
/// file can be recognised as ours later. An engine the shell merely adopted is
/// never recorded and never touched.
static OWNED_ADDR: OnceLock<String> = OnceLock::new();

pub(crate) fn remember_owned_addr(addr: &str) {
    let _ = OWNED_ADDR.set(addr.to_string());
}

/// Where the engine writes its address: the isolated state directory when one is
/// configured (and valid), else the app-data root, else the temp directory. This
/// mirrors the engine's own rule; the file name and root must stay in step with
/// `httpc::discovery_path_for` in the server crate.
fn discovery_path_for(
    cut_home: Option<&std::ffi::OsStr>,
    appdata_root: Option<PathBuf>,
    temp_root: &Path,
) -> Option<PathBuf> {
    if let Some(home) = cut_home.filter(|home| !home.is_empty()) {
        let root = PathBuf::from(home);
        if !root.is_absolute()
            || !root.is_dir()
            || root
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return None;
        }
        return Some(root.join("engine.addr"));
    }
    Some(
        appdata_root
            .unwrap_or_else(|| temp_root.to_path_buf())
            .join("engine.addr"),
    )
}

fn discovery_path() -> Option<PathBuf> {
    discovery_path_for(
        std::env::var_os("SHELLX_CUT_HOME").as_deref(),
        crate::tools::appdata_tools_dir().and_then(|tools| tools.parent().map(Path::to_path_buf)),
        &std::env::temp_dir(),
    )
}

/// The address (first line) and the engine instance (`instance=<id>` line, when
/// the engine wrote one) of an `engine.addr` file.
fn parse_record(text: &str) -> Option<(String, Option<String>)> {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let addr = lines.next()?.to_string();
    let instance = lines
        .find_map(|line| line.strip_prefix("instance="))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Some((addr, instance))
}

/// The address `engine.addr` names, when the file exists and is readable.
pub(crate) fn recorded_addr() -> Option<String> {
    let text = std::fs::read_to_string(discovery_path()?).ok()?;
    parse_record(&text).map(|(addr, _)| addr)
}

/// The instance `engine.addr` records for `addr`, when the file names exactly
/// that address. The adopt path compares it with what the engine reports.
pub(crate) fn recorded_instance_for(addr: &str) -> Option<String> {
    let text = std::fs::read_to_string(discovery_path()?).ok()?;
    let (recorded_addr, instance) = parse_record(&text)?;
    (recorded_addr == addr).then_some(instance).flatten()
}

/// Remove `path` only when it still holds `addr`: a file that names another
/// address belongs to an engine this shell did not start.
fn remove_discovery_if_ours(path: &Path, addr: &str) -> bool {
    match std::fs::read_to_string(path) {
        Ok(text) if parse_record(&text).is_some_and(|(recorded, _)| recorded == addr) => {
            std::fs::remove_file(path).is_ok()
        }
        _ => false,
    }
}

/// Ask `child` to stop and wait up to `grace`; kill it if it is still running.
/// Always reaps the child. Returns true when it stopped without being killed.
#[cfg_attr(not(unix), allow(unused_variables))]
fn stop_child(child: &mut Child, grace: Duration) -> bool {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return true;
    }
    // Windows has no termination signal a windowless console child can receive,
    // so there the engine is killed at once and the caller tidies the file.
    #[cfg(unix)]
    {
        use std::time::Instant;
        // SAFETY: plain signal delivery to the pid of a child this process owns.
        let sent = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) } == 0;
        if sent {
            let deadline = Instant::now() + grace;
            while Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    false
}

/// Remove the discovery file when it still names the engine this shell started.
pub(crate) fn tidy_discovery() {
    if let (Some(path), Some(addr)) = (discovery_path(), OWNED_ADDR.get()) {
        remove_discovery_if_ours(&path, addr);
    }
}

/// Stop the owned engine and remove its discovery file when it still names it.
pub(crate) fn stop_and_tidy(child: &mut Child) {
    stop_child(child, GRACE);
    tidy_discovery();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::process::{Command, Stdio};
    #[cfg(unix)]
    use std::time::Instant;

    #[test]
    fn discovery_file_follows_the_engines_rule() {
        let root = std::env::current_dir().unwrap();
        let appdata = root.join("appdata-root");
        let temp = root.join("temp-root");
        assert_eq!(
            discovery_path_for(Some(root.as_os_str()), Some(appdata.clone()), &temp),
            Some(root.join("engine.addr"))
        );
        assert_eq!(
            discovery_path_for(None, Some(appdata.clone()), &temp),
            Some(appdata.join("engine.addr"))
        );
        assert_eq!(
            discovery_path_for(None, None, &temp),
            Some(temp.join("engine.addr"))
        );
        let relative = std::ffi::OsString::from("relative-home");
        assert_eq!(
            discovery_path_for(Some(relative.as_os_str()), Some(appdata), &temp),
            None
        );
    }

    #[test]
    fn only_a_file_that_still_names_our_engine_is_removed() {
        let dir = std::env::temp_dir().join(format!("shellx-engine-stop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("engine.addr");
        std::fs::write(&path, "127.0.0.1:6161\n").unwrap();
        assert!(!remove_discovery_if_ours(&path, "127.0.0.1:7000"));
        assert!(path.exists(), "another engine's address must be left alone");
        assert!(remove_discovery_if_ours(&path, "127.0.0.1:6161"));
        assert!(!path.exists());
        assert!(!remove_discovery_if_ours(&path, "127.0.0.1:6161"));
        // The file the engine writes now has an instance line after the address.
        std::fs::write(&path, "127.0.0.1:6161\ninstance=abc123\n").unwrap();
        assert!(!remove_discovery_if_ours(&path, "127.0.0.1:7000"));
        assert!(remove_discovery_if_ours(&path, "127.0.0.1:6161"));
        assert!(!path.exists());
        let _ = std::fs::remove_dir(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_responsive_engine_is_asked_to_stop_not_killed() {
        use std::os::unix::process::ExitStatusExt;
        let mut child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        assert!(stop_child(&mut child, Duration::from_secs(5)));
        assert!(started.elapsed() < Duration::from_secs(4));
        // Already reaped: the status is the termination signal, not a kill.
        assert_eq!(
            child.try_wait().unwrap().and_then(|s| s.signal()),
            Some(libc::SIGTERM)
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_engine_that_ignores_the_request_is_killed_after_the_grace() {
        use std::io::Read;
        let mut child = Command::new("sh")
            .args(["-c", "trap '' TERM; echo ready; while :; do sleep 1; done"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        // Wait until the trap is installed, or the TERM would just kill the shell.
        let mut ready = [0u8; 6];
        child
            .stdout
            .as_mut()
            .unwrap()
            .read_exact(&mut ready)
            .unwrap();
        let started = Instant::now();
        assert!(!stop_child(&mut child, Duration::from_millis(300)));
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(child.try_wait().unwrap().is_some(), "child must be reaped");
    }
}
