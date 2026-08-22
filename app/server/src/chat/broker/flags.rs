//! Exact required-flag matching shared by local Agent Chat providers.

/// Return required help flags absent as complete tokens. A future lookalike
/// such as `--model-v2` must never satisfy a required `--model` capability.
pub(crate) fn missing_required_help_tokens<'a>(help: &str, required: &'a [&str]) -> Vec<&'a str> {
    required
        .iter()
        .copied()
        .filter(|token| !help_advertises_flag(help, token))
        .collect()
}

fn help_advertises_flag(help: &str, flag: &str) -> bool {
    help.match_indices(flag).any(|(offset, _)| {
        let before = offset
            .checked_sub(1)
            .and_then(|index| help.as_bytes().get(index).copied());
        let after = help.as_bytes().get(offset + flag.len()).copied();
        before.is_none_or(|byte| !is_help_flag_token_byte(byte))
            && after.is_none_or(|byte| !is_help_flag_token_byte(byte))
    })
}

fn is_help_flag_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}
