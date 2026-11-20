//! The encoder a render really used, as the receipt and the render job result
//! record it.
//!
//! A render that asks for hardware encoding (`hardware:"auto"`, the default)
//! quietly becomes a software render when the machine has no usable hardware
//! encoder. The output is fine, but the person and the agent could not tell which
//! encoder made it or why. This module builds the one record both read.

use cut_core::EncodeReport;
use cut_media::hwencode::SoftwareFallback;

/// Said when hardware was requested, software ran, and no cause was recorded.
/// Never left empty: a software result that was asked to be hardware always
/// explains itself.
const GENERIC_FALLBACK: &str =
    "No usable hardware encoder was found, so this export used software encoding.";

/// ffmpeg's hardware encoders end in the name of their backend.
fn is_hardware_encoder(encoder: &str) -> bool {
    ["_nvenc", "_qsv", "_amf", "_videotoolbox"]
        .iter()
        .any(|suffix| encoder.ends_with(suffix))
}

/// Build the record for `encoder`, the video encoder that really ran.
///
/// `hardware_requested` is whether the request allowed hardware (anything but
/// `hardware:"off"`); `fallback` is why the hardware tier was not chosen, when it
/// was not.
pub(super) fn encode_report(
    encoder: &str,
    hardware_requested: bool,
    fallback: Option<&SoftwareFallback>,
) -> EncodeReport {
    let hardware_used = is_hardware_encoder(encoder);
    let explain = hardware_requested && !hardware_used;
    let (fallback_reason, fallback_detail) = if explain {
        match fallback {
            Some(f) => (Some(f.message.clone()), f.detail.clone()),
            None => (Some(GENERIC_FALLBACK.to_string()), None),
        }
    } else {
        (None, None)
    };
    EncodeReport {
        encoder: encoder.to_string(),
        hardware_requested,
        hardware_used,
        fallback_reason,
        fallback_detail,
        color: None,
        memory_note: None,
        encoder_threads: None,
        pipeline: None,
        pipeline_reason: None,
    }
}

/// Said when the GPU render path was asked for, the standard path ran and the
/// engine named no reason (never left empty).
const GENERIC_PIPELINE: &str = "The GPU render path was asked for (SHELLX_CUT_RENDER_GPU), but \
     this export used the standard render path.";

/// The render path fields: set only when the GPU render path was asked for.
/// `ran` is the media layer's `RenderOutput::pipeline` (`Some("gpu")` when the
/// CUDA graph made the file); `refusal` is why it was not used.
pub(super) fn pipeline_fields(
    requested: bool,
    ran: Option<&str>,
    refusal: Option<String>,
) -> (Option<String>, Option<String>) {
    match (requested, ran) {
        (_, Some("gpu")) => (Some("gpu".into()), None),
        (true, _) => (
            Some("standard".into()),
            Some(refusal.unwrap_or_else(|| GENERIC_PIPELINE.to_string())),
        ),
        (false, _) => (None, None),
    }
}

/// The software encoder a preset's video arguments name (`-c:v`), for the
/// renders that never ask for hardware (render.bundle, render.reframe).
pub(super) fn software_encoder(video_args: &[String]) -> String {
    video_args
        .iter()
        .position(|arg| arg == "-c:v")
        .and_then(|index| video_args.get(index + 1))
        .cloned()
        .unwrap_or_else(|| "libx264".to_string())
}

