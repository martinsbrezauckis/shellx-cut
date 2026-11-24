//! Gathers what the engine knows about every export check and hands it to the
//! shared projection (`cut_perception::export_check_list`). Reads only: no
//! analysis is started, nothing is installed, no judge is called.

use super::on_request_cache::{self, LoadedOnRequestCaches};
use super::*;
use cut_core::export_checks::{ExportCheckFinding, ExportCheckPhase, ExportChecks};
use cut_perception::export_check_list as list;
use std::collections::BTreeMap;

pub(crate) struct Inputs<'a> {
    pub project: &'a cut_core::Project,
    pub edl: &'a cut_core::Edl,
    pub receipts: &'a Path,
    pub at_op: String,
    pub phase: ExportCheckPhase,
    pub render_id: Option<String>,
    pub pregate: Option<&'a cut_perception::PregateReport>,
    pub before_snapshot: Option<&'a ExportChecks>,
    pub receipt_checks: &'a [cut_core::CheckResult],
    /// Placed sources the engine could not probe for this export.
    pub source_errors: &'a BTreeMap<String, CutError>,
    pub memory: list::ResourceFact,
    pub disk: list::ResourceFact,
}

pub(crate) fn current_at_op(store: &cut_core::ProjectStore) -> String {
    store
        .log
        .read_all()
        .ok()
        .and_then(|ops| ops.last().map(|o| o.op_id.clone()))
        .unwrap_or_else(|| "op_000000".into())
}

fn report_evidence(project: &cut_core::Project, receipts: &Path, id: &str) -> list::ReportEvidence {
    let Some(asset) = project.assets.get(id) else {
        return list::ReportEvidence::Missing;
    };
    let Some(rel) = asset.perception.as_ref() else {
        return list::ReportEvidence::Missing;
    };
    if rel != &format!("receipts/{id}.perception.json") {
        return list::ReportEvidence::Unreadable(Box::new(
            CutError::new(
                error_codes::INVALID_ARGS,
                format!("asset '{id}' has an invalid perception receipt pointer"),
                "the cached report path must name this asset's project-local receipt",
            )
            .with_suggested_action("re-run media.perception for this asset"),
        ));
    }
    match cut_perception::load_report(receipts, id) {
        Ok(Some(report)) => list::ReportEvidence::Loaded {
            instruments_run: report.instruments_run,
            attempt: report.analysis_attempt.map(Box::new),
        },
        Ok(None) => list::ReportEvidence::Unreadable(Box::new(
            CutError::new(
                error_codes::NOT_FOUND,
                format!("asset '{id}' perception report is missing"),
                "the project's derived perception receipt is missing",
            )
            .with_suggested_action("re-run media.perception for this asset"),
        )),
        Err(error) => list::ReportEvidence::Unreadable(Box::new(error)),
    }
}

fn probe_flag(asset: &cut_core::Asset, key: &str) -> Option<bool> {
    asset
        .probe
        .as_ref()
        .and_then(|p| p.get(key))
        .and_then(Value::as_bool)
}

fn asset_evidence(project: &cut_core::Project, receipts: &Path, id: &str) -> list::AssetEvidence {
    let asset = project.assets.get(id);
    let kind = asset
        .and_then(|a| a.probe.as_ref())
        .and_then(|p| p.get("kind"))
        .and_then(Value::as_str);
    // Unknown facts waive nothing: both kinds apply unless the cached kind says
    // the file has no picture.
    let has_video = !matches!(kind, Some("audio"));
    let has_audio = match kind {
        Some("image") => false,
        _ => asset
            .and_then(|a| probe_flag(a, "has_audio"))
            .unwrap_or(true),
    };
    let attempts = asset
        .and_then(|a| cut_perception::analysis_attempts::load_attempts(receipts, id, &a.hash).ok())
        .unwrap_or_default();
    list::AssetEvidence {
        id: id.to_string(),
        has_video,
        has_audio,
        report: report_evidence(project, receipts, id),
        attempts,
    }
}

fn finding(reason: &str, evidence: Value) -> ExportCheckFinding {
    ExportCheckFinding {
        reason: reason.into(),
        asset_id: None,
        clip_id: None,
        range_ms: None,
        evidence,
    }
}

