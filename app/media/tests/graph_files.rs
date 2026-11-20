//! A long filter graph is written to a file for ffmpeg; that file lives only
//! as long as the ffmpeg run. Before, every render with a long graph left one
//! in the engine's session folder until the engine stopped, after a success,
//! a failure or a cancel alike, so repeated previews and exports piled them up.

cut_core::isolate_test_process!();

fn graph_files() -> usize {
    let dir = cut_media::private_work::session_dir().expect("session folder");
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(".cut-graph-"))
                .count()
        })
        .unwrap_or(0)
}

/// A graph over the inline limit (16 KiB) that stays shallow (a deep chain of
/// thousands of filters overflows ffmpeg's stack on macOS): 600 small
/// independent `nullsrc,nullsink` chains beside the real one (about 21 KB).
fn long_graph(tail: &str) -> String {
    format!(
        "{}[0:v]{tail}[v]",
        "nullsrc=s=16x16:d=0.1,nullsink;".repeat(600)
    )
}

fn args(graph: String, out: &std::path::Path) -> Vec<String> {
    [
        "-f",
        "lavfi",
        "-i",
        "testsrc2=s=64x36:r=10:d=0.5",
        "-filter_complex",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([
        graph,
        "-map".into(),
        "[v]".into(),
        "-f".into(),
        "null".into(),
    ])
    .chain([out.display().to_string()])
    .collect()
}

#[test]
fn no_graph_file_is_left_after_a_render_that_succeeds_or_fails() {
    let dir = tempfile::tempdir().unwrap();
    let before = graph_files();
    for i in 0..3 {
        cut_media::ffmpeg::run_ffmpeg(&args(
            long_graph("null"),
            &dir.path().join(format!("{i}.nul")),
        ))
        .expect("a long graph renders");
    }
    let failed = cut_media::ffmpeg::run_ffmpeg(&args(
        long_graph("no_such_filter"),
        &dir.path().join("x.nul"),
    ));
    assert!(failed.is_err(), "an unknown filter fails");
    assert_eq!(
        graph_files(),
        before,
        "graph files left in the session folder"
    );
}

/// Exercise the exact file bytes, including escaped commas and newlines, on
/// either side of the portable command-line limit. The direct command is the
/// inline reference even when the engine chooses a graph file.
#[test]
fn graph_file_and_inline_outputs_match_around_the_inline_limit() {
    let dir = tempfile::tempdir().unwrap();
    for bytes in [16 * 1024 - 1, 16 * 1024 + 1] {
        let tail = "[0:v]setpts='if(gte(N\\,0)\\,PTS\\,PTS)'[v]";
        let mut graph = "nullsrc=s=16x16:d=0.1,nullsink;\n".repeat(500);
        graph.push_str(&" ".repeat(bytes - graph.len() - tail.len()));
        graph.push_str(tail);
        assert_eq!(graph.len(), bytes);
        let engine = dir.path().join(format!("engine-{bytes}.mp4"));
        let inline = dir.path().join(format!("inline-{bytes}.mp4"));
        let render_args = |out: &std::path::Path| {
            let mut args = args(graph.clone(), out);
            let format = args.iter().rposition(|a| a == "-f").unwrap();
            args.splice(format..format + 2, ["-c:v".into(), "libx264".into()]);
            args
        };
        cut_media::ffmpeg::run_ffmpeg(&render_args(&engine)).expect("engine graph renders");
        let output = command_output(
            std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
                .args(["-hide_banner", "-nostdin", "-y"])
                .args(render_args(&inline)),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let facts = |path: &std::path::Path| {
            let output = command_output(std::process::Command::new(cut_media::ffmpeg::ffprobe_bin())
                .args([
                    "-v",
                    "error",
                    "-count_frames",
                    "-show_entries",
                    "stream=codec_name,width,height,pix_fmt,r_frame_rate,nb_read_frames,duration",
                    "-of",
                    "json",
                ])
                .arg(path));
            assert!(output.status.success());
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
        };
        let observed = facts(&engine);
        assert_eq!(observed["streams"][0]["nb_read_frames"], "5", "{observed}");
        assert_eq!(observed, facts(&inline));
        assert_eq!(
            std::fs::read(engine).unwrap(),
            std::fs::read(inline).unwrap()
        );
    }
}

#[test]
fn the_graph_file_byte_cap_is_accepted_and_exceeding_it_is_refused() {
    const CAP: usize = 32 * 1024 * 1024;
    let dir = tempfile::tempdir().unwrap();
    let tail = "[0:v]null[v]";
    let mut graph = " ".repeat(CAP - tail.len());
    graph.push_str(tail);
    cut_media::ffmpeg::run_ffmpeg(&args(graph.clone(), &dir.path().join("cap.nul")))
        .expect("a graph at the byte cap renders from a file");
    graph.push(' ');
    let error = cut_media::ffmpeg::run_ffmpeg(&args(graph, &dir.path().join("too-big.nul")))
        .expect_err("a graph above the byte cap is refused before spawning");
    assert_eq!(error.code, cut_core::error_codes::INVALID_ARGS);
    assert!(
        error
            .message
            .contains("more than the 32 MiB a render may use"),
        "{error:?}"
    );
}

fn command_output(command: &mut std::process::Command) -> std::process::Output {
    cut_media::ffmpeg::run_owned_command(
        command,
        &cut_media::ffmpeg::OwnedProcessControl::bounded(
            std::time::Duration::from_secs(30),
            || false,
        ),
        "graph comparison",
    )
    .expect("bounded comparison command finishes")
}
