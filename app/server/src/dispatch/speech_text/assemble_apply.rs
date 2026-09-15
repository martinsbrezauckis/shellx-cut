//! Revision-bound materialization for reviewed Assemble plans.

mod binding;
mod captions;
mod lowering;
mod materialization;
mod planning;

pub(crate) use binding::ASSEMBLE_CAPTION_TRACK_ID;
pub(super) use binding::{
    aspect_label, aspect_parts, bind_plan, binding_value, capture_plan_context,
    ensure_plan_context_current, transcript_content_sha256, Materialization, PlanBinding,
};
pub(super) use materialization::apply_planned_ranges;