fn direct_results(project: &cut_core::Project) -> BTreeMap<String, list::DirectResult> {
    let mut out = BTreeMap::new();
    let cues: Vec<cut_core::CaptionClip> = project
        .tracks
        .iter()
        .filter(|t| t.kind == cut_core::TrackKind::Caption)
        .flat_map(|t| t.clips.iter())
        .filter_map(|c| match c {
            cut_core::Clip::Caption(cc) => Some(cc.clone()),
            _ => None,
        })
        .collect();
    let qc = if cues.is_empty() {
        list::DirectResult::NotApplicable("The project has no caption cues.".into())
    } else {
        let result = cut_perception::caption_qc(&cues, cut_perception::CaptionQcOpts::default());
        if result.get("pass").and_then(Value::as_bool) == Some(true) {
            list::DirectResult::Clear
        } else {
            list::DirectResult::Found(vec![finding(
                "Some captions are too fast, too short, too long or too close together.",
                result.get("violations").cloned().unwrap_or(Value::Null),
            )])
        }
    };
    out.insert("caption_qc".to_string(), qc);
    let brand = match project.brand.as_ref().filter(|b| b.has_constraints()) {
        None => {
            list::DirectResult::NotApplicable("No brand rules are saved for this project.".into())
        }
        Some(kit) => match super::super::brand::normalize_brand(kit.clone(), "stored") {
            Err(error) => list::DirectResult::Failed(error),
            Ok(kit) => {
                let result = super::super::brand::check_project_brand(project, &kit, "stored");
                if result.get("pass").and_then(Value::as_bool) == Some(true) {
                    list::DirectResult::Clear
                } else {
                    list::DirectResult::Found(vec![finding(
                        "The project does not follow its saved brand rules.",
                        result,
                    )])
                }
            }
        },
    };
    out.insert("brand".to_string(), brand);
    out
}

fn on_request_results(
    project: &cut_core::Project,
    edl: &cut_core::Edl,
    caches: &LoadedOnRequestCaches,
    at_op: &str,
) -> BTreeMap<String, list::OnRequestEvidence> {
    let mut out = BTreeMap::new();
    let asset_ids: Vec<String> = {
        let mut ids: Vec<String> = edl
            .segments
            .iter()
            .filter_map(|s| s.asset.clone())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    };
    // Picture levels: a result from an earlier version of the timeline does not
    // describe this one.
    let scopes = caches.signal_scopes.as_ref().filter(|c| c.at_op == at_op);
    out.insert(
        "signal_scopes".into(),
        list::OnRequestEvidence {
            fact: scopes.map(|c| match &c.error {
                Some(error) => list::OnRequestFact::Failed {
                    error: error.clone(),
                    at_op: c.at_op.clone(),
                },
                None => list::OnRequestFact::Ran {
                    findings: c
                        .flags
                        .iter()
                        .map(|flag| {
                            finding(&format!("Picture levels: {flag}."), json!({"flag": flag}))
                        })
                        .collect(),
                    at_op: c.at_op.clone(),
                },
            }),
            command: (edl.duration_ms > 0).then(|| {
                list::command_action("verify.scopes", json!({ "at_ms": edl.duration_ms / 2 }))
            }),
        },
    );
    // Source levels: per placed source file, kept while the file is the same.
    let current: Vec<&on_request_cache::SourceLoudnessEntry> = asset_ids
        .iter()
        .filter_map(|id| {
            let entry = caches.source_loudness.assets.get(id)?;
            let hash = &project.assets.get(id)?.hash;
            (&entry.asset_hash == hash).then_some(entry)
        })
        .collect();
    let first_error = current.iter().find_map(|e| e.error.clone());
    let problems: Vec<ExportCheckFinding> = current
        .iter()
        .filter(|e| e.error.is_none() && (!e.within_tolerance || !e.true_peak_ok))
        .map(|e| {
            let mut f = finding(
                if e.recommendation.is_empty() {
                    "The source sound level is off target."
                } else {
                    &e.recommendation
                },
                json!({"integrated_lufs": e.integrated_lufs, "true_peak_dbtp": e.true_peak_dbtp, "target_lufs": e.target_lufs}),
            );
            f.asset_id = Some(e.asset_id.clone());
            f
        })
        .collect();
    let loudness_fact = if let Some(error) = first_error {
        Some(list::OnRequestFact::Failed {
            error,
            at_op: at_op.into(),
        })
    } else if (current.len() == asset_ids.len() && !current.is_empty()) || !problems.is_empty() {
        Some(list::OnRequestFact::Ran {
            findings: problems,
            at_op: at_op.into(),
        })
    } else {
        None
    };
    let next_asset = asset_ids
        .iter()
        .find(|id| !current.iter().any(|e| &e.asset_id == *id))
        .or_else(|| asset_ids.first());
    out.insert(
        "source_loudness".into(),
        list::OnRequestEvidence {
            fact: loudness_fact,
            command: next_asset
                .map(|id| list::command_action("verify.loudness", json!({ "asset": id }))),
        },
    );
    let delivery = caches.delivery.as_ref().filter(|c| c.at_op == at_op);
    out.insert(
        "delivery".into(),
        list::OnRequestEvidence {
            fact: delivery.map(|c| match &c.error {
                Some(error) => list::OnRequestFact::Failed {
                    error: error.clone(),
                    at_op: c.at_op.clone(),
                },
                None if c.pass => list::OnRequestFact::Ran {
                    findings: vec![],
                    at_op: c.at_op.clone(),
                },
                None => list::OnRequestFact::Ran {
                    findings: vec![finding(
                        "The speaking pace or filler words need a look.",
                        c.flags.clone(),
                    )],
                    at_op: c.at_op.clone(),
                },
            }),
            command: Some(list::command_action("verify.delivery", json!({}))),
        },
    );
    out.insert(
        "reframe_composition".into(),
        list::OnRequestEvidence {
            fact: caches.reframe_qc.as_ref().filter(|c| c.at_op == at_op).map(|c| match &c.error {
                Some(error) => list::OnRequestFact::Failed {
                    error: error.clone(),
                    at_op: c.at_op.clone(),
                },
                None => list::OnRequestFact::Ran {
                    findings: if c.review_count > 0 {
                        vec![finding(
                            "Some scenes need a look at the subject framing.",
                            json!({"review_count": c.review_count, "scene_count": c.scene_count, "reframe_id": c.reframe_id}),
                        )]
                    } else {
                        vec![]
                    },
                    at_op: c.at_op.clone(),
                },
            }),
            // render.qc needs the id of a reframed export; none is known here.
            command: None,
        },
    );
    // Picture review needs the person's consent and a finished file; picture
    // size has no standalone command. Neither gets a button.
    out.insert("judge".into(), list::OnRequestEvidence::default());
    out.insert("output_geometry".into(), list::OnRequestEvidence::default());
    out
}

