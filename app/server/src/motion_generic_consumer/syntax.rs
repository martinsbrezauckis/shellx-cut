use super::canonical::canonical_json;
use super::ConsumerError;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub(super) fn named_version(value: &Value, expected: &str) -> Result<String, ConsumerError> {
    let value = required_object(value, &["name", "version"], "runtime version")?;
    required_exact_string(value, "name", expected, "runtime version")?;
    let version = required_string_from(value, "version", "runtime version", 64)?;
    if !version.as_bytes().first().is_some_and(u8::is_ascii_digit)
        || !version
            .bytes()
            .all(|item| item.is_ascii_alphanumeric() || matches!(item, b'.' | b'-' | b'+'))
    {
        return Err(ConsumerError::refusal(
            "Motion runtime version is not semver-like",
        ));
    }
    Ok(version)
}

pub(super) fn require_protocol(
    value: &Value,
    selected: u64,
    label: &str,
) -> Result<(), ConsumerError> {
    let value = required_object(value, &["min", "max", "preferred"], label)?;
    let min = integer(value.get("min").unwrap_or(&Value::Null), label, 1, 16)? as u64;
    let max = integer(
        value.get("max").unwrap_or(&Value::Null),
        label,
        min as i64,
        16,
    )? as u64;
    let preferred = integer(
        value.get("preferred").unwrap_or(&Value::Null),
        label,
        min as i64,
        max as i64,
    )? as u64;
    if min > selected || max < selected || preferred != selected {
        Err(ConsumerError::refusal(format!(
            "{label} does not negotiate protocol {selected}"
        )))
    } else {
        Ok(())
    }
}

pub(super) fn require_exact_protocol(
    value: &Value,
    version: u64,
    label: &str,
) -> Result<(), ConsumerError> {
    let value = required_object(value, &["min", "max", "preferred"], label)?;
    if integer(
        value.get("min").unwrap_or(&Value::Null),
        label,
        version as i64,
        version as i64,
    )? != version as i64
        || integer(
            value.get("max").unwrap_or(&Value::Null),
            label,
            version as i64,
            version as i64,
        )? != version as i64
        || integer(
            value.get("preferred").unwrap_or(&Value::Null),
            label,
            version as i64,
            version as i64,
        )? != version as i64
    {
        Err(ConsumerError::refusal(format!(
            "{label} is not exactly protocol {version}"
        )))
    } else {
        Ok(())
    }
}

pub(super) fn required_object<'a>(
    value: &'a Value,
    fields: &[&str],
    label: &str,
) -> Result<&'a Map<String, Value>, ConsumerError> {
    let object = value
        .as_object()
        .ok_or_else(|| ConsumerError::refusal(format!("{label} must be an object")))?;
    if object.len() != fields.len()
        || fields.iter().any(|field| !object.contains_key(*field))
        || object.keys().any(|key| !fields.contains(&key.as_str()))
    {
        Err(ConsumerError::refusal(format!(
            "{label} has unknown or missing fields"
        )))
    } else {
        Ok(object)
    }
}

pub(super) fn required_string_from(
    value: &Map<String, Value>,
    field: &str,
    label: &str,
    maximum: usize,
) -> Result<String, ConsumerError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|item| !item.is_empty() && item.len() <= maximum)
        .map(str::to_owned)
        .ok_or_else(|| {
            ConsumerError::refusal(format!(
                "{label} {field} must be a bounded non-empty string"
            ))
        })
}

pub(super) fn required_exact_string(
    value: &Map<String, Value>,
    field: &str,
    expected: &str,
    label: &str,
) -> Result<(), ConsumerError> {
    if required_string_from(value, field, label, expected.len().max(1))? == expected {
        Ok(())
    } else {
        Err(ConsumerError::refusal(format!(
            "{label} {field} is unsupported"
        )))
    }
}

pub(super) fn integer(
    value: &Value,
    label: &str,
    minimum: i64,
    maximum: i64,
) -> Result<i64, ConsumerError> {
    value
        .as_i64()
        .filter(|item| *item >= minimum && *item <= maximum)
        .ok_or_else(|| ConsumerError::refusal(format!("{label} is outside its integer bounds")))
}

