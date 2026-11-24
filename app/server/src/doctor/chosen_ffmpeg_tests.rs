use super::chosen_ffmpeg::{
    apply_missing_chosen_report, missing_chosen_card_for, missing_chosen_hint_for,
    missing_chosen_path_for, report_has_chosen_missing,
};
use super::{scan_minimal, Card, CardSource, CardStatus};
use serde_json::json;

#[test]
fn missing_chosen_path_check_rejects_empty_relative_missing_and_directories() {
    assert_eq!(missing_chosen_path_for(None), None);
    assert_eq!(missing_chosen_path_for(Some("")), None);
    assert_eq!(missing_chosen_path_for(Some("   ")), None);
    assert_eq!(
        missing_chosen_path_for(Some("ffmpeg")),
        Some("ffmpeg".to_string())
    );

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("as-dir");
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(
        missing_chosen_path_for(dir.to_str()),
        Some(dir.display().to_string()),
        "directory at chosen path counts as missing"
    );

    let missing = tmp.path().join("gone-ffmpeg");
    assert_eq!(
        missing_chosen_path_for(missing.to_str()),
        Some(missing.display().to_string())
    );

    let file = tmp.path().join("ffmpeg");
    std::fs::write(&file, b"stub").unwrap();
    assert_eq!(missing_chosen_path_for(file.to_str()), None);
}

#[test]
fn platform_download_branches_produce_plain_hint_and_agent_action() {
    let with_dl = missing_chosen_hint_for(true);
    assert!(with_dl.contains(
        "The chosen video tool file is missing; choose it again or use the app's own download"
    ));
    let card_dl = missing_chosen_card_for("C:\\missing\\ffmpeg.exe", 1, true);
    assert_eq!(card_dl.status, CardStatus::Missing);
    assert_eq!(card_dl.source, Some(CardSource::Missing));
    assert_eq!(card_dl.details["chosen_missing"], true);
    assert_eq!(card_dl.details["runnable"], false);
    assert!(card_dl.details["reason"]
        .as_str()
        .unwrap()
        .contains("C:\\missing\\ffmpeg.exe"));
    assert!(card_dl.details["action"]
        .as_str()
        .unwrap()
        .contains("system.fetch_tool"));

    let no_dl = missing_chosen_hint_for(false);
    assert!(no_dl.contains("The chosen video tool file is missing; choose it again"));
    assert!(!no_dl.contains("app's own download"));
    let card_no_dl = missing_chosen_card_for("/opt/homebrew/bin/ffmpeg", 0, false);
    assert!(card_no_dl.details["action"]
        .as_str()
        .unwrap()
        .contains("details.setup"));
    assert!(card_no_dl.details["action"]
        .as_str()
        .unwrap()
        .contains("system.set_ffmpeg"));
}

#[test]
fn applying_missing_chosen_updates_cached_report_and_gpu_card() {
    let mut report = scan_minimal();
    report.cards = vec![
        Card {
            id: "ffmpeg".into(),
            kind: "tool".into(),
            status: CardStatus::Ok,
            source: Some(CardSource::Env),
            version: Some("8.0.1".into()),
            hint: None,
            details: json!({
                "resolved": "/custom/ffmpeg",
                "runnable": true,
                "selection": {"reason": "chosen", "found": 2},
            }),
        },
        Card {
            id: "ffprobe".into(),
            kind: "tool".into(),
            status: CardStatus::Ok,
            source: Some(CardSource::Env),
            version: Some("8.0.1".into()),
            hint: None,
            details: json!({"resolved": "/custom/ffprobe", "runnable": true}),
        },
        Card {
            id: "gpu-encode".into(),
            kind: "tool".into(),
            status: CardStatus::Ok,
            source: None,
            version: Some("nvenc".into()),
            hint: None,
            details: json!({
                "hardware_available": true,
                "ffmpeg_missing": false,
                "override_setting": "/custom/ffmpeg",
                "size_limits": [{"encoder": "h264_nvenc"}],
            }),
        },
    ];
    report.essential_ok = true;
    assert_eq!(report_has_chosen_missing(&report), None);

    apply_missing_chosen_report(&mut report, "/custom/ffmpeg");

    assert!(!report.essential_ok);
    assert_eq!(report_has_chosen_missing(&report), Some("/custom/ffmpeg"));
    let ffmpeg = report.cards.iter().find(|c| c.id == "ffmpeg").unwrap();
    assert_eq!(ffmpeg.status, CardStatus::Missing);
    assert_eq!(ffmpeg.details["selection"]["found"], 2);
    assert_eq!(ffmpeg.details["selection"]["reason"], "chosen_missing");

    let gpu = report.cards.iter().find(|c| c.id == "gpu-encode").unwrap();
    assert_eq!(gpu.status, CardStatus::Degraded);
    assert_eq!(gpu.details["ffmpeg_missing"], true);
    assert_eq!(gpu.details["chosen_missing"], true);
    assert_eq!(gpu.details["override_setting"], "/custom/ffmpeg");
    assert_eq!(gpu.details["size_limits"], json!([]));
}
