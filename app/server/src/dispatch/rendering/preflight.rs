//! Render resource preflight (items 82, 164, 165): before a render starts,
//! estimate its memory and output size, compare them with what this computer
//! has now, and choose threads, warn or refuse with the numbers and the way out.
//!
//! - Threads: libx264 uses its automatic count (1.5 x cores) when the estimate
//!   fits the memory free now; otherwise the largest ladder count that fits
//!   (`cut_media::render::resources::threads`). The choice is pinned into the
//!   preset (`-threads 0` is automatic), so the render uses exactly what the
//!   plan says; a reduction is said in `resources.memory.note`, the job's
//!   progress message and the receipt (`encode.memory_note`). It is not a
//!   warning: the export works, only slower.
//! - Warn (`warnings[]`, code `memory_short` / `disk_short`): even the fewest
//!   threads do not fit, or the file may not fit. The render may still work
//!   (swap, compressed memory, a smaller file than typical) but the person
//!   should know before it starts. The UI shows these in the export preflight
//!   dialog with "Export anyway".
//! - Refuse: code `out_of_memory` when the estimate is larger than this
//!   computer's memory and swap together (it cannot finish), `not_enough_space`
//!   when even half the typical file does not fit on the output drive.
//!
//! The estimate is a model calibrated by measurement (see cut_media
//! render/resources and the item-164 record): above the measured peak, by at
//! most about 1.5x in the measured cases.

use cut_core::{error_codes, CutError, Edl, Project, VerbWarning};
use cut_media::render::resources::threads;
use cut_media::render::resources::{
    estimate_memory, estimate_output_mb, video_encoder, MemoryEstimate,
};
use serde_json::{json, Value};
use std::path::Path;

const LIGHTER: &str = "keep Faster exports on (hardware encoder), choose HEVC instead of \
     H.264 for 8K, render at a lower resolution, or close other programs";
/// Keep this much free on the output drive besides the file.
const DISK_MARGIN_MB: u64 = 500;

pub(super) struct Preflight {
    /// The `resources` object of the plan and the job result.
    pub json: Value,
    /// Set when the render must not start.
    pub refusal: Option<CutError>,
    pub warnings: Vec<VerbWarning>,
    /// libx264 thread count to pin (`0` = automatic); None for other encoders
    /// or when the request pinned its own.
    pub adapt_threads: Option<u32>,
    /// Set when the threads were reduced to fit memory: the sentence for the
    /// progress message and the receipt.
    pub note: Option<String>,
}

/// What the preflight compares against; read from the machine by
/// [`render_preflight`], passed in directly by tests.
pub(super) struct Machine {
    pub total_mb: Option<u64>,
    pub available_mb: Option<u64>,
    pub swap_mb: u64,
    pub disk_free_mb: Option<u64>,
    /// The person's own limit (`SHELLX_CUT_RENDER_MEMORY_MB`), when set.
    pub limit_mb: Option<u64>,
}

pub(super) struct Plan<'a> {
    pub project: &'a Project,
    pub edl: &'a Edl,
    pub width: u32,
    pub height: u32,
    pub video_args: &'a [String],
    pub audio_kbps: u64,
    pub out: &'a Path,
}

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_TOTAL_MB: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

pub(super) fn render_preflight(plan: &Plan) -> Preflight {
    host_preflight(plan, true, |args| {
        estimate_memory(plan.project, plan.edl, plan.width, plan.height, args)
    })
}