pub(crate) fn captions_tool() -> (list::ToolFact, Option<String>) {
    let install = cut_perception::captions_installation::inspect_captions_installation();
    (
        list::Tools::from_installation(&install),
        install.installation_id,
    )
}

pub(crate) fn collect(input: Inputs<'_>) -> ExportChecks {
    let mut ids: Vec<String> = input
        .edl
        .segments
        .iter()
        .filter_map(|s| s.asset.clone())
        .collect();
    ids.sort();
    ids.dedup();
    let assets: Vec<list::AssetEvidence> = ids
        .iter()
        .map(|id| asset_evidence(input.project, input.receipts, id))
        .collect();
    let timeline = list::TimelineFacts {
        has_picture: input
            .edl
            .segments
            .iter()
            .any(|s| s.track_kind == cut_core::TrackKind::Video && s.asset.is_some()),
        media_clip_count: input
            .edl
            .segments
            .iter()
            .filter(|s| s.asset.is_some())
            .count(),
    };
    let caches = on_request_cache::load_all(input.receipts);
    let on_request = on_request_results(input.project, input.edl, &caches, &input.at_op);
    let direct = direct_results(input.project);
    let (captions, installation_id) = captions_tool();
    let tools = list::Tools {
        captions,
        captions_installation_id: installation_id,
        ffmpeg: list::ToolFact::unknown("not probed for this list"),
        ffprobe: list::ToolFact::unknown("not probed for this list"),
        judge: list::ToolFact::unknown("only read when you ask for a review"),
    };
    list::build_export_checks(&list::ListInput {
        phase: input.phase,
        at_op: input.at_op,
        render_id: input.render_id,
        env: list::Env {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        },
        tools,
        timeline,
        assets: &assets,
        pregate: input.pregate,
        before_snapshot: input.before_snapshot,
        receipt_checks: input.receipt_checks,
        source_errors: input.source_errors,
        direct: &direct,
        on_request: &on_request,
        memory: input.memory,
        disk: input.disk,
    })
}

