use super::*;
use crate::jobs::{run_owned, ProcessControl, ProcessTermination};

#[path = "verify_handlers/adapter_result.rs"]
mod adapter_result;
mod rerun;
pub(super) use rerun::verify_rerun;

/// Resolve `receipts/<render_id>.json` — the explicit id, or the LATEST
/// render receipt when omitted (zero-padded render_NNN names → lexical max =
/// numeric max). Shared by verify.checks and verify.judge.
pub(super) fn resolve_receipt_path(
    receipts: &Path,
    render_id: Option<&str>,
) -> Result<PathBuf, CutError> {
    let path = match render_id {
        Some(id) => {
            if !safe_receipt_id(id) {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("invalid render_id '{id}'"),
                    "render_id must be a receipt id, not a path",
                )
                .with_suggested_action("use a render id such as render_001"));
            }
            receipts.join(format!("{id}.json"))
        }
        None => {
            let mut names: Vec<String> = std::fs::read_dir(receipts)?
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| {
                    n.starts_with("render_") && n.ends_with(".json") && !n.contains(".output")
                })
                .collect();
            names.sort();
            match names.last() {
                Some(n) => receipts.join(n),
                None => {
                    return Err(CutError::new(
                        error_codes::NOT_FOUND,
                        "no render receipts exist yet",
                        "verify reads the receipt render.final produces",
                    )
                    .with_suggested_action("run render.final first"))
                }
            }
        }
    };
    if !path.is_file() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("no receipt for '{}'", render_id.unwrap_or_default()),
            format!("expected {}", path.display()),
        )
        .with_suggested_action("jobs.list shows render jobs; receipts appear when one finishes"));
    }
    Ok(path)
}

fn safe_receipt_id(id: &str) -> bool {
    !id.is_empty()
        && !id.contains("..")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// verify.checks{render_id?} — return the persisted check battery of a
/// render's receipt (render.final auto-runs the battery; this verb reads the
/// evidence). No render_id → the latest receipt.
pub(super) async fn verify_checks(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        render_id: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let path = resolve_receipt_path(&store.receipts_dir(), a.render_id.as_deref())?;
    let receipt: Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    Ok(VerbResult::ok(receipt))
}

/// verify.pacing{} — on-demand pacing report for the CURRENT timeline:
/// in-editor retention/pacing critique nobody integrates). Pure structural
/// analysis of the base video track's shots (no render, no model): shot count,
/// mean/median/shortest/longest shot length, internal cuts, cuts-per-minute.
/// Advisory — the agent/editor reads it to judge pacing; never pass/fail.
pub(super) async fn verify_pacing(state: &AppState) -> Result<VerbResult, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let edl = cut_core::edl_from_project(&store.project);
    Ok(VerbResult::ok(cut_perception::pacing(&edl)))
}

/// verify.pregate{} — PRE-render predictive quality gate (a
/// "pre-render gate + refuse-to-present", made rigorous on our op-log). Reasons
/// over the OPEN project's CURRENT EDL + whatever per-asset perception reports
/// are ALREADY CACHED (loaded the same way score.clip/render.qc load them, but
/// MISSING reports are skipped rather than erroring — the gate works on whatever
/// facts exist) and PREDICTS the render problems with a known structural
/// signature, WITHOUT spending a render. Returns {pass, risks:[{kind, severity,
/// detail, range_ms?}], summary, ...}. pass=false IFF any high-severity risk.
/// HONEST: these are PREDICTIONS (structure + cached facts), not guarantees — a
/// clean pregate is "no KNOWN structural problem", a flagged risk is a strong
/// predictor (an intentional black outro trips empty_tail; a long single take
/// trips slideshow_risk). Read-only — no op, no render, no model.
pub(super) async fn verify_pregate(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    // No args today; parse to reject typos / unknown keys (the parse_args guard).
    #[derive(serde::Deserialize, Default)]
    struct Args {}
    let _a: Args = parse_args(args)?;

    // Snapshot the EDL + fps + referenced asset ids under a SHORT read lock, then
    // release it before loading the (disk-cached) perception reports — each
    // load_perception_report takes its own lock.
    let (edl, fps, asset_ids) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let edl = cut_core::edl_from_project(&store.project);
        let fps = store.project.settings.fps;
        let asset_ids: std::collections::BTreeSet<String> = edl
            .segments
            .iter()
            .filter_map(|s| s.asset.clone())
            .collect();
        (edl, fps, asset_ids)
    };

    // Load whatever perception reports are CACHED; a missing/never-analyzed asset
    // is simply absent from the map (pregate degrades to EDL-only facts for it
    // and reports it as uninstrumented — never an error, never a fake fact).
    let mut reports: std::collections::BTreeMap<String, cut_perception::PerceptionReport> =
        std::collections::BTreeMap::new();
    for id in asset_ids {
        if let Ok(rep) = speech_text::load_perception_report(state, &id).await {
            reports.insert(id, rep);
        }
    }

    let report =
        cut_perception::pregate(&edl, fps, &reports, cut_perception::PregateOpts::default());
    Ok(VerbResult::ok(serde_json::to_value(report)?))
}

