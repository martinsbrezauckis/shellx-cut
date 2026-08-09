//! ffmpeg.rs — subprocess plumbing for ffmpeg/ffprobe (media-engine contract).
//!
//! Role: the ONLY place that spawns ffmpeg/ffprobe. Centralizes binary
//! discovery, deterministic flag policy, stderr capture (for actionable
//! CutError causes) and progress parsing.
//! Dependencies: std::process, cut-core (CutError). Primary callers:
//! probe.rs, proxy.rs, render.rs, frame extraction.

use cut_core::{error_codes, CutError};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

mod process;
pub(crate) use process::current_render_process_control;
pub(crate) use process::scoped_render_process_control;
pub use process::{
    run_bounded_command, run_owned_command, run_owned_command_with_input,
    with_render_process_control, OwnedProcessControl, ProcessStderrLineObserver,
    RenderProcessControl,
};

/// Flags applied to EVERY encode for determinism (media-engine contract): no wall-clock
/// metadata, bitexact where possible. Same input + EDL ⇒ same output hash.
pub const DETERMINISM_FLAGS: &[&str] = &[
    "-map_metadata",
    "-1",
    "-fflags",
    "+bitexact",
    "-flags:v",
    "+bitexact",
    "-flags:a",
    "+bitexact",
];

/// ffmpeg program path. Delegates to [`crate::toolpath::ffmpeg`]: resolves
/// a CONFIGURED / BUNDLED / app-data ffmpeg before falling back to PATH, so the
/// cold Windows install finds the downloaded/bundled binary instead of assuming
/// one is on PATH. Returns an OsString (full path or bare "ffmpeg") — every
/// call site passes it straight to `Command::new`, so no call site changes.
pub fn ffmpeg_bin() -> OsString {
    crate::toolpath::ffmpeg()
}

/// ffprobe program path (same resolution as [`ffmpeg_bin`]).
pub fn ffprobe_bin() -> OsString {
    crate::toolpath::ffprobe()
}

/// ffmpeg program path for a render whose `args` may need a FEATURE-specific build.
/// Scans the filter graph for caption burn-in (`ass=filename=`/`subtitles=`),
/// stabilize (`vidstab*`) and color management (`zscale` — the colorspace hop
/// render::colorspace_filter emits when the project working/output space or a tagged
/// clip input ≠ rec709); when present it resolves a libass/vidstab/libzimg-capable
/// ffmpeg via [`crate::toolpath::ffmpeg_for`] — so on a box whose FASTEST ffmpeg
/// lacks the library (Homebrew 8.x dropped libass AND libzimg) the render still
/// runs on a capable build instead of silently dropping the caption (libass) or
/// hard-failing exit-8 with "No such filter: 'zscale'" (libzimg). No feature filter
/// in the graph ⇒ the normal [`ffmpeg_bin`] (auto-best-HW), so a plain render with
/// NO color hop keeps hardware accel and never forces a zscale-capable build.
///
/// Self-contained seam: EVERY render funnels through `run_ffmpeg` / `render_command`,
/// and the filtergraph is passed INLINE as `-filter_complex` (render::graph_args), so
/// routing the binary here means no render call site has to thread feature context.
/// (Fixes the macOS caption-burn-in regression + the color-managed
/// rec2020 exit-8 render failure.)
fn ffmpeg_bin_for_args(args: &[String]) -> OsString {
    use crate::toolpath::FfmpegFeature;
    let needs = |needle: &str| args.iter().any(|a| a.contains(needle));
    let mut feats = Vec::new();
    if needs("ass=filename=") || needs("subtitles=") {
        feats.push(FfmpegFeature::Libass);
    }
    if needs("vidstab") {
        feats.push(FfmpegFeature::Vidstab);
    }
    // A `zscale` colorspace hop is present ONLY for a color-managed render (working/
    // output/input space ≠ rec709). A default rec709 render emits no hop → no zscale
    // in args → plain ffmpeg_bin(), so it is never forced onto a zscale build.
    if needs("zscale") {
        feats.push(FfmpegFeature::Zscale);
    }
    if feats.is_empty() {
        ffmpeg_bin()
    } else {
        crate::toolpath::ffmpeg_for(&feats)
    }
}

