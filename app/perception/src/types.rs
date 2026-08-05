//! types.rs — perception fact types (perception contract perception.json shape).
//!
//! Role: the typed mirror of `receipts/<asset>.perception.json` — timestamped
//! facts the Rust side treats as the source for checks. The python sidecar
//! EMITS this shape; sidecar.rs validates it; checks.rs consumes it.
//! All times in ms, matching the rest of the system.

use serde::{Deserialize, Serialize};

/// Current perception schema tag (sidecar emits, Rust validates).
pub const PERCEPTION_SCHEMA: &str = "shellx-cut/perception/1";

/// One transcribed word with timing (whisperX word-level timestamps).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WordSpan {
    /// 0-based index into the transcript — transcript.cut_words addresses
    /// words by [start_idx, end_idx] ranges of these indices.
    pub idx: usize,
    pub word: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// ASR confidence 0..1 when the model provides it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Canonical speaker label ("S1".."Sn", arrival order) of the diarized turn
    /// this word overlaps MOST in time. Set by `media.diarize` (additive — a
    /// pre-diarization or non-diarized transcript leaves this `None`; word
    /// consumers that don't care simply ignore it). Backend-agnostic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
}

/// Full word-level transcript of an asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    /// Asset id this transcript belongs to.
    pub asset: String,
    /// Model used, e.g. "whisperx-small" (receipt provenance).
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub words: Vec<WordSpan>,
}

/// One diarized speaker turn — "who spoke when". Times in ms; `speaker` is the
/// canonical ARRIVAL-ORDER label "S1".."Sn" (the speaker who talks earliest is
/// "S1"), so the shape is backend-agnostic (Sortformer is already arrival-ordered;
/// pyannote's arbitrary labels are remapped to the same convention). Turns MAY
/// overlap in time (overlapped speech yields turns from different speakers that
/// overlap). Produced by `media.diarize`, consumed by the speaker-labeled
/// transcript/captions, `edit.multicam_switch` (speaker mode), and `audio.dub`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerTurn {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: String,
}

/// Diarization provenance (receipt honesty — mirrors `Transcript.model`). Records
/// WHICH backend/model produced `PerceptionReport.speaker_turns` and on what
/// device, plus how many distinct speakers were detected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diarization {
    /// "sortformer" | "pyannote" — the adapter backend that ran.
    pub backend: String,
    /// e.g. "sortformer-v2" / "nvidia/diar_streaming_sortformer_4spk-v2".
    pub model: String,
    /// Compute device the inference ran on, e.g. "cuda"/"cpu" (receipt honesty).
    #[serde(default)]
    pub device: String,
    /// Distinct speakers in `speaker_turns`.
    pub num_speakers: u32,
}

/// A detected silence span (silero-vad, cross-checked with ffmpeg
/// silencedetect per perception contract).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SilenceSpan {
    pub start_ms: u64,
    pub end_ms: u64,
    /// Which detector(s) agreed: "silero" | "ffmpeg" | "both".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// A scene cut (PySceneDetect content detector).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneCut {
    pub at_ms: u64,
    /// Detector score at the cut, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
}

/// A detected video defect span — black (ffmpeg blackdetect) or frozen
/// (ffmpeg freezedetect) frames. Emitted by the sidecar's "scenes"
/// instrument; consumed by the black_or_frozen_frames check. Detection
/// thresholds live in the sidecar (blackdetect d=0.3s, freezedetect d=2.0s),
/// so any reported span is by construction over threshold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoSpan {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// The detected content bounding box of a video.
///
/// ffmpeg `cropdetect` sampled across several windows of the clip reports the
/// rectangle of NON-uniform pixels — the actual picture inside any baked-in
/// letterbox/pillarbox bands. The real-world driver: an OBS capture whose
/// window (3840×2048) sat inside a larger canvas (3840×2160) baked 56-px black
/// bands into the SOURCE pixels; the editor imported/edited/rendered/passed it
/// without noticing. `uniform_border` flags exactly that — `edit.crop` fixes
/// it, the `uniform_border` receipt check guards the render against shipping it.
///
/// All fields are SOURCE pixels (the clip's own coordinate space). x/y = the
/// top-left of the content rect inside the full WxH frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentBbox {
    /// Full source frame width (px) — the geometry cropdetect ran against.
    pub frame_width: u32,
    /// Full source frame height (px).
    pub frame_height: u32,
    /// Content rect left edge (px) inside the full frame.
    pub x: u32,
    /// Content rect top edge (px) inside the full frame.
    pub y: u32,
    /// Content rect width (px); == frame_width when no left/right band.
    pub width: u32,
    /// Content rect height (px); == frame_height when no top/bottom band.
    pub height: u32,
    /// True when the content rect is strictly inside the frame by more than
    /// the detector's tolerance on any edge — i.e. uniform-colour bands are
    /// baked into the source pixels (letterbox/pillarbox). The agent should
    /// `edit.crop` to the bbox before rendering.
    pub uniform_border: bool,
    /// How many cropdetect sample windows agreed on this bbox (provenance).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples_agreed: Option<u32>,
}