/// verify.captions{max_cps?, min_duration_ms?, max_duration_ms?, min_gap_ms?,
/// max_chars?} — caption-QC receipt. Reads the caption track's cues and
/// measures each against the timed-text standards (reading speed / display
/// duration / inter-cue gap / line length); thresholds default to the
/// BBC/Netflix values and are overridable per call. Read-only measurement.
pub(super) async fn verify_captions(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        max_cps: Option<f64>,
        min_duration_ms: Option<u64>,
        max_duration_ms: Option<u64>,
        min_gap_ms: Option<u64>,
        max_chars: Option<usize>,
    }
    let a: Args = parse_args(args)?;
    let d = cut_perception::CaptionQcOpts::default();
    let opts = cut_perception::CaptionQcOpts {
        max_cps: a.max_cps.unwrap_or(d.max_cps),
        min_duration_ms: a.min_duration_ms.unwrap_or(d.min_duration_ms),
        max_duration_ms: a.max_duration_ms.unwrap_or(d.max_duration_ms),
        min_gap_ms: a.min_gap_ms.unwrap_or(d.min_gap_ms),
        max_chars: a.max_chars.unwrap_or(d.max_chars),
    };
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let cues: Vec<cut_core::CaptionClip> = store
        .project
        .tracks
        .iter()
        .filter(|t| t.kind == cut_core::TrackKind::Caption)
        .flat_map(|t| t.clips.iter())
        .filter_map(|c| match c {
            cut_core::Clip::Caption(cc) => Some(cc.clone()),
            _ => None,
        })
        .collect();
    Ok(VerbResult::ok(cut_perception::caption_qc(&cues, opts)))
}

/// verify.brand{fonts?, colors?, position?, min_size?, max_size?, aspect?} —
/// PROVE the project's stored caption styles + output
/// geometry conform to a brand spec (font allow-list, colour palette, caption
/// position, font-size bounds, output aspect). Read-only receipt; pass=true only
/// when nothing the brand pins is violated. The agent resolves against this
/// instead of overwriting brand with defaults.
pub(super) async fn verify_brand(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    #[allow(dead_code)]
    struct Args {
        fonts: Option<Vec<String>>,
        colors: Option<Vec<String>>,
        position: Option<String>,
        min_size: Option<u32>,
        max_size: Option<u32>,
        aspect: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let explicit = cut_core::BrandKit {
        fonts: a.fonts,
        colors: a.colors,
        position: a.position,
        min_size: a.min_size,
        max_size: a.max_size,
        aspect: a.aspect,
    };
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let (brand, source) = if explicit.has_constraints() {
        (
            super::brand::normalize_brand(explicit, "explicit")?,
            "explicit",
        )
    } else {
        let stored = store.project.brand.clone().ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                "no brand kit is saved for this project",
                "verify.brand received no explicit constraints and project.brand is empty",
            )
            .with_suggested_action("save a kit with project.brand or pass explicit constraints")
        })?;
        (super::brand::normalize_brand(stored, "stored")?, "stored")
    };
    Ok(VerbResult::ok(super::brand::check_project_brand(
        &store.project,
        &brand,
        source,
    )))
}

/// verify.delivery{asset?, lexicon?, min_wpm?, max_wpm?, max_fillers_per_min?,
/// pause_gap_ms?} — verbal-pacing receipt: speaking rate (WPM), filler density,
/// longest unbroken stretch, measured over the SOURCE transcript(s). Complements
/// verify.pacing (visual). `asset` narrows to one asset; omitted aggregates
/// every asset that has a transcript. Read-only measurement.
pub(super) async fn verify_delivery(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        asset: Option<String>,
        lexicon: Option<Vec<String>>,
        min_wpm: Option<f64>,
        max_wpm: Option<f64>,
        max_fillers_per_min: Option<f64>,
        pause_gap_ms: Option<u64>,
    }
    let a: Args = parse_args(args)?;
    let d = cut_perception::DeliveryOpts::default();
    let opts = cut_perception::DeliveryOpts {
        min_wpm: a.min_wpm.unwrap_or(d.min_wpm),
        max_wpm: a.max_wpm.unwrap_or(d.max_wpm),
        max_fillers_per_min: a.max_fillers_per_min.unwrap_or(d.max_fillers_per_min),
        pause_gap_ms: a.pause_gap_ms.unwrap_or(d.pause_gap_ms),
    };
    let lexicon: Vec<String> = a
        .lexicon
        .unwrap_or_else(|| speech_text::FILLERS.iter().map(|s| s.to_string()).collect())
        .iter()
        .map(|w| w.to_lowercase())
        .collect();
    // Which assets to measure: the named one, else every asset in the project.
    let asset_ids: Vec<String> = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        if let Some(id) = &a.asset {
            speech_text::check_scope(&store.project, Some(id.as_str()), None)?;
            vec![id.clone()]
        } else {
            store.project.assets.keys().cloned().collect()
        }
    };
    let mut per_asset: Vec<Vec<cut_perception::WordSpan>> = Vec::new();
    for id in &asset_ids {
        if let Ok(t) = load_transcript(state, id).await {
            per_asset.push(t.words);
        }
    }
    let slices: Vec<&[cut_perception::WordSpan]> = per_asset.iter().map(|v| v.as_slice()).collect();
    Ok(VerbResult::ok(cut_perception::delivery(
        &slices, &lexicon, opts,
    )))
}