fn host_preflight(
    plan: &Plan,
    check_disk: bool,
    estimate: impl Fn(&[String]) -> MemoryEstimate,
) -> Preflight {
    let host = cut_media::host_memory::host_memory();
    let mb = |b: u64| b / (1024 * 1024);
    let volume = plan
        .out
        .parent()
        .map(crate::doctor::nearest_existing)
        .unwrap_or_else(|| plan.out.to_path_buf());
    #[allow(unused_mut)]
    let mut machine = Machine {
        total_mb: host.map(|h| mb(h.total_bytes)),
        available_mb: host.map(|h| mb(h.available_bytes)),
        swap_mb: host.map(|h| mb(h.swap_bytes)).unwrap_or(0),
        disk_free_mb: check_disk
            .then(|| crate::doctor::free_total_bytes(&volume).0.map(mb))
            .flatten(),
        limit_mb: threads::memory_limit_mb(),
    };
    #[cfg(test)]
    TEST_TOTAL_MB.with(|total| {
        if let Some(total) = total.get() {
            machine.total_mb = Some(total);
            machine.swap_mb = 0;
        }
    });
    decide(plan, &machine, &volume, estimate)
}

mod exports;
pub(super) use exports::{export_preflight, post_pass_preflight};

fn gb(mb: u64) -> String {
    format!("{:.1} GB", mb as f64 / 1024.0)
}

