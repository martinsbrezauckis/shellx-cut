//! Decode imported media references without touching the filesystem.

use super::*;

fn invalid(url: &str, message: &str) -> CutError {
    CutError::new(error_codes::INVALID_ARGS, message, url.to_string())
}

pub(super) fn media_path(url: &str) -> Result<PathBuf, CutError> {
    let bytes = url.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err(invalid(
                url,
                "OTIO media URI has an incomplete percent escape",
            ));
        }
        let high = (bytes[index + 1] as char).to_digit(16);
        let low = (bytes[index + 2] as char).to_digit(16);
        let (Some(high), Some(low)) = (high, low) else {
            return Err(invalid(url, "OTIO media URI has an invalid percent escape"));
        };
        decoded.push((high * 16 + low) as u8);
        index += 3;
    }
    let decoded = String::from_utf8(decoded)
        .map_err(|_| invalid(url, "OTIO media URI is not valid UTF-8"))?;
    if decoded.contains('\0') {
        return Err(invalid(url, "OTIO media URI contains NUL"));
    }
    let mut local = if decoded
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("file://"))
    {
        let rest = &decoded[7..];
        if rest
            .get(..10)
            .is_some_and(|s| s.eq_ignore_ascii_case("localhost/"))
        {
            format!("/{}", &rest[10..])
        } else if rest.starts_with('/') {
            rest.to_string()
        } else {
            return Err(invalid(url, "OTIO media URI has a remote file authority"));
        }
    } else {
        decoded
    };

    // Check both separator forms on every host: a timeline previewed on Unix
    // must not become a network or device reference when opened on Windows.
    let slash = local.replace('\\', "/");
    let verbatim = slash
        .strip_prefix("///?/")
        .or_else(|| slash.strip_prefix("//?/"));
    if let Some(rest) = verbatim {
        // The only admitted device-namespace spelling is a rooted local drive.
        // UNC (including mixed case), GLOBALROOT and Volume aliases stay invalid.
        if rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && rest.as_bytes().get(1) == Some(&b':')
            && rest.as_bytes().get(2) == Some(&b'/')
        {
            let prefix_len = slash.len() - rest.len();
            local = local[prefix_len..].to_string();
        } else {
            return Err(invalid(
                url,
                "OTIO media reference uses a network or device path",
            ));
        }
    } else if slash.starts_with("//") || slash.starts_with("/??/") {
        return Err(invalid(
            url,
            "OTIO media reference uses a network or device path",
        ));
    }

    // A decoded scheme must never be reinterpreted as a missing local file.
    // A one-letter drive prefix remains a supported Windows path.
    if local.contains("://") {
        return Err(invalid(
            url,
            "OTIO media reference uses an unsupported URI scheme",
        ));
    }
    if let Some((scheme, _)) = local.split_once(':') {
        if !(scheme.len() == 1 && scheme.as_bytes()[0].is_ascii_alphabetic())
            && scheme
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
            && scheme
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
        {
            return Err(invalid(
                url,
                "OTIO media reference uses an unsupported URI scheme",
            ));
        }
    }
    for component in local.split(['/', '\\']) {
        let name = component
            .trim_end_matches([' ', '.'])
            .split(['.', ':'])
            .next()
            .unwrap_or("")
            .trim_end_matches(' ');
        let name = name.to_ascii_uppercase();
        let numbered = name
            .strip_prefix("COM")
            .or_else(|| name.strip_prefix("LPT"));
        if matches!(
            name.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
        ) || numbered.is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        }) {
            return Err(invalid(
                url,
                "OTIO media reference uses a Windows device alias",
            ));
        }
    }
    #[cfg(windows)]
    if local.starts_with('/')
        && local.as_bytes().get(2) == Some(&b':')
        && local.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic)
    {
        local.remove(0);
    }
    Ok(PathBuf::from(local))
}