/// verify.loudness{asset, target_lufs?=-14} — integrated-loudness RECEIPT for one
/// audio-bearing asset: integrated LUFS, true peak (dBTP), LRA, gating threshold,
/// the gap to `target_lufs`, and a one-line recommendation. The MEASURE half of the
/// loudness loop (the NORMALIZE half is render.final's `normalize_loudness`; a
/// rendered output's loudness is already checked by verify.checks/render.qc). A
/// fast ffmpeg `loudnorm` analysis pass — ffmpeg-only, no perception venv.
/// Read-only. Standard targets: -14 social, -16 long-form/podcast, -23 EBU R128.
pub(super) async fn verify_loudness(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        asset: String,
        target_lufs: Option<f64>,
    }
    let a: Args = parse_args(args)?;
    let target = a.target_lufs.unwrap_or(-14.0);
    let (src, _hash) = asset_info(state, &a.asset).await?;
    let m = run_blocking("verify.loudness", move || {
        cut_media::loudness::measure(&src)
    })
    .await?;
    // Tolerance: EBU R128 compliance is conventionally ±1 LU of the target.
    let gap = m.integrated_lufs - target; // negative = quieter than target
    let within = gap.abs() <= 1.0;
    // True peak should sit at or below -1 dBTP for safe delivery (no inter-sample
    // clipping after lossy encode). NaN (unmeasurable) → treat as a non-blocking ok.
    let tp_ok = !(m.true_peak_dbtp.is_finite() && m.true_peak_dbtp > -1.0);
    let recommendation = if within && tp_ok {
        format!(
            "ok — {:.1} LUFS is within 1 LU of the {target:.0} target",
            m.integrated_lufs
        )
    } else if !within {
        let dir = if gap < 0.0 { "quieter" } else { "louder" };
        format!(
            "render.final {{normalize_loudness: {target:.0}}} — source is {:.1} LU {dir} than the target",
            gap.abs()
        )
    } else {
        format!(
            "true peak {:.1} dBTP exceeds -1 dBTP — render.final {{normalize_loudness: {target:.0}}} (loudnorm caps TP at -1)",
            m.true_peak_dbtp
        )
    };
    Ok(VerbResult::ok(json!({
        "asset": a.asset,
        "integrated_lufs": (m.integrated_lufs * 100.0).round() / 100.0,
        "true_peak_dbtp": if m.true_peak_dbtp.is_finite() { json!((m.true_peak_dbtp * 100.0).round() / 100.0) } else { Value::Null },
        "lra": (m.lra * 100.0).round() / 100.0,
        "threshold_lufs": if m.threshold_lufs.is_finite() { json!((m.threshold_lufs * 100.0).round() / 100.0) } else { Value::Null },
        "target_lufs": target,
        "gap_lu": (gap * 100.0).round() / 100.0,
        "within_tolerance": within,
        "true_peak_ok": tp_ok,
        "recommendation": recommendation,
        "reference_targets": {"social": -14, "podcast": -16, "ebu_r128": -23},
    })))
}

/// verify.scopes{at_ms?, asset?, scope_images?, kinds?} — VIDEO-SCOPES receipt for
/// one frame: objective signal data (luma min/avg/max, clipping, broadcast-range
/// legality, saturation, white-balance cast, hue) measured with ffmpeg signalstats,
/// so the agent reads NUMBERS instead of guessing colour from a thumbnail (the
/// verify.* philosophy applied to the picture). Source: the COMPOSED timeline frame
/// at `at_ms` (default 0), or a source `asset`'s frame at `at_ms`. `scope_images:true`
/// also renders the classic scope PNGs (vectorscope/waveform/histogram — `kinds`
/// filters which) under exports/scopes/ for a human/VLM to eyeball. ffmpeg-only, no
/// perception venv. Read-only.
pub(super) async fn verify_scopes(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        at_ms: Option<u64>,
        asset: Option<String>,
        scope_images: Option<bool>,
        kinds: Option<Vec<String>>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let at_ms = a.at_ms.unwrap_or(0);
    let want_images = a.scope_images.unwrap_or(false);
    // Which scope images to render (default all three); validate names early.
    let kinds: Vec<cut_media::scopes::ScopeKind> = match &a.kinds {
        Some(ks) => ks
            .iter()
            .map(|k| {
                cut_media::scopes::ScopeKind::parse(k).ok_or_else(|| {
                    CutError::new(
                        error_codes::INVALID_ARGS,
                        format!("unknown scope kind '{k}'"),
                        "kinds: vectorscope | waveform | histogram",
                    )
                })
            })
            .collect::<Result<_, _>>()?,
        None => vec![
            cut_media::scopes::ScopeKind::Vectorscope,
            cut_media::scopes::ScopeKind::Waveform,
            cut_media::scopes::ScopeKind::Histogram,
        ],
    };
    let asset_path = match &a.asset {
        Some(id) => Some(asset_info(state, id).await?.0),
        None => None,
    };
    let (project, edl, dir, _at) = snapshot(state).await?;
    // The frame the scopes describe is written here (kept as evidence); scope images
    // land beside it. `fence_output_path` creates the literal project-relative
    // parent after rejecting links/reparse points, so do not pre-create through
    // an untrusted project component here.
    let frame_path = fence_output_path(
        &dir,
        Some(&format!("exports/scopes/frame_{at_ms}ms.jpg")),
        "exports/scopes/frame.jpg",
        OutputPathPolicy::JPEG,
    )?;
    let source_label = if asset_path.is_some() {
        "asset"
    } else {
        "timeline"
    };
    let frame_for_receipt = frame_path.clone();
    let (scopes, images) = run_blocking("verify.scopes", move || {
        // 1. Materialize the frame to analyze.
        if let Some(src) = &asset_path {
            // A source asset's frame at at_ms (seek-then-decode one frame).
            let secs = format!("{:.3}", at_ms as f64 / 1000.0);
            cut_media::ffmpeg::run_ffmpeg(&[
                "-y".into(),
                "-ss".into(),
                secs,
                "-i".into(),
                src.to_string_lossy().into_owned(),
                "-frames:v".into(),
                "1".into(),
                frame_path.to_string_lossy().into_owned(),
            ])?;
        } else {
            // The COMPOSED timeline frame at at_ms (full project geometry, raw source).
            let jpeg = cut_media::extract_frame(&project, &edl, &dir, at_ms, None)?;
            std::fs::write(&frame_path, jpeg).map_err(|e| {
                CutError::new(
                    cut_core::error_codes::FFMPEG,
                    "failed to write the scope frame",
                    e.to_string(),
                )
            })?;
        }
        // 2. Measure signal stats.
        let scopes = cut_media::scopes::measure(&frame_path)?;
        // 3. Optionally render the scope IMAGES.
        let mut images: Vec<(String, String)> = Vec::new();
        if want_images {
            for kind in &kinds {
                let out = frame_path.with_file_name(format!("scope_{at_ms}ms_{}.png", kind.key()));
                cut_media::scopes::render_scope(&frame_path, *kind, &out)?;
                images.push((kind.key().to_string(), out.to_string_lossy().into_owned()));
            }
        }
        Ok::<_, CutError>((scopes, images))
    })
    .await?;

    // Actionable flags (the agent reads these to decide a grade fix).
    let mut flags: Vec<&str> = Vec::new();
    if scopes.clip_highlights {
        flags.push("clipped_highlights");
    }
    if scopes.clip_shadows {
        flags.push("crushed_shadows");
    }
    if !scopes.broadcast_legal {
        flags.push("illegal_levels");
    }
    if scopes.white_balance != "neutral" {
        flags.push("colour_cast");
    }
    if scopes.sat_max > 118.0 {
        flags.push("over_saturated");
    }
    let pass = scopes.broadcast_legal && !scopes.clip_highlights && !scopes.clip_shadows;
    let r2 = |x: f64| (x * 100.0).round() / 100.0;
    let mut out = json!({
        "at_ms": at_ms,
        "source": source_label,
        "frame": frame_for_receipt,
        "luma": {"min": scopes.y_min, "avg": r2(scopes.y_avg), "max": scopes.y_max},
        "clipping": {"highlights": scopes.clip_highlights, "shadows": scopes.clip_shadows},
        "broadcast_legal": scopes.broadcast_legal,
        "saturation": {"avg": r2(scopes.sat_avg), "max": r2(scopes.sat_max)},
        "white_balance": {"u_avg": r2(scopes.u_avg), "v_avg": r2(scopes.v_avg), "cast": scopes.white_balance},
        "hue": {"avg": r2(scopes.hue_avg), "med": r2(scopes.hue_med)},
        "flags": flags,
        "pass": pass,
    });
    if !images.is_empty() {
        let imgs: serde_json::Map<String, Value> = images
            .into_iter()
            .map(|(k, p)| (k, Value::String(p)))
            .collect();
        out["scopes"] = Value::Object(imgs);
    }
    Ok(VerbResult::ok(out))
}

