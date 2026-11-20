//! selection.rs — which FFmpeg the engine uses, and why.
//!
//! The automatic pick (the best hardware-capable installed FFmpeg) is the
//! engine's own default, so an engine started by the desktop shell and one
//! started by hand (`cutd serve`, how agents run it) choose the same build.
//! Order of precedence, unchanged:
//!   1. `SHELLX_CUT_FFMPEG` (an exact pin: tests, the Runner, power users);
//!   2. the person's choice (Choose ffmpeg / `system.set_ffmpeg`);
//!   3. the automatic pick, when it is on and a hardware-capable build exists;
//!   4. the first FFmpeg in the fixed search order.
//!
//! The automatic pick is OFF for a source build (an engine or test binary in a
//! cargo target folder) so `cargo test` and a developer's `cutd` stay
//! deterministic. `SHELLX_CUT_FFMPEG_AUTO=1` turns it on and `=0` turns it off
//! for any engine.

use super::{ENV_FFMPEG, ENV_FFMPEG_AUTO};
use serde::Serialize;
use std::ffi::OsString;
use std::path::Path;

/// Whether the automatic pick is on for this engine, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomaticPick {
    /// The default for an installed or copied engine.
    On,
    /// `SHELLX_CUT_FFMPEG_AUTO` is set to a true value.
    OnByEnvironment,
    /// `SHELLX_CUT_FFMPEG_AUTO` is set to a false value.
    OffByEnvironment,
    /// This engine is a source build in a cargo target folder.
    OffSourceBuild,
}

impl AutomaticPick {
    pub fn is_on(self) -> bool {
        matches!(self, Self::On | Self::OnByEnvironment)
    }
}

/// The automatic-pick mode for an explicit `SHELLX_CUT_FFMPEG_AUTO` value and
/// "is this a source build".
pub fn automatic_pick_for(env: Option<&str>, source_build: bool) -> AutomaticPick {
    match env.map(str::trim) {
        Some(v)
            if ["1", "true", "yes", "on"]
                .iter()
                .any(|t| v.eq_ignore_ascii_case(t)) =>
        {
            AutomaticPick::OnByEnvironment
        }
        Some(v)
            if ["0", "false", "no", "off"]
                .iter()
                .any(|t| v.eq_ignore_ascii_case(t)) =>
        {
            AutomaticPick::OffByEnvironment
        }
        _ if source_build => AutomaticPick::OffSourceBuild,
        _ => AutomaticPick::On,
    }
}

/// The automatic-pick mode of this engine.
pub fn automatic_pick() -> AutomaticPick {
    static SOURCE_BUILD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let source_build = *SOURCE_BUILD.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .is_some_and(|exe| is_cargo_build_exe(&exe))
    });
    automatic_pick_for(std::env::var(ENV_FFMPEG_AUTO).ok().as_deref(), source_build)
}

/// First line cargo writes into the `CACHEDIR.TAG` of every target folder.
const CARGO_CACHEDIR_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";

/// Is `exe` a binary of a cargo build: `<target>/[<triple>/]{debug,release}/cutd`,
/// or a test binary in `.../{debug,release}/deps/`? Both facts must hold: the
/// folder layout AND cargo's own `CACHEDIR.TAG` at the target root. An installed
/// engine (`/usr/bin`, `<install>\`, `Contents/MacOS`) is never one. Same rule as
/// the perception sidecar's installed-layout check, plus test binaries.
pub fn is_cargo_build_exe(exe: &Path) -> bool {
    let named = |dir: &Path, names: &[&str]| {
        dir.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| names.contains(&name))
    };
    let Some(mut profile) = exe.parent() else {
        return false;
    };
    if named(profile, &["deps", "examples"]) {
        let Some(up) = profile.parent() else {
            return false;
        };
        profile = up;
    }
    if !named(profile, &["debug", "release"]) {
        return false;
    }
    let Some(above) = profile.parent() else {
        return false;
    };
    // `--target <triple>` adds one folder between the target root and the profile.
    [Some(above), above.parent()]
        .into_iter()
        .flatten()
        .any(|root| {
            std::fs::read(root.join("CACHEDIR.TAG"))
                .map(|bytes| bytes.starts_with(CARGO_CACHEDIR_SIGNATURE.as_bytes()))
                .unwrap_or(false)
        })
}

