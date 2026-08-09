//! Platform-neutral values returned after a Linux portal capture closes.

use record_core::{ClickSample, CursorCorrelation, CursorSample, KeySample, ScrollSample};

use crate::cursor_correlation;

pub(crate) struct CapPhase {
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) duration_ms: u64,
    pub(crate) audio: Option<String>,
    pub(crate) input: CapturedInput,
    pub(crate) keys: Vec<KeySample>,
}

pub(crate) struct RecordedInput {
    pub(crate) cursor: Vec<CursorSample>,
    pub(crate) clicks: Vec<ClickSample>,
    pub(crate) scrolls: Vec<ScrollSample>,
    pub(crate) cursor_correlation: CursorCorrelation,
}

pub(crate) enum CapturedInput {
    Correlated(RecordedInput),
    RdevinPending {
        mode: cursor_correlation::SessionInputMode,
        cursor: Vec<CursorSample>,
        clicks: Vec<ClickSample>,
        scrolls: Vec<ScrollSample>,
        portal_geometry: Option<cursor_correlation::PortalCursorGeometry>,
    },
}