/// The same pregate the person saw before the export, over the exported
/// version of the project (cached reports only; nothing is analysed).
fn pregate_snapshot(
    project: &cut_core::Project,
    edl: &cut_core::Edl,
    receipts: &Path,
) -> cut_perception::PregateReport {
    let mut reports = BTreeMap::new();
    for id in edl.segments.iter().filter_map(|s| s.asset.clone()) {
        if let list::ReportEvidence::Loaded { .. } = report_evidence(project, receipts, &id) {
            if let Ok(Some(report)) = cut_perception::load_report(receipts, &id) {
                reports.insert(id, report);
            }
        }
    }
    cut_perception::pregate(
        edl,
        project.settings.fps,
        &reports,
        cut_perception::PregateOpts::default(),
    )
}

/// Memory and disk as the render plan estimated them when the export started.
pub(crate) fn resource_facts(resources: &Value) -> (list::ResourceFact, list::ResourceFact) {
    let from = |part: &str, code_prefix: &str| {
        let findings: Vec<ExportCheckFinding> = resources["warnings"]
            .as_array()
            .map(|w| {
                w.iter()
                    .filter(|w| {
                        w["code"]
                            .as_str()
                            .is_some_and(|c| c.starts_with(code_prefix))
                    })
                    .map(|w| {
                        finding(
                            w["message"].as_str().unwrap_or("The estimate is short."),
                            resources[part].clone(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !resources[part].is_object() {
            list::ResourceFact::NotEstimated
        } else if findings.is_empty() {
            list::ResourceFact::Clear
        } else {
            list::ResourceFact::Found(findings)
        }
    };
    (from("memory", "memory"), from("disk", "disk"))
}

/// The list for a finished export: checks that describe the export input are
/// those of the exported version (evaluated once, with the plan's resource
/// estimate), the others come from the receipt's own check rows.
pub(crate) fn finished(
    project: &cut_core::Project,
    edl: &cut_core::Edl,
    receipts: &Path,
    at_op: &str,
    render_id: &str,
    receipt_checks: &[cut_core::CheckResult],
    source_errors: &BTreeMap<String, CutError>,
    resources: &Value,
) -> ExportChecks {
    let pregate = pregate_snapshot(project, edl, receipts);
    let (memory, disk) = resource_facts(resources);
    let before = collect(Inputs {
        project,
        edl,
        receipts,
        at_op: at_op.to_string(),
        phase: ExportCheckPhase::BeforeExport,
        render_id: Some(render_id.to_string()),
        pregate: Some(&pregate),
        before_snapshot: None,
        receipt_checks: &[],
        source_errors,
        memory: memory.clone(),
        disk: disk.clone(),
    });
    collect(Inputs {
        project,
        edl,
        receipts,
        at_op: at_op.to_string(),
        phase: ExportCheckPhase::FinishedFile,
        render_id: Some(render_id.to_string()),
        pregate: Some(&pregate),
        before_snapshot: Some(&before),
        receipt_checks,
        source_errors,
        memory,
        disk,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_asset() -> cut_core::Project {
        let mut p = cut_core::Project::new("t", Default::default());
        p.assets.insert(
            "a1".into(),
            cut_core::Asset {
                path: "a1.mp4".into(),
                hash: "sha256:aa".into(),
                probe: None,
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        p
    }

    #[test]
    fn an_asset_with_no_report_is_missing_not_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let p = project_with_asset();
        assert!(matches!(
            report_evidence(&p, dir.path(), "a1"),
            list::ReportEvidence::Missing
        ));
    }

    #[test]
    fn a_pointer_to_a_missing_receipt_is_unreadable_with_its_reason() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = project_with_asset();
        p.assets.get_mut("a1").unwrap().perception = Some("receipts/a1.perception.json".into());
        match report_evidence(&p, dir.path(), "a1") {
            list::ReportEvidence::Unreadable(e) => assert_eq!(e.code, error_codes::NOT_FOUND),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn audio_files_have_no_picture_and_unknown_facts_waive_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = project_with_asset();
        let unknown = asset_evidence(&p, dir.path(), "a1");
        assert!(unknown.has_video && unknown.has_audio);
        p.assets.get_mut("a1").unwrap().probe = Some(json!({"kind": "audio"}));
        let audio = asset_evidence(&p, dir.path(), "a1");
        assert!(!audio.has_video && audio.has_audio);
    }

    #[test]
    fn caption_timing_is_not_applicable_without_cues() {
        let p = project_with_asset();
        let direct = direct_results(&p);
        assert!(matches!(
            direct["caption_qc"],
            list::DirectResult::NotApplicable(_)
        ));
        assert!(matches!(
            direct["brand"],
            list::DirectResult::NotApplicable(_)
        ));
    }
}
