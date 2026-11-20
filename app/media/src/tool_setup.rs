//! tool_setup.rs — what to tell a person or an agent when ffmpeg is missing.
//!
//! ffmpeg and ffprobe are external programs. When they are absent, every
//! surface (a refused verb, the doctor card, the first-run setup, the
//! `system.fetch_tool` answer) must say the same thing: what is missing, why
//! it matters, and the one way forward this platform supports. This module is
//! that single source, so the engine error and the UI steps cannot drift.
//!
//! Dependencies: cut-core (CutError), toolpath (where Cut looks).
//! Callers: ffmpeg process spawning (catch-all), the server's media-tool gate,
//! the doctor's ffmpeg cards, and `system.fetch_tool`.

use cut_core::{error_codes, CutError};
use serde::Serialize;

mod download;
pub use download::{
    download_offer_for, pinned_release, ArchiveKind, DownloadOffer, PinnedAsset, PinnedMember,
    PinnedRelease, ProgramCheck, LINUX_X64_FFMPEG, MACOS_ARM64_FFMPEG, WINDOWS_X64_FFMPEG,
};

/// True where `system.fetch_tool {tool:"ffmpeg"}` has a built-in, verified
/// download. Must match the server's fetch registry (a server test pins it).
pub fn builtin_ffmpeg_download_available() -> bool {
    builtin_ffmpeg_download_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// [`builtin_ffmpeg_download_available`] for an explicit platform.
pub fn builtin_ffmpeg_download_for(os: &str, arch: &str) -> bool {
    download_offer_for(os, arch).is_some()
}

/// Which way forward the platform offers for a missing ffmpeg.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupRoute {
    /// Cut downloads a verified build after the person agrees (`system.fetch_tool`).
    BuiltinDownload,
    /// macOS without a built-in download: Homebrew's `ffmpeg-full`, or a chosen build.
    Homebrew,
    /// Linux and other systems without a download: the distribution's
    /// package, or a chosen build.
    PackageManager,
    /// Windows without a download (an arm64 build of Cut): the publisher's
    /// page, then Choose ffmpeg.
    ChooseBuild,
}

/// Where a person gets an FFmpeg-Builds build by hand.
pub const FFMPEG_BUILDS_RELEASES: &str = "https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases";

/// What the common Linux packages contain, checked against their build
/// recipes (Debian `debian/rules`, Fedora and RPM Fusion `ffmpeg.spec`).
pub const LINUX_PACKAGE_FACTS: &str = "Debian and Ubuntu: `sudo apt install ffmpeg` (includes \
     libass, vid.stab and zimg). Fedora: `sudo dnf install ffmpeg-free` (includes those three, but \
     not the H.264 and HEVC software encoders), or RPM Fusion's full build with `sudo dnf swap \
     ffmpeg-free ffmpeg --allowerasing`.";

/// What a person can do after a refused built-in download, on this machine.
/// `retry_may_help` is false when the pinned file is gone for good: then the
/// action must not suggest trying again.
pub fn download_fallback_action(retry_may_help: bool) -> String {
    download_fallback_action_for(std::env::consts::OS, retry_may_help)
}

/// [`download_fallback_action`] for an explicit system.
pub fn download_fallback_action_for(os: &str, retry_may_help: bool) -> String {
    let start = if retry_may_help {
        "Nothing was installed. Try again later, or"
    } else {
        "Nothing was installed, and trying again will not help (this version of ShellX Cut \
         installs only that build). Instead"
    };
    let rest = match os {
        "macos" => "install the complete build with Homebrew (Settings > Video processing shows \
             the steps), or choose an ffmpeg you already have (system.set_ffmpeg)."
            .to_string(),
        "windows" => format!(
            "get a GPL build from the publisher's page {FFMPEG_BUILDS_RELEASES}, unzip it and \
             choose its bin\\ffmpeg.exe with Choose ffmpeg (system.set_ffmpeg; ffprobe.exe must \
             be beside it)."
        ),
        _ => format!(
            "install your distribution's FFmpeg and click Check again. {LINUX_PACKAGE_FACTS} Or \
             choose an ffmpeg you already have (system.set_ffmpeg)."
        ),
    };
    format!("{start} {rest}")
}

/// One command the person may copy into a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetupCommand {
    pub label: String,
    pub command: String,
}

