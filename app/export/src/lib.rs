//! cut-export — pure timeline -> NLE interchange serializers (public verb contract).
//!
//! Formats (the NLE interchange mapping design is the implementation
//! contract; examples/ holds the known-good shapes these mirror):
//! - `fcpxml`   — FCPXML 1.11 for Final Cut Pro          (fcpxml::render, resolve=false)
//! - `resolve`  — FCPXML 1.11 for DaVinci Resolve 17+    (fcpxml::render, resolve=true)
//! - `premiere` — FCP7 XML `xmeml` v5, Premiere dialect  (xmeml::render)
//! - `mlt`      — MLT XML for Shotcut                    (mlt::render)
//! - SRT        — caption track -> SubRip                (srt::render)
//!
//! Boundary: the public entry points take `&serde_json::Value` shaped like the
//! timeline/op-log contract timeline JSON and deserialize into the thin structs in [`model`].
//! Integration (app/server) passes its materialized project.json timeline
//! through unchanged; unknown fields are ignored, so cut-core can evolve
//! without breaking this crate.
//!
//! Captions are intentionally NOT exported in the XML formats (no portable
//! representation); they ship via [`export_srt`]. Callers should
//! surface [`CAPTIONS_NOT_IN_XML_NOTE`] as a verb warning.

pub mod captions_in;
pub mod edl;
pub mod error;
pub mod fcpxml;
pub mod mlt;
pub mod model;
pub mod otio;
pub mod quantize;
pub mod sources;
pub mod srt;
pub mod transcript;
pub mod vtt;
pub mod xmeml;
mod xml;

pub use error::ExportError;
pub use model::ExportTimeline;
pub use quantize::{quantize, QTrack, Quantized, Timebase, XClip, XItem};

/// Standing note for export.xml verb results: caption tracks are excluded by
/// design from every XML format — agents must know it was
/// intentional, not lossy.
pub const CAPTIONS_NOT_IN_XML_NOTE: &str =
    "caption track not exported in NLE XML (no portable representation); use export.srt";

/// XML export format selector — mirrors the public verb contract verb enum
/// `export.xml{format:"fcpxml"|"premiere"|"resolve"}` plus stretch `mlt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XmlFormat {
    Fcpxml,
    Resolve,
    Premiere,
    Mlt,
}

impl XmlFormat {
    /// Parse the verb's string arg. Unknown value -> actionable error.
    pub fn from_str(s: &str) -> Result<Self, ExportError> {
        match s {
            "fcpxml" => Ok(Self::Fcpxml),
            "resolve" => Ok(Self::Resolve),
            "premiere" => Ok(Self::Premiere),
            "mlt" => Ok(Self::Mlt),
            other => Err(ExportError::BadFormat(other.to_string())),
        }
    }

    /// Conventional file extension for the rendered document.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Fcpxml | Self::Resolve => "fcpxml",
            Self::Premiere => "xml",
            Self::Mlt => "mlt",
        }
    }
}

/// Render the timeline JSON to the requested XML format.
///
/// `timeline` must follow the timeline/op-log contract shape (settings/assets/tracks); extra
/// fields are ignored. Returns the full document text (UTF-8, with XML
/// declaration). Errors are actionable per public verb contract (clip id + ms + cause).
pub fn export_xml(timeline: &serde_json::Value, format: XmlFormat) -> Result<String, ExportError> {
    let tl = model::parse_timeline(timeline)?;
    match format {
        XmlFormat::Fcpxml => fcpxml::render(&tl, false),
        XmlFormat::Resolve => fcpxml::render(&tl, true),
        XmlFormat::Premiere => xmeml::render(&tl),
        XmlFormat::Mlt => mlt::render(&tl),
    }
}

/// Render the timeline as a CMX3600 EDL (`export.edl`) — the universal
/// edit-decision-list interchange (Resolve/Premiere/Avid/FCP). `title` names
/// the sequence. Cuts only; transitions/effects/grades/captions are dropped
/// (the format cannot carry them — the verb warns when they are present).
pub fn export_edl(timeline: &serde_json::Value, title: &str) -> Result<String, ExportError> {
    let tl = model::parse_timeline(timeline)?;
    edl::render(&tl, title)
}

/// Render the caption track to SRT (public verb contract `export.srt`).
pub fn export_srt(timeline: &serde_json::Value) -> Result<String, ExportError> {
    let tl = model::parse_timeline(timeline)?;
    srt::render(&tl)
}

/// Render the caption track to WebVTT (`export.vtt`) — the HTML5 `<track>`
/// caption standard for web-published video.
pub fn export_vtt(timeline: &serde_json::Value) -> Result<String, ExportError> {
    let tl = model::parse_timeline(timeline)?;
    vtt::render(&tl)
}

/// Render the caption track to a readable transcript (`export.transcript`) —
/// the script of the final cut for show notes / repurposing. `format` selects
/// txt|md; `timestamps` (md only) prefixes paragraphs with `[m:ss]`.
pub fn export_transcript(
    timeline: &serde_json::Value,
    format: transcript::TranscriptFormat,
    timestamps: bool,
) -> Result<String, ExportError> {
    let tl = model::parse_timeline(timeline)?;
    transcript::render(&tl, format, timestamps)
}