pub(super) fn string_array(
    value: &Value,
    label: &str,
    minimum: usize,
    maximum: usize,
    item_maximum: usize,
) -> Result<Vec<String>, ConsumerError> {
    let values = value
        .as_array()
        .ok_or_else(|| ConsumerError::refusal(format!("{label} must be an array")))?;
    if values.len() < minimum || values.len() > maximum {
        return Err(ConsumerError::refusal(format!(
            "{label} item count is outside bounds"
        )));
    }
    let values = values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|item| !item.is_empty() && item.len() <= item_maximum)
                .map(str::to_owned)
                .ok_or_else(|| {
                    ConsumerError::refusal(format!("{label} contains an invalid string"))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    require_sorted(values.clone(), label)?;
    Ok(values)
}

pub(super) fn require_sorted(values: Vec<String>, label: &str) -> Result<(), ConsumerError> {
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        Err(ConsumerError::refusal(format!(
            "{label} must be strictly code-unit sorted"
        )))
    } else {
        Ok(())
    }
}

pub(super) fn checked_fingerprint(
    value: &Map<String, Value>,
    label: &str,
) -> Result<String, ConsumerError> {
    let supplied = sha256_string(&required_string_from(value, "fingerprint", label, 64)?)?;
    let mut content = value.clone();
    content.remove("fingerprint");
    if supplied
        != hex::encode(Sha256::digest(
            canonical_json(&Value::Object(content)).as_bytes(),
        ))
    {
        Err(ConsumerError::refusal(format!(
            "{label} fingerprint does not bind its canonical content"
        )))
    } else {
        Ok(supplied)
    }
}

pub(super) fn sha256_string(value: &str) -> Result<String, ConsumerError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|item| item.is_ascii_hexdigit() && !item.is_ascii_uppercase())
    {
        Ok(value.to_owned())
    } else {
        Err(ConsumerError::refusal(
            "fingerprint is not lowercase SHA-256",
        ))
    }
}

pub(super) fn identifier(value: &str, label: &str) -> Result<String, ConsumerError> {
    if value.len() <= 128
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value.bytes().all(|item| {
            item.is_ascii_lowercase()
                || item.is_ascii_digit()
                || matches!(item, b'.' | b'_' | b':' | b'-')
        })
    {
        Ok(value.to_owned())
    } else {
        Err(ConsumerError::refusal(format!(
            "{label} is not a bounded lowercase identifier"
        )))
    }
}

pub(super) fn capability_id(value: &str) -> Result<String, ConsumerError> {
    let Some((prefix, version)) = value.rsplit_once('@') else {
        return Err(ConsumerError::refusal(
            "capability descriptor id is not versioned",
        ));
    };
    if prefix.is_empty()
        || prefix.len() > 120
        || version.is_empty()
        || version.len() > 8
        || !version.bytes().all(|item| item.is_ascii_digit())
        || !prefix
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_lowercase)
        || !prefix.bytes().all(|item| {
            item.is_ascii_lowercase()
                || item.is_ascii_digit()
                || matches!(item, b'.' | b'_' | b':' | b'-')
        })
    {
        Err(ConsumerError::refusal(
            "capability descriptor id is not versioned",
        ))
    } else {
        Ok(value.to_owned())
    }
}

pub(super) fn schema_id(value: &str) -> Result<String, ConsumerError> {
    let Some((namespace, name_and_version)) = value.split_once('/') else {
        return Err(ConsumerError::refusal(
            "connector request schema id is unsafe",
        ));
    };
    let Some((name, version)) = name_and_version.rsplit_once('@') else {
        return Err(ConsumerError::refusal(
            "connector request schema id is unsafe",
        ));
    };
    if namespace.is_empty()
        || namespace.len() > 64
        || name.is_empty()
        || name.len() > 96
        || version.is_empty()
        || version.len() > 8
        || !namespace
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_lowercase)
        || !namespace.bytes().all(|item| {
            item.is_ascii_lowercase() || item.is_ascii_digit() || matches!(item, b'.' | b'-')
        })
        || !name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        || !name.bytes().all(|item| {
            item.is_ascii_lowercase()
                || item.is_ascii_digit()
                || matches!(item, b'.' | b'_' | b'/' | b'-')
        })
        || !version.bytes().all(|item| item.is_ascii_digit())
    {
        Err(ConsumerError::refusal(
            "connector request schema id is unsafe",
        ))
    } else {
        Ok(value.to_owned())
    }
}

pub(super) fn opaque_reference(value: &str) -> bool {
    value.len() <= 128
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value.bytes().all(|item| {
            item.is_ascii_lowercase() || item.is_ascii_digit() || matches!(item, b'.' | b'_' | b'-')
        })
}

pub(super) fn unsafe_field_id(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [
        "path",
        "url",
        "uri",
        "argv",
        "command",
        "executable",
        "callback",
        "provider",
        "environment",
        "module",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}
