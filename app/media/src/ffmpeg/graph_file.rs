//! A long filter graph goes to ffmpeg in a file, not on the command line.
//!
//! One command-line argument may hold at most 128 KiB on Linux
//! (`MAX_ARG_STRLEN`) and a whole Windows command line 32 767 characters, so a
//! timeline of a few hundred cuts could not render at all: the spawn failed
//! with "Argument list too long" (seen on the box with 300 cuts, row 194). A
//! graph longer than [`INLINE_LIMIT`] is written to a file in the engine's
//! private work folder and passed as `-/filter_complex <file>` (ffmpeg 7 and
//! later) or `-filter_complex_script <file>` (older builds). Shorter graphs stay
//! inline, so their command lines and outputs are unchanged. A graph file
//! lives exactly as long as the ffmpeg run that reads it: the [`GraphFiles`]
//! owner is held until the child has exited (success, failure or cancel) and
//! deletes the file when dropped, so repeated renders do not pile files up in
//! the engine's session folder. A graph above [`MAX_GRAPH_BYTES`] is refused.

/// Longest graph passed inline (characters): well inside the Windows limit
/// together with the rest of a render's command line.
const INLINE_LIMIT: usize = 16 * 1024;

/// Largest filter graph a render may write (bytes). A 300-cut timeline's
/// graph is about 0.2 MiB; far above this, the timeline is refused rather than
/// writing an unbounded file.
const MAX_GRAPH_BYTES: usize = 32 * 1024 * 1024;

/// The graph files of one ffmpeg run: each is deleted when this is dropped.
/// Hold it until the child process has exited.
#[derive(Default)]
#[must_use = "a graph file is deleted when its owner drops: hold it while ffmpeg runs"]
pub(crate) struct GraphFiles(Vec<tempfile::TempPath>);

/// `args` with every over-long `-filter_complex` value moved to a file, and
/// the owner of those files. On a write failure the graph stays inline (the
/// spawn then reports the limit); a graph above [`MAX_GRAPH_BYTES`] is refused.
pub(super) fn long_graphs_from_files(
    program: &std::ffi::OsStr,
    args: &[String],
) -> Result<(Vec<String>, GraphFiles), cut_core::CutError> {
    // Read the selected build only if this command actually needs a graph file.
    let option = || {
        let version = crate::hwencode::ffmpeg_version_at(program);
        graph_file_option(version.as_deref())
    };
    move_long_graphs(args, option)
}

fn graph_file_option(version: Option<&str>) -> &'static str {
    let major = version
        .map(|v| v.strip_prefix('n').unwrap_or(v))
        .and_then(|v| v.split('.').next())
        .and_then(|v| v.parse::<u32>().ok());
    if major.is_some_and(|v| v < 7) {
        "-filter_complex_script"
    } else {
        "-/filter_complex"
    }
}

fn move_long_graphs(
    args: &[String],
    option: impl Fn() -> &'static str,
) -> Result<(Vec<String>, GraphFiles), cut_core::CutError> {
    let mut out = Vec::with_capacity(args.len());
    let mut files = GraphFiles::default();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-filter_complex" && args.get(i + 1).is_some_and(|g| g.len() > INLINE_LIMIT) {
            let graph = &args[i + 1];
            if graph.len() > MAX_GRAPH_BYTES {
                return Err(too_big(graph.len()));
            }
            if let Some(path) = write_graph(graph) {
                out.push(option().to_string());
                out.push(path.display().to_string());
                files.0.push(path);
                i += 2;
                continue;
            }
        }
        out.push(args[i].clone());
        i += 1;
    }
    Ok((out, files))
}

fn too_big(bytes: usize) -> cut_core::CutError {
    cut_core::CutError::new(
        cut_core::error_codes::INVALID_ARGS,
        format!(
            "this render's filter graph is {:.1} MiB, more than the {} MiB a render may use",
            bytes as f64 / (1024.0 * 1024.0),
            MAX_GRAPH_BYTES / (1024 * 1024)
        ),
        "the timeline holds more cuts, clips or effects than one render can describe",
    )
    .with_suggested_action("export the timeline in parts (export.range), or nest parts of it")
}

fn write_graph(graph: &str) -> Option<tempfile::TempPath> {
    use std::io::Write;
    let dir = crate::private_work::session_dir().ok()?;
    let mut file = tempfile::Builder::new()
        .prefix(".cut-graph-")
        .suffix(".txt")
        .tempfile_in(&dir)
        .ok()?;
    file.write_all(graph.as_bytes()).ok()?;
    file.flush().ok()?;
    Some(file.into_temp_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_long_graph_moves_to_a_file() {
        let short = vec![
            "-i".to_string(),
            "a.mp4".into(),
            "-filter_complex".into(),
            "[0:v]null[v]".into(),
        ];
        assert_eq!(
            move_long_graphs(&short, || "-/filter_complex").unwrap().0,
            short
        );
        let long_graph = format!("[0:v]null[v0];{}", "[v0]null[v0];".repeat(2000));
        let args = vec![
            "-filter_complex".to_string(),
            long_graph.clone(),
            "-map".into(),
            "[v0]".into(),
        ];
        let (got, files) = move_long_graphs(&args, || "-/filter_complex").unwrap();
        assert_eq!(got[0], "-/filter_complex");
        assert_eq!(std::fs::read_to_string(&got[1]).unwrap(), long_graph);
        assert_eq!(&got[2..], &args[2..]);
        // The file lives as long as its owner (held while ffmpeg runs).
        drop(files);
        assert!(
            !std::path::Path::new(&got[1]).exists(),
            "graph file left behind"
        );
    }

    #[test]
    fn the_file_option_follows_the_selected_build_version() {
        for version in ["4.4.2", "5.1.7-0+deb12u1", "6.1.1-3ubuntu5"] {
            assert_eq!(graph_file_option(Some(version)), "-filter_complex_script");
        }
        for version in ["7.0", "8.0.1", "n9.0.2-39-g7adfbe40dc", "N-12345-gabc"] {
            assert_eq!(graph_file_option(Some(version)), "-/filter_complex");
        }
        assert_eq!(graph_file_option(None), "-/filter_complex");
    }

    #[test]
    fn older_builds_receive_the_same_graph_and_remaining_arguments() {
        let graph = "nullsrc=s=16x16:d=0.1,nullsink;\n".repeat(600);
        let args = vec![
            "-filter_complex".into(),
            graph.clone(),
            "-f".into(),
            "null".into(),
        ];
        let (modern, _modern_files) = move_long_graphs(&args, || "-/filter_complex").unwrap();
        let (older, _older_files) = move_long_graphs(&args, || "-filter_complex_script").unwrap();
        assert_eq!(modern[0], "-/filter_complex");
        assert_eq!(older[0], "-filter_complex_script");
        assert_eq!(std::fs::read_to_string(&modern[1]).unwrap(), graph);
        assert_eq!(std::fs::read_to_string(&older[1]).unwrap(), graph);
        assert_eq!(&modern[2..], &args[2..]);
        assert_eq!(&older[2..], &args[2..]);
    }

    #[test]
    fn a_graph_above_the_limit_is_refused_and_writes_nothing() {
        let huge = "[0:v]null[v0];".repeat(MAX_GRAPH_BYTES / 14 + 2);
        let args = vec!["-filter_complex".to_string(), huge];
        let e = move_long_graphs(&args, || "-/filter_complex")
            .err()
            .expect("refused");
        assert!(e.message.contains("MiB a render may use"), "{e:?}");
    }
}