/// Why the FFmpeg in use is the one in use (stable codes agents can read).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionReason {
    /// `SHELLX_CUT_FFMPEG` names it.
    Environment,
    /// The person chose it (Choose ffmpeg / `system.set_ffmpeg`).
    Chosen,
    /// The automatic pick: a hardware-capable build.
    AutomaticHardware,
    /// The automatic pick found several builds, none hardware-capable, so the
    /// first in the search order is used.
    AutomaticNoHardware,
    /// The only FFmpeg found.
    OnlyOneFound,
    /// The automatic pick is off; the first in the search order is used.
    AutomaticOff,
    /// No FFmpeg was found.
    NotFound,
}

/// Which FFmpeg is in use and why, for the doctor.
#[derive(Debug, Clone, Serialize)]
pub struct FfmpegSelection {
    /// The program in use (same value as [`super::ffmpeg`]).
    #[serde(skip)]
    pub program: OsString,
    pub reason: SelectionReason,
    pub automatic: AutomaticPick,
    /// How many distinct FFmpeg builds were found.
    pub found: usize,
    /// One plain sentence for a person or an agent.
    pub text: String,
}

/// How [`super::ffmpeg`] reached its answer.
pub(super) enum Step {
    Environment,
    Chosen,
    AutomaticHardware,
    SearchOrder,
}

/// The FFmpeg in use and why. Runs the same choice as [`super::ffmpeg`] and
/// adds the count of builds found (a folder walk, no process starts beyond the
/// automatic pick's own cached probes).
pub fn ffmpeg_selection() -> FfmpegSelection {
    let (program, step) = super::choose_ffmpeg();
    let automatic = automatic_pick();
    let found = super::ffmpeg_candidates().len();
    let present = matches!(
        step,
        Step::Environment | Step::Chosen | Step::AutomaticHardware
    ) || super::locate_media_tool("ffmpeg").is_some();
    let reason = match step {
        Step::Environment => SelectionReason::Environment,
        Step::Chosen => SelectionReason::Chosen,
        Step::AutomaticHardware => SelectionReason::AutomaticHardware,
        Step::SearchOrder if !present => SelectionReason::NotFound,
        Step::SearchOrder if !automatic.is_on() => SelectionReason::AutomaticOff,
        Step::SearchOrder if found <= 1 => SelectionReason::OnlyOneFound,
        Step::SearchOrder => SelectionReason::AutomaticNoHardware,
    };
    FfmpegSelection {
        text: describe(reason, automatic, found),
        program,
        reason,
        automatic,
        found,
    }
}

/// The plain sentence for a reason.
pub fn describe(reason: SelectionReason, automatic: AutomaticPick, found: usize) -> String {
    match reason {
        SelectionReason::Environment => format!(
            "In use because {ENV_FFMPEG} names it; that setting always wins over a choice and \
             the automatic pick."
        ),
        SelectionReason::Chosen => "In use because it was chosen (Choose ffmpeg or \
             system.set_ffmpeg); a choice wins over the automatic pick."
            .into(),
        SelectionReason::AutomaticHardware if found <= 1 => "Picked automatically: the only \
             FFmpeg found, and it can encode with the graphics hardware."
            .into(),
        SelectionReason::AutomaticHardware => format!(
            "Picked automatically: of the {found} FFmpeg builds found, this one can encode with \
             the graphics hardware."
        ),
        SelectionReason::AutomaticNoHardware => format!(
            "Picked automatically: none of the {found} FFmpeg builds found can encode with the \
             graphics hardware, so the first in the search order is used."
        ),
        SelectionReason::OnlyOneFound => {
            "In use because it is the only FFmpeg found (it cannot encode with the graphics \
             hardware)."
                .into()
        }
        SelectionReason::AutomaticOff => {
            let why = if automatic == AutomaticPick::OffByEnvironment {
                format!("{ENV_FFMPEG_AUTO} turns it off")
            } else {
                format!("this engine is a source build (set {ENV_FFMPEG_AUTO}=1 to turn it on)")
            };
            format!(
                "The first FFmpeg in the search order; the automatic pick is off because {why}."
            )
        }
        SelectionReason::NotFound => "No FFmpeg was found.".into(),
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
