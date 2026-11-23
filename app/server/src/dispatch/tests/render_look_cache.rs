use super::*;

async fn look_call(state: &AppState, verb: &str, args: Value) -> Value {
    let r = dispatch(state, verb, args, test_actor()).await;
    assert!(r.ok, "{verb}: {:?}", r.error);
    r.result.unwrap()
}

async fn look_export(state: &AppState, path: &Path) -> (Value, f64) {
    let start = std::time::Instant::now();
    let reply = look_call(
        state,
        "render.final",
        json!({"path":path,"preset":"draft","hardware":"off"}),
    )
    .await;
    let id = reply["job_id"].as_str().unwrap();
    for _ in 0..24000 {
        let job = state.jobs.get(id).unwrap();
        assert_ne!(job.state, crate::jobs::JobState::Failed, "{:?}", job.error);
        if job.state == crate::jobs::JobState::Done {
            return (
                serde_json::to_value(job).unwrap(),
                start.elapsed().as_secs_f64(),
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("export timed out");
}

fn look_rgb(path: &Path, at: &str, w: usize, h: usize) -> Vec<u8> {
    let o = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-threads", "1", "-ss", at, "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.stdout.len(), w * h * 3);
    o.stdout
}

fn look_segments(project: &Path) -> usize {
    std::fs::read_dir(project.join(".cache/segrender"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    e.file_name().to_string_lossy().starts_with("seg_")
                        && e.path().extension().is_some_and(|s| s == "mp4")
                })
                .count()
        })
        .unwrap_or(0)
}

async fn look_cache_case() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let benchmark = std::env::var_os("LOOK_BENCHMARK").is_some();
    let (w, h, duration) = if benchmark {
        (1920, 1080, 60)
    } else {
        (640, 360, 1)
    };
    let project = root.path().join("look.cutproj");
    look_call(
        &state,
        "project.create",
        json!({"name":"look","dir":project}),
    )
    .await;
    for (id, color) in [("a1", "blue"), ("a2", "orange")] {
        let path = root.path().join(format!("{id}.mp4"));
        assert!(std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("color=c={color}:s=640x360:r=30:d={duration}"),
                "-pix_fmt",
                "yuv420p",
                "-y"
            ])
            .arg(&path)
            .status()
            .unwrap()
            .success());
        look_call(&state, "media.import", json!({"path":path,"proxy":false})).await;
        let mut ready = false;
        for _ in 0..6_000 {
            ready = state.project.read().await.as_ref().unwrap().project.assets[id]
                .probe
                .is_some();
            if ready {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(ready);
    }
    // Wait for the import auto-placement turn, not just its probe.
    for _ in 0..6_000 {
        if state
            .project
            .read()
            .await
            .as_ref()
            .unwrap()
            .project
            .find_clip("c1")
            .is_some()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    look_call(
        &state,
        "project.format",
        json!({"width":w,"height":h,"fps":30}),
    )
    .await;
    look_call(
        &state,
        "edit.add_track",
        json!({"id":"overlay","kind":"video"}),
    )
    .await;
    look_call(&state,"edit.insert",json!({"asset":"a2","track":"overlay","at_ms":0,"src_range_ms":[0,duration*1000],"ripple":false})).await;
    let exports = project.join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    let bench_shape = std::env::var("LOOK_BENCHMARK").unwrap_or_default();
    let looks = if benchmark {
        vec![bench_shape.as_str()]
    } else {
        vec![
            "circle", "rounded", "radius", "rect", "ring", "no_ring", "scale", "opacity",
        ]
    };
    for look in looks {
        let shape = match look {
            "rect" => "rect",
            "rounded" | "radius" => "rounded",
            _ => "circle",
        };
        let mut args = json!({"clip":"c2","shape":shape,"scale":0.4,"x":0.5,"y":0.2});
        if look == "radius" {
            args["radius"] = json!(0.45);
        }
        if look == "ring" || (benchmark && look == "circle") {
            args["ring"] = json!({"colour":"#ffffff","width":0.04});
        }
        look_call(&state, "edit.transform", args).await;
        if look == "scale" || look == "opacity" {
            look_call(&state,"edit.keyframe",json!({"clip":"c2","param":look,"points":[{"t_ms":0,"value":0.5},{"t_ms":1000,"value":0.5}]})).await;
        }
        let path = exports.join(format!("{look}.mp4"));
        let before = look_segments(&project);
        let (_, seconds) = look_export(&state, &path).await;
        eprintln!("LOOK {look} {w}x{h} {duration}s export_seconds={seconds:.3}");
        if let Ok(prefix) = std::env::var("LOOK_IMAGE_PREFIX") {
            std::fs::copy(&path, format!("{prefix}-{look}.mp4")).unwrap();
        }
        if benchmark {
            continue;
        }
        let made = look_segments(&project);
        let rgb = look_rgb(&path, "0.5", 640, 360);
        let red = |x: usize, y: usize| rgb[(y * 640 + x) * 3];
        match look {
            "circle" | "radius" => assert!(red(348, 100) < 30, "circle corner"),
            "rounded" | "rect" => {
                assert!(red(348, 100) > 180, "{look} export reused an older shape")
            }
            "ring" => assert!(rgb[(80 * 640 + 448) * 3 + 2] > 180, "ring must be white"),
            "no_ring" => assert!(
                rgb[(80 * 640 + 448) * 3 + 2] < 50,
                "old ring must disappear"
            ),
            "scale" => assert!(red(530, 200) < 30, "Scale key must shrink the bubble"),
            "opacity" => assert!(
                (80..170).contains(&red(384, 136)),
                "opacity key must change pixels"
            ),
            _ => unreachable!(),
        }
        if look != "no_ring" {
            assert!(made > before, "{look}: changed look reused an old segment");
        }
        let (_, seconds) = look_export(&state, &exports.join(format!("{look}-repeat.mp4"))).await;
        eprintln!("LOOK repeat {look} export_seconds={seconds:.3}");
        assert_eq!(
            look_segments(&project),
            made,
            "unchanged look must reuse its segments"
        );
    }
}

#[test]
fn changed_overlay_look_never_reuses_an_older_export() {
    if std::env::var_os("LOOK_CACHE_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","dispatch::tests::render_look_cache::changed_overlay_look_never_reuses_an_older_export","--nocapture"])
            .env("LOOK_CACHE_CHILD","1").env("SHELLX_CUT_RENDER_SEGMENT_SEC","0")
            .status().unwrap();
        assert!(status.success(), "isolated look-cache test failed");
        return;
    }
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(look_cache_case());
        })
        .unwrap()
        .join()
        .unwrap();
}
