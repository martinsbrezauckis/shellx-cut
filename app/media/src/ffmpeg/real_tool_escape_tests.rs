//! Real-tool checks of the escaping in `escape.rs`: each runs the FFmpeg on
//! this machine (or the one named by SHELLX_CUT_FFMPEG/_FFPROBE) on text that
//! did not come from Cut, with planted files that would show up if that text
//! were read as syntax. The tests return early, with a note, when FFmpeg is
//! not installed.

use super::*;
use std::path::PathBuf;

fn tools_present() -> bool {
    let ok = Command::new(ffmpeg_bin()).arg("-version").output().is_ok()
        && Command::new(ffprobe_bin()).arg("-version").output().is_ok();
    if !ok {
        eprintln!("FFmpeg not installed: real-tool escaping check not run");
    }
    ok
}

fn sample() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/assets/first-edit-sample.mp4")
}

const IDENTITY_LUT: &str =
    "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
#[cfg(unix)]
const RED_LUT: &str = "LUT_3D_SIZE 2\n1 0 0\n1 0 0\n1 0 0\n1 0 0\n1 0 0\n1 0 0\n1 0 0\n1 0 0\n";

/// One RGB24 frame of `vf` applied to a mid-grey picture, rendered with the
/// child's working folder set to `cwd` (so anything a stray filter writes by
/// a relative name lands there).
fn grey_frame_through(vf: &str, cwd: &Path) -> Result<Vec<u8>, String> {
    let out = cwd.join("frame.rgb");
    let result = Command::new(ffmpeg_bin())
        .current_dir(cwd)
        .args(["-hide_banner", "-nostdin", "-y", "-v", "error"])
        .args(["-f", "lavfi", "-i", "color=c=0x808080:s=16x16:d=0.04"])
        .args([
            "-vf",
            vf,
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
        ])
        .arg(&out)
        .output()
        .map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).into_owned());
    }
    std::fs::read(&out).map_err(|e| e.to_string())
}

fn is_grey(rgb: &[u8]) -> bool {
    !rgb.is_empty()
        && rgb
            .chunks(3)
            .all(|p| p[0].abs_diff(0x80) < 8 && p[1].abs_diff(0x80) < 8 && p[2].abs_diff(0x80) < 8)
}

/// Names that each carry filtergraph syntax. Every one is a real identity LUT
/// file; if any part of a name were read as syntax, FFmpeg would fail, open a
/// different file, or write `canary.txt` next to it.
fn hostile_lut_names() -> Vec<&'static str> {
    let mut names = vec![
        "it's.cube",
        "a,b;c[d]e.cube",
        "q'],metadata=mode=print,[x.cube",
        "100%=50.cube",
        " lead.cube",
        "-dash.cube",
        "x.cube'=interp=nearest,metadata=mode=print'.cube",
    ];
    if cfg!(unix) {
        names.extend([
            "x.cube:interp=nearest,metadata=mode=print:file=canary.txt,null.cube",
            "q'],metadata=mode=print:file=canary.txt,[x.cube",
            r"back\slash.cube",
            r"\':ends\\.cube",
            "new\nline.cube",
        ]);
    }
    names
}

#[test]
fn lut_path_with_filtergraph_syntax_in_its_name_is_one_literal_value() {
    if !tools_present() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    for name in hostile_lut_names() {
        let lut = temp.path().join(name);
        std::fs::write(&lut, IDENTITY_LUT).unwrap();
        let vf = format!("lut3d=file={},format=rgb24", escape_filter_path(&lut));
        let frame = grey_frame_through(&vf, temp.path())
            .unwrap_or_else(|e| panic!("{name:?}: FFmpeg refused {vf}: {e}"));
        assert!(
            is_grey(&frame),
            "{name:?}: the named identity LUT was not the one applied"
        );
        assert!(
            !temp.path().join("canary.txt").exists(),
            "{name:?}: part of the name ran as a filter"
        );
        std::fs::remove_file(&lut).unwrap();
    }
}

/// On Unix a backslash is an ordinary character of a file name. The old
/// escaper rewrote it to `/`, so `a\b.cube` reached FFmpeg as `a/b.cube`.
#[cfg(unix)]
#[test]
fn lut_path_with_a_backslash_opens_that_file_and_not_another() {
    if !tools_present() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let named = temp.path().join(r"a\b.cube");
    std::fs::write(&named, IDENTITY_LUT).unwrap();
    std::fs::create_dir(temp.path().join("a")).unwrap();
    std::fs::write(temp.path().join("a").join("b.cube"), RED_LUT).unwrap();
    let vf = format!("lut3d=file={},format=rgb24", escape_filter_path(&named));
    let frame = grey_frame_through(&vf, temp.path()).unwrap();
    assert!(
        is_grey(&frame),
        "a different file than {named:?} was read: {vf}"
    );
}

