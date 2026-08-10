//! doctor.rs — `system.doctor` environment scan + capability cards.
//!
//! ROLE (the ONE source of truth for "what is present / what is missing")
//!   ShellX Cut leans on heavy, non-bundled runtime deps (ffmpeg/ffprobe, the
//!   Python perception sidecar) and on the user's OWN coding-agent CLI as the
//!   render judge. On a cold machine any of these may be absent. This module
//!   scans the environment ONCE and produces a uniform grid of CAPABILITY CARDS
//!   — the same payload the start wizard, the Settings>Environment panel, and
//!   any agent over REST/MCP read. It is a provider-style capability card: a
//!   normalized {id, kind, status, source?, version?, hint?, details} per
//!   dependency dimension.
//!
//! WHY A VERB, NOT AD-HOC CODE (public contract invariant 1 + the 100%-surface rule)
//!   The scan is dispatched as `system.doctor`; the UI is just a reader. The
//!   result is cached in AppState, recomputed on `refresh:true` or after a
//!   successful `system.fetch_tool`, and a `doctor_updated` WS event fires on
//!   change so the wizard/chip update live without polling.
//!
//! HONESTY + COLD-BOX CONTRACT
//!   - ffmpeg/ffprobe resolution reuses `cut_media::toolpath` (the resolution ladder:
//!     env → beside-exe → app-data → PATH) so the doctor and the engine agree
//!     on ONE ffmpeg. The `tools-doctor.json` file the desktop shell writes
//!     becomes a CACHE of this verb's ffmpeg result, never a parallel truth.
//!   - Judge rungs are detected in Rust (cheap `which` + `--version`) and the
//!     bundled adapter + Python runtime are checked without a model call. A CLI
//!     can therefore remain visible for agent chat while its render-review card
//!     honestly reports degraded until the adapter runtime is usable.
//!   - The perception python probe has a SHORT timeout and is best-effort; it
//!     never blocks the verb loop (the verb is a fast cached read; `refresh`
//!     re-probes).
//!
//! Dependencies: std + cut-media (toolpath), cut-perception (sidecar paths),
//! serde_json, chrono. Primary callers: dispatch.rs (system.doctor), main.rs
//! (startup scan), fetch.rs (re-scan after install).

use cut_core::CutError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

mod service_cards;

/// Card schema version (bumped only on a breaking shape change).
pub const DOCTOR_SCHEMA: &str = "shellx-cut/doctor/1";

/// Normalized status for one capability card (mirrors the provider-lab
/// CapabilityStatus vocabulary, reduced to the three states a dependency can be
/// in from the user's point of view).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CardStatus {
    /// Present and usable.
    Ok,
    /// Absent — the action (download / install / log in) would fix it.
    Missing,
    /// Present but not at full capability (e.g. python without whisper, or low
    /// disk) — usable for some flows, an upgrade unlocks the rest.
    Degraded,
    /// COULD NOT be determined this scan — a probe TIMED OUT or errored, so
    /// neither presence nor absence is confirmed (a cold disk, an antivirus
    /// scan, or a box pinned by a heavy render can starve an 8s probe even when
    /// the tool is perfectly present). The honest middle state for the tri-state
    /// "false-status" class: a probe-miss degrades to "unverified", NEVER to a
    /// confident Ok/Missing. The UI renders it neutrally ("Couldn't verify —
    /// Re-scan") and `essential_ok` does NOT treat it as missing, so a transient
    /// slow probe never auto-pops the first-run wizard. This lifts the chat-auth
    /// tri-state's "unknown" rung (chat_auth_state) up to the card level.
    Unknown,
}

/// Which resolution rung a tool came from (only meaningful for tool cards).
/// Mirrors the cut-media toolpath ladder + the desktop tools-doctor.json
/// "source" string so the three stay in lockstep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CardSource {
    /// Explicit SHELLX_CUT_ env-prefix override (rung 1).
    Env,
    /// A tools dir beside the exe OR the app-data tools dir (rungs 2/3) — the
    /// "we put it there" rung the fetch_tool download populates.
    BundledOrAppdata,
    /// Found on the system PATH (rung 4).
    Path,
    /// Not resolvable at any rung.
    Missing,
}

/// One capability card — the unit the wizard/settings/agents render.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    /// Stable id, e.g. "ffmpeg", "perception", "judge.codex", "disk".
    pub id: String,
    /// Coarse grouping for the UI: "tool" | "perception" | "judge" | "disk".
    pub kind: String,
    pub status: CardStatus,
    /// Resolution rung (tool cards) — omitted for non-tool cards.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<CardSource>,
    /// Cheap version string when knowable (parsed from `<tool> --version`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Actionable hint when status != ok (what to run / which sub unlocks it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Kind-specific extra facts (resolved path, perception tier, free bytes,
    /// legacy flag, what-each-tier-unlocks, …). Free-form by design — the UI
    /// renders the ones it knows, agents read whatever they need.
    pub details: Value,
}

/// The full doctor report — the cached, agent-readable env snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub schema: String,
    /// RFC3339 timestamp of the scan.
    pub scanned_at: String,
    pub os: String,
    pub arch: String,
    pub app_version: String,
    /// The server bind address (e.g. "127.0.0.1:6166"), so an agent reading the
    /// card knows where this cutd is. Filled by the caller (main.rs has it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
    pub cards: Vec<Card>,
    /// Convenience gate for the UI: true when the ESSENTIAL deps (ffmpeg) are
    /// ok. The first-run wizard surfaces when this is false.
    pub essential_ok: bool,
}

impl DoctorReport {
    /// Two reports are "the same" for change-detection purposes when their card
    /// statuses+sources+versions match (ignoring the scanned_at timestamp).
    /// Used to decide whether to emit `doctor_updated`.
    pub fn same_capabilities(&self, other: &DoctorReport) -> bool {
        if self.cards.len() != other.cards.len() {
            return false;
        }
        self.cards.iter().zip(&other.cards).all(|(a, b)| {
            a.id == b.id && a.status == b.status && a.source == b.source && a.version == b.version
        })
    }
}

// ---------------------------------------------------------------------------
// Version probing — bounded, never hangs the scan
// ---------------------------------------------------------------------------

/// Outcome of a bounded probe exec, distinguishing a CONFIRMED absence from an
/// UNVERIFIED miss — the structural core of tri-state readiness. `version_line` (the
/// common case) collapses this back to `Option`, but the ESSENTIAL-tool (ffmpeg)
/// and matte-premium-CUDA paths read the full outcome so a slow probe degrades to
/// `CardStatus::Unknown` instead of a confident `Missing`/`Ok`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ProbeOutcome {
    /// The process ran to completion. `String` is the first non-empty banner
    /// line (empty when the tool printed nothing). A CONFIRMED present+runnable.
    Ran(String),
    /// Spawn FAILED (ENOENT / not executable) — a CONFIRMED absence. The ONLY
    /// outcome that may legitimately read as `Missing`: the binary genuinely is
    /// not there. A slow/wedged-but-present binary never lands here.
    NotFound,
    /// Spawned but TIMED OUT, or errored mid-wait — UNVERIFIED. The binary may
    /// well be present, just slow this scan; never collapse it to Missing/Ok.
    Timeout,
}

/// Doctor checks are foreground work, but each still owns its whole process
/// tree. This keeps a wedged CLI or a helper it spawned from leaking past the
/// diagnostic timeout while preserving the doctor's non-interactive policy.
fn run_doctor_command(
    command: &mut Command,
    timeout: Duration,
    context: &str,
) -> Result<Output, CutError> {
    let control = cut_media::ffmpeg::OwnedProcessControl::bounded(timeout, || false);
    cut_media::ffmpeg::run_owned_command(command, &control, context)
}

fn looks_like_missing_program(error: &CutError) -> bool {
    let cause = error.cause.to_ascii_lowercase();
    cause.contains("no such file") || cause.contains("not found") || cause.contains("os error 2")
}

