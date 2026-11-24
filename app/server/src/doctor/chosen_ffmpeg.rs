//! Chosen FFmpeg existence validation for doctor reads and scans.
//!
//! When a person picks a custom FFmpeg via Settings / `system.set_ffmpeg`,
//! every setup read (`system.doctor`, Settings, or the setup card) checks via
//! `Path::is_file` that the chosen file still exists before reporting Ready,
//! without spawning binaries on cached reads. If the file was removed, doctor
//! reports FFmpeg missing with plain user guidance and agent `reason`/`action`
//! instead of silently falling back or returning stale Ready state.

use super::{essential_ok_for, Card, CardSource, CardStatus, DoctorReport, NO_FFMPEG_EXPORT_HINT};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) const MISSING_CHOSEN_HINT_WITH_DOWNLOAD: &str =
    "The chosen video tool file is missing; choose it again or use the app's own download.";
pub(crate) const MISSING_CHOSEN_HINT_NO_DOWNLOAD: &str =
    "The chosen video tool file is missing; choose it again, use automatic selection, or install FFmpeg from the setup steps below.";

/// Returns the persisted chosen FFmpeg path when `SHELLX_CUT_FFMPEG` is unset
/// and the chosen path does not currently resolve to an existing file.
pub(crate) fn missing_chosen_path() -> Option<String> {
    if std::env::var_os(cut_media::toolpath::ENV_FFMPEG).is_some_and(|v| !v.is_empty()) {
        return None;
    }
    let chosen = cut_media::toolpath::read_override_setting()?;
    missing_chosen_path_for(Some(&chosen))
}

pub(crate) fn missing_chosen_path_for(chosen: Option<&str>) -> Option<String> {
    let chosen = chosen?.trim();
    if chosen.is_empty() {
        return None;
    }
    let path = Path::new(chosen);
    (!(path.is_absolute() && path.is_file())).then(|| chosen.to_string())
}

pub(crate) fn missing_chosen_hint_for(builtin_download: bool) -> &'static str {
    if builtin_download {
        MISSING_CHOSEN_HINT_WITH_DOWNLOAD
    } else {
        MISSING_CHOSEN_HINT_NO_DOWNLOAD
    }
}

pub(crate) fn agent_reason(chosen: &str) -> String {
    format!("The chosen FFmpeg file '{chosen}' is missing or is no longer a regular file.")
}

pub(crate) fn agent_action_for(builtin_download: bool) -> &'static str {
    if builtin_download {
        "Choose a runnable FFmpeg with system.set_ffmpeg {path}, reset to automatic with system.set_ffmpeg {path: null}, or download the app's own FFmpeg with system.fetch_tool {tool: \"ffmpeg\", accept_license: true}."
    } else {
        "Choose a runnable FFmpeg with system.set_ffmpeg {path}, reset to automatic with system.set_ffmpeg {path: null}, or follow details.setup for this platform."
    }
}

pub(crate) fn missing_chosen_card(chosen: &str, found_count: usize) -> Card {
    missing_chosen_card_for(
        chosen,
        found_count,
        cut_media::tool_setup::builtin_ffmpeg_download_available(),
    )
}

pub(crate) fn missing_chosen_card_for(
    chosen: &str,
    found_count: usize,
    builtin_download: bool,
) -> Card {
    let hint = missing_chosen_hint_for(builtin_download);
    let reason = agent_reason(chosen);
    let action = agent_action_for(builtin_download);
    Card {
        id: "ffmpeg".into(),
        kind: "tool".into(),
        status: CardStatus::Missing,
        source: Some(CardSource::Missing),
        version: None,
        hint: Some(hint.to_string()),
        details: json!({
            "resolved": chosen,
            "runnable": false,
            "chosen_missing": true,
            "chosen_path": chosen,
            "reason": reason,
            "action": action,
            "suggested_action": action,
            "version_advice": Value::Null,
            "essential": true,
            "install_dir": cut_media::toolpath::appdata_tools_dir()
                .map(|p| p.join("ffmpeg").display().to_string()),
            "setup": cut_media::tool_setup::ffmpeg_setup_guide(),
            "selection": {
                "reason": "chosen_missing",
                "automatic": cut_media::toolpath::automatic_pick(),
                "found": found_count,
                "text": hint,
            },
            "libass": Value::Null,
            "libvidstab": Value::Null,
            "zscale": Value::Null,
            "can_burn_captions": Value::Null,
            "can_stabilize": Value::Null,
            "can_color_manage": Value::Null,
            "libx265": Value::Null,
            "libvpx_vp9": Value::Null,
            "libsvtav1": Value::Null,
            "libx265_10bit": Value::Null,
            "libsvtav1_10bit": Value::Null,
            "can_export_hevc": Value::Null,
            "can_export_vp9": Value::Null,
            "can_export_av1": Value::Null,
            "can_export_hevc_10bit": Value::Null,
            "can_export_av1_10bit": Value::Null,
        }),
    }
}

pub(crate) fn apply_missing_to_gpu_card(card: &mut Card, chosen: &str) {
    card.status = CardStatus::Degraded;
    card.hint = Some(NO_FFMPEG_EXPORT_HINT.to_string());
    if let Some(details) = card.details.as_object_mut() {
        details.insert("ffmpeg_missing".into(), Value::Bool(true));
        details.insert("chosen_missing".into(), Value::Bool(true));
        details.insert("override_setting".into(), Value::String(chosen.to_string()));
        details.insert("enable_help".into(), Value::Null);
        details.insert("size_limits".into(), json!([]));
    }
}

pub(crate) fn report_has_chosen_missing(report: &DoctorReport) -> Option<&str> {
    let card = report.cards.iter().find(|c| c.id == "ffmpeg")?;
    if card.status != CardStatus::Missing {
        return None;
    }
    if card.details.get("chosen_missing").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    card.details.get("chosen_path").and_then(Value::as_str)
}

pub(crate) fn apply_missing_chosen_report(report: &mut DoctorReport, chosen: &str) {
    let found_count = report
        .cards
        .iter()
        .find(|c| c.id == "ffmpeg")
        .and_then(|c| c.details.get("selection")?.get("found")?.as_u64())
        .map(|n| n as usize)
        .unwrap_or(0);
    let updated_ffmpeg = missing_chosen_card(chosen, found_count);
    if let Some(slot) = report.cards.iter_mut().find(|c| c.id == "ffmpeg") {
        *slot = updated_ffmpeg;
    } else {
        report.cards.insert(0, updated_ffmpeg);
    }
    if let Some(gpu) = report.cards.iter_mut().find(|c| c.id == "gpu-encode") {
        apply_missing_to_gpu_card(gpu, chosen);
    }
    report.essential_ok = essential_ok_for(&report.cards);
    report.scanned_at = chrono::Utc::now().to_rfc3339();
}
