//! Thin input model mirroring the timeline/op-log contract timeline JSON.
//!
//! Deliberately NOT cut-core's types: this crate stays decoupled behind a
//! serde_json::Value boundary (integration deserializes the real timeline
//! into these). Every field beyond the essentials is optional with safe
//! defaults, and unknown fields are ignored — probe JSON shape can evolve
//! freely. Aliases cover the likely cut-media probe spellings.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::error::ExportError;

/// Parse a timeline/op-log contract timeline JSON value into the export model.
pub fn parse_timeline(v: &serde_json::Value) -> Result<ExportTimeline, ExportError> {
    serde_json::from_value(v.clone()).map_err(|e| ExportError::BadInput(e.to_string()))
}

/// Top-level timeline (timeline/op-log contract project.json timeline snapshot).
#[derive(Debug, Clone, Deserialize)]
pub struct ExportTimeline {
    pub settings: Settings,
    #[serde(default)]
    pub assets: BTreeMap<String, ExportAsset>,
    #[serde(default)]
    pub tracks: Vec<ExportTrack>,
}

/// Timeline settings — width/height/fps/audio_rate (timeline/op-log contract defaults).
#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_fps")]
    pub fps: f64,
    #[serde(default = "default_audio_rate")]
    pub audio_rate: u32,
}

fn default_width() -> u32 {
    1920
}
fn default_height() -> u32 {
    1080
}
fn default_fps() -> f64 {
    30.0
}
fn default_audio_rate() -> u32 {
    48_000
}

/// One assets-map entry. `probe` is the normalized probe JSON subset we need;
/// missing probe falls back to timeline settings + clip-derived duration.
#[derive(Debug, Clone, Deserialize)]
pub struct ExportAsset {
    pub path: String,
    #[serde(default)]
    pub probe: Option<AssetProbe>,
}

/// Probe facts used by the serializers.
/// All optional — wrong/missing probe degrades to safe defaults, never errors.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AssetProbe {
    /// Full SOURCE duration in ms (FCPXML asset duration — NOT timeline length).
    #[serde(default, alias = "duration")]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub has_video: Option<bool>,
    #[serde(default)]
    pub has_audio: Option<bool>,
    #[serde(default, alias = "channels")]
    pub audio_channels: Option<u32>,
    #[serde(default, alias = "audio_rate")]
    pub sample_rate: Option<u32>,
    /// Source embedded SMPTE start timecode "HH:MM:SS:FF" (rare; default 0).
    #[serde(default, alias = "start_timecode")]
    pub timecode: Option<String>,
    // FCPXML colorSpace mapping inputs; default is always safe.
    #[serde(default)]
    pub pix_fmt: Option<String>,
    #[serde(default)]
    pub color_space: Option<String>,
    #[serde(default)]
    pub color_primaries: Option<String>,
    #[serde(default)]
    pub color_transfer: Option<String>,
}

/// One timeline track. `kind` ∈ {"video","audio","caption"}; others ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct ExportTrack {
    #[serde(default)]
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub clips: Vec<ExportClip>,
}