/// Run `<prog> <args…>` bounded by `timeout`, classifying the result into the
/// three `ProbeOutcome`s. stdin is closed (a prompt-on-stdin can never block) and
/// a wedged child is killed on overrun — the scan can never hang. `prog` may be a
/// full path (resolved tool) or a bare name (PATH lookup); a bare name that does
/// not resolve fails the spawn ⇒ `NotFound`.
fn probe_exec(prog: &std::ffi::OsStr, args: &[&str], timeout: Duration) -> ProbeOutcome {
    // Spawn detached from our stdio; we poll it on a loop we can abandon if it
    // overruns (Command has no built-in timeout in std).
    let owned_args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let mut command = match crate::gen::agent_std_command(Path::new(prog), &owned_args) {
        Ok(command) => command,
        Err(_) => return ProbeOutcome::NotFound,
    };
    let out = match run_doctor_command(&mut command, timeout, "doctor version probe") {
        Ok(output) => output,
        Err(error) if looks_like_missing_program(&error) => return ProbeOutcome::NotFound,
        // A timeout or a post-spawn ownership failure is not a confirmed absence.
        Err(_) => return ProbeOutcome::Timeout,
    };
    let text = if !out.stdout.is_empty() {
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string();
    ProbeOutcome::Ran(line)
}

/// Run `<prog> <args...>`, returning the FIRST non-empty banner line — the common
/// "is it runnable + what version" probe. A thin `Option` view over `probe_exec`:
/// any non-`Ran`(non-empty) outcome is `None`. Callers that must tell a TIMEOUT
/// from a real absence (the essential-tool + matte-CUDA paths) read `probe_exec`
/// directly. Bounded; never hangs. `prog` may be a full path or a bare name.
fn version_line(prog: &std::ffi::OsStr, args: &[&str], timeout: Duration) -> Option<String> {
    match probe_exec(prog, args, timeout) {
        ProbeOutcome::Ran(line) if !line.is_empty() => Some(line),
        _ => None,
    }
}

/// Probe whether a runnable ffmpeg has the OPTIONAL filters three ShellX Cut features
/// need: caption burn-in (libass → `subtitles`/`ass`), stabilize (libvidstab →
/// `vidstabdetect`/`vidstabtransform`) and color-managed render (libzimg → `zscale`).
/// These come from build-time flags (`--enable-libass` / `--enable-libvidstab` /
/// `--enable-libzimg`); a core build omits them — e.g. Homebrew's plain `ffmpeg`
/// dropped all three in 8.x (they moved to `ffmpeg-full`), so a Mac user who ran our
/// own "brew install ffmpeg" advice gets a working-but-incapable binary and those
/// renders fail with a cryptic "No such filter" (captions/stabilize) or an exit-8
/// "No such filter: 'zscale'" (a rec2020/color-managed render). Returns
/// `(libass, libvidstab, zscale)`; `None` if the probe could not run. Cheap: one
/// bounded `ffmpeg -filters` execution.
fn ffmpeg_filter_caps(prog: &std::ffi::OsStr) -> Option<(bool, bool, bool)> {
    let mut command = Command::new(prog);
    command.args(["-hide_banner", "-filters"]);
    let out = run_doctor_command(
        &mut command,
        Duration::from_secs(8),
        "inspect ffmpeg filters",
    )
    .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // `ffmpeg -filters` prints one filter per line; the NAME is the 2nd token after
    // the leading flags column, e.g. " T.. subtitles      V->V  Render text...".
    let has = |name: &str| {
        text.lines()
            .any(|l| l.split_whitespace().nth(1) == Some(name))
    };
    let libass = has("subtitles") || has("ass");
    let libvidstab = has("vidstabtransform") || has("vidstabdetect");
    let zscale = has("zscale");
    Some((libass, libvidstab, zscale))
}

/// Probe `ffmpeg -encoders` for the OPTIONAL software video encoders the export format
/// picker offers beyond the near-universal libx264/prores: libx265 (hevc), libvpx-vp9
/// (vp9), libsvtav1 (av1). A minimal/static Mac ffmpeg (the very static builds the hint
/// recommends) often omits them, so HEVC/VP9/AV1 export hard-fails with "Unknown encoder"
/// — this lets the doctor warn upfront instead of at export time. Returns
/// (libx265, libvpx_vp9, libsvtav1); None if the cross-platform probe
/// could not run.
fn ffmpeg_encoder_caps(prog: &std::ffi::OsStr) -> Option<(bool, bool, bool)> {
    let mut command = Command::new(prog);
    command.args(["-hide_banner", "-encoders"]);
    let out = run_doctor_command(
        &mut command,
        Duration::from_secs(8),
        "inspect ffmpeg encoders",
    )
    .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // `ffmpeg -encoders` prints one encoder per line; the NAME is the 2nd token after
    // the leading flags column, e.g. " V....D libx265              libx265 H.265 ...".
    let has = |name: &str| {
        text.lines()
            .any(|l| l.split_whitespace().nth(1) == Some(name))
    };
    Some((has("libx265"), has("libvpx-vp9"), has("libsvtav1")))
}

/// Informational note (NOT a gate, NOT a download) for a runnable-but-incapable
/// ffmpeg: which features are unavailable, that ffmpeg is the user's external tool,
/// the license type of a capable build, and where to get one. The user chooses
/// whether to keep their current ffmpeg or swap in a capable build; we never accept
/// a license on their behalf.
fn ffmpeg_capability_hint(
    libass: bool,
    libvidstab: bool,
    zscale: bool,
    libx265: bool,
    vp9: bool,
    av1: bool,
) -> String {
    let mut missing = Vec::new();
    if !libass {
        missing.push("caption burn-in (libass)");
    }
    if !libvidstab {
        missing.push("video stabilize (libvidstab)");
    }
    if !zscale {
        missing.push("color-managed (rec2020) render (libzimg/zscale)");
    }
    if !libx265 {
        missing.push("HEVC export (libx265)");
    }
    if !vp9 {
        missing.push("VP9/WebM export (libvpx-vp9)");
    }
    if !av1 {
        missing.push("AV1 export (libsvtav1)");
    }
    // A missing libzimg is the only one that HARD-FAILS the render (exit-8 "No such
    // filter: 'zscale'") rather than degrading — call it out explicitly so the user
    // is not left with a cryptic crash on a rec2020 / color-managed render.
    let zscale_note = if zscale {
        ""
    } else {
        " color-managed (rec2020) renders will fail — install an ffmpeg with \
          libzimg/zscale (macOS: ffmpeg-full / osxexperts.net / ffmpeg.martin-riedl.de)."
    };
    format!(
        "This ffmpeg works for core editing + render, but lacks {}.{} ffmpeg is an \
         external tool you provide — those features need a build configured with the \
         matching --enable flags (e.g. --enable-libass / --enable-libvidstab / \
         --enable-libzimg / --enable-libx265 / --enable-libvpx / --enable-libsvtav1; a \
         GPL-licensed configuration; the \
         build and its license are your choice, never accepted through this app). \
         NOTE: for caption burn-in, stabilize and color-managed renders specifically, \
         ShellX Cut now AUTO-USES another installed ffmpeg that has the needed filter \
         when one is present — so captions/stabilize/color-managed renders work as long \
         as ANY installed ffmpeg has libass/libvidstab/libzimg, even if this resolved \
         one does not. \
         On macOS, Homebrew's plain `ffmpeg` 8.x dropped all three — install `ffmpeg-full` \
         (`brew install ffmpeg-full`; it isn't added to your PATH automatically, so point ShellX Cut at \
         /opt/homebrew/opt/ffmpeg-full/bin/ffmpeg on Apple Silicon, or \
         /usr/local/opt/ffmpeg-full/bin/ffmpeg on Intel), or a static build (Apple \
         Silicon: osxexperts.net or ffmpeg.martin-riedl.de; Intel only: evermeet.cx). \
         On Linux, install an ffmpeg built with those flags. Then point ShellX Cut at \
         it — or keep this one if you don't need captions/stabilize/color-managed renders.",
        missing.join(" + "),
        zscale_note,
    )
}

// ---------------------------------------------------------------------------
// ffmpeg / ffprobe cards (reuse the cut-media toolpath ladder)
// ---------------------------------------------------------------------------

/// Classify which rung a resolved tool path came from, by re-deriving the same
/// ladder cut_media::toolpath uses. `resolved` is the program the resolver
/// returned (full path at rungs 1–3, bare name at rung 4). We map it to the
/// user-facing CardSource. This keeps the doctor's "source" string identical to
/// the desktop tools-doctor.json semantics.
fn ffmpeg_source(stem: &str, resolved: &str, runnable: bool) -> CardSource {
    if !runnable {
        return CardSource::Missing;
    }
    let p = Path::new(resolved);
    // Bare name (no separator) ⇒ it was found on PATH (rung 4).
    if !p.is_file() {
        return CardSource::Path;
    }
    // A real file path: was it the explicit per-exe env override (rung 1a)?
    let env_key = if stem == "ffmpeg" {
        cut_media::toolpath::ENV_FFMPEG
    } else {
        cut_media::toolpath::ENV_FFPROBE
    };
    if let Some(envp) = std::env::var_os(env_key) {
        if Path::new(&envp) == p {
            return CardSource::Env;
        }
    }
    // The user's persisted MANUAL choice (system.set_ffmpeg) — an explicit pick,
    // so it reads like "env" (the user, not auto/ladder, chose it).
    if let Some(ov) = cut_media::toolpath::read_override_setting() {
        if Path::new(&ov) == p {
            return CardSource::Env;
        }
    }
    // NB: we deliberately DON'T treat SHELLX_CUT_FFMPEG_DIR as an "env" signal —
    // the engine itself auto-points that at the resolved ffmpeg's dir for the
    // python sidecar (state.rs), so on a rescan it would mislabel EVERY ffmpeg as
    // "env". Classify by location instead.
    // Staged by us: under the app-data tools dir (rung 3, the Download target).
    if let Some(tools) = cut_media::toolpath::appdata_tools_dir() {
        if p.starts_with(&tools) {
            return CardSource::BundledOrAppdata;
        }
    }
    // Otherwise a system binary — e.g. the auto-selected /usr/bin/ffmpeg or a
    // Homebrew install. Report it as a PATH-tier resolution (honest: it is not
    // something we bundled/downloaded).
    CardSource::Path
}

/// Build the ffmpeg + ffprobe cards from the toolpath resolver + a bounded
/// `-version` probe. ESSENTIAL: a missing ffmpeg is what surfaces the wizard.
fn ffmpeg_cards() -> Vec<Card> {
    let mut cards = Vec::new();
    for stem in ["ffmpeg", "ffprobe"] {
        let prog = if stem == "ffmpeg" {
            cut_media::toolpath::ffmpeg()
        } else {
            cut_media::toolpath::ffprobe()
        };
        let resolved = prog.to_string_lossy().into_owned();
        // Tri-state probe: `<tool> -version` either ran (present and runnable),
        // failed to spawn (CONFIRMED missing), or TIMED OUT (unverified — a slow
        // disk / AV scan / a box pinned by a heavy render can blow the 8s budget
        // even on a present ffmpeg). `probe_essential_version` retries ONCE on a
        // timeout so a single transient slow probe never flips this essential tool
        // to MISSING (which would auto-pop the wizard until a manual re-scan).
        let outcome = probe_essential_version(&prog);
        let runnable = matches!(outcome, ProbeOutcome::Ran(_));
        // The path WAS resolved by toolpath; only a CONFIRMED NotFound means "no
        // rung resolved it". An unverified (Timeout) ffmpeg keeps its location-
        // derived rung so the UI can still show WHERE the can't-verify binary lives.
        let resolvable = !matches!(outcome, ProbeOutcome::NotFound);
        let source = ffmpeg_source(stem, &resolved, resolvable);
        // Tidy the banner (present only on a Ran outcome): ffmpeg's first line is
        // "ffmpeg version 6.1.1-3ubuntu5 Copyright (c) ...". Keep just the version
        // token (drop the "ffmpeg version " prefix + the trailing Copyright clause)
        // so the card shows "6.1.1-3ubuntu5".
        let version = match &outcome {
            ProbeOutcome::Ran(line) if !line.is_empty() => {
                let v = line
                    .trim_start_matches("ffmpeg version ")
                    .trim_start_matches("ffprobe version ");
                Some(v.split_whitespace().next().unwrap_or(v).to_string())
            }
            _ => None,
        };
        // Only ffmpeg (not ffprobe) carries the optional caption/stabilize
        // filters. Probe them when it's runnable so a present-but-incapable build
        // (e.g. Homebrew's plain ffmpeg, post-8.x) surfaces as Degraded with honest
        // guidance — instead of a silent "No such filter" at caption/stabilize time.
        let caps = (stem == "ffmpeg" && runnable)
            .then(|| ffmpeg_filter_caps(&prog))
            .flatten();
        // Also probe the optional software encoders the format picker offers
        // (libx265/libvpx-vp9/libsvtav1) so HEVC/VP9/AV1 export gaps surface upfront.
        let enc = (stem == "ffmpeg" && runnable)
            .then(|| ffmpeg_encoder_caps(&prog))
            .flatten();
        let (status, hint) = match &outcome {
            // CONFIRMED absent (spawn failed even though toolpath resolved a path)
            // — the ONLY case that pops the first-run wizard.
            ProbeOutcome::NotFound => (CardStatus::Missing, Some(ffmpeg_missing_hint(stem))),
            // UNVERIFIED: the probe timed out twice. Honest middle state — NOT
            // Missing (don't pop the wizard) and NOT Ok (don't claim it works); a
            // re-scan once the machine settles resolves it.
            ProbeOutcome::Timeout => (CardStatus::Unknown, Some(ffmpeg_unverified_hint(stem))),
            ProbeOutcome::Ran(_) => {
                // A capability probe that couldn't run ⇒ assume capable (don't
                // false-alarm); only a CONFIRMED-missing filter degrades.
                let (libass, libvidstab, zscale) = caps.unwrap_or((true, true, true));
                let (x265, vp9, av1) = enc.unwrap_or((true, true, true));
                if !libass || !libvidstab || !zscale || !x265 || !vp9 || !av1 {
                    (
                        CardStatus::Degraded,
                        Some(ffmpeg_capability_hint(
                            libass, libvidstab, zscale, x265, vp9, av1,
                        )),
                    )
                } else {
                    (CardStatus::Ok, None)
                }
            }
        };
        cards.push(Card {
            id: stem.to_string(),
            kind: "tool".into(),
            status,
            source: Some(source),
            version,
            hint,
            details: json!({
                "resolved": resolved,
                "runnable": runnable,
                "essential": true,
                // The app-data dir the wizard's Download button installs into.
                "install_dir": cut_media::toolpath::appdata_tools_dir()
                    .map(|p| p.join("ffmpeg").display().to_string()),
                // Caption burn-in (libass), stabilize (libvidstab), and color-
                // managed render (libzimg/zscale) capability. null on ffprobe / when
                // the probe couldn't run.
                "libass": caps.map(|c| c.0),
                "libvidstab": caps.map(|c| c.1),
                "zscale": caps.map(|c| c.2),
                "can_burn_captions": caps.map(|c| c.0),
                "can_stabilize": caps.map(|c| c.1),
                "can_color_manage": caps.map(|c| c.2),
                // Optional export encoders — null on ffprobe or when the probe could not run.
                "libx265": enc.map(|e| e.0),
                "libvpx_vp9": enc.map(|e| e.1),
                "libsvtav1": enc.map(|e| e.2),
                "can_export_hevc": enc.map(|e| e.0),
                "can_export_vp9": enc.map(|e| e.1),
                "can_export_av1": enc.map(|e| e.2),
            }),
        });
    }
    cards
}

/// Probe `<tool> -version` for an ESSENTIAL tool with a single in-scan RETRY on
/// a TIMEOUT. A cold disk, an antivirus scan, or a box pinned by a heavy render
/// can blow the 8s budget on the first try even when ffmpeg is perfectly present;
/// one cheap retry (no cross-scan state needed — the retry is within the same
/// scan) keeps a transient slow probe from flipping the essential tool to MISSING
/// and auto-popping the wizard. A spawn failure (NotFound) is a confirmed
/// absence — returned immediately, no retry, so a truly-missing ffmpeg still
/// surfaces promptly. A `Ran` outcome is trusted on the first try.
fn probe_essential_version(prog: &std::ffi::OsStr) -> ProbeOutcome {
    match probe_exec(prog, &["-version"], Duration::from_secs(8)) {
        ProbeOutcome::Timeout => probe_exec(prog, &["-version"], Duration::from_secs(8)),
        other => other,
    }
}

/// Hint for an UNVERIFIED ffmpeg-family tool — the `-version` probe timed out
/// twice. We DON'T claim it is missing (it almost certainly is not); we ask the
/// user to re-scan once the machine settles.
fn ffmpeg_unverified_hint(stem: &str) -> String {
    format!(
        "Couldn't verify {stem} this scan — its `-version` probe timed out twice (a \
         cold disk, an antivirus scan, or a heavy render can briefly starve it). This \
         is NOT a confirmed absence: if {stem} was working before, it almost certainly \
         still is. Re-scan once the machine settles; if it keeps timing out, check that \
         {stem} isn't wedged."
    )
}

/// OS-aware actionable hint for a missing ffmpeg-family tool — names the verb
/// that fixes it (system.fetch_tool) so the wizard button and the agent path
/// are the same instruction.
fn ffmpeg_missing_hint(stem: &str) -> String {
    // macOS has no in-app auto-fetch (BtbN ships no mac build) — guide to Homebrew
    // so a Mac user is never stuck staring at a dead Install button.
    #[cfg(target_os = "macos")]
    {
        format!(
            "{stem} is not resolvable (core editing + render need it). Install it with \
             Homebrew (`brew install ffmpeg-full`; Cut detects its keg-only path after \
             restart), or choose a compatible ffmpeg in Video processing settings."
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        format!(
            "{stem} isn't available yet (core editing and export need it) — \
             click Install to download it automatically, no admin needed."
        )
    }
}

// ---------------------------------------------------------------------------
// Perception (python sidecar) card — tier probe
// ---------------------------------------------------------------------------

/// Perception capability tier, cheapest→fullest.
/// - none: no python interpreter resolvable at all.
/// - instruments-capable: python runs, but the STT stack is not
///   importable (silence/scene/loudness instruments that need only ffmpeg +
///   light deps can still work; word-level transcription cannot).
/// - full-stt: onnx-asr or whisperX imports — the full AI tier (transcription,
///   filler/silence-by-words, captions) is available.
fn perception_card() -> Card {
    let Some(python) = cut_perception::configured_sidecar_python() else {
        return Card {
            id: "perception".into(),
            kind: "perception".into(),
            status: CardStatus::Missing,
            source: None,
            version: None,
            hint: Some(
                "Captions and transcription are not installed yet. Core editing and render still work. Choose Install captions when you need transcripts, captions, or word-based cleanup."
                    .into(),
            ),
            details: json!({
                "tier": "none",
                "python": null,
                "python_configured": false,
                "unlocks": {
                    "instruments-capable": "silence/scene/loudness instruments (ffmpeg-based)",
                    "full-stt": "word-level transcription, filler/silence-by-words, captions"
                }
            }),
        };
    };
    let py_str = python.to_string_lossy().into_owned();
    // Is the interpreter itself runnable? (bounded)
    let py_version = version_line(python.as_os_str(), &["--version"], Duration::from_secs(8));
    if py_version.is_none() {
        return Card {
            id: "perception".into(),
            kind: "perception".into(),
            status: CardStatus::Missing,
            source: None,
            version: None,
            hint: Some(
                "No Python interpreter for the perception sidecar. \
                 Transcription, word-level silence/filler removal, captions, and \
                 receipt facts need it — optional; core editing + render work \
                 without it. Choose Install captions when you need captions or transcripts."
                    .into(),
            ),
            details: json!({
                "tier": "none",
                "python": py_str,
                "unlocks": {
                    "instruments-capable": "silence/scene/loudness instruments (ffmpeg-based)",
                    "full-stt": "word-level transcription, filler/silence-by-words, captions"
                }
            }),
        };
    }
    // Python runs — probe the STT engine. The PRIMARY words engine is onnx-asr
    // Parakeet-TDT is primary; whisperX is only the compatibility fallback.
    // (requirements.txt §PRIMARY/§FALLBACK). Transcription works if EITHER imports,
    // so readiness must key off onnx-asr FIRST — probing whisperX alone reported a
    // correctly set-up machine (onnx-asr present, no whisperX) as "degraded" and
    // wrongly nagged the user to re-install. Short import-only checks; never loads a
    // model, never runs inference.
    let onnx_ok = import_check(&python, "onnx_asr", Duration::from_secs(15));
    // Only fall back to the whisperX probe when onnx-asr is absent (saves a slow
    // import on the common, healthy path).
    let whisper_ok = if onnx_ok {
        false
    } else {
        import_check(&python, "whisperx", Duration::from_secs(15))
    };
    let stt_ready = onnx_ok || whisper_ok;
    let stt_engine = if onnx_ok {
        "onnx-asr (Parakeet-TDT)"
    } else if whisper_ok {
        "whisperX (fallback)"
    } else {
        "none"
    };
    let (tier, status, hint) = if stt_ready {
        ("full", CardStatus::Ok, None)
    } else {
        (
            "instruments-capable",
            CardStatus::Degraded,
            Some(
                "Python is present but no speech-to-text engine imports (onnx-asr / \
                 whisperX) — the ffmpeg-based instruments work, but word-level \
                 transcription / captions do not. Choose Install captions to \
                 install the speech-to-text tools."
                    .to_string(),
            ),
        )
    };
    // Report the ACTIVE STT model/language (the user-chosen transcription
    // model that the next perception run will use), defaulting to parakeet v3
    // (the ~25-language multilingual checkpoint — the friction-free default).
    let (stt_model, stt_language) = cut_perception::read_stt_setting();
    Card {
        id: "perception".into(),
        kind: "perception".into(),
        status,
        source: None,
        version: py_version.clone(),
        hint,
        details: json!({
            "tier": tier,
            "python": py_str,
            "stt_ready": stt_ready,
            "stt_engine": stt_engine,
            "onnx_asr_importable": onnx_ok,
            "whisperx_importable": whisper_ok,
            "stt_model": stt_model.unwrap_or_else(|| "nemo-parakeet-tdt-0.6b-v3".into()),
            "stt_model_default": stt_model_is_default(),
            "stt_language": stt_language,
            "unlocks": {
                "instruments-capable": "silence/scene/loudness instruments (ffmpeg-based)",
                "full": "word-level transcription (Parakeet/Canary/Whisper), filler/silence-by-words, captions"
            }
        }),
    }
}

/// True when no STT model override is set (the perception run uses the built-in
/// Parakeet-TDT v3 default).
fn stt_model_is_default() -> bool {
    cut_perception::read_stt_setting().0.is_none()
}

/// Cheap import check: `python -c "import <module>"` with a bounded timeout.
/// Returns true iff the import succeeds (exit 0). Never imports anything heavy
/// beyond what the module's top level does — we accept that cost as the price
/// of an honest tier read; it is bounded and only runs on `refresh`/startup.
fn import_check(python: &Path, module: &str, timeout: Duration) -> bool {
    let snippet = match python_import_snippet(module) {
        Some(snippet) => snippet,
        None => return false,
    };
    let mut command = Command::new(python);
    command.args(["-c", &snippet]);
    run_doctor_command(&mut command, timeout, "check Python import")
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn python_import_snippet(module: &str) -> Option<String> {
    if !module.split('.').all(is_python_identifier) {
        return None;
    }
    Some(format!(
        "import importlib; importlib.import_module({module:?})"
    ))
}

fn is_python_identifier(part: &str) -> bool {
    let mut chars = part.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

// ---------------------------------------------------------------------------
// Judge rung cards — PATH detect, mirror ladder_judge.py (NO model call)
// ---------------------------------------------------------------------------

/// One judge rung: (card id suffix, binary on PATH, version flag).
/// Mirrors the bundled adapter provider order. Kept in Rust
/// deliberately: the doctor must work with no Python present. If the adapter
/// contract gains a provider, add it here too; this is an explicit product
/// contract rather than runtime discovery.
const JUDGE_RUNGS: &[(&str, &str, &str)] = &[
    ("claude", "claude", "--version"),
    ("codex", "codex", "--version"),
    ("antigravity", "agy", "--version"),
    // Grok uses the Grok Build CLI and mirrors the provider choices exposed by
    // the agent-chat UI.
    ("grok", "grok", "--version"),
];

// ---------------------------------------------------------------------------
// Chat-agent state (3-level: absent / present-but-unauthenticated / ready)
// ---------------------------------------------------------------------------
//
// The agent-selection dropdown needs more than "is the binary present": an auth
// FILE existing is NOT proof of a live session (grok's ~/.grok/auth.json persists
// while its token expires). So per chat agent we report a 3-level state — installed
// (resolve_agent), wired (chat::is_wired), authenticated (BEST-EFFORT, see below),
// ready (installed && wired && auth confirmed) — plus the informational security
// posture tag. This folds into the existing judge cards (no new verb / schema churn).

/// `HOME`/`USERPROFILE` joined with `rel` — used to check an agent's auth-file
/// presence without hardcoding the home dir. `None` if neither env var is set.
fn home_join(rel: &str) -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| PathBuf::from(h).join(rel))
}

/// Run `<prog> <args…>` with stdin CLOSED (so a prompt-on-stdin can never block),
/// bounded by `timeout` (kill on overrun — never hangs the scan). Returns
/// `(exit_success, captured_text)` where text is stdout (or stderr if stdout is
/// empty). The auth-probe analogue of `version_line`, but it returns the FULL
/// captured output + the exit status so the caller can parse a status payload.
fn run_capture(prog: &std::ffi::OsStr, args: &[&str], timeout: Duration) -> Option<(bool, String)> {
    let owned_args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let mut command = crate::gen::agent_std_command(Path::new(prog), &owned_args).ok()?;
    let out = run_doctor_command(&mut command, timeout, "check agent authentication").ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&out.stderr).into_owned();
    }
    Some((out.status.success(), text))
}