/// Strip the Windows verbatim-path prefix before handing a path to ffmpeg.
///
/// Rust/Windows file APIs accept `\\?\C:\…` and `\\?\UNC\server\share\…`,
/// but ffmpeg's own URL/path parser does not consistently accept those forms.
/// In particular, segmented renders fail to open both their concat list and
/// the segment files named inside it. Keep ordinary arguments unchanged.
fn normalize_ffmpeg_arg(arg: &str) -> String {
    if let Some(path) = arg.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{path}");
    }
    arg.strip_prefix(r"\\?\").unwrap_or(arg).to_string()
}

fn normalize_ffmpeg_args(args: &[String]) -> Vec<String> {
    args.iter().map(|arg| normalize_ffmpeg_arg(arg)).collect()
}

/// Whether the selected vidstab-capable ffmpeg exposes the optional
/// `fileformat` setting on each half of the two-pass pipeline. Some builds mix
/// a newer detector with a legacy transform filter; in that configuration the
/// detector defaults to binary while the transform still reads text, producing
/// an unreadable `.trf`. Probe the two filters independently so render.rs can
/// request portable ASCII only where the option exists.
pub(crate) fn vidstab_fileformat_support() -> Result<(bool, bool), CutError> {
    static SUPPORT: OnceLock<(bool, bool)> = OnceLock::new();
    if let Some(support) = SUPPORT.get() {
        return Ok(*support);
    }
    let bin = crate::toolpath::ffmpeg_for(&[crate::toolpath::FfmpegFeature::Vidstab]);
    let detect = vidstab_filter_support(&bin, "filter=vidstabdetect")?;
    let transform = vidstab_filter_support(&bin, "filter=vidstabtransform")?;
    let Some(support) = detect.zip(transform) else {
        // An unavailable/broken executable remains a normal render fallback,
        // but never turns a transient probe failure into permanent capability
        // state. Cancellation/deadline errors returned above remain terminal.
        return Ok((false, false));
    };
    let _ = SUPPORT.set(support);
    Ok(support)
}

fn vidstab_filter_support(bin: &std::ffi::OsStr, filter: &str) -> Result<Option<bool>, CutError> {
    let mut command = Command::new(bin);
    command.args(["-hide_banner", "-h", filter]);
    match run_bounded_command(&mut command, "inspect vidstab filter") {
        Ok(output) => {
            if !output.status.success() {
                return Ok(Some(false));
            }
            let mut help = output.stdout;
            help.extend_from_slice(&output.stderr);
            Ok(Some(String::from_utf8_lossy(&help).contains("fileformat")))
        }
        Err(error) if is_owner_interruption(&error) => Err(error),
        Err(_) => Ok(None),
    }
}

// --- render resource governance ---------------------------------------------
// A heavy / long render must FINISH THE JOB without being able to wedge the box.
// We do NOT hard-cap memory (that would kill the job). Instead, on Linux a render
// runs inside a transient systemd scope with cgroup-v2 `MemoryHigh` (~75% of RAM,
// for memory-heavy desktop media work): at that ceiling the kernel THROTTLES + reclaims pages to
// swap/disk, so memory stops growing and the render keeps going (slower) rather
// than OOM-ing the machine during a large composite.
// `MemoryMax` (leave ~1GB for the OS) is a last-resort backstop confining any kill
// to the render's OWN cgroup — never sshd / the desktop. Every render is `nice`d.
// `MemoryHigh` throttles allocations while `MemoryMax` is a hard stop.
// macOS/Windows have no cgroups but auto-compress /
// page to disk, so the fallback there is a plain nice'd spawn. The real memory
// BOUND is segmented rendering (render.rs) — this is the safety net beneath it.
// Tunables: SHELLX_CUT_RENDER_MEM_HIGH_PCT (75, range 10..=95), SHELLX_CUT_RENDER_NICE (10).

