//! Bounded diagnostics and provider-pinning checks for judge adapter results.

use serde_json::Value;

fn stream_tail(bytes: &[u8], limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    let start = (count > limit)
        .then(|| trimmed.char_indices().nth(count - limit))
        .flatten()
        .map_or(0, |(index, _)| index);
    trimmed[start..].to_string()
}

/// Return every non-empty adapter stream, labelled and bounded per stream.
///
/// Some CLIs return their structured error envelope on stdout.  A nonzero
/// process exit therefore must not be reduced to stderr alone.
pub(super) fn process_diagnostics(stdout: &[u8], stderr: &[u8], limit: usize) -> String {
    let mut parts = Vec::new();
    for (name, bytes) in [("stdout", stdout), ("stderr", stderr)] {
        let detail = stream_tail(bytes, limit);
        if !detail.is_empty() {
            parts.push(format!("{name}: {detail}"));
        }
    }
    if parts.is_empty() {
        "(no output on stdout or stderr)".into()
    } else {
        parts.join(" | ")
    }
}

/// An explicitly requested rung must report that same provider.  `auto` is
/// intentionally unpinned because its documented contract is to step down.
pub(super) fn validate_requested_provider(envelope: &Value, requested: &str) -> Result<(), String> {
    if requested == "auto" {
        return Ok(());
    }
    let reported = envelope
        .pointer("/backend/provider")
        .and_then(Value::as_str);
    if reported != Some(requested) {
        return Err(format!(
            "judge adapter was forced to provider {requested:?} but reported {reported:?}; refusing a substituted provider"
        ));
    }
    if let Some(selected) = envelope.pointer("/ladder/selected").and_then(Value::as_str) {
        if selected != requested {
            return Err(format!(
                "judge ladder was forced to provider {requested:?} but selected {selected:?}; refusing a substituted rung"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{process_diagnostics, validate_requested_provider};
    use serde_json::json;

    #[test]
    fn diagnostic_keeps_stdout_envelope_and_stderr() {
        let detail = process_diagnostics(
            br#"{"is_error":true,"result":"OAuth expired"}"#,
            b"launcher warning",
            200,
        );
        assert!(detail.contains("stdout: {\"is_error\":true"), "{detail}");
        assert!(detail.contains("stderr: launcher warning"), "{detail}");
    }

    #[test]
    fn explicit_provider_rejects_substituted_ladder_rung() {
        let envelope = json!({
            "backend": {"provider": "grok"},
            "ladder": {"selected": "grok"},
        });
        let err = validate_requested_provider(&envelope, "claude").unwrap_err();
        assert!(err.contains("forced to provider"), "{err}");
        assert!(validate_requested_provider(&envelope, "auto").is_ok());
    }
}