pub(super) fn decide(
    plan: &Plan,
    machine: &Machine,
    volume: &Path,
    estimate: impl Fn(&[String]) -> MemoryEstimate,
) -> Preflight {
    let mut warnings = Vec::new();
    let mut refusal = None;
    let mut adapt_threads = None;
    let mut note = None;

    // What the render may plan with: free memory now, or the person's limit.
    let budget = threads::budget_mb(machine.available_mb, machine.limit_mb);
    let limited = machine.limit_mb.is_some() && budget == machine.limit_mb;
    let mut mem = estimate(plan.video_args);
    let mut automatic_threads = None;
    let unpinned_x264 = video_encoder(plan.video_args) == Some("libx264")
        && !plan.video_args.iter().any(|a| a == "-threads");
    if let (true, Some(automatic)) = (unpinned_x264, mem.encoder_threads) {
        let with = |n: u32| {
            let mut args = plan.video_args.to_vec();
            args.extend(["-threads".to_string(), n.to_string()]);
            estimate(&args)
        };
        let auto_mb = mem.total_mb;
        let choice = threads::choose(automatic, budget, |n| {
            if n == automatic {
                auto_mb
            } else {
                with(n).total_mb
            }
        });
        automatic_threads = Some(automatic);
        if choice.reduced() {
            mem = with(choice.chosen);
            adapt_threads = Some(choice.chosen);
            note = budget.map(|b| threads::reduction_sentence(&choice, b, limited));
        } else {
            adapt_threads = Some(0);
        }
    }
    let mem_fits = budget.map(|a| mem.total_mb <= a);
    if let Some(total) = machine.total_mb {
        if mem.total_mb > total + machine.swap_mb {
            refusal = Some(
                CutError::new(
                    error_codes::OUT_OF_MEMORY,
                    format!(
                        "this render needs about {} of memory; this computer has {} \
                         (memory and swap together {})",
                        gb(mem.total_mb),
                        gb(total),
                        gb(total + machine.swap_mb)
                    ),
                    "the render plan's memory estimate (decoder, encoder, held frames) is \
                     larger than this computer can provide",
                )
                .with_suggested_action(format!("Make the render lighter: {LIGHTER}.")),
            );
        }
    }
    if refusal.is_none() && mem_fits == Some(false) {
        warnings.push(warning(
            "memory_short",
            format!(
                "This render needs about {} of memory{} and about {} is {}. It may run very \
                 slowly or be stopped. To make it lighter: {LIGHTER}.",
                gb(mem.total_mb),
                match adapt_threads {
                    Some(n) if n > 0 => format!(" even with {n} encoder threads"),
                    _ => String::new(),
                },
                gb(budget.unwrap_or(0)),
                if limited {
                    format!("allowed ({})", threads::MEMORY_LIMIT_ENV)
                } else {
                    "free now".to_string()
                }
            ),
            json!({"estimate_mb": mem.total_mb, "available_mb": machine.available_mb, "limit_mb": machine.limit_mb}),
        ));
    }

    let fps = plan.project.settings.fps;
    let out_mb = estimate_output_mb(
        plan.edl.duration_ms,
        plan.width,
        plan.height,
        fps,
        plan.video_args,
        plan.audio_kbps,
    );
    let disk_fits = machine
        .disk_free_mb
        .map(|free| out_mb + DISK_MARGIN_MB <= free);
    if let Some(free) = machine.disk_free_mb {
        let short_by = (out_mb + DISK_MARGIN_MB).saturating_sub(free);
        if out_mb / 2 > free && refusal.is_none() {
            refusal = Some(
                CutError::new(
                    error_codes::NOT_ENOUGH_SPACE,
                    format!(
                        "not enough free space for this render on {}: it needs about {}, {} \
                         is free",
                        volume.display(),
                        gb(out_mb),
                        gb(free)
                    ),
                    "the typical file size for this length, size and encoder is larger than \
                     the free space on the output drive",
                )
                .with_suggested_action(format!(
                    "Free at least {} on that drive, or choose another output folder \
                     (project.set_output_dir; in the app Export > Choose folder).",
                    gb(short_by)
                ))
                .with_requirement("disk_space"),
            );
        } else if disk_fits == Some(false) {
            warnings.push(warning(
                "disk_short",
                format!(
                    "This render may not fit on {}: a typical file of this length and size is \
                     about {} and {} is free. Free about {} or choose another output folder.",
                    volume.display(),
                    gb(out_mb),
                    gb(free),
                    gb(short_by)
                ),
                json!({"estimate_mb": out_mb, "free_mb": free}),
            ));
        }
    }

    let json = json!({
        "memory": {
            "estimate_mb": mem.total_mb,
            "parts_mb": {
                "decode": mem.decode_mb,
                "encode": mem.encode_mb,
                "held_frames": mem.held_frames_mb,
                "overlays": mem.overlays_mb,
            },
            "windows": mem.windows,
            "encoder_threads": mem.encoder_threads,
            "automatic_encoder_threads": automatic_threads,
            "adapted_encoder_threads": adapt_threads.filter(|n| *n > 0),
            "note": note,
            "available_mb": machine.available_mb,
            "limit_mb": machine.limit_mb,
            "total_mb": machine.total_mb,
            "swap_mb": machine.swap_mb,
            "fits": mem_fits,
        },
        "disk": {
            "estimate_mb": out_mb,
            "free_mb": machine.disk_free_mb,
            "volume": volume.display().to_string(),
            "fits": disk_fits,
        },
        "warnings": warnings.iter().map(|w| json!({"code": w.code, "message": w.message})).collect::<Vec<_>>(),
    });
    Preflight {
        json,
        refusal,
        warnings,
        adapt_threads,
        note,
    }
}

/// A render that failed after a preflight that already said memory was short
/// gets those numbers in its error, so a generic encoder failure (on Windows an
/// allocation failure often surfaces as "Error while opening encoder") still
/// points at memory with the way out.
pub(super) fn note_memory(mut error: CutError, resources: &Value) -> CutError {
    let memory = &resources["memory"];
    if error.code != error_codes::FFMPEG || memory["fits"] != json!(false) {
        return error;
    }
    let (Some(need), Some(free)) = (
        memory["estimate_mb"].as_u64(),
        memory["available_mb"].as_u64(),
    ) else {
        return error;
    };
    error.message = format!(
        "{}; this render needed about {} of memory and about {} was free when it started",
        error.message,
        gb(need),
        gb(free)
    );
    if error.suggested_action.is_none() {
        error.suggested_action = Some(format!("Make the render lighter: {LIGHTER}."));
    }
    error
}

fn warning(code: &str, message: String, detail: Value) -> VerbWarning {
    VerbWarning {
        code: code.to_string(),
        message,
        detail: detail.as_object().cloned().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests;