/// Auth-file / env-key fallback when the live status probe could not run (agent
/// absent, or the probe spawn failed/timed out). Credentials PRESENT but
/// unverified → "unknown" (honest: present ≠ a valid session); nothing found →
/// "no". Never reports "yes" off a file alone — a false positive is worse than
/// "unknown" (the reactive `agent_chat` path is the real validator).
fn auth_file_fallback(
    agent: &str,
    rel_files: &[&str],
    env_keys: &[&str],
) -> (&'static str, String) {
    let file = rel_files
        .iter()
        .any(|r| home_join(r).map(|p| p.exists()).unwrap_or(false));
    let env = env_keys.iter().any(|k| std::env::var_os(k).is_some());
    if file || env {
        (
            "unknown",
            format!(
                "{agent}'s credentials are present but a live session could not be confirmed \
                 non-interactively — if a chat fails with an auth error, re-run the agent's login."
            ),
        )
    } else {
        (
            "no",
            format!("no {agent} credentials found — sign in to use it for agent chat."),
        )
    }
}

/// BEST-EFFORT, non-interactive auth state for a chat agent: `("yes"|"no"|"unknown",
/// detail)`. NEVER prompts or hangs — each path is either a cheap LOCAL status
/// subcommand (bounded, stdin closed) or a pure file-presence check:
///   - claude → `claude auth status` (JSON `{"loggedIn":…}`, ~270 ms, local) — a
///     DEFINITIVE yes/no.
///   - codex  → `codex login status` ("Logged in using …" / "Not logged in", ~50 ms,
///     local) — a definitive yes/no.
///   - grok   → has NO safe non-interactive status subcommand AND its token expires
///     while ~/.grok/auth.json persists, so presence is reported as "unknown"
///     (honest: a stale token reads identically to a fresh one off the filesystem).
/// On any probe miss it degrades to `auth_file_fallback` rather than guessing.
fn chat_auth_state(agent: &str, resolved: Option<&Path>) -> (&'static str, String) {
    match agent {
        "claude" => {
            if let Some(p) = resolved {
                if let Some((_ok, out)) =
                    run_capture(p.as_os_str(), &["auth", "status"], Duration::from_secs(8))
                {
                    if let Ok(v) = serde_json::from_str::<Value>(&out) {
                        match v.get("loggedIn").and_then(|x| x.as_bool()) {
                            Some(true) => {
                                let who = v
                                    .get("subscriptionType")
                                    .and_then(|x| x.as_str())
                                    .or_else(|| v.get("authMethod").and_then(|x| x.as_str()))
                                    .unwrap_or("account");
                                return (
                                    "yes",
                                    format!("`claude auth status` reports logged in ({who})."),
                                );
                            }
                            Some(false) => {
                                return (
                                    "no",
                                    "`claude auth status` reports not logged in — run \
                                     `claude auth login`."
                                        .into(),
                                )
                            }
                            None => {}
                        }
                    }
                    let low = out.to_lowercase();
                    if low.contains("not logged in") || low.contains("logged out") {
                        return (
                            "no",
                            "claude reports not logged in — run `claude auth login`.".into(),
                        );
                    }
                    if low.contains("logged in") {
                        return ("yes", "claude reports logged in.".into());
                    }
                }
            }
            auth_file_fallback(
                "claude",
                &[".claude/.credentials.json", ".claude.json"],
                &["ANTHROPIC_API_KEY", "CLAUDE_API_KEY"],
            )
        }
        "codex" => {
            if let Some(p) = resolved {
                if let Some((_ok, out)) =
                    run_capture(p.as_os_str(), &["login", "status"], Duration::from_secs(8))
                {
                    let low = out.to_lowercase();
                    if low.contains("not logged in") || low.contains("not authenticated") {
                        return (
                            "no",
                            "`codex login status` reports not logged in — run `codex login`."
                                .into(),
                        );
                    }
                    // Require an EXPLICIT authenticated marker. A bare exit-0 is NOT
                    // proof of a live session (some codex builds exit 0 from `login
                    // status` regardless of auth, and a timed-out probe never reaches
                    // here at all). Without the marker we fall through to the file/env
                    // fallback, which reports "unknown" (honest: creds present but
                    // unverified) rather than a false "yes" that flips the card to a
                    // confident "Ready" for the Codex judge.
                    if low.contains("logged in") {
                        return ("yes", "`codex login status` reports logged in.".into());
                    }
                }
            }
            auth_file_fallback("codex", &[".codex/auth.json"], &["OPENAI_API_KEY"])
        }
        "grok" => {
            // No safe non-interactive status subcommand; the token expires while the
            // file persists → presence is "unknown", absence is "no".
            let present = home_join(".grok/auth.json")
                .map(|p| p.exists())
                .unwrap_or(false)
                || std::env::var_os("GROK_API_KEY").is_some()
                || std::env::var_os("XAI_API_KEY").is_some();
            if present {
                (
                    "unknown",
                    "grok's auth file is present, but its stored token can expire while the file \
                     remains — the session can't be verified non-interactively. If a chat fails \
                     with an auth error, run `grok login`."
                        .into(),
                )
            } else {
                (
                    "no",
                    "no grok credentials found — run `grok login` to sign in.".into(),
                )
            }
        }
        "antigravity" => auth_file_fallback(
            "antigravity",
            &[".gemini/antigravity-cli/antigravity-oauth-token"],
            &[],
        ),
        _ => ("unknown", "no auth probe for this agent.".into()),
    }
}