/// Musical beat grid from the lightweight energy-peak detector (not librosa/madmom).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeatGrid {
    pub bpm: f32,
    pub beats_ms: Vec<u64>,
}

/// Loudness facts (ffmpeg ebur128): integrated LUFS, true peak, per-window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Loudness {
    pub integrated_lufs: f64,
    pub true_peak_dbtp: f64,
    /// Momentary/short-term windows for silence/spike localization.
    #[serde(default)]
    pub windows: Vec<LoudnessWindow>,
}

/// One loudness measurement window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoudnessWindow {
    pub at_ms: u64,
    pub momentary_lufs: f64,
}

/// One frame's subject observation in a [`SubjectTrack`] (auto-reframe; the
/// `subject` instrument).
///
/// All coordinates are NORMALIZED to `[0,1]` against the analyzed frame, so the
/// track is BOTH resolution- and aspect-INDEPENDENT: it is extracted once (on a
/// cheap proxy) and then drives a moving crop to ANY target aspect at full source
/// resolution, with no re-analysis. `cx`/`cy` = the framing point (where the crop
/// centre should point); `fx1..fy2` = the focus rectangle to keep in frame (drives
/// zoom). A frame with no detected subject leaves all of these `None` — the render
/// holds the last crop / centres (the receipt records the gap).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubjectFrame {
    /// Frame index (0-based) in the analyzed stream.
    pub f: u32,
    /// Presentation time of this frame (ms).
    pub t_ms: u64,
    /// Framing-point X, normalized `[0,1]`. `None` ⇒ no subject this frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cx: Option<f32>,
    /// Framing-point Y, normalized `[0,1]`. `None` ⇒ no subject this frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cy: Option<f32>,
    /// Focus-rect left/top/right/bottom, normalized `[0,1]` (the box to keep in
    /// frame). `None` ⇒ no subject this frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx1: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fy1: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx2: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fy2: Option<f32>,
    /// Detector confidence `[0,1]` of the chosen subject (`0.0` when none).
    #[serde(default)]
    pub conf: f32,
    /// Scene index (the render resets the crop choice at each PySceneDetect cut).
    #[serde(default)]
    pub scene: u32,
    /// Subject class ("person", "dog", …); empty when no subject this frame.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cls: String,
    /// ByteTrack id of the chosen subject (continuity across frames), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tid: Option<i64>,
}

fn default_subject_fps_source() -> String {
    "measured".to_string()
}

/// Per-asset subject track for auto-reframe (the `subject` instrument; perception contract
/// reframe rework.
///
/// The heavy local-CV analysis (YOLO-seg + ByteTrack + saliency, optionally face/
/// pose) reduced to a smoothable, **aspect-independent, normalized** path. Emitted
/// ON DEMAND (NOT at import — most clips are never reframed) and cached by
/// `asset_hash`; the render (`cut_media::render` reframe mode) consumes it together
/// with a target aspect and applies the deterministic damped-spring smoothing +
/// crop. One track serves 9:16 / 1:1 / 4:5 alike.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubjectTrack {
    /// Frames-per-second of the analyzed stream (maps frame index ↔ time).
    pub fps: f32,
    /// Provenance for `fps`: "measured" when OpenCV reported a usable value,
    /// "fallback" when the sidecar had to use a default for timestamp math.
    #[serde(default = "default_subject_fps_source")]
    pub fps_source: String,
    /// Human-readable warning when `fps_source == "fallback"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps_warning: Option<String>,
    /// Analyzed frame width (px) — provenance; coords are normalized against it.
    pub frame_width: u32,
    /// Analyzed frame height (px).
    pub frame_height: u32,
    /// Scene-start frame indices (0-based) — the render resets the crop at each.
    #[serde(default)]
    pub scenes: Vec<u32>,
    /// Segmentation model that produced the track (provenance/receipt).
    #[serde(default)]
    pub seg_model: String,
    /// Compute device the analysis ran on, e.g. "cuda"/"cpu" (receipt honesty).
    #[serde(default)]
    pub device: String,
    /// Subject classes considered (the preset's class set).
    #[serde(default)]
    pub classes: Vec<String>,
    /// Whether the active-speaker heuristic (audio-gated mouth motion) was
    /// available and applied — true only when the analyzed stream had audio.
    /// Receipt honesty: false ⇒ framing used face/saliency only, no speaker cue.
    #[serde(default)]
    pub speaker_aware: bool,
    /// Whether the face detector was available for face/eye-line framing.
    /// Receipt honesty: false ⇒ framing fell back to body/saliency centers.
    #[serde(default)]
    pub face_aware: bool,
    /// Director-model scene indices whose subject the
    /// foundation model decided (via a `direction` brief), vs the CV ranker.
    /// Empty ⇒ pure CV framing. Receipt honesty for "who chose the subject".
    #[serde(default)]
    pub directed_scenes: Vec<u32>,
    /// Per-frame subject observations (one per analyzed frame).
    #[serde(default)]
    pub frames: Vec<SubjectFrame>,
}