#[cfg(target_os = "linux")]
pub(crate) fn total_ram_bytes() -> u64 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("MemTotal:")
                    .and_then(|r| r.split_whitespace().next())
                    .and_then(|n| n.parse::<u64>().ok())
            })
        })
        .map(|kb| kb.saturating_mul(1024))
        .unwrap_or(0)
}
#[cfg(not(target_os = "linux"))]
pub(crate) fn total_ram_bytes() -> u64 {
    0
}

/// True when renders can be cgroup-soft-limited (the parallel segmented path uses
/// this to decide whether to bound each window's memory + fan out). Re-exports the
/// cached probe so render.rs doesn't duplicate it.
pub(crate) fn cgroup_governance_available() -> Result<bool, CutError> {
    soft_limit_available()
}

/// Probe ONCE whether renders can be cgroup-soft-limited via `systemd-run --user
/// --scope` (needs Linux + systemd + a working user session). Cached.
fn soft_limit_available() -> Result<bool, CutError> {
    static AVAIL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if let Some(available) = AVAIL.get() {
        return Ok(*available);
    }
    if total_ram_bytes() == 0 {
        let _ = AVAIL.set(false);
        return Ok(false);
    }
    let mut command = Command::new("systemd-run");
    command
        .args([
            "--user",
            "--scope",
            "--quiet",
            "-p",
            "MemoryHigh=64M",
            "--",
            "true",
        ])
        .stdin(std::process::Stdio::null());
    match run_bounded_command(&mut command, "probe systemd render governance") {
        Ok(output) => {
            let available = output.status.success();
            let _ = AVAIL.set(available);
            Ok(available)
        }
        Err(error) if is_owner_interruption(&error) => Err(error),
        Err(_) => Ok(false),
    }
}

fn is_owner_interruption(error: &CutError) -> bool {
    error.code == error_codes::RENDER_CANCELLED || error.message == "operation timed out"
}

fn render_nice() -> i32 {
    std::env::var("SHELLX_CUT_RENDER_NICE")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(10)
}

/// Opt-in render thread cap (`SHELLX_CUT_RENDER_THREADS`). Returns the global
/// `-threads N -filter_complex_threads N` flag pair, or an empty vec when unset.
///
/// WHY OPT-IN (default = ffmpeg auto = one thread per core): each encode/filter
/// thread carries its OWN frame buffers, so on a constrained box fewer threads =
/// a smaller resident footprint (it fits under the cgroup `MemoryHigh` ceiling
/// and spills less) at the cost of speed — the "work within fewer resources"
/// lever that complements the memory governance. We do NOT cap by default
/// because (a) it would slow every render on a healthy box and (b) changing the
/// thread count changes the libx264 frame-threading bitstream, so leaving it at
/// the per-machine auto value keeps same-machine render reproducibility intact.
/// Only the RENDER path uses this (it routes through `render_command`); probe /
/// fast-scrub go through the plain `run_ffmpeg` and are never capped.
/// `-filter_complex_threads` is the lever that matters most for the overlay /
/// composite graph; `-threads` caps the encoder. Both are global options, so
/// they sit before `-i` (ahead of `-filter_complex` and the output).
fn render_thread_flags() -> Vec<String> {
    match std::env::var("SHELLX_CUT_RENDER_THREADS")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|n| *n >= 1)
    {
        Some(n) => vec![
            "-threads".into(),
            n.to_string(),
            "-filter_complex_threads".into(),
            n.to_string(),
        ],
        None => Vec::new(),
    }
}

