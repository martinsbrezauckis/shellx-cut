//! Which running jobs a project change may cancel without asking (item 135).
//!
//! A project change cancels every running job of the old project. Before, it
//! asked only for five kinds (render, render_queue, reframe, bundle,
//! screen_record_export); every other job, including work a person or agent
//! had explicitly asked for, was cancelled and only listed afterwards. The rule
//! is now the other way round: a project change asks before it cancels any job
//! EXCEPT the few below, which nobody asked for by name and whose loss costs
//! nothing that has to be redone by hand. A job kind added later asks by
//! default.
//!
//! Every kind the server creates, and why (checked 2026-11-20 at 4ebf18c99):
//! - asks (a result someone asked for would be lost): render, render_queue,
//!   reframe, reframe-direct (renders the edit for the director sheet),
//!   reframe-qc (the QC sheet of a reframe), bundle, screen_record_export,
//!   screen_record_copy_raw (copies a saved recording to a chosen place),
//!   screen_record_polish (the editable clip of a recording that just
//!   stopped; added 2026-11-20 so a project change cannot move it elsewhere),
//!   autopilot and recipe (run renders), asset_generate and judge (work of the
//!   person's own agent CLI, their quota), verify-rerun, transcribe, diarize,
//!   perception and media_intelligence (analysis asked for by verb),
//!   motion_import_apply, cache_rebuild, fetch_tool, setup_perception and
//!   setup_matte (downloads and installs the person started).
//! - may be cancelled without asking (still listed in `cancelled_jobs`):
//!   import_chain, proxy and enrich (automatic follow-ups of an import; the
//!   import itself is already done), proxy_check (the check of an opened
//!   project's proxies runs again at the next open), title_remake (a missing
//!   title is drawn again when next needed), cache_purge (stopping a
//!   deletion loses nothing).

const QUIET_CANCEL_KINDS: &[&str] = &[
    "import_chain",
    "proxy",
    "proxy_check",
    "title_remake",
    "enrich",
    "cache_purge",
];

/// True when a project change must ask before cancelling a job of this kind.
pub(crate) fn asks_before_cancel(kind: &str) -> bool {
    !QUIET_CANCEL_KINDS.contains(&kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn person_started_work_asks_and_automatic_work_does_not() {
        for kind in [
            "render",
            "render_queue",
            "reframe",
            "reframe-direct",
            "reframe-qc",
            "bundle",
            "screen_record_export",
            "screen_record_copy_raw",
            "screen_record_polish",
            "autopilot",
            "recipe",
            "asset_generate",
            "judge",
            "transcribe",
            "fetch_tool",
            "a_kind_added_later",
        ] {
            assert!(asks_before_cancel(kind), "{kind}");
        }
        for kind in [
            "import_chain",
            "proxy",
            "proxy_check",
            "title_remake",
            "enrich",
            "cache_purge",
        ] {
            assert!(!asks_before_cancel(kind), "{kind}");
        }
    }
}