// ---------------------------------------------------------------------------
// verify.judge — bundled subscription-CLI judge adapter wire-in. The adapter
// drives the user's own coding-agent CLI as a subprocess judge: frames and
// instrument facts in, post-filtered structured verdict out, using the user's
// existing subscription rather than API keys. CUTD_JUDGE_ADAPTER is only the
// explicit operator/test override.
// ---------------------------------------------------------------------------

/// Envelope schema tag the adapter emits and the receipt stores.
const JUDGE_SCHEMA: &str = "shellx-cut/judge-review/1";

const JUDGE_ADAPTER_REL: &str = "judge/adapters/ladder_judge.py";

fn judge_adapter_from(explicit: Option<PathBuf>, sidecar_entrypoint: &Path) -> Option<PathBuf> {
    if let Some(path) = explicit {
        // An explicit override is authoritative. A typo must not silently fall
        // through to a different bundled implementation.
        return path.is_file().then_some(path);
    }
    let perception_dir = sidecar_entrypoint.parent()?;
    let bundled = perception_dir.join(JUDGE_ADAPTER_REL);
    bundled.is_file().then_some(bundled)
}

/// Locate the installed/source-tree judge adapter. Installed builds ship it
/// inside the same perception resource payload as instruments.py; source builds
/// find that payload through cut-perception's normal dev-checkout rung. The env
/// override remains the deterministic test and operator seam.
pub(super) fn find_judge_adapter() -> Option<PathBuf> {
    let explicit = std::env::var_os("CUTD_JUDGE_ADAPTER")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    let (_, sidecar_entrypoint) = cut_perception::sidecar_paths();
    judge_adapter_from(explicit, &sidecar_entrypoint)
}

#[cfg(test)]
mod judge_adapter_contract_tests {
    use super::{judge_adapter_from, validate_judge_envelope, JUDGE_SCHEMA};
    use serde_json::json;

    #[test]
    fn bundled_adapter_is_discovered_beside_perception_entrypoint() {
        let dir = tempfile::tempdir().unwrap();
        let perception = dir.path().join("perception");
        let adapter = perception.join("judge/adapters/ladder_judge.py");
        std::fs::create_dir_all(adapter.parent().unwrap()).unwrap();
        std::fs::write(perception.join("instruments.py"), b"# fixture\n").unwrap();
        std::fs::write(&adapter, b"# fixture\n").unwrap();
        assert_eq!(
            judge_adapter_from(None, &perception.join("instruments.py")),
            Some(adapter)
        );
    }

    #[test]
    fn explicit_missing_adapter_does_not_fall_through() {
        let dir = tempfile::tempdir().unwrap();
        let perception = dir.path().join("perception");
        let adapter = perception.join("judge/adapters/ladder_judge.py");
        std::fs::create_dir_all(adapter.parent().unwrap()).unwrap();
        std::fs::write(&adapter, b"# fixture\n").unwrap();
        assert_eq!(
            judge_adapter_from(
                Some(dir.path().join("operator-typo.py")),
                &perception.join("instruments.py")
            ),
            None
        );
    }

