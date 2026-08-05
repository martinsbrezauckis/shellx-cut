use super::*;

#[test]
fn command_uses_motion_bin_template_root_and_preview_sets() {
    let _guard = MOTION_ENV_LOCK.blocking_lock();
    let tmp = tempfile::tempdir().unwrap();
    let package = tmp.path().join("templates").join("editable-lower-third");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("manifest.json"), "{}").unwrap();

    let _bin = EnvRestore::set(ENV_MOTION_BIN, "/opt/shellx-motion/bin/shellx-motion");
    let _template_root = EnvRestore::set(ENV_MOTION_TEMPLATE_ROOT, tmp.path().join("templates"));
    let _root = EnvRestore::remove(ENV_MOTION_ROOT);
    let _timeout = EnvRestore::set(ENV_MOTION_TIMEOUT_MS, "45000");

    let mut params = Map::new();
    params.insert("title".to_string(), json!("Launch"));
    params.insert("accentColor".to_string(), json!("#00c2ff"));
    let request = MotionTemplateRequest {
        template: "editable-lower-third".to_string(),
        params,
        policy: MotionTemplatePolicy::Preview,
        out_dir: tmp.path().join("out"),
        at_ms: 1250,
        track: "overlay-2".to_string(),
        duration_ms: Some(1800),
        dry_run_render: true,
        checkpoint: true,
        rationale: None,
        motion_job_id: Some("cut:template-preview-1".into()),
        caller_scope: tmp.path().join("workspace.cutproj"),
    };

    let spec = build_motion_template_command(&request).expect("command builds");
    assert_eq!(spec.program, "/opt/shellx-motion/bin/shellx-motion");
    assert_eq!(spec.cwd, None);
    assert_eq!(spec.timeout_ms, 45_000);
    assert!(spec.args.starts_with(&[
        "connector".to_string(),
        "template-to-cut".to_string(),
        package.display().to_string(),
        "--out".to_string(),
        request.out_dir.display().to_string(),
        "--cut-import-mode".to_string(),
        "rendered_media".to_string(),
    ]));
    assert!(spec.args.contains(&"--dry-run-render".to_string()));
    assert!(spec.args.windows(2).any(|w| w == ["--start-ms", "1250"]));
    assert!(spec.args.windows(2).any(|w| w == ["--duration-ms", "1800"]));
    assert!(spec.args.windows(2).any(|w| w == ["--track", "overlay-2"]));
    assert!(spec.args.windows(2).any(|w| w == ["--set", "title=Launch"]));
    assert!(spec
        .args
        .windows(2)
        .any(|w| w == ["--set", "accentColor=#00c2ff"]));
    assert!(spec.args.windows(2).any(|w| {
        w[0] == "--caller-id"
            && w[1] == crate::motion_runtime::motion_caller_id(&request.caller_scope)
    }));
    assert!(spec
        .args
        .windows(2)
        .any(|w| w == ["--job-id", "cut:template-preview-1"]));
}

#[test]
fn command_uses_motion_bin_for_script_to_cut_preview() {
    let _guard = MOTION_ENV_LOCK.blocking_lock();
    let tmp = tempfile::tempdir().unwrap();
    let script_path = tmp.path().join("scripted-video.json");
    fs::write(&script_path, "{}").unwrap();

    let _bin = EnvRestore::set(ENV_MOTION_BIN, "/opt/shellx-motion/bin/shellx-motion");
    let _template_root = EnvRestore::remove(ENV_MOTION_TEMPLATE_ROOT);
    let _root = EnvRestore::remove(ENV_MOTION_ROOT);
    let _timeout = EnvRestore::set(ENV_MOTION_TIMEOUT_MS, "45000");

    let request = MotionScriptRequest {
        script: None,
        script_path: Some(script_path.clone()),
        policy: MotionScriptPolicy::Preview,
        out_dir: tmp.path().join("out"),
        at_ms: 1250,
        track: "overlay-2".to_string(),
        duration_ms: Some(1800),
        dry_run_render: true,
        checkpoint: true,
        rationale: None,
        motion_job_id: Some("cut:script-preview-1".into()),
        caller_scope: tmp.path().join("workspace.cutproj"),
    };

    let spec = build_motion_script_command(&request, &script_path).expect("command builds");
    assert_eq!(spec.program, "/opt/shellx-motion/bin/shellx-motion");
    assert_eq!(spec.cwd, None);
    assert_eq!(spec.timeout_ms, 45_000);
    assert_eq!(
        spec.args,
        vec![
            "connector".to_string(),
            "script-to-cut".to_string(),
            script_path.display().to_string(),
            "--out".to_string(),
            request.out_dir.display().to_string(),
            "--cut-import-mode".to_string(),
            "rendered_media".to_string(),
            "--start-ms".to_string(),
            "1250".to_string(),
            "--track".to_string(),
            "overlay-2".to_string(),
            "--duration-ms".to_string(),
            "1800".to_string(),
            "--dry-run-render".to_string(),
            "--job-id".to_string(),
            "cut:script-preview-1".to_string(),
            "--caller-id".to_string(),
            crate::motion_runtime::motion_caller_id(&request.caller_scope),
        ]
    );
}