/// The full instrument run for one media file —
/// `receipts/<asset>.perception.json` (perception contract output).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerceptionReport {
    /// Always PERCEPTION_SCHEMA.
    pub schema: String,
    /// "sha256:<hex>" of the analyzed file — the cache key.
    pub asset_hash: String,
    /// Which file was analyzed (path at analysis time, forensic only).
    pub source_path: String,
    /// Which instruments actually ran for this report (provenance + cache
    /// completeness: a cached WordsOnly report must not satisfy a Full
    /// request — empty `scenes` is indistinguishable from "not run" without
    /// this).
    #[serde(default)]
    pub instruments_run: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<Transcript>,
    #[serde(default)]
    pub silences: Vec<SilenceSpan>,
    #[serde(default)]
    pub scenes: Vec<SceneCut>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beats: Option<BeatGrid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness: Option<Loudness>,
    /// Black video spans in the file (blackdetect; "scenes" instrument).
    #[serde(default)]
    pub black_spans: Vec<VideoSpan>,
    /// Frozen video spans in the file (freezedetect; "scenes" instrument).
    #[serde(default)]
    pub frozen_spans: Vec<VideoSpan>,
    /// Detected content bounding box / uniform-border flag (cropdetect,
    /// "scenes" instrument — video assets only). None when not measured (the
    /// instrument did not run, or no video stream — keeps audio-only reports
    /// and legacy reports loading unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_bbox: Option<ContentBbox>,
    /// Subject track for auto-reframe (the `subject` instrument; video-only,
    /// ON-DEMAND — not part of the import instrument set). `None` unless the
    /// subject instrument ran. Normalized + aspect-independent; the render
    /// derives the moving crop from it. Additive — pre-reframe reports (and every
    /// non-reframe perception run) load unchanged with this `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_track: Option<SubjectTrack>,
    /// Diarized speaker turns ("who spoke when"; the `media.diarize` verb).
    /// Empty unless diarization ran. Arrival-order labels "S1".."Sn"; turns may
    /// overlap. Additive — every existing report (and every non-diarize run)
    /// loads unchanged with this empty, exactly like `silences`/`scenes`.
    #[serde(default)]
    pub speaker_turns: Vec<SpeakerTurn>,
    /// Diarization provenance (backend/model/device/num_speakers). `None` unless
    /// `media.diarize` ran. Additive — pre-diarization reports load unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diarization: Option<Diarization>,
}

#[cfg(test)]
mod tests {
    use super::SubjectTrack;

    #[test]
    fn subject_track_preserves_fps_fallback_provenance() {
        let track: SubjectTrack = serde_json::from_value(serde_json::json!({
            "fps": 30.0,
            "fps_source": "fallback",
            "fps_warning": "OpenCV did not report a usable FPS; using 30fps for subject timestamps",
            "frame_width": 1280,
            "frame_height": 720,
            "frames": []
        }))
        .expect("subject track should load with fallback fps provenance");

        assert_eq!(track.fps_source, "fallback");
        assert_eq!(
            track.fps_warning.as_deref(),
            Some("OpenCV did not report a usable FPS; using 30fps for subject timestamps")
        );
    }

    #[test]
    fn subject_track_defaults_fps_source_to_measured_for_old_receipts() {
        let track: SubjectTrack = serde_json::from_value(serde_json::json!({
            "fps": 24.0,
            "frame_width": 1920,
            "frame_height": 1080,
            "frames": []
        }))
        .expect("old subject track receipts should remain compatible");

        assert_eq!(track.fps_source, "measured");
        assert_eq!(track.fps_warning, None);
    }

    #[test]
    fn subject_track_defaults_face_awareness_for_old_receipts() {
        let track: SubjectTrack = serde_json::from_value(serde_json::json!({
            "fps": 24.0,
            "frame_width": 1920,
            "frame_height": 1080,
            "frames": []
        }))
        .expect("old subject track receipts should remain compatible");

        assert!(!track.face_aware);

        let track: SubjectTrack = serde_json::from_value(serde_json::json!({
            "fps": 24.0,
            "frame_width": 1920,
            "frame_height": 1080,
            "face_aware": true,
            "frames": []
        }))
        .expect("new subject track receipts should preserve face-awareness");

        assert!(track.face_aware);
    }
}