    #[test]
    fn judge_envelope_validation_rejects_fake_completed_results() {
        let valid = json!({
            "schema": JUDGE_SCHEMA,
            "status": "completed",
            "review": {
                "verdict": "pass",
                "confidence": 0.9,
                "summary": "looks coherent",
                "issues": []
            }
        });
        assert_eq!(validate_judge_envelope(&valid), Ok("completed"));

        let invalid = json!({
            "schema": JUDGE_SCHEMA,
            "status": "completed",
            "review": null
        });
        assert!(validate_judge_envelope(&invalid)
            .unwrap_err()
            .contains("no review object"));
    }
}

fn judge_cli_path() -> Option<std::ffi::OsString> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    for binary in ["claude", "codex", "agy", "grok"] {
        let Some(parent) =
            crate::gen::resolve_agent(binary).and_then(|path| path.parent().map(Path::to_path_buf))
        else {
            continue;
        };
        if !dirs.iter().any(|dir| dir == &parent) {
            dirs.push(parent);
        }
    }
    std::env::join_paths(dirs).ok()
}

/// Build the judge's EDIT INTENT block (docs/public/JUDGE_REVIEW.md §5): the ops (verb +
/// rationale) that produced this render — everything after the PREVIOUS
/// render receipt's `at_op`, up to this receipt's `at_op`. Falls back to the
/// whole log for the first render. Budgeted: most recent 40 ops (the judge
/// needs the gist, not the log — docs/public/JUDGE_REVIEW.md §4 digest discipline).
fn judge_intent(
    store: &ProjectStore,
    receipts_dir: &Path,
    receipt: &cut_core::RenderReceipt,
) -> String {
    // Previous receipt = the highest at_op strictly below this receipt's
    // (op ids are zero-padded, so lexical compare == numeric compare).
    let mut since: Option<String> = None;
    if let Ok(rd) = std::fs::read_dir(receipts_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("render_") || !name.ends_with(".json") || name.contains(".output")
            {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(e.path()) else {
                continue;
            };
            let Ok(r) = serde_json::from_str::<cut_core::RenderReceipt>(&text) else {
                continue;
            };
            if r.render_id != receipt.render_id
                && r.at_op < receipt.at_op
                && since.as_deref().is_none_or(|s| r.at_op.as_str() > s)
            {
                since = Some(r.at_op);
            }
        }
    }
    let ops = store.log.read_all().unwrap_or_default();
    let mut lines: Vec<String> = ops
        .iter()
        .filter(|o| {
            o.op_id.as_str() <= receipt.at_op.as_str()
                && since
                    .as_deref()
                    .is_none_or(|cursor| o.op_id.as_str() > cursor)
        })
        .map(|o| match &o.rationale {
            Some(r) => format!("{} {} — {}", o.op_id, o.verb, r),
            None => format!("{} {}", o.op_id, o.verb),
        })
        .collect();
    let total = lines.len();
    if total > 40 {
        lines = lines.split_off(total - 40);
        lines.insert(0, format!("(… {} earlier ops omitted)", total - 40));
    }
    format!(
        "Render {} of project '{}' at op {} (preset {}). Edit ops since the previous receipt:\n{}",
        receipt.render_id,
        store.project.name,
        receipt.at_op,
        receipt.preset,
        if lines.is_empty() {
            "(none recorded)".to_string()
        } else {
            lines.join("\n")
        }
    )
}

/// Synthesize a minimal honest judge envelope for failures the adapter never
/// got the chance to report itself (script/python missing, spawn error,
/// outer timeout, garbage output). Same schema the adapter emits, so the
/// receipt's judge slot has ONE shape (docs/public/JUDGE_REVIEW.md §7 honesty semantics:
/// `not_run` = backend unavailable, `error` = attempted and failed; `review`
/// is null either way — NEVER a fabricated verdict).
fn synth_judge_envelope(status: &str, reason: String) -> Value {
    json!({
        "schema": JUDGE_SCHEMA,
        "ts": OpRecord::now_ts(),
        "backend": {"name": "cli", "provider": "claude", "watched": false, "listened": false},
        "status": status,
        "not_run_reason": reason,
        "review": null,
    })
}

fn validate_judge_envelope(envelope: &Value) -> Result<&str, String> {
    let object = envelope
        .as_object()
        .ok_or_else(|| "judge envelope must be a JSON object".to_string())?;
    if object.get("schema").and_then(Value::as_str) != Some(JUDGE_SCHEMA) {
        return Err(format!("judge envelope schema must be {JUDGE_SCHEMA}"));
    }
    let status = object
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| "judge envelope status is missing".to_string())?;
    if !matches!(status, "completed" | "not_run" | "error") {
        return Err(format!("unsupported judge envelope status {status:?}"));
    }
    if status == "completed" {
        let review = object
            .get("review")
            .and_then(Value::as_object)
            .ok_or_else(|| "completed judge envelope has no review object".to_string())?;
        let verdict = review
            .get("verdict")
            .and_then(Value::as_str)
            .ok_or_else(|| "completed judge review has no verdict".to_string())?;
        if !matches!(verdict, "pass" | "fail" | "needs_review") {
            return Err(format!("unsupported judge verdict {verdict:?}"));
        }
        if !review.get("confidence").is_some_and(Value::is_number) {
            return Err("completed judge review has no numeric confidence".into());
        }
        if !review.get("issues").is_some_and(Value::is_array) {
            return Err("completed judge review has no issues array".into());
        }
        if !review.get("summary").is_some_and(Value::is_string) {
            return Err("completed judge review has no summary".into());
        }
    } else if object.get("review").is_some_and(|review| !review.is_null()) {
        return Err(format!("{status} judge envelope must not carry a review"));
    }
    Ok(status)
}