/// The complete setup guidance for ffmpeg + ffprobe on this machine. Carried in
/// the doctor's ffmpeg card (`details.setup`) and rendered by the first-run
/// setup; the same facts form the agent's `suggested_action`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FfmpegSetupGuide {
    pub route: SetupRoute,
    /// `system.fetch_tool {tool:"ffmpeg"}` works here.
    pub fetch_available: bool,
    /// Numbered steps for a person who has never used a terminal.
    pub steps: Vec<String>,
    /// Commands to copy, in step order (empty for the built-in download).
    pub commands: Vec<SetupCommand>,
    /// Where the person can read more before running anything.
    pub help_url: Option<String>,
    /// What a self-chosen ffmpeg must contain (the "choose your own" route).
    pub custom_build: String,
    /// The built-in download and its consent text, where one exists. `steps`
    /// and `commands` then describe the other way (e.g. Homebrew on a Mac).
    pub download: Option<DownloadOffer>,
}

/// The requirement for a self-chosen build, shared by every route.
pub const CUSTOM_BUILD_NEEDS: &str = "Choose an ffmpeg that has ffprobe in the same folder and \
     includes libass (captions), vid.stab (stabilize), zimg (color management), libx264 and \
     libx265 (H.264 and HEVC export).";

/// The official Homebrew installer command, as published on https://brew.sh.
pub const HOMEBREW_INSTALL_COMMAND: &str =
    "/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"";

/// `brew install ffmpeg-full` by full path: a new Homebrew is not on the
/// terminal's PATH until its "Next steps" are run, but the full path works at once.
fn homebrew_ffmpeg_command(arch: &str) -> String {
    let brew = if arch == "aarch64" {
        "/opt/homebrew/bin/brew"
    } else {
        "/usr/local/bin/brew"
    };
    format!("{brew} install ffmpeg-full")
}