/// `(MemoryHigh, MemoryMax)` bytes for a render cgroup, or `None` if total RAM is
/// unknown. High = soft throttle+spill ceiling (~75%); Max = total − 1GB backstop.
fn render_mem_budget() -> Option<(u64, u64)> {
    let total = total_ram_bytes();
    if total == 0 {
        return None;
    }
    let pct = std::env::var("SHELLX_CUT_RENDER_MEM_HIGH_PCT")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|p| (10..=95).contains(p))
        .unwrap_or(75);
    let high = total / 100 * pct;
    let one_gb = 1024 * 1024 * 1024;
    let max = total.saturating_sub(one_gb).max(high + 256 * 1024 * 1024);
    Some((high, max))
}

/// Build the ffmpeg `Command` for a RENDER, governed so it cannot wedge the box:
/// a soft memory ceiling (cgroup `MemoryHigh`, throttle+spill — NOT a kill) + nice
/// on Linux+systemd; a plain `nice`d spawn elsewhere. `flags` precede `args`.
fn render_command(flags: &[&str], args: &[String]) -> Result<Command, CutError> {
    render_command_governed(flags, args, None, None)
}

/// `render_command` with explicit governance overrides for the PARALLEL segmented
/// path: `mem_override = Some((high, max))` pins this window's cgroup memory cap
/// (so N concurrent windows collectively fit under the budget — each gets
/// budget/N), and `threads_override = Some(n)` pins its thread count (so N windows
/// × n threads ≈ cores). `None` for either falls back to the single-render
/// defaults (75%-of-total ceiling / the env thread cap).
fn render_command_governed(
    flags: &[&str],
    args: &[String],
    mem_override: Option<(u64, u64)>,
    threads_override: Option<u32>,
) -> Result<Command, CutError> {
    let normalized_args = normalize_ffmpeg_args(args);
    let bin = ffmpeg_bin_for_args(&normalized_args);
    let nice = render_nice();
    // Thread cap, injected after the global flags (precedes `-i`/`-filter_complex`/
    // output). An explicit override (parallel path) wins; else the env knob.
    let threads = match threads_override {
        Some(n) => vec![
            "-threads".into(),
            n.to_string(),
            "-filter_complex_threads".into(),
            n.to_string(),
        ],
        None => render_thread_flags(),
    };
    if soft_limit_available()? {
        if let Some((high, max)) = mem_override.or_else(render_mem_budget) {
            // Nice= is not a valid SCOPE property (no exec context) — apply nice
            // by running ffmpeg under the `nice` tool INSIDE the memory-governed
            // scope. The scope gives cgroup MemoryHigh/Max; `nice` gives CPU courtesy.
            let mut cmd = Command::new("systemd-run");
            cmd.args(["--user", "--scope", "--collect", "--quiet"])
                .arg("-p")
                .arg(format!("MemoryHigh={high}"))
                .arg("-p")
                .arg(format!("MemoryMax={max}"))
                .arg("--")
                .arg("nice")
                .arg("-n")
                .arg(nice.to_string())
                .arg(&bin)
                .args(flags)
                .args(&threads)
                .args(&normalized_args);
            return Ok(cmd);
        }
    }
    // Fallback: nice'd plain ffmpeg (`nice` is POSIX — present on Linux/macOS).
    #[cfg(unix)]
    {
        let mut cmd = Command::new("nice");
        cmd.arg("-n")
            .arg(nice.to_string())
            .arg(&bin)
            .args(flags)
            .args(&threads)
            .args(&normalized_args);
        Ok(cmd)
    }
    #[cfg(not(unix))]
    {
        let mut cmd = Command::new(&bin);
        cmd.args(flags).args(&threads).args(&normalized_args);
        Ok(cmd)
    }
}