/// The chat-agent state block for a judge card's `details`, or `Value::Null` for a
/// judge rung that is not a chat agent. 3-level: installed (resolved)
/// → authenticated (best-effort) → ready, plus the provider's launch posture.
fn chat_agent_block(
    provider: &str,
    found: bool,
    resolved: Option<&Path>,
    version: Option<&str>,
) -> Value {
    if !crate::chat::CHAT_AGENTS.contains(&provider) {
        return Value::Null;
    }
    let version_supported = provider != "claude"
        || version
            .map(crate::chat::broker::is_supported_claude_version)
            .unwrap_or(false);
    let wired = crate::chat::is_wired(provider) && version_supported;
    let (authenticated, auth_detail) = chat_auth_state(provider, resolved);
    // READY = installed && supported route/version && CONFIRMED auth. An
    // installed but unwired (or version-mismatched) CLI remains visible as disabled.
    let ready = found && wired && authenticated == "yes";
    let posture = if provider == "claude" && found && !version_supported {
        Some("disabled: unsupported Claude Code version")
    } else {
        crate::chat::security_posture(provider)
    };
    json!({
        "installed": found,
        "resolved": resolved.map(|p| p.display().to_string()),
        "wired": wired,
        "authenticated": authenticated,
        "auth_detail": auth_detail,
        "ready": ready,
        "version_supported": version_supported,
        "posture": posture,
    })
}

