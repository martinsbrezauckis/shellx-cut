//! Shell/engine version identity for coherent desktop installs.
//!
//! The Windows registry and shell executable can both report a new version
//! while a locked `cutd.exe` remains old. The shell therefore reads the live
//! engine's own `/api/agent` identity before navigating to its UI and refuses a
//! mismatch. Artifact hashes remain a packaging responsibility; this module
//! owns the runtime version invariant and its actionable repair message.

fn response_body(raw: &str) -> Result<&str, String> {
    let status = raw
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "engine identity response had no HTTP status".to_string())?;
    if status != 200 {
        return Err(format!("engine identity endpoint returned HTTP {status}"));
    }
    raw.split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .ok_or_else(|| "engine identity response had no body".to_string())
}

/// The product name the engine reports; anything else answering at the
/// engine address is another program and is never adopted.
const PRODUCT_NAME: &str = "ShellX Cut";

fn identity_json(raw: &str) -> Result<serde_json::Value, String> {
    let value: serde_json::Value = serde_json::from_str(response_body(raw)?)
        .map_err(|e| format!("engine identity response was not valid JSON: {e}"))?;
    if value.get("product").and_then(serde_json::Value::as_str) != Some(PRODUCT_NAME) {
        return Err(format!(
            "the program answering at the engine address does not identify itself as {PRODUCT_NAME}, so it was not used. Close that program or open ShellX Cut again so its engine takes another port"
        ));
    }
    Ok(value)
}

/// The instance value the engine reports on `/api/agent`, if it reports one
/// (an engine from before the instance existed does not).
pub(crate) fn engine_instance_from_http(raw: &str) -> Option<String> {
    identity_json(raw)
        .ok()?
        .get("instance")
        .and_then(serde_json::Value::as_str)
        .filter(|instance| !instance.is_empty())
        .map(str::to_owned)
}

/// The instance the engine wrote into `engine.addr` must be the one that answers
/// at that address. When the file records an instance the peer must report
/// exactly it; only a file without an instance line skips the comparison.
pub(crate) fn require_same_instance(
    recorded: Option<&str>,
    reported: Option<&str>,
) -> Result<(), String> {
    match (recorded, reported) {
        // engine.addr has no instance line (written by an older engine): there
        // is nothing to compare, and only product and version were checked.
        (None, _) => Ok(()),
        (Some(recorded), Some(reported)) if recorded == reported => Ok(()),
        // The file names an instance: the peer must report exactly that value.
        // A peer that reports none (another program, or an older engine) fails.
        (Some(_), _) => Err(
            "engine.addr was written by a different ShellX Cut engine than the one answering at that address (an earlier engine instance left the file behind, or the program there does not report an instance). Close every ShellX Cut window, end any process named cutd, then open ShellX Cut again; if it still fails, delete the stale engine.addr file"
                .to_string(),
        ),
    }
}

pub(crate) fn engine_version_from_http(raw: &str) -> Result<String, String> {
    let value = identity_json(raw)?;
    value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .filter(|version| !version.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "engine identity response did not contain a version".to_string())
}

pub(crate) fn require_coherent_versions(
    shell_version: &str,
    engine_version: &str,
) -> Result<(), String> {
    if shell_version == engine_version {
        return Ok(());
    }
    Err(format!(
        "engine version mismatch: the desktop shell is v{shell_version}, but cutd is v{engine_version}. Close every ShellX Cut window, then reinstall ShellX Cut v{shell_version} from the official release before editing. If no ShellX Cut window is open, an engine left by an earlier session is still running: end the process named cutd, then open ShellX Cut again"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_identity_rejects_the_mixed_install_seen_in_the_field() {
        let raw = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"product\":\"ShellX Cut\",\"version\":\"0.6.105\"}";
        let engine = engine_version_from_http(raw).expect("version");
        assert_eq!(engine, "0.6.105");
        let error = require_coherent_versions("0.6.106", &engine).unwrap_err();
        assert!(error.contains("desktop shell is v0.6.106"));
        assert!(error.contains("cutd is v0.6.105"));
        // The same mismatch is what an engine left behind by an earlier session
        // looks like, so the message says how to end it.
        assert!(error.contains("end the process named cutd"));
        assert!(require_coherent_versions("0.6.106", "0.6.106").is_ok());
    }

    #[test]
    fn another_program_is_never_taken_for_the_engine() {
        let other = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"product\":\"Some Other Tool\",\"version\":\"0.6.106\"}";
        let error = engine_version_from_http(other).unwrap_err();
        assert!(error.contains("does not identify itself as ShellX Cut"));
        let none = "HTTP/1.1 200 OK\r\n\r\n{\"version\":\"0.6.106\"}";
        assert!(engine_version_from_http(none).is_err());
        assert_eq!(engine_instance_from_http(other), None);
    }

    #[test]
    fn a_recorded_instance_must_be_reported_exactly() {
        let raw = "HTTP/1.1 200 OK\r\n\r\n{\"product\":\"ShellX Cut\",\"version\":\"0.6.106\",\"instance\":\"abc\"}";
        assert_eq!(engine_instance_from_http(raw).as_deref(), Some("abc"));
        assert!(require_same_instance(Some("abc"), Some("abc")).is_ok());
        assert!(require_same_instance(None, Some("abc")).is_ok());
        assert!(require_same_instance(None, None).is_ok());
        // The file records an instance: a peer that omits it is refused.
        assert!(require_same_instance(Some("abc"), None).is_err());
        let error = require_same_instance(Some("abc"), Some("zzz")).unwrap_err();
        assert!(error.contains("delete the stale engine.addr"));
    }

    #[test]
    fn runtime_identity_fails_closed_on_missing_or_failed_identity() {
        assert!(engine_version_from_http("HTTP/1.1 404 Not Found\r\n\r\n{}").is_err());
        assert!(engine_version_from_http("HTTP/1.1 200 OK\r\n\r\n{}").is_err());
    }
}