/// One clip — media, gap, or caption, discriminated by which fields are set
/// (matches timeline/op-log contract: media={asset,src_in_ms,src_out_ms}, gap={kind:"gap",
/// duration_ms}, caption={text,range_ms}). A struct-of-options instead of an
/// untagged enum keeps serde errors actionable and tolerates extra fields.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExportClip {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub asset: Option<String>,
    #[serde(default)]
    pub src_in_ms: Option<u64>,
    #[serde(default)]
    pub src_out_ms: Option<u64>,
    /// Gap clips only.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// Audio stream index in the source (currently stream 0).
    #[serde(default)]
    pub stream: Option<i32>,
    #[serde(default)]
    pub gain_db: Option<f64>,
    /// Rich edit attributes that the interchange serializers may not carry.
    /// Keeping these in the model prevents silent serde drops and lets callers
    /// produce honest "not represented" warnings.
    #[serde(default)]
    pub effects: Vec<Value>,
    #[serde(default)]
    pub grade: Option<Value>,
    #[serde(default)]
    pub grade_stack: Vec<Value>,
    #[serde(default)]
    pub grade_windows: Vec<Value>,
    #[serde(default)]
    pub transform: Option<Value>,
    #[serde(default)]
    pub crop: Option<Value>,
    #[serde(default)]
    pub fade: Option<Value>,
    #[serde(default)]
    pub xfade_in_ms: u64,
    #[serde(default)]
    pub xfade_kind: Option<String>,
    #[serde(default = "default_clip_speed")]
    pub speed: f64,
    #[serde(default)]
    pub speed_ramp: Option<ExportSpeedRamp>,
    #[serde(default)]
    pub matte: Option<Value>,
    #[serde(default)]
    pub mask: Option<Value>,
    #[serde(default)]
    pub reverse: bool,
    #[serde(default)]
    pub freeze: Option<Value>,
    #[serde(default)]
    pub animation: Option<Value>,
    #[serde(default)]
    pub keyframes: Vec<Value>,
    #[serde(default)]
    pub eq: Option<Value>,
    #[serde(default)]
    pub stabilize: Option<Value>,
    #[serde(default)]
    pub input_color_space: Option<Value>,
    #[serde(default)]
    pub nest: Option<String>,
    // Caption clips only.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub range_ms: Option<[u64; 2]>,
}

fn default_clip_speed() -> f64 {
    1.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExportSpeedRampPoint {
    pub at_ms: u64,
    pub factor: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExportSpeedRamp {
    #[serde(default)]
    pub points: Vec<ExportSpeedRampPoint>,
    pub segments: usize,
}

impl ExportClip {
    /// True if this is a `{"kind":"gap"}` clip.
    pub fn is_gap(&self) -> bool {
        self.kind.as_deref() == Some("gap")
    }

    /// Clip id for error messages (falls back to "?" — never panics).
    pub fn id_str(&self) -> String {
        self.id.clone().unwrap_or_else(|| "?".to_string())
    }
}

// ---------------------------------------------------------------------------
// Shared helpers used by multiple serializers.
// ---------------------------------------------------------------------------

/// File stem (name without extension) — clip/asset display names in every format.
pub fn file_stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// RFC 3986 percent-encoded `file://` URI from an absolute path (the export mapping
/// media-rep rule). Keeps unreserved chars, '/', and ':' (Windows drive);
/// everything else %XX. Windows "C:/..." gains the leading slash.
pub fn file_uri(path: &str) -> String {
    let path = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path.to_string()
    };
    let windows_path = path.starts_with(r"\\")
        || (path.as_bytes().get(1) == Some(&b':')
            && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic));
    let path = if windows_path {
        path.replace('\\', "/")
    } else {
        path
    };
    let mut s = String::from("file://");
    if !path.starts_with('/') {
        s.push('/');
    }
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                s.push(b as char)
            }
            _ => s.push_str(&format!("%{b:02X}")),
        }
    }
    s
}

/// Parse source SMPTE start timecode "HH:MM:SS:FF" (or ';' separators) into
/// frames at the rounded fps. Unparseable -> 0 (safe default).
pub fn parse_smpte(tc: &str, fps_round: i64) -> i64 {
    let parts: Vec<i64> = tc
        .replace(';', ":")
        .split(':')
        .map(|p| p.parse::<i64>().unwrap_or(0))
        .collect();
    match parts.as_slice() {
        [h, m, s, ff] => (h * 3600 + m * 60 + s) * fps_round + ff,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::file_uri;

    #[test]
    fn file_uri_normalizes_windows_verbatim_and_native_paths() {
        let expected = "file:///C:/Users/Editor/A%20B/clip.mp4";
        assert_eq!(file_uri(r"\\?\C:\Users\Editor\A B\clip.mp4"), expected);
        assert_eq!(file_uri(r"C:\Users\Editor\A B\clip.mp4"), expected);
        assert_eq!(
            file_uri("/media/A B/clip.mp4"),
            "file:///media/A%20B/clip.mp4"
        );
    }
}