/// Run ffmpeg with `args`, capturing stderr. On nonzero exit returns a
/// CutError with code "ffmpeg" whose cause carries the stderr TAIL (last
/// ~2KB) — that is where ffmpeg puts the actual reason.
pub fn run_ffmpeg(args: &[String]) -> Result<(), CutError> {
    let normalized_args = normalize_ffmpeg_args(args);
    let mut command = Command::new(ffmpeg_bin_for_args(&normalized_args));
    command
        .args(["-hide_banner", "-nostdin", "-y"])
        .args(&normalized_args);
    let out = process::command_output_with_current_control(&mut command, "ffmpeg")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: String = stderr
            .chars()
            .skip(stderr.chars().count().saturating_sub(2000))
            .collect();
        return Err(CutError::new(
            error_codes::FFMPEG,
            format!("ffmpeg exited with {}", out.status),
            tail,
        ));
    }
    Ok(())
}

/// Run ffmpeg where the last arg is the output path, publishing it atomically.
/// ffmpeg writes a unique temp sibling first; only a successful encode is renamed
/// to `out`. This prevents killed/failed encodes from becoming permanent cache
/// hits at the final path.
pub fn run_ffmpeg_atomic_output(args: &[String], out: &Path) -> Result<(), CutError> {
    crate::atomic_output::run_with_atomic_output(args, out, run_ffmpeg)
}

/// Run ffmpeg streaming `-progress` to stdout, mapping `out_time_us` against
/// `total_ms` into `on_progress(0.0..=1.0)` callbacks (server contract job_progress
/// events). stderr is collected on a side thread so a chatty encode can never
/// deadlock the pipe; on failure its tail becomes the CutError cause.
pub fn run_ffmpeg_with_progress(
    args: &[String],
    total_ms: u64,
    on_progress: &dyn Fn(f32),
) -> Result<(), CutError> {
    // Renders run governed (cgroup MemoryHigh soft-limit + nice) so a heavy / long
    // timeline throttles+spills instead of OOM-wedging the box. See render_command.
    let cmd = render_command(
        &["-hide_banner", "-nostdin", "-y", "-progress", "pipe:1"],
        args,
    )?;
    drive_ffmpeg(cmd, total_ms, on_progress)
}

/// Progress-reporting counterpart to [`run_ffmpeg_atomic_output`]. Long final
/// renders retain job progress while their final path is published only after a
/// successful encode.
pub fn run_ffmpeg_with_progress_atomic_output(
    args: &[String],
    out: &Path,
    total_ms: u64,
    on_progress: &dyn Fn(f32),
) -> Result<(), CutError> {
    crate::atomic_output::run_with_atomic_output(args, out, |tmp_args| {
        run_ffmpeg_with_progress(tmp_args, total_ms, on_progress)
    })
}

/// Run ONE segmented-render window with an explicit per-window cgroup memory cap
/// (`high`/`max` bytes) + thread count — the parallel path runs N of these at
/// once, each capped at budget/N so they collectively fit under the RAM budget
/// (and each pinned to cores/N threads). Falls back to the same streamed-progress
/// driver. On non-Linux / no-systemd boxes the caps are inert (plain nice'd spawn).
pub fn run_render_window(
    args: &[String],
    total_ms: u64,
    on_progress: &dyn Fn(f32),
    high: u64,
    max: u64,
    threads: u32,
) -> Result<(), CutError> {
    let cmd = render_command_governed(
        &["-hide_banner", "-nostdin", "-y", "-progress", "pipe:1"],
        args,
        Some((high, max)),
        Some(threads),
    )?;
    drive_ffmpeg(cmd, total_ms, on_progress)
}

/// Spawn a prepared ffmpeg `Command`, stream its `-progress` to `on_progress`,
/// drain stderr concurrently, and surface a CutError (with the stderr tail) on a
/// nonzero exit. Shared by the single-render and per-window paths.
fn drive_ffmpeg(
    mut cmd: Command,
    total_ms: u64,
    on_progress: &dyn Fn(f32),
) -> Result<(), CutError> {
    process::drive_with_current_control(&mut cmd, total_ms, on_progress)
}

