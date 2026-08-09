//! Cross-platform project-name policy for newly created or renamed projects.

use super::*;

fn validate_plain_name(name: &str) -> Result<(), CutError> {
    let invalid = name.trim().is_empty()
        || name != name.trim()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        || Path::new(name).is_absolute();
    if invalid {
        return Err(invalid_name());
    }
    Ok(())
}

fn invalid_name() -> CutError {
    CutError::new(
        codes::INVALID_ARGS,
        "project name is not portable across Windows, macOS, and Linux",
        "the name is empty, path-like, reserved, or contains characters unsupported by a shipped desktop",
    )
    .with_suggested_action(
        "use 1 to 120 letters, numbers, spaces, '-' or '_' without trailing dots or spaces",
    )
}

fn windows_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|number| {
                matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
}

pub(super) fn validate_project_name(name: &str) -> Result<(), CutError> {
    validate_plain_name(name)?;
    let invalid = name.chars().count() > 120
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        || windows_device_name(name);
    if invalid {
        return Err(invalid_name());
    }
    Ok(())
}

/// Historical logs created before the portable policy must remain readable.
/// Replay retains the old path-safety boundary but does not retroactively
/// reject a display name that another platform once allowed.
pub(super) fn validate_logged_project_name(name: &str) -> Result<(), CutError> {
    validate_plain_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_names_accept_normal_and_unicode_labels() {
        for name in ["Documentary 2026", "cut_v2", "Rīga interview", "COM10"] {
            validate_project_name(name).unwrap();
        }
    }

    #[test]
    fn portable_names_reject_windows_devices_suffixes_and_characters() {
        for name in [
            "CON",
            "con.txt",
            "PRN",
            "AUX.mov",
            "NUL",
            "COM1",
            "com9.notes",
            "LPT1",
            "lpt9.txt",
            "trailing.",
            "trailing ",
            "bad:name",
            "bad*name",
            "bad?name",
            "bad|name",
            "bad<name",
            "bad>name",
            "bad\"name",
            "control\u{001f}",
        ] {
            assert!(
                validate_project_name(name).is_err(),
                "'{name}' must be refused on every platform"
            );
        }
    }

    #[test]
    fn historical_reserved_display_names_remain_replayable() {
        validate_logged_project_name("CON").unwrap();
        validate_logged_project_name("legacy:name").unwrap();
        assert!(validate_logged_project_name("../escape").is_err());
    }
}
