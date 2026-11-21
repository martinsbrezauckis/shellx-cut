//! Optional advice about the runnable FFmpeg, independent of capability gates.

use super::Card;
use cut_media::toolpath::SelectionReason;
use serde_json::{json, Value};

pub(super) fn for_version(version: Option<&str>, os: &str, arch: &str) -> Option<Value> {
    let version = version?;
    let major = version
        .strip_prefix('n')
        .unwrap_or(version)
        .split('.')
        .next()?
        .parse::<u32>()
        .ok()?;
    if major >= 7 {
        return None;
    }
    let upgrade = match (os, arch) {
        ("windows" | "linux", "x86_64") => "Download a newer FFmpeg if you want to update.",
        ("macos", _) => {
            "Use Homebrew to install or upgrade ffmpeg-full, then choose that FFmpeg."
        }
        ("linux", _) => "Use your distribution's package manager for FFmpeg 7 or newer, or choose a newer build.",
        _ => "Choose an FFmpeg 7 or newer with ffprobe beside it if you want to update.",
    };
    Some(json!({
        "in_use": version,
        "tested_versions": ["8.0.1", "9.0.2"],
        "message": format!("FFmpeg {version} is in use; ShellX Cut is tested with FFmpeg 8.0.1 and 9.0.2."),
        "upgrade": upgrade,
    }))
}

pub(super) fn preserve_choice(card: &mut Card, reason: SelectionReason) {
    if matches!(
        reason,
        SelectionReason::Chosen | SelectionReason::Environment
    ) {
        if let Some(upgrade) = card.details["version_advice"]["upgrade"].as_str() {
            card.details["version_advice"]["upgrade"] =
                format!("{upgrade} Your chosen FFmpeg stays selected until you change it.").into();
        }
    }
}