/// Escape a filesystem path for use INSIDE a filter argument (e.g.
/// `ass=filename=...`). Filter-graph parsing eats `\`, `:`, `'` and `,` —
/// the documented escaping is backslash + single-quote wrapping; we escape
/// the four metacharacters individually which survives both parse levels.
/// Escape a filesystem path for embedding as a VALUE inside an ffmpeg
/// `filter_complex` graph (e.g. `ass=filename=<here>`, `lut3d=file=<here>`).
///
/// Returns a **single-quoted** token. ffmpeg parses a filtergraph in two
/// levels — the graph tokenizer, then each filter's own `opt=val:opt=val`
/// option parser — and a Windows path (`C:\Users\…`) trips BOTH: the drive
/// colon looks like an option separator and the backslash looks like an
/// escape. Empirically (ffmpeg N-125019, isolated via `-filter_complex_script`
/// so argv/shell translation can't confound it) the ONLY robust form is
/// quote-wrapped AND colon-escaped — quoting alone or escaping alone both fail
/// with `No option name near '\Users…'`:
///   - `filename=C\:\\Users\\…`   → FAIL   (old behaviour: escaped, not quoted)
///   - `filename='C:/Users/…'`     → FAIL   (quoted, colon not escaped)
///   - `filename=C\:/Users/…`      → FAIL   (escaped, not quoted)
///   - `filename='C\:/Users/…'`    → PASS   ← this function's output
///
/// Backslashes are converted to forward slashes (Windows ffmpeg accepts `/`
/// and it needs no escaping inside the quotes, unlike `\`). The colon is still
/// escaped because the quote alone does not protect it from the filter-option
/// parser. The single quotes also make paths containing spaces work on every
/// platform. A literal apostrophe cannot remain inside an ffmpeg single-quoted
/// token: close the token, send a triply escaped apostrophe through both parser
/// levels, then reopen it (`'\\\''`). Verified with a real LUT file whose
/// parent directory contains an apostrophe. Callers must embed the result
/// WITHOUT adding their own quotes.
pub fn escape_filter_path(path: &Path) -> String {
    let inner = normalize_ffmpeg_arg(&path.display().to_string())
        .replace('\\', "/") // Windows backslash → forward slash (ffmpeg-safe, no escaping needed)
        .replace('\'', r"'\\\''") // close quote, preserve apostrophe across both parsers, reopen
        .replace(':', "\\:"); // drive colon: escaped even though quoted (filter-option parser)
    format!("'{inner}'")
}

/// Build one concat-demuxer list-file line for `path`.
///
/// The concat demuxer parses a small text format, not a shell command. It still
/// treats backslash and single quote as token syntax, so raw paths like
/// `exports/bob's cut/seg.mp4` corrupt the list unless escaped before writing
/// `file '<path>'`.
pub fn concat_demuxer_file_line(path: &Path) -> String {
    let escaped = normalize_ffmpeg_arg(&path.display().to_string())
        .replace('\\', "\\\\")
        .replace('\'', "\\'");
    format!("file '{escaped}'")
}

