//! macos_signature.rs — accept a downloaded macOS binary only when it is an
//! arm64 executable signed with the publisher's Apple Developer ID.
//!
//! The SHA-256 pin already fixes the bytes; this check proves who built them
//! independently of the download host. It asks Apple's `codesign` (by its
//! absolute path, never via PATH) to verify the signature strictly against a
//! designated requirement: an Apple-anchored Developer ID Application
//! certificate whose team is the pinned one. Ad-hoc, unsigned, Apple-platform
//! and other-team binaries all fail it. Gatekeeper never sees these files
//! (an app's own downloads carry no quarantine flag), so this is the check
//! that stands in for it.

/// The designated requirement for a Developer ID Application signature by
/// `team`: Apple anchor, Developer ID intermediate, Developer ID Application
/// leaf, and the leaf's organisational unit (the Team ID).
pub(crate) fn developer_id_requirement(team: &str) -> Result<String, String> {
    // A Team ID is ten upper-case letters or digits; anything else must never
    // reach the requirement language.
    if team.len() != 10
        || !team
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return Err(format!("'{team}' is not an Apple Team ID"));
    }
    Ok(format!(
        "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists \
         and certificate leaf[field.1.2.840.113635.100.6.1.13] exists \
         and certificate leaf[subject.OU] = \"{team}\""
    ))
}

/// Verify `binary`'s signature against [`developer_id_requirement`].
#[cfg(target_os = "macos")]
pub(crate) fn verify_developer_id(binary: &std::path::Path, team: &str) -> Result<(), String> {
    let requirement = developer_id_requirement(team)?;
    let mut command = std::process::Command::new("/usr/bin/codesign");
    command
        .arg("--verify")
        .arg("--strict")
        .arg(format!("-R={requirement}"))
        .arg(binary);
    let output = crate::dispatch::run_bounded_foreground_command(&mut command, "signature check")
        .map_err(|e| format!("Apple's codesign could not run: {}", e.message))?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(match output.status.code() {
        Some(3) => {
            format!("it is not signed by the publisher's Apple Developer ID (team {team}): {said}")
        }
        _ => format!("its signature is missing or broken: {said}"),
    })
}

/// Signature checks need Apple's tools; other systems never take this route.
#[cfg(not(target_os = "macos"))]
pub(crate) fn verify_developer_id(_binary: &std::path::Path, team: &str) -> Result<(), String> {
    developer_id_requirement(team)?;
    Err("Apple signature checks run only on macOS".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_requirement_pins_developer_id_and_team() {
        let r = developer_id_requirement("KU3N25YGLU").unwrap();
        assert!(r.starts_with("anchor apple generic"));
        assert!(
            r.contains("1.2.840.113635.100.6.2.6"),
            "Developer ID intermediate"
        );
        assert!(
            r.contains("1.2.840.113635.100.6.1.13"),
            "Developer ID Application leaf"
        );
        assert!(r.ends_with("certificate leaf[subject.OU] = \"KU3N25YGLU\""));
        for bad in [
            "",
            "ku3n25yglu",
            "KU3N25YGL",
            "KU3N25YGLU\" or true",
            "KU3N25YGL\"",
        ] {
            assert!(developer_id_requirement(bad).is_err(), "{bad}");
        }
    }
}

/// Real signatures on a Mac: the pinned publisher binary passes, an ad-hoc
/// copy, an unsigned copy, an Apple system binary and (when one is named) an
/// other-team binary fail. Set SHELLX_CUT_TEST_PUBLISHER_FFMPEG to the
/// downloaded ffmpeg; SHELLX_CUT_TEST_OTHER_TEAM_BINARY optionally names a
/// Developer ID binary of another team. Without the first variable the test
/// skips (CI has no download).
#[cfg(all(test, target_os = "macos"))]
mod real_signature_tests {
    use super::*;

    #[test]
    fn real_publisher_signature_passes_and_others_fail() {
        let Some(real) = std::env::var_os("SHELLX_CUT_TEST_PUBLISHER_FFMPEG") else {
            eprintln!("skipped: SHELLX_CUT_TEST_PUBLISHER_FFMPEG is not set");
            return;
        };
        let cut_media::tool_setup::ProgramCheck::MacArm64DeveloperId { team } =
            cut_media::tool_setup::MACOS_ARM64_FFMPEG.check
        else {
            panic!("the macOS release checks a Developer ID");
        };
        let real = std::path::PathBuf::from(real);
        verify_developer_id(&real, team).expect("the publisher's own binary passes");

        let dir = tempfile::tempdir().unwrap();
        let adhoc = dir.path().join("adhoc");
        std::fs::copy(&real, &adhoc).unwrap();
        let signed = std::process::Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(&adhoc)
            .status()
            .unwrap();
        assert!(signed.success());
        let error = verify_developer_id(&adhoc, team).unwrap_err();
        assert!(error.contains("not signed by the publisher"), "{error}");

        let unsigned = dir.path().join("unsigned");
        std::fs::copy(&real, &unsigned).unwrap();
        let stripped = std::process::Command::new("/usr/bin/codesign")
            .arg("--remove-signature")
            .arg(&unsigned)
            .status()
            .unwrap();
        assert!(stripped.success());
        let error = verify_developer_id(&unsigned, team).unwrap_err();
        assert!(error.contains("missing or broken"), "{error}");

        let error = verify_developer_id(std::path::Path::new("/bin/ls"), team).unwrap_err();
        assert!(error.contains("not signed by the publisher"), "{error}");

        if let Some(other) = std::env::var_os("SHELLX_CUT_TEST_OTHER_TEAM_BINARY") {
            let error = verify_developer_id(std::path::Path::new(&other), team).unwrap_err();
            assert!(error.contains("not signed by the publisher"), "{error}");
        }
    }
}
