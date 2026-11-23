use super::*;

async fn call(state: &AppState, verb: &str, args: Value) -> Value {
    let r = dispatch(state, verb, args, test_actor()).await;
    assert!(r.ok, "{verb}: {:?}", r.error);
    r.result.unwrap()
}

async fn import(state: &AppState, path: &Path, proxy: bool) {
    let r = call(state, "media.import", json!({"path":path,"proxy":proxy})).await;
    let id = r["asset_id"].as_str().unwrap_or("a1");
    for _ in 0..6_000 {
        let ready = {
            let g = state.project.read().await;
            let a = &g.as_ref().unwrap().project.assets[id];
            a.probe.is_some() && (!proxy || a.proxy.is_some())
        };
        if ready {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("import timed out");
}

async fn export(state: &AppState, path: &Path) {
    let r = call(
        state,
        "render.final",
        json!({"path":path,"hardware":"off","preset":"draft"}),
    )
    .await;
    for _ in 0..12000 {
        let j = state.jobs.get(r["job_id"].as_str().unwrap()).unwrap();
        assert_ne!(j.state, crate::jobs::JobState::Failed, "{:?}", j.error);
        if j.state == crate::jobs::JobState::Done {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("export timed out");
}

fn media(path: &Path, filter: &str, vfr: bool) {
    let mut c = std::process::Command::new("ffmpeg");
    c.args(["-v", "error", "-f", "lavfi", "-i", filter]);
    if vfr {
        c.args([
            "-vf",
            "select='not(eq(mod(n,7),0))',setpts=PTS+0.137/TB",
            "-fps_mode",
            "vfr",
        ]);
    }
    assert!(c
        .args(["-pix_fmt", "yuv420p", "-y"])
        .arg(path)
        .status()
        .unwrap()
        .success());
}

fn rgb(path: &Path, at: u64, w: usize, h: usize) -> Vec<u8> {
    let mut command = std::process::Command::new("ffmpeg");
    command.args(["-v", "error", "-threads", "1"]);
    if at > 0 {
        command.args(["-ss", &(at as f64 / 1000.0).to_string()]);
    }
    let o = command
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.stdout.len(), w * h * 3, "{} at={at}", path.display());
    o.stdout
}

fn bounds(pixels: &[u8], w: usize) -> (usize, usize, usize, usize) {
    let mut b = (w, pixels.len() / 3 / w, 0, 0);
    for (i, p) in pixels.as_chunks::<3>().0.iter().enumerate() {
        if p[0] > 60 {
            let (x, y) = (i % w, i / w);
            b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
        }
    }
    assert!(b.2 > b.0 && b.3 > b.1, "missing red overlay");
    b
}

async fn keys_case() {
    for vfr in [false, true] {
        if let Ok(rate) = std::env::var("WINDOW_KEYS_RATE") {
            if (rate == "vfr") != vfr {
                continue;
            }
        }
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        let project = root.path().join("keys.cutproj");
        call(
            &state,
            "project.create",
            json!({"name":"keys","dir":project}),
        )
        .await;
        let base = root.path().join("base.mp4");
        media(&base, "color=c=0x000020:s=1920x1080:r=30:d=37", false);
        import(&state, &base, false).await;
        let camera = root.path().join("camera.mp4");
        media(
            &camera,
            &format!(
                "color=c=red:s=640x480:r={}:d=39,drawbox=x=300:y=0:w=40:h=480:color=white:t=fill",
                if vfr { 120 } else { 30 }
            ),
            vfr,
        );
        import(&state, &camera, false).await;
        call(
            &state,
            "project.format",
            json!({"width":1920,"height":1080,"fps":30}),
        )
        .await;
        call(
            &state,
            "edit.add_track",
            json!({"id":"camera","kind":"video"}),
        )
        .await;
        call(&state,"edit.insert",json!({"asset":"a2","track":"camera","at_ms":0,"src_range_ms":[0,36564],"ripple":false})).await;
        for (param, points) in [
            (
                "pos_x",
                json!([{"t_ms":0,"value":0.836},{"t_ms":16296,"value":0.04}]),
            ),
            (
                "pos_y",
                json!([{"t_ms":0,"value":0.74},{"t_ms":16296,"value":0.04}]),
            ),
            (
                "scale",
                json!([{"t_ms":0,"value":1.0},{"t_ms":32830,"value":1.542}]),
            ),
            (
                "opacity",
                json!([{"t_ms":0,"value":1.0},{"t_ms":25000,"value":0.5}]),
            ),
        ] {
            call(
                &state,
                "edit.keyframe",
                json!({"clip":"c2","param":param,"interp":"hold","points":points}),
            )
            .await;
        }
        let exports = project.join("exports");
        std::fs::create_dir_all(&exports).unwrap();
        {
            let g = state.project.read().await;
            let probe = g.as_ref().unwrap().project.assets["a2"]
                .probe
                .as_ref()
                .unwrap();
            eprintln!("CAMERA vfr={vfr} probe={probe}");
            if vfr {
                assert_eq!(probe["variable_frame_rate"], true);
                assert!((95.0..110.0).contains(&probe["fps"].as_f64().unwrap()));
                assert!(
                    probe["raw"]["format"]["start_time"]
                        .as_str()
                        .unwrap()
                        .parse::<f64>()
                        .unwrap()
                        > 0.0
                );
            }
        }
        for shape in ["circle", "rect"] {
            if let Ok(only) = std::env::var("WINDOW_KEYS_SHAPE") {
                if only != shape {
                    continue;
                }
            }
            let mut transform = json!({"clip":"c2","x":0.836,"y":0.74,"scale":0.124});
            if std::env::var_os("WINDOW_KEYS_118").is_none() {
                transform["shape"] = json!(shape);
            }
            if shape == "circle" {
                transform["ring"] = json!({"colour":"#ffffff","width":0.02});
            }
            call(&state, "edit.transform", transform).await;
            let movie = exports.join(format!("{shape}.mp4"));
            export(&state, &movie).await;
            let windows = std::fs::read_dir(project.join(".cache/segrender"))
                .unwrap()
                .flatten()
                .filter(|p| {
                    p.file_name().to_string_lossy().starts_with("seg_")
                        && p.path().extension().is_some_and(|x| x == "mp4")
                })
                .count();
            assert!(windows >= 2, "must exercise a real multi-window export");
            let mut stripe0 = 0;
            let mut small_stripe0 = 0;
            for at in [4000, 20000, 34000] {
                let late = at > 16296;
                let resized = at > 32830;
                let full = rgb(&movie, at, 1920, 1080);
                let b = bounds(&full, 1920);
                let (mut x, y) = if late { (76, 42) } else { (1604, 798) };
                // Published 0.6.118 software rectangles already letterboxed 4:3.
                // Published rectangle conform has a 1/8-box inset until content zoom removes it.
                if shape == "rect" && !resized {
                    x += 30;
                }
                assert!(
                    b.0.abs_diff(x) <= 5 && b.1.abs_diff(y) <= 5,
                    "export {shape} vfr={vfr} at={at}: bounds={b:?}, expected {x},{y}"
                );
                let mut export_stripe = 0;
                if shape == "circle" {
                    let d = if resized { 366 } else { 238 };
                    assert!((b.2 - b.0).abs_diff(d) <= 6, "circle Scale at {at}: {b:?}");
                } else {
                    let y = (b.1 + b.3) / 2;
                    let stripe = (b.0..=b.2)
                        .filter(|x| {
                            let i = (y * 1920 + x) * 3;
                            full[i + 1] > if at > 25000 { 80 } else { 180 }
                        })
                        .count();
                    export_stripe = stripe;
                    if !resized {
                        stripe0 = stripe;
                    } else {
                        assert!(
                            stripe * 10 > stripe0 * 13,
                            "rectangle Scale: {stripe0} -> {stripe}"
                        );
                    }
                }
                let i = (((b.1 + b.3) / 2) * 1920 + b.0 + (b.2 - b.0) / 4) * 3;
                if at > 25000 {
                    assert!(
                        (90..170).contains(&full[i]),
                        "export opacity at {at}: {:?}",
                        &full[i..i + 3]
                    );
                }
                for h in [360, 1080] {
                    let f = call(
                        &state,
                        "render.frame",
                        json!({"at_ms":at,"compose":true,"h":h}),
                    )
                    .await;
                    let path = Path::new(f["path"].as_str().unwrap());
                    let w = h * 16 / 9;
                    let image = rgb(path, 0, w, h);
                    let p = bounds(&image, w);
                    if shape == "rect" {
                        let row = (p.1 + p.3) / 2;
                        let stripe = (p.0..=p.2)
                            .filter(|x| {
                                image[(row * w + x) * 3 + 1] > if at > 25000 { 80 } else { 180 }
                            })
                            .count();
                        if h == 1080 {
                            assert!(
                                stripe.abs_diff(export_stripe) <= 3,
                                "single-frame rectangle Scale disagrees with export"
                            );
                        } else if !resized {
                            small_stripe0 = stripe;
                        } else {
                            assert!(stripe>small_stripe0,"reduced rectangle Scale did not change stripe width: {small_stripe0} -> {stripe}");
                        }
                    }
                    for (a, b) in [(p.0, b.0), (p.1, b.1), (p.2, b.2), (p.3, b.3)] {
                        assert!(
                            (a * 1080 / h).abs_diff(b) <= if h == 360 { 16 } else { 4 },
                            "frame/export {shape} vfr={vfr} h={h} at={at}: {p:?} vs {b}"
                        );
                    }
                    let i = (((p.1 + p.3) / 2) * w + p.0 + (p.2 - p.0) / 4) * 3;
                    if at > 25000 {
                        assert!((80..180).contains(&image[i]), "frame opacity");
                    }
                    if h == 360 {
                        if let Ok(prefix) = std::env::var("WINDOW_KEYS_IMAGES") {
                            std::fs::copy(path, format!("{prefix}-{vfr}-{shape}-{at}.jpg"))
                                .unwrap();
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn camera_hold_keys_keep_clip_time_across_render_windows() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(keys_case())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn aspect_case() {
    for proxy in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        let project = root.path().join("aspect.cutproj");
        call(
            &state,
            "project.create",
            json!({"name":"aspect","dir":project}),
        )
        .await;
        let base = root.path().join("base.mp4");
        media(&base, "color=c=blue:s=1920x1080:r=30:d=2", false);
        import(&state, &base, false).await;
        let red = root.path().join("red.mp4");
        let wide = std::env::var_os("ASPECT_WIDE").is_some();
        media(
            &red,
            if wide {
                "color=c=red:s=320x180:r=30:d=2"
            } else {
                "color=c=red:s=320x240:r=30:d=2"
            },
            false,
        );
        import(&state, &red, proxy).await;
        call(
            &state,
            "edit.add_track",
            json!({"kind":"video","id":"overlay"}),
        )
        .await;
        call(&state,"edit.insert",json!({"asset":"a2","track":"overlay","at_ms":0,"src_range_ms":[0,2000],"ripple":false})).await;
        call(
            &state,
            "edit.transform",
            json!({"clip":"c2","x":0.6,"y":0.1,"scale":0.25}),
        )
        .await;
        let exports = project.join("exports");
        std::fs::create_dir_all(&exports).unwrap();
        let movie = exports.join("rect.mp4");
        export(&state, &movie).await;
        if let Ok(prefix) = std::env::var("ASPECT_MOVIE_PREFIX") {
            std::fs::copy(&movie, format!("{prefix}-{wide}-{proxy}.mp4")).unwrap();
        }
        let mut frames = vec![(rgb(&movie, 1000, 1920, 1080), 1920, 1080)];
        for h in [360, 1080] {
            let f = call(
                &state,
                "render.frame",
                json!({"at_ms":1000,"compose":true,"h":h}),
            )
            .await;
            frames.push((
                rgb(Path::new(f["path"].as_str().unwrap()), 0, h * 16 / 9, h),
                h * 16 / 9,
                h,
            ));
        }
        for (pixels, w, h) in frames {
            for frac in [0.01, 0.1, 0.5, 0.9, 0.99] {
                let x = (w as f64 * (0.6 + 0.25 * frac)) as usize;
                let y = (h as f64 * 0.2) as usize;
                let i = (y * w + x) * 3;
                // known: product decision pending (0.6.120): a rectangle overlay should take its source's shape
                let expected_red = wide || frac == 0.5;
                let matches = if expected_red {
                    pixels[i] > 180
                } else {
                    pixels[i] < 40 && pixels[i + 1] < 40 && pixels[i + 2] < 100
                };
                assert!(matches, "published rectangle contain: red={expected_red} proxy={proxy} size={w}x{h} x={x}: {:?}", &pixels[i..i+3]);
            }
        }
    }
}

#[test]
fn rectangular_four_three_source_keeps_published_bars_in_frames_and_export() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(aspect_case())
        })
        .unwrap()
        .join()
        .unwrap();
}