/// Run ffprobe in JSON mode against `path` → parsed `serde_json::Value`
/// (`-show_format -show_streams`). Raw shape; probe.rs normalizes it.
pub fn ffprobe_json(path: &Path) -> Result<serde_json::Value, CutError> {
    let mut command = Command::new(ffprobe_bin());
    command
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path);
    let out = run_bounded_command(&mut command, "ffprobe").map_err(|e| {
        CutError::new(
            error_codes::FFMPEG,
            "failed to spawn ffprobe",
            e.to_string(),
        )
    })?;
    if !out.status.success() {
        return Err(CutError::new(
            error_codes::FFMPEG,
            format!("ffprobe failed on {}", path.display()),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| {
        CutError::new(
            error_codes::FFMPEG,
            "ffprobe emitted unparseable JSON",
            e.to_string(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Locks in the verified ffmpeg filtergraph path escaping,
    ///. The ONLY form that parses on Windows ffmpeg is
    /// single-quoted + forward-slashed + escaped-colon; the previous output
    /// (`C\:\\Users\\…`, escaped but unquoted) failed with
    /// `No option name near '\Users…'` whenever a render burned in captions
    /// (`ass=filename=`) or applied a LUT grade (`lut3d=file=`).
    #[test]
    fn escape_filter_path_quotes_forward_slashes_and_escapes_colon() {
        // `\\` in the Rust literal = one real backslash → a Windows-style path.
        let got = escape_filter_path(Path::new("C:\\Users\\Example\\proj\\burnin.ass"));
        assert_eq!(got, "'C\\:/Users/Example/proj/burnin.ass'");
        assert!(
            got.starts_with('\'') && got.ends_with('\''),
            "must be single-quoted"
        );
    }

    /// Unix paths get the same quoting (also makes spaces-in-path work) and
    /// have no colon/backslash to transform.
    #[test]
    fn escape_filter_path_quotes_unix_path_with_space() {
        let got = escape_filter_path(Path::new("/home/u/My Clips/burnin.ass"));
        assert_eq!(got, "'/home/u/My Clips/burnin.ass'");
    }

    #[test]
    fn escape_filter_path_preserves_apostrophe_across_both_parsers() {
        let got = escape_filter_path(Path::new("/home/u/editor's assets/identity.cube"));
        assert_eq!(got, r"'/home/u/editor'\\\''s assets/identity.cube'");
    }

    #[test]
    fn concat_demuxer_file_line_escapes_token_syntax() {
        let got = concat_demuxer_file_line(Path::new("/tmp/bob's cut/seg\\01.mp4"));
        assert_eq!(got, "file '/tmp/bob\\'s cut/seg\\\\01.mp4'");
    }

    #[test]
    fn normalize_ffmpeg_args_strips_windows_verbatim_paths() {
        let got = normalize_ffmpeg_args(&[
            r"\\?\C:\Users\Example\project\concat.txt".into(),
            r"\\?\UNC\server\share\segment.mp4".into(),
            "-filter_complex".into(),
        ]);
        assert_eq!(
            got,
            vec![
                r"C:\Users\Example\project\concat.txt",
                r"\\server\share\segment.mp4",
                "-filter_complex",
            ]
        );
    }

    #[test]
    fn concat_demuxer_file_line_strips_windows_verbatim_prefix() {
        let got = concat_demuxer_file_line(Path::new(r"\\?\C:\Users\Example\project\seg_01.mp4"));
        assert_eq!(got, r"file 'C:\\Users\\Example\\project\\seg_01.mp4'");
    }

    /// Render thread cap is OFF by default (empty → ffmpeg auto = output
    /// unchanged) and, when set, emits BOTH the encoder (`-threads`) and the
    /// filter-graph (`-filter_complex_threads`) caps at the requested count.
    /// Env is process-global, so serialize the set/clear within one test.
    #[test]
    fn render_thread_flags_off_by_default_and_caps_both_when_set() {
        // Default (unset): no flags — every render keeps ffmpeg's per-core auto.
        std::env::remove_var("SHELLX_CUT_RENDER_THREADS");
        assert!(render_thread_flags().is_empty());

        // Set: caps the encoder AND the filter graph at N.
        std::env::set_var("SHELLX_CUT_RENDER_THREADS", "4");
        assert_eq!(
            render_thread_flags(),
            vec![
                "-threads".to_string(),
                "4".into(),
                "-filter_complex_threads".into(),
                "4".into(),
            ]
        );

        // Garbage / zero is ignored (treated as unset — never emits `-threads 0`).
        std::env::set_var("SHELLX_CUT_RENDER_THREADS", "0");
        assert!(render_thread_flags().is_empty());
        std::env::set_var("SHELLX_CUT_RENDER_THREADS", "notanumber");
        assert!(render_thread_flags().is_empty());
        std::env::remove_var("SHELLX_CUT_RENDER_THREADS");
    }
}