/// The list entry as the renderer and the preview write it.
fn list_lines(segment: &Path) -> Result<String, CutError> {
    owned_concat_file_lines(segment)
}

/// The concat list as the renderer and the preview write it, with the
/// product's own list runner.
fn run_owned_list(list_dir: &Path, segment: &Path) -> Result<PathBuf, CutError> {
    let list = list_dir.join("concat.txt");
    let line = list_lines(segment)?;
    std::fs::write(&list, format!("{line}\n")).unwrap();
    let output = list_dir.join("joined.mp4");
    let args = vec![
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list.display().to_string(),
        "-c".into(),
        "copy".into(),
        output.display().to_string(),
    ];
    run_owned_concat_atomic_output(&args, &output)?;
    Ok(output)
}

/// A folder only its owner can write, as Cut's output staging requires.
fn private_dir(path: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
        .create(path)
        .unwrap();
    #[cfg(not(unix))]
    std::fs::create_dir(path).unwrap();
}

fn duration_s(path: &Path) -> f64 {
    ffprobe_json(path).unwrap()["format"]["duration"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn concat_list_keeps_a_project_folder_with_an_apostrophe() {
    if !tools_present() {
        return;
    }
    let temp = crate::atomic_output::private_test_tempdir();
    let project = temp.path().join("O'Brien's project");
    private_dir(&project);
    let segment = project.join("seg_0001.mp4");
    std::fs::copy(sample(), &segment).unwrap();
    let joined = run_owned_list(&project, &segment)
        .expect("a segment inside a folder named with an apostrophe must be joined");
    assert!((duration_s(&joined) - duration_s(&sample())).abs() < 0.2);
}

/// A received project folder whose name holds a quote and line breaks. Written
/// unchecked into the list, the rest of the name became directives: here a
/// second `file` that FFmpeg resolves against the engine's own working folder
/// (outside the project), so another file's content joined the output.
#[cfg(unix)]
#[test]
fn concat_list_refuses_a_folder_name_that_would_add_directives() {
    if !tools_present() {
        return;
    }
    let temp = crate::atomic_output::private_test_tempdir();
    let engine_cwd = temp.path().join("home");
    std::fs::create_dir(&engine_cwd).unwrap();
    std::fs::copy(sample(), engine_cwd.join("outside-secret.mp4")).unwrap();
    let project = temp
        .path()
        .join("p'\noption format_whitelist mov\nfile file:outside-secret.mp4\n#");
    // The text before the quote names a file the same archive can carry next
    // to the folder, so the first entry of the broken list still opens.
    std::fs::copy(sample(), temp.path().join("p\\")).unwrap();
    private_dir(&project);
    let segment = project.join("seg_0001.mp4");
    std::fs::copy(sample(), &segment).unwrap();

    let line = match list_lines(&segment) {
        Err(error) => {
            assert_eq!(error.code, error_codes::INVALID_ARGS);
            assert!(error.message.contains("line break"), "{error:?}");
            assert!(error.suggested_action.unwrap().contains("rename"));
            return;
        }
        Ok(line) => line,
    };
    // Reached only when the name was written into the list: show what FFmpeg
    // then does, from the engine's working folder.
    let list = project.join("concat.txt");
    std::fs::write(&list, format!("{line}\n")).unwrap();
    let output = temp.path().join("joined.mp4");
    let status = Command::new(ffmpeg_bin())
        .current_dir(&engine_cwd)
        .args(["-hide_banner", "-nostdin", "-y", "-v", "error"])
        .args(
            normalize_input_args(
                &[
                    "-f".into(),
                    "concat".into(),
                    "-safe".into(),
                    "0".into(),
                    "-i".into(),
                    list.display().to_string(),
                ],
                true,
            )
            .unwrap(),
        )
        .args(["-c", "copy"])
        .arg(&output)
        .status()
        .unwrap();
    let joined = if status.success() {
        duration_s(&output)
    } else {
        0.0
    };
    panic!(
        "the folder name was written into the concat list (exit {status}); joined duration \
         {joined:.2}s vs one segment {:.2}s",
        duration_s(&sample())
    );
}

fn caption_project(text: &str, style_font: Option<&str>) -> cut_core::Project {
    use cut_core::{CaptionClip, Clip, ProjectSettings, Track, TrackKind};
    let mut p = cut_core::Project::new(
        "esc",
        ProjectSettings {
            width: 320,
            height: 180,
            fps: 25.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    let style_ref = style_font.map(|font| {
        p.caption_styles.insert(
            "s1".into(),
            cut_core::CaptionStyle {
                font: font.into(),
                size: 40,
                color: "#fff".into(),
                bg: None,
                pos: Some("center".into()),
                extra: Default::default(),
            },
        );
        "s1".to_string()
    });
    p.tracks.push(Track {
        id: "cap1".into(),
        kind: TrackKind::Caption,
        clips: vec![Clip::Caption(CaptionClip {
            id: "c1".into(),
            text: text.into(),
            style_ref,
            range_ms: [0, 5000],
        })],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    p
}

/// Counts of (white-ish, red, green) pixels of the first frame of a black
/// picture with `ass` burned in.
fn burn_in_colours(ass: &str) -> (usize, usize, usize) {
    burn_in_colours_named(ass, "burnin.ass")
}

fn burn_in_colours_named(ass: &str, file_name: &str) -> (usize, usize, usize) {
    let temp = tempfile::tempdir().unwrap();
    let ass_path = temp.path().join(file_name);
    std::fs::write(&ass_path, ass).unwrap();
    let out = temp.path().join("frame.rgb");
    let result = Command::new(ffmpeg_bin())
        .args(["-hide_banner", "-nostdin", "-y", "-v", "error"])
        .args(["-f", "lavfi", "-i", "color=c=black:s=320x180:d=0.2"])
        .arg("-vf")
        .arg(format!("ass=filename={}", escape_filter_path(&ass_path)))
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let rgb = std::fs::read(out).unwrap();
    let mut counts = (0, 0, 0);
    for p in rgb.chunks(3) {
        let (r, g, b) = (p[0], p[1], p[2]);
        if r > 180 && g > 180 && b > 180 {
            counts.0 += 1;
        } else if r > 180 && g < 80 && b < 80 {
            counts.1 += 1;
        } else if g > 180 && r < 80 && b < 80 {
            counts.2 += 1;
        }
    }
    counts
}

fn has_libass() -> bool {
    let out = Command::new(ffmpeg_bin())
        .args(["-hide_banner", "-h", "filter=ass"])
        .output();
    let ok = out.is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("this FFmpeg has no libass: caption burn-in check not run");
    }
    ok
}

#[test]
fn caption_text_burns_in_as_typed_text_without_tags_or_extra_lines() {
    if !tools_present() || !has_libass() {
        return;
    }
    // Control: ordinary text is drawn (white pixels), nothing coloured.
    let plain = crate::captions::captions_to_ass(&caption_project("HELLO", None)).unwrap();
    let (white, red, green) = burn_in_colours(&plain);
    assert!(white > 20, "no caption drawn at all; is a font installed?");
    assert_eq!((red, green), (0, 0));

    // A colour override tag, and a CR that would start a new event line.
    let hostile = "{\\c&H0000FF&}RRRR\rDialogue: 0,0:00:00.00,0:00:05.00,Default,,0,0,0,,{\\an7\\c&H00FF00&\\fs60}GGGG";
    let ass = crate::captions::captions_to_ass(&caption_project(hostile, None)).unwrap();
    let (white, red, green) = burn_in_colours(&ass);
    assert_eq!(red, 0, "a tag in caption text changed the colour");
    assert_eq!(green, 0, "caption text added a subtitle event of its own");
    assert!(white > 20, "the caption text itself must still be shown");
}

/// Whitespace at either end of a name is kept (the parser drops it unless
/// escaped). lut3d chooses its reader by extension, so this uses `ass`.
#[test]
fn subtitle_file_named_with_edge_whitespace_is_still_that_file() {
    if !tools_present() || !has_libass() {
        return;
    }
    let ass = crate::captions::captions_to_ass(&caption_project("HELLO", None)).unwrap();
    let mut names = vec![" lead.ass"];
    if cfg!(unix) {
        names.push("trail.ass ");
        names.push("tab.ass\t");
    }
    for name in names {
        let (white, _, _) = burn_in_colours_named(&ass, name);
        assert!(
            white > 20,
            "{name:?}: the caption file was not the one read"
        );
    }
}

#[test]
fn caption_style_font_cannot_add_lines_to_the_subtitle_file() {
    let font = "Inter\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, \
                MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:05.00,Default,,0,0,0,,\
                {\\an7\\c&H00FF00&\\fs60}GGGG";
    match crate::captions::captions_to_ass(&caption_project("hi", Some(font))) {
        Err(error) => {
            assert_eq!(error.code, error_codes::INVALID_ARGS);
            assert!(error
                .suggested_action
                .unwrap()
                .contains("captions.set_style"));
        }
        Ok(ass) => {
            if tools_present() && has_libass() {
                let (_, _, green) = burn_in_colours(&ass);
                panic!("font name was written into the subtitle file ({green} injected pixels)");
            }
            panic!("font name was written into the subtitle file: {ass}");
        }
    }
}

/// ffprobe reads a bare argument starting with `-` as one of its options.
#[test]
fn ffprobe_reads_a_name_that_starts_with_a_dash_as_a_file() {
    if !tools_present() {
        return;
    }
    let error = ffprobe_json(Path::new("-version")).unwrap_err();
    assert!(
        error.message.starts_with("ffprobe failed on"),
        "the name ran as an ffprobe option: {error:?}"
    );
}
