//! Deadline control for preparatory children.

use super::{current_render_process_control, RenderProcessControl};
use std::time::{Duration, Instant};

pub(crate) fn bounded_current_control(timeout: Duration) -> RenderProcessControl {
    let mut control = current_render_process_control()
        .unwrap_or_else(|| RenderProcessControl::bounded(timeout, || false));
    control.deadline = control.deadline.min(Instant::now() + timeout);
    control
}
