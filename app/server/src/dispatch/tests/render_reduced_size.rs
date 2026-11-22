use super::*;

fn reduced_test_rgb(path: &Path) -> Vec<u8> {
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-threads", "1", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.len(), 640 * 360 * 3);
    output.stdout
}

#[test]
fn reduced_composed_frame_conforms_proxy_before_gap_concat() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let root = tempfile::tempdir().unwrap();
                    let state = AppState::new();
                    let source = root.path().join("orange.mp4");
                    assert!(std::process::Command::new("ffmpeg")
                        .args(["-v", "error", "-f", "lavfi", "-i",
                            "color=c=orange:s=640x360:r=30:d=1", "-pix_fmt", "yuv420p", "-y"])
                        .arg(&source).status().unwrap().success());
                    for (verb, args) in [
                        ("project.create", json!({"name":"reduced", "dir":root.path().join("reduced.cutproj")})),
                        ("media.import", json!({"path":source})),
                    ] {
                        let r = dispatch(&state, verb, args, test_actor()).await;
                        assert!(r.ok, "{verb}: {:?}", r.error);
                    }
                    let mut ready = false;
                    for _ in 0..600 {
                        {
                            let g = state.project.read().await;
                            let p = &g.as_ref().unwrap().project;
                            ready = p.assets.get("a1").is_some_and(|a| a.proxy.is_some())
                                && p.find_clip("c1").is_some();
                        }
                        if ready { break; }
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    assert!(ready, "import must produce a real proxy and auto-placed clip");
                    let moved = dispatch(&state, "edit.move",
                        json!({"clip":"c1", "to_track":"v1", "at_ms":4000}), test_actor()).await;
                    assert!(moved.ok, "{:?}", moved.error);
                    let format = dispatch(&state, "project.format",
                        json!({"width":1920,"height":1080}), test_actor()).await;
                    assert!(format.ok, "{:?}", format.error);
                    {
                        let g = state.project.read().await;
                        let p = &g.as_ref().unwrap().project;
                        let edl = cut_core::edl_from_project(p);
                        assert!(edl.duration_ms >= 5000);
                    }
                    std::fs::create_dir_all(root.path().join("reduced.cutproj/exports")).unwrap();
                    let final_render = dispatch(&state, "render.final", json!({
                        "width":640,"height":360,"preset":"draft",
                        "path":root.path().join("reduced.cutproj/exports/reduced.mp4")
                    }), test_actor()).await;
                    assert!(final_render.ok, "final: {:?}", final_render.error);
                    let job_id = final_render.result.unwrap()["job_id"].as_str().unwrap().to_string();
                    let mut finished = false;
                    for _ in 0..1200 {
                        let job = state.jobs.get(&job_id).unwrap();
                        assert_ne!(job.state, crate::jobs::JobState::Failed, "{:?}", job.error);
                        if job.state == crate::jobs::JobState::Done { finished = true; break; }
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    assert!(finished, "reduced final render timed out");
                    eprintln!("reduced render.final: PASS");
                    for draft in [false, true] {
                        let preview = dispatch(&state, "render.preview", json!({"at_ms":0,"duration_ms":5000,"draft":draft}), test_actor()).await;
                        assert!(preview.ok, "preview draft={draft}: {:?}", preview.error);
                        eprintln!("render.preview draft={draft}: PASS");
                    }
                    let h = std::env::var("REDUCED_TEST_HEIGHT").ok()
                        .map(|v| v.parse::<u32>().unwrap()).unwrap_or(360);
                    let frame = dispatch(&state, "render.frame",
                        json!({"at_ms":4500, "compose":true, "h":h}), test_actor()).await;
                    assert!(frame.ok, "{:?}", frame.error);
                    let result = frame.result.unwrap();
                    assert_eq!(result["height"], h);
                    if let Ok(path) = std::env::var("REDUCED_TEST_IMAGE") {
                        std::fs::copy(result["path"].as_str().unwrap(), path).unwrap();
                    }
                    // An ordinary overlay at 4s, over the now-empty base tail.
                    for (verb, args) in [
                        ("edit.move", json!({"clip":"c1","to_track":"v1","at_ms":0})),
                        ("edit.add_track", json!({"kind":"video","id":"overlay"})),
                        ("edit.insert", json!({"asset":"a1","track":"overlay","at_ms":4000,"src_range_ms":[0,1000],"ripple":false})),
                    ] {
                        let r = dispatch(&state, verb, args, test_actor()).await;
                        assert!(r.ok, "{verb}: {:?}", r.error);
                    }
                    // Identity rectangles have no placement scale to hide a bad conform.
                    let identity = dispatch(&state, "render.frame",
                        json!({"at_ms":4500,"compose":true,"h":h}), test_actor()).await;
                    assert!(identity.ok, "identity rectangle: {:?}", identity.error);
                    assert_eq!(identity.result.unwrap()["height"], h);
                    for shape in ["rect", "circle", "rounded"] {
                        let r = dispatch(&state, "edit.transform",
                            json!({"clip":"c2","shape":shape,"scale":0.3,"x":0.6,"y":0.3}), test_actor()).await;
                        assert!(r.ok, "{shape}: {:?}", r.error);
                        let r = dispatch(&state, "render.frame",
                            json!({"at_ms":4500,"compose":true,"h":h}), test_actor()).await;
                        assert!(r.ok, "{shape}: {:?}", r.error);
                        let result = r.result.unwrap();
                        if shape == "rect" && h == 1080 {
                            if let Ok(path) = std::env::var("REDUCED_TEST_RECT_IMAGE") {
                                std::fs::copy(result["path"].as_str().unwrap(), path).unwrap();
                            }
                            return; // full-size byte-identity evidence mode
                        }
                        assert_eq!(h, 360);
                        if let Ok(prefix) = std::env::var("REDUCED_TEST_SHAPE_PREFIX") {
                            std::fs::copy(result["path"].as_str().unwrap(), format!("{prefix}-{shape}.jpg")).unwrap();
                        }
                        let rgb = reduced_test_rgb(Path::new(result["path"].as_str().unwrap()));
                        let red = |x: usize, y: usize| rgb[(y * 640 + x) * 3];
                        assert!(red(480, 204) > 200, "{shape}: opaque centre");
                        if shape != "rect" {
                            assert!(red(386, 110) < 20, "{shape}: transparent corner");
                            assert!(red(500, 204) > 200, "{shape}: original diameter");
                            for (param, value) in [("scale", 0.5), ("opacity", 0.5)] {
                                let r = dispatch(&state, "edit.keyframe", json!({"clip":"c2","param":param,"points":[{"t_ms":0,"value":1.0},{"t_ms":500,"value":value},{"t_ms":1000,"value":value}]}), test_actor()).await;
                                assert!(r.ok, "{param}: {:?}", r.error);
                            }
                            let r = dispatch(&state, "render.frame", json!({"at_ms":4500,"compose":true,"h":360}), test_actor()).await;
                            assert!(r.ok, "{shape} Scale/opacity: {:?}", r.error);
                            let result = r.result.unwrap();
                            let rgb = reduced_test_rgb(Path::new(result["path"].as_str().unwrap()));
                            let red = |x: usize, y: usize| rgb[(y * 640 + x) * 3];
                            assert!((90..160).contains(&red(432, 156)), "{shape}: keyed opacity");
                            assert!(red(500, 156) < 20, "{shape}: keyed diameter shrinks");
                            // Restore identity keys for the next shape.
                            for param in ["scale", "opacity"] {
                                let r = dispatch(&state, "edit.keyframe", json!({"clip":"c2","param":param,"points":[{"t_ms":0,"value":1.0},{"t_ms":1000,"value":1.0}]}), test_actor()).await;
                                assert!(r.ok, "{:?}", r.error);
                            }
                        }
                    }
                });
        }).unwrap().join().unwrap();
}