/// Guidance for this machine.
pub fn ffmpeg_setup_guide() -> FfmpegSetupGuide {
    ffmpeg_setup_guide_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// Guidance for an explicit platform (tests cover every branch on any host).
pub fn ffmpeg_setup_guide_for(os: &str, arch: &str) -> FfmpegSetupGuide {
    let download = download_offer_for(os, arch);
    if os == "macos" {
        // Apple silicon downloads the pinned build first; Homebrew's complete
        // build stays the other way (and the way to get Stabilize).
        let fetch_available = download.is_some();
        return FfmpegSetupGuide {
            route: if fetch_available {
                SetupRoute::BuiltinDownload
            } else {
                SetupRoute::Homebrew
            },
            fetch_available,
            steps: vec![
                "Open Terminal (Applications > Utilities > Terminal).".into(),
                "Install Homebrew: copy the first command, paste it into Terminal and press Return. It asks for your Mac login password and can take several minutes.".into(),
                "Install video processing: copy the second command, paste it and press Return.".into(),
                "Come back to ShellX Cut and click Check again.".into(),
            ],
            commands: vec![
                SetupCommand {
                    label: "Install Homebrew".into(),
                    command: HOMEBREW_INSTALL_COMMAND.into(),
                },
                SetupCommand {
                    label: "Install FFmpeg".into(),
                    command: homebrew_ffmpeg_command(arch),
                },
            ],
            help_url: Some("https://brew.sh".into()),
            custom_build: CUSTOM_BUILD_NEEDS.into(),
            download,
        };
    }
    if download.is_some() {
        return FfmpegSetupGuide {
            route: SetupRoute::BuiltinDownload,
            fetch_available: true,
            steps: Vec::new(),
            commands: Vec::new(),
            help_url: Some("https://github.com/FFmpeg-Builds/FFmpeg-Builds".into()),
            custom_build: CUSTOM_BUILD_NEEDS.into(),
            download,
        };
    }
    if os == "windows" {
        return FfmpegSetupGuide {
            route: SetupRoute::ChooseBuild,
            fetch_available: false,
            steps: vec![
                format!(
                    "Download a GPL FFmpeg build for your processor from the publisher's page \
                     {FFMPEG_BUILDS_RELEASES} (a \"gpl\" zip; winarm64 for ARM)."
                ),
                "Unzip it.".into(),
                "In ShellX Cut choose Choose ffmpeg… and pick bin\\ffmpeg.exe (ffprobe.exe is \
                 beside it)."
                    .into(),
            ],
            commands: Vec::new(),
            help_url: Some(FFMPEG_BUILDS_RELEASES.into()),
            custom_build: CUSTOM_BUILD_NEEDS.into(),
            download: None,
        };
    }
    FfmpegSetupGuide {
        route: SetupRoute::PackageManager,
        fetch_available: false,
        steps: vec![
            "Install ffmpeg with your distribution's package manager (it provides ffprobe too)."
                .into(),
            LINUX_PACKAGE_FACTS.into(),
            "Come back to ShellX Cut and click Check again.".into(),
        ],
        commands: vec![
            SetupCommand {
                label: "Debian or Ubuntu".into(),
                command: "sudo apt install ffmpeg".into(),
            },
            SetupCommand {
                label: "Fedora".into(),
                command: "sudo dnf install ffmpeg-free".into(),
            },
        ],
        help_url: Some("https://ffmpeg.org/download.html".into()),
        custom_build: CUSTOM_BUILD_NEEDS.into(),
        download: None,
    }
}

/// The next step an agent can relay or act on, for this machine.
pub fn ffmpeg_setup_action() -> String {
    ffmpeg_setup_action_for(&ffmpeg_setup_guide())
}

/// [`ffmpeg_setup_action`] for an explicit guide.
pub fn ffmpeg_setup_action_for(guide: &FfmpegSetupGuide) -> String {
    let choose =
        "or call system.set_ffmpeg {\"path\":\"<ffmpeg>\"} with a build that has ffprobe beside it";
    match guide.route {
        SetupRoute::BuiltinDownload => {
            let consent = guide
                .download
                .as_ref()
                .map(|d| d.consent.as_str())
                .unwrap_or("Download FFmpeg, a separate GPL program.");
            let other = if guide.commands.is_empty() {
                String::new()
            } else {
                " (Settings > Video processing also shows the Homebrew steps)".to_string()
            };
            format!(
                "Show the person this and ask for their agreement: \"{consent}\" When they \
                 agree, call system.fetch_tool {{\"tool\":\"ffmpeg\",\"accept_license\":true}} \
                 and wait for its job{other}; {choose}. Then retry."
            )
        }
        SetupRoute::Homebrew => {
            let install = guide
                .commands
                .iter()
                .find(|c| c.label == "Install FFmpeg")
                .map(|c| c.command.as_str())
                .unwrap_or("brew install ffmpeg-full");
            format!(
                "Cut has no built-in FFmpeg download on macOS. Ask the person to install it: \
                 Settings > Video processing shows the steps (install Homebrew from \
                 https://brew.sh, then run `{install}` in Terminal), {choose}. Then call \
                 system.doctor {{\"refresh\":true}} and retry."
            )
        }
        SetupRoute::PackageManager => format!(
            "Ask the person to install ffmpeg with the distribution's package manager \
             ({LINUX_PACKAGE_FACTS}), {choose}. Then call system.doctor \
             {{\"refresh\":true}} and retry."
        ),
        SetupRoute::ChooseBuild => format!(
            "Cut has no built-in FFmpeg download for this Windows build. Ask the person to get a \
             GPL build from {FFMPEG_BUILDS_RELEASES} and unzip it, then call system.set_ffmpeg \
             {{\"path\":\"<folder>\\\\bin\\\\ffmpeg.exe\"}} (ffprobe.exe beside it). Then call \
             system.doctor {{\"refresh\":true}} and retry."
        ),
    }
}

/// The refusal for a step that needs `stem` ("ffmpeg" or "ffprobe") when it is
/// not installed. Code `tool_missing`; the step changed nothing.
pub fn missing_tool_error(stem: &str) -> CutError {
    let looked = if cfg!(target_os = "macos") {
        "the chosen tool, the app's tools folder, Homebrew and PATH"
    } else {
        "the chosen tool, the app's tools folder and PATH"
    };
    CutError::new(
        error_codes::TOOL_MISSING,
        format!("{stem} is not installed"),
        format!(
            "ShellX Cut needs ffmpeg and ffprobe to import, preview and export, and found no \
             {stem} (looked in {looked})"
        ),
    )
    .with_suggested_action(ffmpeg_setup_action())
    .with_requirement(if stem == "ffprobe" {
        "ffprobe"
    } else {
        "ffmpeg"
    })
}

/// What a render feature needs and what it is called for a person.
fn feature_words(
    feature: crate::toolpath::FfmpegFeature,
) -> (&'static str, &'static str, &'static str) {
    use crate::toolpath::FfmpegFeature;
    match feature {
        FfmpegFeature::Vidstab => ("Stabilize", "vid.stab", "turn Stabilize off on those clips"),
        FfmpegFeature::Libass => (
            "Burning captions into the video",
            "libass",
            "remove the caption clips or export them as a subtitle file",
        ),
        FfmpegFeature::Zscale => (
            "Color-managed export",
            "zimg",
            "set the project color back to rec709 or remove the clip color tags",
        ),
    }
}

/// The refusal when the installed ffmpeg builds lack a filter a step needs
/// (e.g. the downloaded macOS build has no vid.stab). Code `tool_missing`,
/// because the remedy is the same: get a build that has it.
pub fn missing_feature_error(feature: crate::toolpath::FfmpegFeature) -> CutError {
    missing_feature_error_for(feature, std::env::consts::OS, std::env::consts::ARCH)
}

/// [`missing_feature_error`] for an explicit platform.
pub fn missing_feature_error_for(
    feature: crate::toolpath::FfmpegFeature,
    os: &str,
    arch: &str,
) -> CutError {
    let (what, library, without) = feature_words(feature);
    let get = if os == "macos" {
        format!(
            "Install Homebrew's complete FFmpeg, which has {library}: Settings > Video processing \
             shows the steps (Homebrew from https://brew.sh, then `{}`); Cut then uses it for this \
             automatically. Or choose a build with {library} via system.set_ffmpeg.",
            homebrew_ffmpeg_command(arch)
        )
    } else {
        format!(
            "Install or choose an FFmpeg built with {library} (system.set_ffmpeg {{\"path\":...}})."
        )
    };
    CutError::new(
        error_codes::TOOL_MISSING,
        format!("{what} needs FFmpeg with {library}, and the installed FFmpeg has none"),
        format!("Without it, {without}."),
    )
    .with_suggested_action(format!(
        "{get} Then call system.doctor {{\"refresh\":true}} and retry."
    ))
    .with_requirement(match feature {
        crate::toolpath::FfmpegFeature::Vidstab => "ffmpeg_vidstab",
        crate::toolpath::FfmpegFeature::Libass => "ffmpeg_libass",
        crate::toolpath::FfmpegFeature::Zscale => "ffmpeg_zimg",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_filter_names_the_feature_library_and_both_ways_out() {
        use crate::toolpath::FfmpegFeature;
        let error = missing_feature_error_for(FfmpegFeature::Vidstab, "macos", "aarch64");
        assert_eq!(error.code, error_codes::TOOL_MISSING);
        assert!(error
            .message
            .starts_with("Stabilize needs FFmpeg with vid.stab"));
        assert!(error.cause.contains("turn Stabilize off"));
        let action = error.suggested_action.unwrap();
        assert!(
            action.contains("/opt/homebrew/bin/brew install ffmpeg-full"),
            "{action}"
        );
        assert!(action.contains("system.set_ffmpeg"));
        let error = missing_feature_error_for(FfmpegFeature::Libass, "linux", "x86_64");
        assert!(error.message.contains("libass"));
        assert!(!error.suggested_action.unwrap().contains("Homebrew"));
        let error = missing_feature_error_for(FfmpegFeature::Zscale, "windows", "x86_64");
        assert!(error.message.contains("zimg"));
    }

    #[test]
    fn apple_silicon_offers_the_download_first_and_keeps_homebrew_steps() {
        let guide = ffmpeg_setup_guide_for("macos", "aarch64");
        assert_eq!(guide.route, SetupRoute::BuiltinDownload);
        assert!(guide.fetch_available);
        let offer = guide.download.as_ref().expect("a download offer");
        assert!(offer.consent.contains("Martin Riedl"));
        assert_eq!(guide.commands.len(), 2);
        assert_eq!(guide.commands[0].command, HOMEBREW_INSTALL_COMMAND);
        assert_eq!(
            guide.commands[1].command,
            "/opt/homebrew/bin/brew install ffmpeg-full"
        );
        assert!(guide.steps.iter().any(|s| s.contains("password")));
        assert!(guide.steps.last().unwrap().contains("Check again"));
        let action = ffmpeg_setup_action_for(&guide);
        assert!(action.contains("\"accept_license\":true"), "{action}");
        assert!(
            action.contains(&offer.consent),
            "the agent relays the same consent"
        );
        assert!(action.contains("Homebrew"));
        assert!(action.contains("system.set_ffmpeg"));
    }

    #[test]
    fn intel_macs_have_no_download_and_get_the_homebrew_route() {
        let guide = ffmpeg_setup_guide_for("macos", "x86_64");
        assert_eq!(guide.route, SetupRoute::Homebrew);
        assert!(!guide.fetch_available && guide.download.is_none());
        let action = ffmpeg_setup_action_for(&guide);
        assert!(action.contains("https://brew.sh"));
        assert!(!action.contains("system.fetch_tool"));
    }

    #[test]
    fn intel_mac_uses_the_intel_homebrew_prefix() {
        let guide = ffmpeg_setup_guide_for("macos", "x86_64");
        assert_eq!(
            guide.commands[1].command,
            "/usr/local/bin/brew install ffmpeg-full"
        );
    }

    #[test]
    fn download_platforms_point_the_agent_at_fetch_tool() {
        for os in ["windows", "linux"] {
            let guide = ffmpeg_setup_guide_for(os, "x86_64");
            assert_eq!(guide.route, SetupRoute::BuiltinDownload);
            let action = ffmpeg_setup_action_for(&guide);
            assert!(action.contains("system.fetch_tool"));
            assert!(action.contains("GNU GPL version 3"), "{action}");
        }
        assert_eq!(
            ffmpeg_setup_guide_for("linux", "aarch64").route,
            SetupRoute::PackageManager
        );
    }

    #[test]
    fn platforms_without_a_download_get_true_ways_forward() {
        // Linux arm64 (a source build of Cut): the distribution's packages,
        // with what each contains.
        let linux = ffmpeg_setup_guide_for("linux", "aarch64");
        assert_eq!(linux.route, SetupRoute::PackageManager);
        assert!(linux.download.is_none() && !linux.fetch_available);
        let commands: Vec<_> = linux.commands.iter().map(|c| c.command.as_str()).collect();
        assert_eq!(
            commands,
            ["sudo apt install ffmpeg", "sudo dnf install ffmpeg-free"]
        );
        assert!(linux
            .steps
            .iter()
            .any(|s| s.contains("vid.stab") && s.contains("RPM Fusion")));
        // Windows arm64 (a source build of Cut): never apt; the publisher's page.
        let windows = ffmpeg_setup_guide_for("windows", "aarch64");
        assert_eq!(windows.route, SetupRoute::ChooseBuild);
        assert!(windows
            .steps
            .iter()
            .any(|s| s.contains(FFMPEG_BUILDS_RELEASES)));
        let action = ffmpeg_setup_action_for(&windows);
        assert!(!action.contains("apt"), "{action}");
        assert!(action.contains("system.set_ffmpeg"));
    }

    #[test]
    fn a_refused_download_offers_each_platform_its_own_other_ways() {
        let mac = download_fallback_action_for("macos", true);
        assert!(mac.starts_with("Nothing was installed. Try again later, or install"));
        assert!(mac.contains("Homebrew"));
        // A file the publisher removed: never suggest trying again.
        let gone = download_fallback_action_for("windows", false);
        assert!(gone.contains("trying again will not help") && !gone.contains("Try again later"));
        assert!(gone.contains(FFMPEG_BUILDS_RELEASES));
        let windows = download_fallback_action_for("windows", true);
        assert!(windows.contains(FFMPEG_BUILDS_RELEASES) && windows.contains("Choose ffmpeg"));
        assert!(!windows.contains("Homebrew") && !windows.contains("apt"));
        let linux = download_fallback_action_for("linux", true);
        for needle in [
            "sudo apt install ffmpeg",
            "sudo dnf install ffmpeg-free",
            "sudo dnf swap ffmpeg-free ffmpeg --allowerasing",
            "libass, vid.stab and zimg",
        ] {
            assert!(linux.contains(needle), "{needle}: {linux}");
        }
    }

    #[test]
    fn missing_tool_error_names_the_tool_and_the_way_forward() {
        let error = missing_tool_error("ffprobe");
        assert_eq!(error.code, error_codes::TOOL_MISSING);
        assert_eq!(error.message, "ffprobe is not installed");
        assert!(error.cause.contains("import, preview and export"));
        let action = error.suggested_action.expect("an action");
        assert!(action.contains("system.set_ffmpeg"));
    }
}
