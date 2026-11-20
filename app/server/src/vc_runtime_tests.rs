use super::*;

#[test]
fn downloads_only_from_microsofts_documented_permalinks() {
    assert_eq!(
        download_url("x86_64"),
        Some("https://aka.ms/vc14/vc_redist.x64.exe")
    );
    assert_eq!(
        download_url("aarch64"),
        Some("https://aka.ms/vc14/vc_redist.arm64.exe")
    );
    assert_eq!(download_url("x86"), None);
    assert!(OFFICIAL_PAGE.starts_with("https://learn.microsoft.com/"));
    assert_eq!(PUBLISHER, "Microsoft Corporation");
}

#[test]
fn installer_exit_codes_are_told_apart() {
    assert_eq!(
        classify_exit(0),
        InstallerExit::Installed {
            restart_needed: false
        }
    );
    assert_eq!(
        classify_exit(3010),
        InstallerExit::Installed {
            restart_needed: true
        }
    );
    assert_eq!(
        classify_exit(1641),
        InstallerExit::Installed {
            restart_needed: true
        }
    );
    assert_eq!(classify_exit(1638), InstallerExit::AlreadyPresent);
    for declined in [1602, 1223, 0x8007_0642, 0x8007_04C7] {
        assert_eq!(
            classify_exit(declined),
            InstallerExit::Declined,
            "{declined}"
        );
    }
    assert_eq!(classify_exit(5100), InstallerExit::Failed(5100));
}

#[test]
fn every_outcome_is_told_with_a_way_forward() {
    let declined = install_error(InstallerExit::Declined, true);
    assert!(
        declined
            .message
            .contains("administrator prompt was declined"),
        "{}",
        declined.message
    );
    assert!(declined
        .suggested_action
        .unwrap()
        .contains("approve the Windows prompt"));

    let standard = install_error(InstallerExit::Declined, false);
    assert_eq!(
        standard.message,
        "This Windows account cannot approve installing the Microsoft Visual C++ runtime"
    );
    let action = standard.suggested_action.unwrap();
    assert!(action
        .starts_with("Ask an administrator to install the Microsoft Visual C++ Redistributable"));
    assert!(action.contains(OFFICIAL_PAGE));

    let failed = install_error(InstallerExit::Failed(5100), true);
    assert!(failed.message.contains("code 5100"));
    assert!(failed.suggested_action.unwrap().contains(OFFICIAL_PAGE));

    let download = download_error(CutError::new("io", "offline", "no route"));
    assert!(download
        .message
        .contains("could not be downloaded from Microsoft"));
    assert!(download.suggested_action.unwrap().contains(OFFICIAL_PAGE));

    let signature = signature_error("the signer is 'Someone', not Microsoft Corporation");
    assert!(signature
        .message
        .contains("did not pass the Windows signature check, so it was not run"));
    assert!(signature.cause.contains("Someone"));
}

#[test]
fn a_feature_without_it_refuses_and_points_at_the_entry() {
    let error = missing_error("Captions and transcription");
    assert_eq!(error.code, "tool_missing");
    // The registry's requirement (schema/requirements.json `vc_runtime`).
    assert_eq!(error.requirement.as_ref().unwrap().id, "vc_runtime");
    let completed = crate::requirements::complete(error.clone());
    let named = completed.requirement.as_ref().unwrap();
    assert_eq!(
        named.name.as_deref(),
        Some("Microsoft Visual C++ Redistributable")
    );
    assert_eq!(
        error.message,
        "Captions and transcription cannot start: the Microsoft Visual C++ runtime is not installed on this PC"
    );
    let action = error.suggested_action.unwrap();
    assert!(
        action.contains("Settings > AI & transcription > Microsoft Visual C++ runtime > Install"),
        "{action}"
    );
    // The person's text names no verb; the agent's route is in the registry entry.
    assert!(!action.contains("system.fetch_tool"), "{action}");
    assert!(!action.contains("Agents"), "{action}");
    assert!(
        action.contains("Windows asks for administrator permission"),
        "{action}"
    );
    assert!(action.contains(OFFICIAL_PAGE), "{action}");
    let get = named.get.as_ref().expect("the registry's way forward");
    assert_eq!(get["verb"], "system.fetch_tool");
    assert_eq!(get["args"]["tool"], TOOL_ID);
}

#[test]
fn the_card_is_one_named_tool_with_its_source_and_its_users() {
    let missing = Status {
        applicable: true,
        installed: false,
        version: None,
        missing_files: vec!["msvcp140.dll".into()],
    };
    let card = card_for(&missing, "x86_64");
    assert_eq!(card.id, TOOL_ID);
    assert_eq!(card.status, CardStatus::Missing);
    assert_eq!(card.details["fetch_available"], true);
    assert_eq!(
        card.details["download_url"],
        "https://aka.ms/vc14/vc_redist.x64.exe"
    );
    assert_eq!(card.details["publisher"], "Microsoft Corporation");
    assert_eq!(card.details["needed_by"][0], "Captions and transcription");
    assert_eq!(card.details["removed_by_uninstall"], false);
    // No licence-acceptance machinery: the action is the plain install, and
    // the licence is named beside it (owner 2026-11-20).
    assert_eq!(
        card.details["fetch_action"]["args"],
        serde_json::json!({"tool": "vc-runtime"})
    );
    assert!(card.details["about"]
        .as_str()
        .unwrap()
        .contains(LICENSE_URL));
    assert!(card.hint.unwrap().contains(OFFICIAL_PAGE));

    let installed = Status {
        applicable: true,
        installed: true,
        version: Some("v14.44.35211.00".into()),
        missing_files: Vec::new(),
    };
    let card = card_for(&installed, "x86_64");
    assert_eq!(card.status, CardStatus::Ok);
    assert_eq!(card.version.as_deref(), Some("v14.44.35211.00"));
    assert_eq!(card.details["fetch_available"], false);
    assert!(card.hint.is_none());
}

#[cfg(not(windows))]
#[test]
fn nothing_is_needed_off_windows() {
    assert!(doctor_card().is_none());
    assert!(require_for("Captions and transcription").is_ok());
}
