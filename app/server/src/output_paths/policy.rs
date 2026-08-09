//! Verb-specific output filename policy.
//!
//! The canonical path fence owns *where* a writer may write. This module owns
//! the complementary question: which filename suffix can represent the bytes a
//! particular verb produces. Keeping that distinction explicit prevents a
//! fenced `export.srt` from creating a misleading `notes.md`, for example.

use cut_core::{error_codes, CutError};
use std::path::Path;

/// The suffixes a single file-writing verb is allowed to produce.
///
/// A policy is intentionally supplied by each call site rather than being a
/// global media allowlist: render.final{format:"vp9"} writes WebM, while
/// render.final{format:"prores"} writes MOV, and neither may silently write
/// the other container.
#[derive(Debug, Clone, Copy)]
pub(crate) enum OutputPathPolicy {
    Exact(&'static str),
    OneOf(&'static [&'static str]),
}

impl OutputPathPolicy {
    pub(crate) const MP4: Self = Self::Exact("mp4");
    pub(crate) const WEBM: Self = Self::Exact("webm");
    pub(crate) const MOV: Self = Self::Exact("mov");
    pub(crate) const GIF: Self = Self::Exact("gif");
    pub(crate) const JPEG: Self = Self::OneOf(&["jpg", "jpeg"]);
    pub(crate) const SRT: Self = Self::Exact("srt");
    pub(crate) const VTT: Self = Self::Exact("vtt");
    pub(crate) const ASS: Self = Self::Exact("ass");
    pub(crate) const OTIO: Self = Self::Exact("otio");
    pub(crate) const EDL: Self = Self::Exact("edl");
    pub(crate) const TXT: Self = Self::Exact("txt");
    pub(crate) const HTML: Self = Self::Exact("html");
    pub(crate) const JSON: Self = Self::Exact("json");

    pub(crate) const fn exact(extension: &'static str) -> Self {
        Self::Exact(extension)
    }

    fn accepts(self, actual: &str) -> bool {
        match self {
            Self::Exact(extension) => actual == extension,
            Self::OneOf(extensions) => extensions.contains(&actual),
        }
    }

    fn expected(self) -> String {
        match self {
            Self::Exact(extension) => format!(".{extension}"),
            Self::OneOf(extensions) => extensions
                .iter()
                .map(|extension| format!(".{extension}"))
                .collect::<Vec<_>>()
                .join(" or "),
        }
    }

    pub(crate) fn validate(self, path: &Path) -> Result<(), CutError> {
        let actual = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        if actual
            .as_deref()
            .is_some_and(|extension| self.accepts(extension))
        {
            return Ok(());
        }

        let expected = self.expected();
        Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("output path must end in {expected}"),
            format!("this verb writes {}; got {}", expected, path.display()),
        )
        .with_suggested_action(format!("choose an output filename ending in {expected}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_declared_container_suffix() {
        assert!(OutputPathPolicy::MP4
            .validate(Path::new("final.MP4"))
            .is_ok());
        assert!(OutputPathPolicy::MP4
            .validate(Path::new("final.webm"))
            .is_err());
        assert!(OutputPathPolicy::JPEG
            .validate(Path::new("frame.jpeg"))
            .is_ok());
        assert!(OutputPathPolicy::JPEG
            .validate(Path::new("frame.png"))
            .is_err());
    }
}
