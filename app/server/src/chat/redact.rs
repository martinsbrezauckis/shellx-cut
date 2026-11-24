//! Secret and credential scrubbing for persisted and returned project chats.

use serde_json::Value;

const REDACTED: &str = "[REDACTED]";

const PREFIXES: &[(&str, usize)] = &[
    ("sk-ant-", 8),
    ("sk-proj-", 8),
    ("sk-", 12),
    ("xai-", 8),
    ("AIza", 12),
    ("ghp_", 12),
    ("gho_", 12),
    ("ghu_", 12),
    ("ghs_", 12),
    ("ghr_", 12),
];

const ENV_KEY_HINTS: &[&str] = &[
    "_API_KEY=",
    "_AUTH_TOKEN=",
    "_ACCESS_TOKEN=",
    "_SECRET=",
    "_TOKEN=",
    "API_KEY=",
];

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')
}

/// Replace common API keys, bearer tokens, and KEY=secret assignments with `[REDACTED]`.
pub(crate) fn redact_secrets(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if !input.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let rest = &input[i..];

        if let Some(len) = match_bearer(rest) {
            out.push_str("Bearer ");
            out.push_str(REDACTED);
            i += len;
            continue;
        }
        if let Some((prefix_len, total_len)) =
            match_env_assignment(rest, i > 0 && is_token_byte(bytes[i - 1]))
        {
            out.push_str(&rest[..prefix_len]);
            out.push_str(REDACTED);
            i += total_len;
            continue;
        }
        if let Some(total_len) = match_prefixed_secret(rest, i > 0 && is_token_byte(bytes[i - 1])) {
            out.push_str(REDACTED);
            i += total_len;
            continue;
        }

        let ch = rest.chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn match_prefixed_secret(rest: &str, preceded_by_token: bool) -> Option<usize> {
    if preceded_by_token {
        return None;
    }
    for (prefix, min_tail) in PREFIXES {
        if let Some(after) = rest.strip_prefix(prefix) {
            let tail_len = after.bytes().take_while(|&b| is_token_byte(b)).count();
            if tail_len >= *min_tail {
                return Some(prefix.len() + tail_len);
            }
        }
    }
    None
}

fn match_bearer(rest: &str) -> Option<usize> {
    if rest.len() < 8
        || !rest
            .as_bytes()
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"bearer "))
    {
        return None;
    }
    let after = &rest[7..];
    let spaces = after.bytes().take_while(|&b| b == b' ').count();
    let token_start = 7 + spaces;
    let tok_len = rest[token_start..]
        .bytes()
        .take_while(|&b| is_token_byte(b))
        .count();
    (tok_len >= 8).then_some(token_start + tok_len)
}

fn match_env_assignment(rest: &str, preceded_by_token: bool) -> Option<(usize, usize)> {
    if preceded_by_token {
        return None;
    }
    let name_len = rest
        .bytes()
        .take_while(|&b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        .count();
    if name_len < 4 || rest.as_bytes().get(name_len) != Some(&b'=') {
        return None;
    }
    let name_and_eq = &rest[..=name_len];
    if !ENV_KEY_HINTS.iter().any(|hint| name_and_eq.ends_with(hint)) {
        return None;
    }
    let val_start = name_len + 1;
    let val_len = rest[val_start..]
        .bytes()
        .take_while(|&b| !b.is_ascii_whitespace() && !matches!(b, b'"' | b'\'' | b',' | b';'))
        .count();
    (val_len >= 6).then_some((val_start, val_start + val_len))
}

/// Recursively scrub string values inside an arbitrary JSON value.
pub(crate) fn redact_value(val: &mut Value) {
    match val {
        Value::String(s) => {
            let clean = redact_secrets(s);
            if clean != *s {
                *s = clean;
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_value(item);
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                redact_value(v);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_common_provider_keys_and_tokens() {
        let input = "Use sk-ant-api03-SECRET12345 and xai-SECRET12345 plus Bearer tok_secret_987 and OPENAI_API_KEY=sk-proj-SECRET987654";
        let scrubbed = redact_secrets(input);
        for raw in [
            "sk-ant-api03",
            "SECRET12345",
            "tok_secret_987",
            "SECRET987654",
        ] {
            assert!(!scrubbed.contains(raw), "leaked {raw} in {scrubbed}");
        }
        assert!(scrubbed.contains("[REDACTED]"));
        assert!(scrubbed.contains("OPENAI_API_KEY=[REDACTED]"));
    }

    #[test]
    fn redacts_without_panicking_on_multibyte_utf8_across_prefix_boundaries() {
        for pad in ["", "a", "aa", "aaa", "Bearer ", "b"] {
            let input = format!("{pad}😀😀😀 sk-ant-api03-SECRET12345 😀😀");
            let scrubbed = redact_secrets(&input);
            assert!(!scrubbed.contains("SECRET12345"), "{scrubbed}");
            assert!(scrubbed.contains("😀"), "{scrubbed}");
        }
    }
}