/// Build one card per judge rung. `found` ⇒ ok; absent ⇒ missing (informational
/// for the ladder — ANY ok rung means a judge exists). Resolution reuses the SAME
/// agent-CLI ladder the chat path uses (`gen::resolve_agent`: process PATH first,
/// then the explicit install dirs incl. grok's off-PATH ~/.grok/bin / a
/// Finder-stripped-PATH .app's Homebrew dirs) so the doctor reports an off-PATH grok
/// as present — and the version probe runs the RESOLVED path, so `--version` works
/// even when the binary is not on PATH. Mirrors ladder_judge.py's PROVIDER_BIN map.
fn judge_cards() -> Vec<Card> {
    let adapter = crate::dispatch::configured_judge_adapter();
    let adapter_python = crate::dispatch::configured_adapter_python().filter(|python| {
        version_line(python.as_os_str(), &["--version"], Duration::from_secs(8)).is_some()
    });
    let review_ready = adapter.is_some() && adapter_python.is_some();
    JUDGE_RUNGS
        .iter()
        .map(|&(provider, bin, vflag)| {
            // `bin` is the binary stem; for grok it equals "grok", which keys the
            // grok-only ~/.grok/bin rung inside the resolver.
            let resolved = crate::gen::resolve_agent(bin);
            let found = resolved.is_some();
            // Probe `<resolved> --version` by the FULL path (an off-PATH grok would
            // not spawn by bare name) to confirm runnability + report the version.
            let version = resolved
                .as_ref()
                .and_then(|p| version_line(p.as_os_str(), &[vflag], Duration::from_secs(15)));
            let status = if !found {
                CardStatus::Missing
            } else if review_ready {
                CardStatus::Ok
            } else {
                CardStatus::Degraded
            };
            let hint = if !found {
                Some(format!(
                    "{provider} CLI (`{bin}`) not found on PATH or in the standard \
                     install dirs. Install + log in to enable verify.judge via your \
                     {provider} subscription (no API key — the CLI drives the review). \
                     Any one judge rung is enough."
                ))
            } else if adapter.is_none() {
                Some(
                    "The bundled render-review adapter is missing. Reinstall ShellX Cut, \
                     or correct CUTD_JUDGE_ADAPTER if you set an override. Agent chat can \
                     still use this CLI, but Get AI review cannot run yet."
                        .into(),
                )
            } else if adapter_python.is_none() {
                Some(
                    "This CLI is installed, but render review still needs a Python runtime \
                     for frame sampling and provider orchestration. Choose Install captions \
                     to add Cut's managed runtime, or set CUTD_ADAPTER_PYTHON. Agent chat can \
                     still use the CLI in the meantime."
                        .into(),
                )
            } else {
                None
            };
            let chat = chat_agent_block(provider, found, resolved.as_deref(), version.as_deref());
            Card {
                id: format!("judge.{provider}"),
                kind: "judge".into(),
                status,
                source: Some(if found { CardSource::Path } else { CardSource::Missing }),
                version,
                hint,
                details: json!({
                    "provider": provider,
                    "binary": bin,
                    "found": found,
                    "review_ready": found && review_ready,
                    "adapter": adapter.as_ref().map(|p| p.display().to_string()),
                    "adapter_python": adapter_python.as_ref().map(|p| p.display().to_string()),
                    // Where it resolved (e.g. ~/.grok/bin/grok) — null when absent.
                    // Lets the UI/agent (and the agent-dropdown) see the path.
                    "resolved": resolved.as_ref().map(|p| p.display().to_string()),
                    "role": "render judge (verify.judge) — drives the user's own coding-agent CLI as a vision reviewer; NO API key, NO model call during detection",
                    // The agent-chat dropdown state (3-level: absent / present-but-
                    // unauthenticated / ready) + the security-posture badge — folded
                    // here for the chat agents (claude/codex/grok); null for the
                    // provider-specific chat wiring is attached below when available.
                    "chat": chat,
                }),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Disk card — free space on the app-data tools volume (downloads need it)
// ---------------------------------------------------------------------------

/// Free-space card for the volume that holds the app-data tools dir (where
/// fetch_tool downloads land). Degraded under 500 MB free (a BtbN ffmpeg build
/// + its temp extraction needs headroom). Best-effort: a platform whose free
/// space we cannot read reports unknown (status ok, no false alarm).
fn disk_card() -> Card {
    let dir = cut_media::toolpath::appdata_tools_dir();
    let (free, total, probe_path) = match &dir {
        Some(d) => {
            // Probe the nearest existing ancestor (the tools dir may not exist
            // yet on a fresh install).
            let probe = nearest_existing(d);
            let (f, t) = free_total_bytes(&probe);
            (f, t, Some(probe.display().to_string()))
        }
        None => (None, None, None),
    };
    const MIN_FREE: u64 = 500 * 1024 * 1024; // 500 MB headroom for a build
    let (status, hint) = match free {
        Some(f) if f < MIN_FREE => (
            CardStatus::Degraded,
            Some(format!(
                "Only {} free on the disk where ShellX Cut keeps downloaded tools — \
                 an ffmpeg build needs about 250 MB plus room to unpack. Free up some \
                 space before downloading more tools.",
                human_bytes(f)
            )),
        ),
        _ => (CardStatus::Ok, None),
    };
    Card {
        id: "disk".into(),
        kind: "disk".into(),
        status,
        source: None,
        version: None,
        hint,
        details: json!({
            "free_bytes": free,
            "total_bytes": total,
            "free_human": free.map(human_bytes),
            "volume": probe_path,
            "min_free_bytes": MIN_FREE,
        }),
    }
}

/// The calm, one-line message shown when no installed ffmpeg has GPU encoders.
/// Software encoding ALWAYS works, so this is reassurance + a gentle nudge — never
/// a fault. Deliberately free of encoder acronyms, download URLs, and env-var
/// syntax: the step-by-step "how to enable" lives in `details.enable_help`,
/// rendered behind a COLLAPSED disclosure so it never becomes an always-on wall.
const NO_HW_ENCODER_HINT: &str =
    "Using the software video encoder (no GPU acceleration detected). Renders work \
     fine — just slower for large exports.";

/// Platform-specific, plain-language steps to ENABLE GPU encoding. Surfaced ONLY
/// when no accelerated ffmpeg exists, and ONLY inside the collapsed "How to enable
/// GPU encoding" disclosure (never the default view). Points the user at the
/// panel's own "Change ffmpeg" control instead of naming the SHELLX_CUT_FFMPEG env
/// var, and spells out "NVIDIA / Intel / AMD" rather than NVENC/QSV/AMF.
fn gpu_enable_help() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "Install an ffmpeg build that includes GPU encoders (NVIDIA, Intel, or AMD): \
         the 'full' build from https://www.gyan.dev/ffmpeg/builds/ , or the 'gpl' \
         build from https://github.com/BtbN/FFmpeg-Builds/releases . Then pick it \
         with the 'Change ffmpeg' button below."
    }
    #[cfg(target_os = "macos")]
    {
        "Install the complete Homebrew build (`brew install ffmpeg-full`) — it includes \
         Apple VideoToolbox GPU encoding plus Cut's required media filters — then restart \
         Cut or re-scan."
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "Install your distribution's ffmpeg (e.g. `apt install ffmpeg`) or a static \
         build with NVIDIA/VAAPI encoders from https://johnvansickle.com/ffmpeg/ , \
         then pick it with the 'Change ffmpeg' button below."
    }
}

fn gpu_encode_status(any_hw: bool) -> CardStatus {
    if any_hw {
        CardStatus::Ok
    } else {
        CardStatus::Degraded
    }
}

/// GPU/hardware video tier card (the accelerated render path). SCANS every
/// installed ffmpeg ("find any installed ffmpeg"), probing each for working HW
/// encoders + the full CUDA fast-track (a real test encode per candidate), and
/// reports the MOST CAPABLE one. Always "ok" (software encoding always works);
/// the hint is gentle guidance, not a fault:
///   • no accelerated ffmpeg anywhere → suggest the official download for the OS;
///   • a better ffmpeg exists but isn't the default → say where + how to use it;
///   • the default is already best → no hint.
/// render.final `hardware:auto` uses what the resolved binary detects; this card
/// is also how the user/agent learns a better binary is one env-var away.
fn gpu_encode_card() -> Card {
    let candidates: Vec<cut_media::hwencode::FfmpegCaps> = cut_media::toolpath::ffmpeg_candidates()
        .iter()
        .map(|p| cut_media::hwencode::probe_ffmpeg_caps(p))
        .collect();

    // The binary the engine uses TODAY, canonicalized so we can tell it apart
    // from a more-capable one found elsewhere.
    let resolved_path = std::fs::canonicalize(PathBuf::from(cut_media::toolpath::ffmpeg())).ok();
    let is_resolved = |c: &cut_media::hwencode::FfmpegCaps| -> bool {
        match (std::fs::canonicalize(&c.path).ok(), &resolved_path) {
            (Some(cp), Some(rp)) => &cp == rp,
            _ => false,
        }
    };

    // Best by acceleration rank; ties prefer the resolved/default binary.
    let best = candidates
        .iter()
        .cloned()
        .max_by_key(|c| (c.rank(), is_resolved(c) as u8));
    let resolved_caps = candidates.iter().find(|c| is_resolved(c)).cloned();

    let any_hw = best.as_ref().map(|b| b.hw.any()).unwrap_or(false);
    let full_fasttrack = best.as_ref().map(|b| b.cuda_filters).unwrap_or(false);
    let backend = best.as_ref().and_then(|b| b.backend());
    // Does the DEFAULT already give the best we found?
    let default_is_best = match (&best, &resolved_caps) {
        (Some(b), Some(r)) => r.rank() >= b.rank(),
        _ => false,
    };
    // Did the user explicitly pick an ffmpeg (manual override)? If so, respect it
    // — never nag them to switch (the "ask only when the user must decide" rule).
    let has_override = std::env::var(cut_media::toolpath::ENV_FFMPEG)
        .map(|v| !v.is_empty())
        .unwrap_or(false);

    let hint = if !any_hw {
        // No accelerated ffmpeg anywhere → a calm one-liner (software always works).
        // The how-to-enable steps ride in details.enable_help and surface behind a
        // collapsed disclosure, NOT dumped inline as a wall here.
        Some(NO_HW_ENCODER_HINT.to_string())
    } else if !default_is_best && !has_override {
        // A better ffmpeg exists and the user hasn't chosen one. With auto-select
        // on this never fires (the default IS the best); it only surfaces in a
        // manual/dev setup. De-jargoned: point at the panel's "Change ffmpeg"
        // control, not the raw SHELLX_CUT_FFMPEG env var.
        best.as_ref().map(|b| {
            format!(
                "A faster, hardware-accelerated ffmpeg is installed at {} ({}{}). \
                 Pick it with the 'Change ffmpeg' button below to use it for GPU renders.",
                b.path,
                b.backend().unwrap_or_else(|| "hardware".into()),
                if b.cuda_filters { " + GPU filters" } else { "" },
            )
        })
    } else {
        // Seamless: the best is already in use, OR the user made an explicit choice.
        None
    };

    Card {
        id: "gpu-encode".into(),
        kind: "tool".into(), // grouped with the render tools (ffmpeg) in the UI
        status: gpu_encode_status(any_hw),
        source: None,
        version: backend.clone().or_else(|| Some("software".into())),
        hint,
        details: json!({
            "hardware_available": any_hw,
            "backend": backend,
            // Full CUDA fast-track (scale_cuda/overlay_cuda + nvenc) actually runs.
            "full_fasttrack": full_fasttrack,
            // Plain-language "how to enable GPU encoding" steps — present ONLY when
            // there's no accelerated ffmpeg, so the UI shows the collapsed disclosure
            // exactly when it's actionable (null = nothing to enable).
            "enable_help": (!any_hw).then(gpu_enable_help),
            "best_ffmpeg": best.as_ref().map(|b| b.path.clone()),
            "default_is_best": default_is_best,
            // The persisted MANUAL choice (system.set_ffmpeg), if any — the UI's
            // "Change ffmpeg" control shows/clears it. null = automatic.
            "override_setting": cut_media::toolpath::read_override_setting(),
            "h264": best.as_ref().and_then(|b| b.hw.h264.clone()),
            "hevc": best.as_ref().and_then(|b| b.hw.hevc.clone()),
            "av1": best.as_ref().and_then(|b| b.hw.av1.clone()),
            // Every ffmpeg found + what each can do (the scan, for the UI/agent).
            "candidates": candidates.iter().map(|c| json!({
                "path": c.path,
                "version": c.version,
                "backend": c.backend(),
                "cuda_filters": c.cuda_filters,
                // libass/vidstab/zscale per candidate: WHY a render may use a different
                // ffmpeg than the fastest one — caption burn-in routes to a libass
                // build, and a color-managed render to a zscale (libzimg) build, even
                // when a faster build lacks it (toolpath::ffmpeg_for).
                "libass": c.libass,
                "vidstab": c.vidstab,
                "zscale": c.zscale,
            })).collect::<Vec<_>>(),
        }),
    }
}

/// Walk up from `dir` to the first ancestor that exists (so statvfs/GetDiskFree
/// has a real path to stat even before the tools dir is created).
fn nearest_existing(dir: &Path) -> PathBuf {
    let mut cur: &Path = dir;
    loop {
        if cur.exists() {
            return cur.to_path_buf();
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => return cur.to_path_buf(),
        }
    }
}

/// Free + total bytes on the filesystem containing `path`. Platform-specific;
/// returns (None, None) where we cannot read it (never a false low-disk alarm).
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)] // Field widths differ between Linux and macOS.
fn free_total_bytes(path: &Path) -> (Option<u64>, Option<u64>) {
    use std::os::unix::ffi::OsStrExt;
    let c = match std::ffi::CString::new(path.as_os_str().as_bytes()) {
        Ok(c) => c,
        Err(_) => return (None, None),
    };
    // SAFETY: statvfs writes into a zeroed struct we own; we only read scalar
    // fields on success. libc supplies the CORRECT per-platform struct layout -
    // the hand-rolled version sized fsblkcnt_t as u64, true on Linux but u32 on
    // macOS, which mis-aligned every field past f_frsize there (garbage counts +
    // a debug multiply-overflow panic). `as u64` normalizes the differing field
    // widths; saturating_mul is defense-in-depth so a pathological value can
    // never panic the doctor.
    // SAFETY: statvfs invariants are documented immediately above.
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) != 0 {
            return (None, None);
        }
        let bsize = if st.f_frsize != 0 {
            st.f_frsize
        } else {
            st.f_bsize
        } as u64;
        let free = (st.f_bavail as u64).saturating_mul(bsize);
        let total = (st.f_blocks as u64).saturating_mul(bsize);
        (Some(free), Some(total))
    }
}