/// Outcome of one adapter run: the envelope to attach (always present —
/// honesty means even failures leave a structured record on the receipt) +
/// the actionable cause when the run failed hard (job → Failed).
struct JudgeRun {
    envelope: Value,
    /// Mirrors envelope.status: "completed" | "not_run" | "error".
    status: String,
    /// Set when status == "error": the cause for jobs.fail.
    fail_cause: Option<String>,
}

/// Spawn the subscription-CLI judge adapter on a render and collect its
/// envelope. Blocking-free: tokio::process + outer wall-clock timeout
/// (CUTD_JUDGE_TIMEOUT_S, default 360 s — a global review measures ~2.5 min;
/// the adapter's own inner CLI timeout is set 60 s shorter so the adapter
/// reports its timeout itself where possible). Both process streams are
/// captured into failure diagnostics because a CLI error envelope may be on
/// stdout rather than stderr.
async fn run_judge_adapter(
    adapter: Option<&Path>,
    render: &Path,
    perception: Option<&Path>,
    intent: &str,
    bundle_dir: &Path,
    provider: &str,
) -> JudgeRun {
    // Adapter path is resolved at VERB time (find_judge_adapter) and passed
    // in — env/config must be read where the verb runs, not inside the
    // detached job task (deterministic, and the test seam stays race-free).
    let Some(adapter) = adapter else {
        return JudgeRun {
            envelope: synth_judge_envelope(
                "not_run",
                "bundled judge adapter is missing or CUTD_JUDGE_ADAPTER does not point to a file — reinstall ShellX Cut or correct CUTD_JUDGE_ADAPTER; honest not_run, no backend available".into(),
            ),
            status: "not_run".into(),
            fail_cause: None,
        };
    };
    if let Err(e) = std::fs::create_dir_all(bundle_dir) {
        let cause = format!(
            "cannot create judge bundle dir {}: {e}",
            bundle_dir.display()
        );
        return JudgeRun {
            envelope: synth_judge_envelope("error", cause.clone()),
            status: "error".into(),
            fail_cause: Some(cause),
        };
    }
    let outer_s: u64 = std::env::var("CUTD_JUDGE_TIMEOUT_S")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(360);
    let inner_s = outer_s.saturating_sub(60).max(60);
    let out_file = bundle_dir.join("envelope.json");
    // The adapter imports stdlib only, but on clean macOS a bare python spawn
    // opens Apple's Command Line Tools installer prompt. Use an explicit or
    // managed runtime there; Linux/Windows may still use PATH Python.
    let Some(python) = configured_adapter_python() else {
        return JudgeRun {
            envelope: synth_judge_envelope(
                "not_run",
                format!(
                    "no adapter Python configured (set {ENV_ADAPTER_PYTHON} or install the ShellX Cut perception runtime) — honest not_run"
                ),
            ),
            status: "not_run".into(),
            fail_cause: None,
        };
    };
    let mut cmd = tokio::process::Command::new(python);
    cmd.arg(adapter)
        .arg("review")
        .arg("--provider") // judge backend: auto|claude|codex|antigravity|grok
        .arg(provider)
        .arg("--render")
        .arg(render)
        .arg("--intent")
        .arg(intent)
        .arg("--bundle-dir")
        .arg(bundle_dir)
        .arg("--keep-bundle") // bundle (frames + envelope) kept for audit
        .arg("--out")
        .arg(&out_file)
        .arg("--timeout")
        .arg(inner_s.to_string())
        .current_dir(bundle_dir)
        .stdin(std::process::Stdio::null());
    if let Some(path) = judge_cli_path() {
        // Doctor and the adapter must agree about CLIs installed outside the
        // inherited PATH (for example ~/.grok/bin or Finder-launched macOS).
        cmd.env("PATH", path);
    }
    if let Some(p) = perception {
        cmd.arg("--perception").arg(p);
    }
    let control = ProcessControl::for_operation(std::time::Duration::from_secs(outer_s));
    let output = match run_owned(&mut cmd, None, &control).await {
        Ok(output) => output,
        Err(error) if error.io_kind() == Some(std::io::ErrorKind::NotFound) => {
            return JudgeRun {
                envelope: synth_judge_envelope(
                    "not_run",
                    "adapter Python not found — the judge adapter cannot run; honest not_run"
                        .into(),
                ),
                status: "not_run".into(),
                fail_cause: None,
            };
        }
        Err(error) => {
            let cause = match error.termination() {
                Some(ProcessTermination::DeadlineExceeded) => format!(
                    "judge adapter exceeded the {outer_s}s wall-clock timeout (CUTD_JUDGE_TIMEOUT_S) and was stopped"
                ),
                Some(ProcessTermination::Cancelled(reason)) => {
                    format!("judge adapter cancelled ({})", reason.label())
                }
                None => format!("judge adapter failed: {error}"),
            };
            return JudgeRun {
                envelope: synth_judge_envelope("error", cause.clone()),
                status: "error".into(),
                fail_cause: Some(cause),
            };
        }
    };
    if !output.status.success() {
        // Exit 2 = the adapter refused the input (e.g. perception sanity
        // check failed) before sending the full review request. Preserve both streams: some
        // judge CLIs report their structured error envelope on stdout.
        let cause = format!(
            "judge adapter exit {}: {}",
            output.status.code().unwrap_or(-1),
            adapter_result::process_diagnostics(&output.stdout, &output.stderr, 800)
        );
        return JudgeRun {
            envelope: synth_judge_envelope("error", cause.clone()),
            status: "error".into(),
            fail_cause: Some(cause),
        };
    }
    // Envelope: prefer --out (stdout also carries it, but the file survives
    // any stray prints); fall back to stdout.
    let text = std::fs::read_to_string(&out_file)
        .unwrap_or_else(|_| String::from_utf8_lossy(&output.stdout).to_string());
    match serde_json::from_str::<Value>(&text) {
        Ok(envelope) => {
            let status = match validate_judge_envelope(&envelope) {
                Ok(status) => status.to_string(),
                Err(reason) => {
                    let cause = format!("judge adapter emitted an invalid envelope: {reason}");
                    return JudgeRun {
                        envelope: synth_judge_envelope("error", cause.clone()),
                        status: "error".into(),
                        fail_cause: Some(cause),
                    };
                }
            };
            if let Err(reason) = adapter_result::validate_requested_provider(&envelope, provider) {
                let cause = format!("judge adapter provider verification failed: {reason}");
                return JudgeRun {
                    envelope: synth_judge_envelope("error", cause.clone()),
                    status: "error".into(),
                    fail_cause: Some(cause),
                };
            }
            // The adapter reports its OWN failures honestly with exit 0 +
            // status error/not_run — surface error as a failed job, with the
            // adapter's reason as the cause.
            let fail_cause = (status == "error").then(|| {
                envelope
                    .get("not_run_reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("adapter reported status=error without a reason")
                    .to_string()
            });
            JudgeRun {
                envelope,
                status,
                fail_cause,
            }
        }
        Err(e) => {
            let cause = format!(
                "judge adapter emitted unparseable envelope JSON ({e}); adapter streams: {}",
                adapter_result::process_diagnostics(&output.stdout, &output.stderr, 400)
            );
            JudgeRun {
                envelope: synth_judge_envelope("error", cause.clone()),
                status: "error".into(),
                fail_cause: Some(cause),
            }
        }
    }
}

/// Attach a judge envelope to a persisted receipt, re-persist, RE-EMIT
/// `receipt_ready` with the updated receipt. Event-design decision: NO new
/// event type — agents already key on `receipt_ready` (the event-ordering contract) and the
/// judge attachment is observable as `receipt.judge` presence on the second
/// emission. `receipt.pass` stays checks-only: a judge review NEVER flips
/// instrument verdicts (docs/public/JUDGE_REVIEW.md §7 — arbitration belongs to the receipt
/// consumer; the verdict lives at receipt.judge.review.verdict).
pub(super) fn attach_judge_to_receipt(
    state: &AppState,
    receipts_dir: &Path,
    receipt_path: &Path,
    envelope: Value,
) -> Result<cut_core::RenderReceipt, CutError> {
    let receipt_path = fenced_existing_file_under_dir(
        receipts_dir,
        receipt_path,
        "render receipt",
        "use a render id from the current project's receipts directory",
    )?;
    let mut receipt: cut_core::RenderReceipt =
        serde_json::from_str(&std::fs::read_to_string(&receipt_path)?)?;
    receipt.judge = Some(envelope);
    std::fs::write(&receipt_path, serde_json::to_string_pretty(&receipt)?)?;
    state.events.publish(Event::ReceiptReady {
        receipt: receipt.clone(),
    });
    Ok(receipt)
}

/// verify.judge{render_id?, backend?} — IS a job (the background-job contract; ~2.5 min per
/// global review). Resolves the render's receipt, spawns the subscription-CLI
/// judge adapter (cli_judge.py) on the rendered file with the receipt's own
/// output-perception facts + the op-derived edit intent, and attaches the
/// judge-review envelope at `RenderReceipt.judge` (persisted + receipt_ready
/// re-emitted). Honesty: no adapter/CLI ⇒ job COMPLETES with a structured
/// `not_run` envelope attached — never an error, never a fake pass; adapter
/// failure ⇒ `error` envelope attached AND the job fails with the cause.
///
/// No op is appended: render.final itself appends none (receipts/, not
/// ops.jsonl, is the evidence store) and
/// a judge op would have to survive op-log replay. Revisit if receipts
/// become op-addressed.
pub(super) async fn verify_judge(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        render_id: Option<String>,
        backend: Option<String>,
    }
    let a: Args = parse_args(args)?;
    // Backend registry: the value selects WHICH judge adapter
    // access ladder runs, and maps to ladder_judge.py's --provider:
    //   None | "cli" | "auto" -> "auto"  : walk the ladder
    //                                       claude->codex->antigravity->grok,
    //                                       first detected CLI wins; none -> skip.
    //   "claude" | "codex" | "antigravity" | "grok" : force that rung (honest
    //                                       not_run if its CLI is absent — never
    //                                       falls through).
    // "antigravity" maps to Google's `agy` CLI; "grok" maps to Grok Build.
    // "cli" is kept as a backward-compatible alias for "auto" (the pre-ladder
    // default meant "the subscription-CLI judge"; that is now the whole ladder).
    // Unknown ids fail FAST, before a job exists.
    let provider: &str = match a.backend.as_deref() {
        None | Some("cli") | Some("auto") => "auto",
        Some(p @ ("claude" | "codex" | "antigravity" | "grok")) => p,
        Some(other) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown judge backend '{other}'"),
                "valid backends: \"auto\" (walk the ladder claude->codex->antigravity->grok; default), \"claude\", \"codex\", \"antigravity\", \"grok\"",
            )
            .with_suggested_action("omit backend, or pass auto|claude|codex|antigravity|grok"));
        }
    };
    let provider = provider.to_string();
    // Resolve everything the job needs UNDER the read lock, then release it —
    // the review takes minutes and must not block the verb loop (the background-job contract).
    let (receipts_dir, receipt_path, render_id, render_abs, perception_arg, intent, bundle_dir) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let receipts = store.receipts_dir();
        let receipt_path = resolve_receipt_path(&receipts, a.render_id.as_deref())?;
        let receipt: cut_core::RenderReceipt =
            serde_json::from_str(&std::fs::read_to_string(&receipt_path)?)?;
        let render_abs = PathBuf::from(&receipt.output_path);
        if !render_abs.is_file() {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                format!("render output missing: {}", receipt.output_path),
                "the receipt's output file moved or was deleted since render.final",
            )
            .with_suggested_action("re-run render.final, then verify.judge the new render_id"));
        }
        // The render's own instrument facts (render.final writes
        // <render_id>.output.perception.json) — passed explicitly so the
        // judge reasons in RENDER coordinates; the
        // adapter hash-verifies it. Missing → adapter auto-resolves by
        // content hash from the receipts dir / regenerates via the sidecar.
        let pcept = receipts.join(format!("{}.output.perception.json", receipt.render_id));
        let perception_arg = pcept.is_file().then_some(pcept);
        let intent = judge_intent(store, &receipts, &receipt);
        // Bundle = the CLI judge's whole working world (frames + envelope).
        // PROJECT-local, never /tmp: sandboxed CLIs may deny temp-root reads.
        // Kept for audit.
        let bundle_dir = store.dir.join(".scratch/judge").join(&receipt.render_id);
        (
            receipts,
            receipt_path,
            receipt.render_id,
            render_abs,
            perception_arg,
            intent,
            bundle_dir,
        )
    };
    // Resolve the adapter NOW (env override / repo discovery) — the job task
    // must not read process env after the verb returned (test-seam race).
    let adapter = find_judge_adapter();

    let job = state.jobs.create("judge");
    let job_id = job.job_id.clone();
    let render_id_out = render_id.clone();
    let st = state.clone();
    let jobs = state.jobs.clone();
    jobs.spawn(&job_id, async move {
        let jid = job.job_id.clone();
        st.jobs.progress(
            &jid,
            0.05,
            Some("judge: subscription-CLI review (frames + instrument facts)".into()),
        );
        let run = run_judge_adapter(
            adapter.as_deref(),
            &render_abs,
            perception_arg.as_deref(),
            &intent,
            &bundle_dir,
            &provider,
        )
        .await;
        st.jobs
            .progress(&jid, 0.9, Some("judge: attaching review to receipt".into()));
        let attached =
            attach_judge_to_receipt(&st, &receipts_dir, &receipt_path, run.envelope.clone());
        match (attached, run.fail_cause) {
            (Ok(_receipt), None) => {
                // completed | not_run — both are honest terminal results.
                let mut result = json!({
                    "render_id": render_id,
                    "status": run.status,
                    "receipt": receipt_path,
                });
                if run.status == "completed" {
                    let review = &run.envelope["review"];
                    let verdict = review["verdict"].as_str().unwrap_or("");
                    result["verdict"] = review["verdict"].clone();
                    result["confidence"] = review["confidence"].clone();
                    result["issues"] = json!(review["issues"].as_array().map_or(0, |a| a.len()));
                    // NORMALIZED gate OUTCOME (additive — the raw model `verdict`
                    // is unchanged). A render the judge FAILS is an actionable
                    // `reject`, not merely advisory: a gate / autopilot can branch
                    // on `outcome` without knowing each CLI's verdict vocabulary.
                    // Kept as a SEPARATE field because `verdict` is the model's own
                    // word (pass|fail|needs_review) and a contract field already;
                    // `outcome` is the normalized decision.
                    //   pass -> approve · fail -> reject · else -> advisory
                    let (outcome, reason) = match verdict {
                        "pass" => ("approve", "judge approved the render".to_string()),
                        "fail" => (
                            "reject",
                            review["summary"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .map(|s| format!("judge rejected the render: {s}"))
                                .unwrap_or_else(|| "judge rejected the render".to_string()),
                        ),
                        other => (
                            "advisory",
                            format!("judge returned '{other}' — needs a human look (advisory)"),
                        ),
                    };
                    result["outcome"] = json!(outcome);
                    result["outcome_reason"] = json!(reason);
                } else {
                    result["reason"] = run.envelope["not_run_reason"].clone();
                    // No judge ran ⇒ cannot approve or reject ⇒ advisory.
                    result["outcome"] = json!("advisory");
                    result["outcome_reason"] =
                        json!("no judge available — render NOT reviewed (advisory)");
                }
                st.jobs.finish(&jid, result);
            }
            (Ok(_receipt), Some(cause)) => {
                // Attempted and failed: the error envelope IS on the receipt
                // (honest history); the job fails with the actionable cause.
                st.jobs.fail(
                    &jid,
                    CutError::new(error_codes::JOB_FAILED, "judge adapter failed", cause)
                        .with_suggested_action(
                            "receipt.judge.not_run_reason has the full reason; check `claude` CLI login + python3, then re-run verify.judge",
                        ),
                );
            }
            (Err(e), cause) => {
                // Receipt update itself failed — that error wins (the review,
                // if any, is preserved in the bundle's envelope.json).
                let e = match cause {
                    Some(c) => e.with_suggested_action(format!(
                        "adapter also failed: {c}; bundle kept at {}",
                        bundle_dir.display()
                    )),
                    None => e.with_suggested_action(format!(
                        "review envelope preserved at {}/envelope.json",
                        bundle_dir.display()
                    )),
                };
                st.jobs.fail(&jid, e);
            }
        }
    });
    Ok(VerbResult::ok(
        json!({"job_id": job_id, "render_id": render_id_out}),
    ))
}
