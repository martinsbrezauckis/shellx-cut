//! Encoder ownership exchanged under the existing serialized frame callback.

use std::time::Instant;

use record_core::Result;

use crate::windows_wgc_encoder::WgcVideoEncoder;
use crate::windows_wgc_timing::WgcTimingRecorder;

pub(crate) struct WriterSwap {
    pub(crate) encoder: WgcVideoEncoder,
    pub(crate) timing: WgcTimingRecorder,
}

pub(crate) struct RetiredWriter {
    pub(crate) encoder: WgcVideoEncoder,
    pub(crate) timing: WgcTimingRecorder,
    pub(crate) boundary_at: Instant,
}

pub(crate) type RotateWriter = Box<dyn FnMut(WriterSwap) -> Result<RetiredWriter> + Send>;
pub(crate) type CloseCapture = Box<dyn FnOnce(Option<Instant>) -> Result<()> + Send>;
