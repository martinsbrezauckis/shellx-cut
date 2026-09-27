// Safe facts for a failed GNOME shortcut write. The command contains the
// installed executable path, so never include any field's contents in logs.
pub(crate) fn readback_diagnostic(
    expected: &(String, String, String),
    observed: &(String, String, String),
) -> String {
    [
        ("name", &expected.0, &observed.0),
        ("binding", &expected.1, &observed.1),
        ("command", &expected.2, &observed.2),
    ]
    .into_iter()
    .map(|(field, wanted, actual)| {
        let bytes = |value: &str| value.len().min(4096);
        format!(
            "{field}:match={} expected_bytes={} observed_bytes={}",
            wanted == actual,
            bytes(wanted),
            bytes(actual)
        )
    })
    .collect::<Vec<_>>()
    .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_readback_fields_without_disclosing_values() {
        let expected = (
            "ShellX Cut: Toggle recording".to_string(),
            "F9".to_string(),
            "'/private/physical/executable' --record-hotkey-forwarder".to_string(),
        );
        let observed = ("".to_string(), "F8".to_string(), "".to_string());
        let message = readback_diagnostic(&expected, &observed);
        assert!(message.contains("name:match=false expected_bytes=28 observed_bytes=0"));
        assert!(message.contains("binding:match=false expected_bytes=2 observed_bytes=2"));
        assert!(message.contains("command:match=false expected_bytes=56 observed_bytes=0"));
        assert!(!message.contains("ShellX") && !message.contains("F8"));
        assert!(!message.contains("/private/") && !message.contains("--record-hotkey-forwarder"));
        assert!(readback_diagnostic(&expected, &expected).contains("command:match=true"));
    }

    #[test]
    fn reported_length_is_bounded() {
        let expected = ("x".repeat(4097), "F9".to_string(), "".to_string());
        let observed = ("".to_string(), "F9".to_string(), "".to_string());
        assert!(readback_diagnostic(&expected, &observed)
            .contains("name:match=false expected_bytes=4096 observed_bytes=0"));
    }
}
