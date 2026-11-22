//! Optional square overlay appearance; omitted defaults preserve old project bytes.
use crate::{error_codes, ClipTransform, CutError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayShape {
    #[default]
    Rect,
    Circle,
    Rounded,
}

impl OverlayShape {
    pub fn is_rect(&self) -> bool {
        *self == Self::Rect
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayRing {
    /// Width as a fraction of the bubble width; recorder look: 0.02 (min 2px).
    pub width: f64,
    /// CSS hex RGB or RGBA; recorder look: #ffffffe6.
    pub colour: String,
}

impl ClipTransform {
    pub(crate) fn validate_shape_track(
        &self,
        project: &crate::Project,
        track_id: &str,
    ) -> Result<(), CutError> {
        if !self.shape.is_rect()
            && project
                .tracks
                .iter()
                .find(|t| t.kind == crate::TrackKind::Video && !t.clips.is_empty())
                .is_some_and(|t| t.id == track_id)
        {
            return Err(CutError::new(error_codes::INVALID_ARGS, "shape applies only to overlay video clips", "add a video track above the base canvas and place the bubble there; rect transforms still work on the base"));
        }
        Ok(())
    }

    pub(crate) fn validate_shape(&self) -> Result<(), CutError> {
        if self.radius.is_some_and(|r| {
            self.shape != OverlayShape::Rounded || !r.is_finite() || !(0.0..=0.5).contains(&r)
        }) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "radius requires shape rounded and must be within [0, 0.5]",
                "radius is a fraction of the square bubble width",
            ));
        }
        if let Some(ring) = &self.ring {
            let colour = ring.colour.strip_prefix('#').unwrap_or("");
            if self.shape.is_rect()
                || !ring.width.is_finite()
                || !(0.0 < ring.width && ring.width <= 0.25)
                || !matches!(colour.len(), 6 | 8)
                || !colour.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(CutError::new(error_codes::INVALID_ARGS,
                    "ring requires circle or rounded, width in (0, 0.25], and colour #RRGGBB or #RRGGBBAA",
                    "recorder look: width 0.02, colour #ffffffe6"));
            }
        }
        Ok(())
    }
}