/// The encoder that produced the file. The GPU fast-track encodes with NVENC
/// whatever encoder the request resolved to, so its output is named for that.
pub(super) fn encoder_that_ran(resolved: &str, pipeline: Option<&str>) -> String {
    if pipeline == Some("gpu") {
        "h264_nvenc".to_string()
    } else {
        resolved.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fallback() -> SoftwareFallback {
        SoftwareFallback {
            message: "No usable hardware encoder was found, so this export used software \
                      encoding. The graphics driver is too old for the hardware encoder in \
                      this ffmpeg. Update the graphics driver, then restart ShellX Cut."
                .into(),
            detail: Some("h264_nvenc: Required: 13.1 Found: 13.0".into()),
        }
    }

    #[test]
    fn software_after_a_hardware_request_says_why() {
        let report = encode_report("libx264", true, Some(&fallback()));
        assert_eq!(report.encoder, "libx264");
        assert!(report.hardware_requested);
        assert!(!report.hardware_used);
        assert!(report
            .fallback_reason
            .as_deref()
            .is_some_and(|r| r.starts_with("No usable hardware encoder was found")));
        assert!(report
            .fallback_detail
            .as_deref()
            .is_some_and(|d| d.contains("Found: 13.0")));
    }

    #[test]
    fn a_missing_cause_still_gets_the_plain_sentence() {
        let report = encode_report("libx264", true, None);
        assert_eq!(report.fallback_reason.as_deref(), Some(GENERIC_FALLBACK));
    }

    #[test]
    fn hardware_off_is_not_a_fallback() {
        let report = encode_report("libx264", false, Some(&fallback()));
        assert!(!report.hardware_requested);
        assert!(!report.hardware_used);
        assert_eq!(report.fallback_reason, None);
        assert_eq!(report.fallback_detail, None);
    }

    #[test]
    fn a_hardware_encoder_that_ran_names_itself_without_a_reason() {
        for name in ["h264_nvenc", "hevc_qsv", "av1_amf", "h264_videotoolbox"] {
            let report = encode_report(name, true, Some(&fallback()));
            assert!(report.hardware_used, "{name}");
            assert_eq!(report.fallback_reason, None, "{name}");
        }
        assert!(!is_hardware_encoder("libx264"));
        assert!(!is_hardware_encoder("libsvtav1"));
        assert!(!is_hardware_encoder("prores_ks"));
    }

    #[test]
    fn the_gpu_pipeline_is_named_for_its_encoder() {
        assert_eq!(encoder_that_ran("libx264", Some("gpu")), "h264_nvenc");
        assert_eq!(encoder_that_ran("libx264", None), "libx264");
    }

    /// The plan a person or agent gets before encoding already names the encoder
    /// and, when hardware is asked for and software would run, says why. Runs on
    /// any machine: with a hardware encoder the plan names it, without one the
    /// plan carries the reason. `hardware:"off"` is never reported as a fallback.
    #[test]
    fn dry_run_plan_names_the_encoder_and_any_fallback() {
        use super::super::*;
        let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let actor = || Actor {
                kind: cut_core::ActorKind::Agent,
                name: "encode-report-test".into(),
                via: "test".into(),
                request: None,
            };
            let dir = tempfile::tempdir().unwrap();
            let state = AppState::new();
            let created = dispatch(
                &state,
                "project.create",
                json!({"name":"t","dir": dir.path().join("t.cutproj")}),
                actor(),
            )
            .await;
            assert!(created.ok, "{:?}", created.error);
            let media_file = dir.path().join("clip.mp4");
            std::fs::write(&media_file, b"x").unwrap();
            dispatch(&state, "media.import", json!({"path": media_file}), actor()).await;
            crate::dispatch::media::update_asset(&state, "a1", |a| {
                a.probe = Some(
                    json!({"kind":"video","width":1920,"height":1080,"duration_ms":5000,"has_audio":true}),
                );
            })
            .await
            .unwrap();
            dispatch(
                &state,
                "edit.insert",
                json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,5000],"ripple":false}),
                actor(),
            )
            .await;

            let auto = dispatch(&state, "render.final", json!({"dry_run": true}), actor()).await;
            assert!(auto.ok, "{:?}", auto.error);
            let plan = auto.result.unwrap();
            let encode = &plan["encode"];
            assert!(
                encode["encoder"].as_str().is_some_and(|e| !e.is_empty()),
                "the plan must name the encoder: {plan}"
            );
            assert_eq!(encode["hardware_requested"], json!(true));
            if encode["hardware_used"] == json!(false) {
                assert!(
                    encode["fallback_reason"]
                        .as_str()
                        .is_some_and(|r| r.contains("software encoding")),
                    "software after a hardware request must say why: {plan}"
                );
            } else {
                assert!(encode.get("fallback_reason").is_none(), "{plan}");
            }

            let off = dispatch(
                &state,
                "render.final",
                json!({"dry_run": true, "hardware": "off"}),
                actor(),
            )
            .await;
            assert!(off.ok, "{:?}", off.error);
            let encode = &off.result.unwrap()["encode"];
            assert_eq!(encode["encoder"], json!("libx264"));
            assert_eq!(encode["hardware_requested"], json!(false));
            assert_eq!(encode["hardware_used"], json!(false));
            assert!(encode.get("fallback_reason").is_none());

            // A format with no hardware tier names that, not a driver problem.
            let prores = dispatch(
                &state,
                "render.final",
                json!({"dry_run": true, "format": "prores"}),
                actor(),
            )
            .await;
            assert!(prores.ok, "{:?}", prores.error);
            let encode = &prores.result.unwrap()["encode"];
            assert_eq!(encode["hardware_used"], json!(false));
            assert!(encode["fallback_reason"]
                .as_str()
                .is_some_and(|r| r.contains("This format has no hardware encoder")));
        });
    }

    /// render.bundle and render.reframe never ask for hardware. Their receipts
    /// carry the same record as render.final and say so: software, not requested.
    #[test]
    fn bundle_receipt_on_disk_names_the_software_encoder() {
        let dir = tempfile::tempdir().unwrap();
        let facts = cut_perception::RenderFacts {
            duration_ms: 1000,
            loudness: None,
            output_report: None,
            streams: Vec::new(),
        };
        let output = cut_media::render::RenderOutput {
            path: dir.path().join("clip.mp4"),
            hash: "sha256:x".into(),
            duration_ms: 1000,
            preset: "standard".into(),
            width: None,
            height: None,
            fit: None,
            pipeline: None,
        };
        let encode = encode_report(
            &software_encoder(&cut_media::render::RenderPreset::default().video_args),
            false,
            None,
        );
        let receipt = super::super::compute_bundle_receipt(
            &facts,
            &output,
            "bundle_0_1000_9x16",
            "op_1",
            -14,
            dir.path(),
            encode,
        )
        .unwrap();
        assert_eq!(receipt.encode.as_ref().unwrap().encoder, "libx264");
        let text = std::fs::read_to_string(dir.path().join("bundle_0_1000_9x16.json")).unwrap();
        let on_disk: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(on_disk["encode"]["encoder"], "libx264");
        assert_eq!(on_disk["encode"]["hardware_requested"], false);
        assert_eq!(on_disk["encode"]["hardware_used"], false);
        assert!(on_disk["encode"].get("fallback_reason").is_none());
    }

    #[test]
    fn reframe_receipt_record_is_software_and_not_requested() {
        let record = super::super::reframe_encode_record();
        assert_eq!(record.encoder, "libx264");
        assert!(!record.hardware_requested && !record.hardware_used);
        assert_eq!(record.fallback_reason, None);
    }

    #[test]
    fn an_unused_gpu_render_path_says_why() {
        assert_eq!(pipeline_fields(false, None, None), (None, None));
        assert_eq!(
            pipeline_fields(true, Some("gpu"), None),
            (Some("gpu".into()), None)
        );
        let why = "The GPU render path was asked for (SHELLX_CUT_RENDER_GPU), but the timeline \
                   has captions, which the GPU render path cannot draw, so this export used the \
                   standard render path.";
        assert_eq!(
            pipeline_fields(true, None, Some(why.into())),
            (Some("standard".into()), Some(why.into()))
        );
        let (ran, reason) = pipeline_fields(true, None, None);
        assert_eq!(ran.as_deref(), Some("standard"));
        assert!(reason.unwrap().contains("standard render path"));
    }
}