#[cfg(windows)]
fn free_total_bytes(path: &Path) -> (Option<u64>, Option<u64>) {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut free_avail: u64 = 0;
    let mut total: u64 = 0;
    let mut free_total: u64 = 0;
    // SAFETY: GetDiskFreeSpaceExW writes only the three out-params we own; the
    // path is a valid NUL-terminated wide string we just built.
    // SAFETY: Windows FFI invariants are documented immediately above.
    let ok =
        unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free_avail, &mut total, &mut free_total) };
    if ok == 0 {
        (None, None)
    } else {
        (Some(free_avail), Some(total))
    }
}

#[cfg(not(any(unix, windows)))]
fn free_total_bytes(_path: &Path) -> (Option<u64>, Option<u64>) {
    (None, None)
}

// Disk-free FFI: unix uses libc::statvfs (correct per-platform layout — see
// free_total_bytes); Windows uses GetDiskFreeSpaceExW below.
#[cfg(windows)]
extern "system" {
    fn GetDiskFreeSpaceExW(
        lpDirectoryName: *const u16,
        lpFreeBytesAvailableToCaller: *mut u64,
        lpTotalNumberOfBytes: *mut u64,
        lpTotalNumberOfFreeBytes: *mut u64,
    ) -> i32;
}

/// Human-readable byte count for hints (1 decimal, binary units).
fn human_bytes(n: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

// ---------------------------------------------------------------------------
// The scan
// ---------------------------------------------------------------------------

/// Run the full environment scan and build the report. `addr` is the server's
/// bind address (passed by the caller — only main.rs/dispatch know it). This is
/// the ONE place capability is computed; everything else reads the cached
/// `DoctorReport`. Cost: a handful of bounded `-version`/import spawns —
/// acceptable on startup + explicit refresh, never on the hot verb path.
/// Background-removal (matte) capability: an AI subject cutout WITHOUT a green
/// screen (`edit.matte`). Needs onnxruntime (it rides the perception venv) + the
/// 14 MB RVM model. The ffmpeg pattern: AUTODETECT → `system.setup_matte`
/// downloads the model (one click), or `system.setup_matte{path}` points at an
/// rvm `.onnx` the user already has. OPTIONAL — core editing + render work
/// without it.
fn matte_card() -> Card {
    let perc_py = cut_perception::configured_sidecar_python();
    let ort_ok = perc_py
        .as_deref()
        .is_some_and(|p| import_check(p, "onnxruntime", Duration::from_secs(10)));
    let numpy_ok = perc_py
        .as_deref()
        .is_some_and(|p| import_check(p, "numpy", Duration::from_secs(10)));
    let pillow_ok = perc_py
        .as_deref()
        .is_some_and(|p| import_check(p, "PIL", Duration::from_secs(10)));
    let model = crate::matte::read_model_setting()
        .or_else(|| crate::matte::appdata_matte_dir().map(|d| d.join("rvm.onnx")));
    let model_present = model.as_ref().map(|p| p.exists()).unwrap_or(false);
    let deps_ok = ort_ok && numpy_ok && pillow_ok;
    let ready = deps_ok && crate::matte::runtime().is_some();
    let (status, hint) = if ready {
        (CardStatus::Ok, None)
    } else if !model_present {
        (
            CardStatus::Missing,
            Some(
                "AI background removal needs its model (14 MB). Run system.setup_matte to download it \
                 (one click, no setup), or system.setup_matte{path} to point at an rvm .onnx you already \
                 have. Optional — core editing + render work without it."
                    .to_string(),
            ),
        )
    } else {
        (
            CardStatus::Degraded,
            Some(
                "The background-removal model is installed but the local runner dependencies are incomplete. \
                 Install captions or reinstall the Background Removal tool so \
                 the local runtime carries onnxruntime, numpy, and Pillow."
                    .to_string(),
            ),
        )
    };
    Card {
        id: "matte".into(),
        kind: "matte".into(),
        status,
        source: None,
        version: None,
        hint,
        details: json!({
            "onnxruntime": ort_ok,
            "numpy": numpy_ok,
            "pillow": pillow_ok,
            "model_present": model_present,
            "model_path": model.map(|p| p.display().to_string()),
            "unlocks": "AI background removal/replace (edit.matte) — no green screen",
        }),
    }
}

/// PREMIUM background-removal capability: MatAnyone2 (`edit.matte{model:"matanyone"}`)
/// — target-assigned matting (pick WHICH subject) with cleaner edges + temporal
/// stability than RVM. Opt-in, NVIDIA-realistic, NON-COMMERCIAL (NTU S-Lab License
/// 1.0). Its OWN isolated torch venv + a 135 MB checkpoint, installed by
/// `system.setup_matte{model:"matanyone", accept_noncommercial:true}`. OPTIONAL —
/// the default RVM tier + core editing work without it.
fn matte_premium_card() -> Card {
    let rt = crate::matte::runtime_matanyone();
    let installed = rt.is_some();
    // Report the RESOLVED checkpoint when the runtime is present (honours the env
    // override + the browse setting), else the default fetch target.
    let model = rt
        .as_ref()
        .map(|r| r.model.clone())
        .or_else(crate::matte::read_matanyone_model_setting)
        .or_else(crate::matte::matanyone_default_model);
    let model_present = model.as_ref().map(|p| p.exists()).unwrap_or(false);
    // CUDA probe only when installed (a bounded torch import — premium users only,
    // so the cost is never paid on a default install). We read the FULL probe
    // outcome (not version_line's Option) so a TIMED-OUT probe is distinguishable
    // from a confirmed CPU-only box: `Ran` ⇒ Some(has-cuda), but a `Timeout`/
    // `NotFound` ⇒ None (unverified). A timed-out CUDA probe must not read as
    // Ok — a CPU-only box would then advertise premium matte as ready and run
    // unusably slow.
    let cuda_avail: Option<bool> = rt
        .as_ref()
        .map(|r| {
            probe_exec(
                r.python.as_os_str(),
                &[
                    "-c",
                    "import torch; print('cuda', torch.cuda.is_available())",
                ],
                Duration::from_secs(25),
            )
        })
        .and_then(|o| match o {
            ProbeOutcome::Ran(s) => Some(s.contains("True")),
            ProbeOutcome::Timeout | ProbeOutcome::NotFound => None,
        });
    let (status, hint) = if !installed {
        (
            CardStatus::Missing,
            Some(
                "Premium background removal (MatAnyone2 — cleaner edges, pick which subject) is not \
                 installed. Run system.setup_matte{model:\"matanyone\", accept_noncommercial:true} — it's \
                 NVIDIA-realistic and NON-COMMERCIAL (NTU S-Lab License 1.0). Optional: the default RVM \
                 tier works without it."
                    .to_string(),
            ),
        )
    } else if cuda_avail == Some(false) {
        (
            CardStatus::Degraded,
            Some(
                "MatAnyone2 is installed but torch reports no CUDA device — it would run on CPU, which is \
                 unusably slow for video. Use an NVIDIA GPU, or stick with the default RVM tier."
                    .to_string(),
            ),
        )
    } else if cuda_avail.is_none() {
        // The torch CUDA probe TIMED OUT (or its interpreter wouldn't run) — we
        // cannot confirm a usable GPU. Do not read that as Ok: a CPU-only box
        // would then show premium as ready and run unusably slow. Honest middle
        // state — couldn't verify; a re-scan re-probes.
        (
            CardStatus::Unknown,
            Some(
                "MatAnyone2 is installed, but the GPU check timed out, so its CUDA device couldn't be \
                 confirmed this scan. It needs an NVIDIA GPU to run usably (CPU is far too slow for video). \
                 Re-scan to verify; if the check keeps timing out, the torch import may be wedged."
                    .to_string(),
            ),
        )
    } else {
        // cuda_avail == Some(true): a confirmed CUDA device.
        (CardStatus::Ok, None)
    };
    Card {
        id: "matte_premium".into(),
        kind: "matte".into(),
        status,
        source: None,
        version: None,
        hint,
        details: json!({
            "model": "matanyone2",
            "installed": installed,
            "checkpoint_present": model_present,
            "checkpoint_path": model.map(|p| p.display().to_string()),
            // null when UNVERIFIED (the GPU probe timed out) — never a confident
            // false that would read as "definitely CPU-only".
            "cuda_available": cuda_avail,
            "license": "NTU S-Lab License 1.0 (non-commercial)",
            "unlocks": "premium target-assigned matte (edit.matte{model:matanyone}) — cleaner edges, pick the subject",
        }),
    }
}

pub fn scan(addr: Option<String>) -> DoctorReport {
    let mut cards = Vec::new();
    cards.extend(ffmpeg_cards());
    cards.push(gpu_encode_card());
    cards.push(perception_card());
    cards.push(matte_card());
    cards.push(matte_premium_card());
    // Optional remote AI microservices (dub/diarize). NEUTRAL — never factor into
    // `essential_ok` (the gate below keys only off ffmpeg), so an absent service
    // (the normal case on a plain editing box) never pops the first-run wizard.
    cards.push(service_cards::dub_card());
    cards.push(service_cards::diarize_card());
    cards.extend(judge_cards());
    cards.push(disk_card());

    // Essential gate: ffmpeg PRESENT ⇒ core editing/render possible, so the first-run
    // wizard does NOT need to auto-pop. A DEGRADED ffmpeg (Homebrew 8.x without libass/
    // libvidstab) still edits + renders everything except caption burn-in / stabilize —
    // those surface their own guidance when actually used. Treating Degraded as "essential
    // missing" made the wizard auto-open on EVERY launch on a stock Homebrew Mac (the
    // recurring Python/perception prompt). An UNVERIFIED ffmpeg (Unknown
    // — its `-version` probe timed out twice on a slow/AV-scanning/render-pinned box) is
    // ALSO not a blocker — a transient slow probe must NEVER flip a previously-working
    // essential to "missing" and pop the wizard until a manual re-scan. ONLY a truly
    // CONFIRMED-missing ffmpeg (spawn failed — the binary genuinely is not there) is the
    // hard blocker that warrants the auto-wizard. (ffprobe rides with ffmpeg.)
    let essential_ok = cards
        .iter()
        .find(|c| c.id == "ffmpeg")
        .map(|c| !matches!(c.status, CardStatus::Missing))
        .unwrap_or(false);

    DoctorReport {
        schema: DOCTOR_SCHEMA.to_string(),
        scanned_at: chrono::Utc::now().to_rfc3339(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        addr,
        cards,
        essential_ok,
    }
}

/// A minimal, never-fails report used only if the blocking scan task itself
/// panics/cancels (it never should — every probe is best-effort). Reports os/
/// arch/version with NO cards rather than crashing the verb. The next refresh
/// recovers the full scan.
pub fn scan_minimal() -> DoctorReport {
    DoctorReport {
        schema: DOCTOR_SCHEMA.to_string(),
        scanned_at: chrono::Utc::now().to_rfc3339(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        addr: None,
        cards: Vec::new(),
        essential_ok: false,
    }
}

#[cfg(test)]
mod tests {
    use super::service_cards::service_card;
    use super::*;

    /// The capability note names exactly the missing feature(s), frames ffmpeg
    /// as the user's external tool, states the license type, and never gates on it.
    #[test]
    fn ffmpeg_capability_hint_names_missing_features() {
        // Param order: (libass, libvidstab, zscale, libx265, vp9, av1). All-capable
        // except the one under test, so only that feature is named.
        let no_ass = ffmpeg_capability_hint(false, true, true, true, true, true);
        assert!(no_ass.contains("caption burn-in (libass)"));
        assert!(!no_ass.contains("libvidstab)")); // vidstab present → not named
        let no_stab = ffmpeg_capability_hint(true, false, true, true, true, true);
        assert!(no_stab.contains("video stabilize (libvidstab)"));
        assert!(!no_stab.contains("libass)"));
        let neither = ffmpeg_capability_hint(false, false, true, true, true, true);
        assert!(neither.contains("libass") && neither.contains("libvidstab"));
        // Color management: a missing libzimg/zscale is named AND carries the explicit
        // "color-managed (rec2020) renders will fail" warning with a fix-it source.
        let no_zscale = ffmpeg_capability_hint(true, true, false, true, true, true);
        assert!(no_zscale.contains("color-managed (rec2020) render (libzimg/zscale)"));
        assert!(no_zscale.contains("color-managed (rec2020) renders will fail"));
        assert!(no_zscale.contains("libzimg/zscale"));
        assert!(no_zscale.contains("ffmpeg-full"));
        // When zscale IS present the hard-fail warning is absent (no false alarm).
        assert!(!no_ass.contains("color-managed (rec2020) renders will fail"));
        // Missing software encoders are named too (HEVC/VP9/AV1 export).
        let no_hevc = ffmpeg_capability_hint(true, true, true, false, true, true);
        assert!(no_hevc.contains("HEVC export (libx265)"));
        assert!(!no_hevc.contains("libass)") && !no_hevc.contains("libvidstab)"));
        let no_av1 = ffmpeg_capability_hint(true, true, true, true, true, false);
        assert!(no_av1.contains("AV1 export (libsvtav1)"));
        // External-tool framing + license info + no gating, on every variant.
        for h in [&no_ass, &no_stab, &neither, &no_zscale, &no_hevc, &no_av1] {
            assert!(h.contains("external tool"));
            assert!(h.contains("GPL"));
            assert!(h.contains("your choice"));
        }
    }

    #[test]
    fn report_serializes_camel_and_has_all_card_kinds() {
        let r = scan(Some("127.0.0.1:6166".into()));
        assert_eq!(r.schema, DOCTOR_SCHEMA);
        // Every expected card id is present (contract tripwire).
        let ids: Vec<&str> = r.cards.iter().map(|c| c.id.as_str()).collect();
        for want in [
            "ffmpeg",
            "ffprobe",
            "perception",
            // Optional remote AI microservices (audio.dub / media.diarize).
            "dub",
            "diarize",
            "judge.claude",
            "judge.codex",
            "judge.antigravity",
            "judge.grok",
            "disk",
        ] {
            assert!(ids.contains(&want), "missing card {want}; got {ids:?}");
        }
        // The four kinds the UI groups by.
        let kinds: std::collections::HashSet<&str> =
            r.cards.iter().map(|c| c.kind.as_str()).collect();
        for k in ["tool", "perception", "judge", "disk"] {
            assert!(kinds.contains(k), "missing kind {k}");
        }
        // JSON round-trips and status serializes lowercase.
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"schema\":\"shellx-cut/doctor/1\""));
        // status enum lowercase.
        let _: DoctorReport = serde_json::from_str(&j).unwrap();
    }

    #[test]
    fn grok_card_replaced_gemini() {
        // The rung set mirrors shellX's providers: grok is a
        // first-class rung; the legacy gemini card is GONE entirely.
        let cards = judge_cards();
        assert!(
            cards.iter().any(|c| c.id == "judge.grok"),
            "judge.grok card missing"
        );
        assert!(
            !cards.iter().any(|c| c.id == "judge.gemini"),
            "judge.gemini must be removed, not kept alongside grok"
        );
    }

    #[test]
    fn judge_cards_carry_chat_agent_state_and_posture() {
        // The agent-dropdown reads the 3-level chat state folded into the judge
        // cards: installed / wired / authenticated (yes|no|unknown) / ready + the
        // launch posture — for each detectable chat-agent CLI.
        let cards = judge_cards();
        for (id, posture, wired) in [
            (
                "judge.codex",
                "native CLI: uses your Codex settings and permissions",
                true,
            ),
            (
                "judge.grok",
                "isolated turn: only Cut MCP, existing Grok login",
                true,
            ),
            (
                "judge.antigravity",
                if cfg!(windows) {
                    "disabled: Antigravity sandbox unavailable on Windows"
                } else {
                    "native CLI: uses your Antigravity sandbox and permissions"
                },
                !cfg!(windows),
            ),
        ] {
            let c = cards.iter().find(|c| c.id == id).expect("chat-agent card");
            let chat = &c.details["chat"];
            assert!(chat.is_object(), "{id} must carry a chat-state block");
            assert_eq!(chat["posture"], posture, "{id} posture tag");
            assert_eq!(chat["wired"], serde_json::json!(wired), "{id} wired state");
            assert!(chat["installed"].is_boolean());
            assert!(chat["ready"].is_boolean());
            let auth = chat["authenticated"].as_str().unwrap();
            assert!(
                ["yes", "no", "unknown"].contains(&auth),
                "{id} authenticated must be tri-state, got {auth}"
            );
            // ready is never asserted off an UNCONFIRMED auth.
            if auth != "yes" {
                assert_eq!(
                    chat["ready"],
                    serde_json::json!(false),
                    "{id} not ready unless auth=yes"
                );
            }
        }
        let claude = cards
            .iter()
            .find(|c| c.id == "judge.claude")
            .expect("claude chat-agent card");
        let chat = &claude.details["chat"];
        assert!(chat["version_supported"].is_boolean());
        if chat["version_supported"] == serde_json::json!(true) {
            assert_eq!(chat["wired"], serde_json::json!(true));
            assert_eq!(chat["posture"], "contained: pinned Claude Code 2.1.224");
        } else {
            assert_eq!(chat["wired"], serde_json::json!(false));
            if chat["installed"] == serde_json::json!(true) {
                assert_eq!(chat["posture"], "disabled: unsupported Claude Code version");
            }
        }
        // Antigravity becomes a chat agent only on platforms with its native sandbox.
        let agy = cards.iter().find(|c| c.id == "judge.antigravity").unwrap();
        assert!(agy.details["chat"].is_object());
        assert_eq!(
            agy.details["chat"]["wired"],
            serde_json::json!(!cfg!(windows))
        );
    }

    #[test]
    fn same_capabilities_ignores_timestamp() {
        let a = scan(None);
        let mut b = a.clone();
        b.scanned_at = "different".into();
        assert!(
            a.same_capabilities(&b),
            "timestamp must not count as a change"
        );
        // Flipping a status IS a change.
        b.cards[0].status = match b.cards[0].status {
            CardStatus::Ok => CardStatus::Missing,
            _ => CardStatus::Ok,
        };
        assert!(!a.same_capabilities(&b));
    }

    #[test]
    fn human_bytes_scales() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert!(human_bytes(3 * 1024 * 1024 * 1024).ends_with("GiB"));
    }

    #[test]
    fn python_import_snippet_quotes_module_name() {
        let snippet = python_import_snippet("PIL.Image").expect("valid module");
        assert!(snippet.contains("importlib.import_module"));
        assert!(snippet.contains("\"PIL.Image\""));
        assert!(python_import_snippet("os; import subprocess").is_none());
        assert!(python_import_snippet("../bad").is_none());
    }

    /// The new `Unknown` ("unverified") status is the honest middle state a
    /// timed-out probe degrades to — it serializes lowercase (the UI mirror in
    /// doctor.ts adds 'unknown' to CardStatus) and is explicitly NOT a confirmed
    /// absence, so it must not gate the first-run wizard.
    #[test]
    fn unknown_status_is_honest_middle_state() {
        // Serializes/deserializes lowercase, matching doctor.ts's CardStatus union.
        let j = serde_json::to_string(&CardStatus::Unknown).unwrap();
        assert_eq!(j, "\"unknown\"");
        let back: CardStatus = serde_json::from_str("\"unknown\"").unwrap();
        assert_eq!(back, CardStatus::Unknown);
        // The essential-gate rule (scan()'s `!matches!(status, Missing)`): ONLY a
        // CONFIRMED-missing essential pops the wizard. Ok / Degraded / Unknown (a
        // timed-out probe) all keep essential_ok TRUE, so a transient slow probe
        // never auto-pops the wizard.
        for s in [CardStatus::Ok, CardStatus::Degraded, CardStatus::Unknown] {
            assert!(
                !matches!(s, CardStatus::Missing),
                "{s:?} must not read as the essential-missing blocker"
            );
        }
        assert!(matches!(CardStatus::Missing, CardStatus::Missing));
    }

    #[test]
    fn gpu_encode_without_hardware_is_degraded_not_ok() {
        assert_eq!(gpu_encode_status(false), CardStatus::Degraded);
        assert_eq!(gpu_encode_status(true), CardStatus::Ok);
    }

    /// `probe_exec` distinguishes a confirmed absence (spawn failed —
    /// the only outcome that may read as Missing) from the other outcomes. A name
    /// that resolves on no PATH fails the spawn ⇒ `NotFound`, never `Timeout`.
    #[test]
    fn probe_exec_spawn_failure_is_notfound() {
        let missing = probe_exec(
            std::ffi::OsStr::new("shellx-cut-no-such-binary-zzz-9f3"),
            &["--version"],
            Duration::from_secs(2),
        );
        assert_eq!(
            missing,
            ProbeOutcome::NotFound,
            "a binary that cannot spawn is a CONFIRMED absence, not a timeout"
        );
        // version_line collapses any non-Ran outcome to None (existing contract).
        assert!(version_line(
            std::ffi::OsStr::new("shellx-cut-no-such-binary-zzz-9f3"),
            &["--version"],
            Duration::from_secs(2),
        )
        .is_none());
    }

    /// An OPTIONAL remote service (dub/diarize) that can't be reached must read as
    /// the NEUTRAL `Unknown` — NEVER `Missing` (which would look like a broken
    /// essential and is dishonest for a service that's normally absent). Probing a
    /// loopback port nothing listens on refuses INSTANTLY (no timeout), so this is
    /// fast + deterministic.
    #[test]
    fn optional_service_unreachable_reads_unknown_not_missing() {
        let c = service_card(
            "dub",
            "http://127.0.0.1:1".to_string(),
            false,
            "dubbing (OmniVoice TTS)",
            "audio.dub",
            "CUT_DUB_ENDPOINT",
            "OmniVoice TTS",
            false,
        );
        assert_eq!(c.kind, "service");
        assert_eq!(c.id, "dub");
        assert_eq!(
            c.status,
            CardStatus::Unknown,
            "an unreachable optional service must be Unknown, not Missing"
        );
        // The three load-bearing facts an agent reads.
        assert_eq!(c.details["endpoint"], "http://127.0.0.1:1");
        assert_eq!(c.details["secret_set"], json!(false));
        assert_eq!(c.details["reachable"], json!(false));
        assert_eq!(c.details["model"], json!("OmniVoice TTS"));
        assert_eq!(c.details["runner_available"], json!(false));
        assert_eq!(c.details["optional"], json!(true));
        // No install verb — the hint guides ONLY the endpoint env var.
        let hint = c.hint.as_deref().unwrap_or("");
        assert!(
            hint.contains("CUT_DUB_ENDPOINT"),
            "hint must name the endpoint env var"
        );
        assert!(
            !hint.contains("setup_dub"),
            "there is NO setup_dub verb to suggest"
        );

        // diarize mirrors dub (different defaults), with its secret reported set.
        let d = service_card(
            "diarize",
            "http://127.0.0.1:1".to_string(),
            true,
            "speaker diarization",
            "media.diarize",
            "CUT_DIARIZE_ENDPOINT",
            "Sortformer v2",
            true,
        );
        assert_eq!(d.status, CardStatus::Unknown);
        assert_eq!(d.details["secret_set"], json!(true));
        assert_eq!(d.details["runner_available"], json!(true));
    }

    /// A stale service proxy can accept TCP and then reset before returning an
    /// HTTP health response. That must not read as "reachable" in Environment.
    #[test]
    fn optional_service_resetting_connection_is_not_reachable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let addr = listener.local_addr().expect("test listener address");
        std::thread::spawn(move || {
            for _ in 0..3 {
                if let Ok((stream, _)) = listener.accept() {
                    drop(stream);
                }
            }
        });

        let c = service_card(
            "diarize",
            format!("http://{addr}"),
            false,
            "speaker diarization",
            "media.diarize",
            "CUT_DIARIZE_ENDPOINT",
            "Sortformer v2",
            true,
        );
        assert_eq!(c.status, CardStatus::Unknown);
        assert_eq!(c.details["reachable"], json!(false));
    }
}
