//! vc_runtime.rs — the Microsoft Visual C++ Redistributable as one named tool.
//!
//! ROLE
//!   On Windows, the Python packages behind captions and the other perception
//!   features (onnxruntime, ctranslate2, torch and its family) import the
//!   Microsoft Visual C++ runtime (MSVCP140.dll and related files) and do not
//!   ship it; a clean Windows does not have it. ShellX Cut does not ship or
//!   fetch these files from anyone else: the person installs Microsoft's own
//!   Redistributable, system-wide, and Windows asks them for administrator
//!   permission (owner decision 2026-11-20).
//!
//!   This module is the ONE place that knows this tool: its id, its state on
//!   this PC, where it comes from, what needs it, the refusal a feature gives
//!   without it, and its install. The doctor shows it as a card, the
//!   Environment tools list renders that card, `system.fetch_tool
//!   {tool:"vc-runtime"}` installs it, and features call [`require_for`].
//!
//! SOURCE (Microsoft's own, nothing else)
//!   The permalinks on Microsoft's "Latest supported Visual C++ Redistributable
//!   downloads" page ([`OFFICIAL_PAGE`]): `https://aka.ms/vc14/vc_redist.x64.exe`
//!   and `https://aka.ms/vc14/vc_redist.arm64.exe`. They always serve the
//!   current version, so no hash can be pinned. Instead the downloaded file runs
//!   only after Windows' own signature check (WinVerifyTrust) accepts it, its
//!   chain ends at a Microsoft root (CertVerifyCertificateChainPolicy with the
//!   Microsoft-root policy) and its signer's organization is Microsoft
//!   Corporation. Microsoft's installer is started with its documented
//!   `/install /passive /norestart`: the person sees its progress, Windows
//!   shows its own administrator prompt, and the app never answers it.
//!
//! The uninstaller never removes it: it belongs to Windows and other programs.

// Only Windows installs or checks this runtime; elsewhere the install half of the module is unused by design.
#![cfg_attr(not(windows), allow(dead_code))]

use cut_core::{error_codes, CutError};
use serde_json::{json, Value};

use crate::doctor::{Card, CardStatus};

#[cfg(windows)]
#[path = "vc_runtime/windows.rs"]
mod windows;

/// Stable id: doctor card id and `system.fetch_tool` tool id.
pub const TOOL_ID: &str = "vc-runtime";
/// What a person reads.
pub const TITLE: &str = "Microsoft Visual C++ runtime";
pub const PUBLISHER: &str = "Microsoft Corporation";
/// Microsoft's page that lists the official permalinks.
pub const OFFICIAL_PAGE: &str =
    "https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist";
/// Microsoft's licence terms for the Redistributable (linked from that page).
pub const LICENSE_URL: &str = "https://aka.ms/VCRedistLicense";
pub const LICENSE_NAME: &str =
    "Microsoft Software License Terms for the Microsoft Visual C++ Redistributable";
/// The features that need it on Windows, from the import tables of their
/// Python packages (onnxruntime, ctranslate2, torch, torchaudio, torchvision).
pub const NEEDED_BY: &[&str] = &[
    "Captions and transcription",
    "Perception tools (scenes, auto-reframe, OCR, translation)",
    "Cleaner background removal",
];
/// Files Windows' loader must find in the system folder.
pub const SYSTEM_FILES: &[&str] = &[
    "msvcp140.dll",
    "msvcp140_1.dll",
    "vcruntime140.dll",
    "vcruntime140_1.dll",
];
/// Microsoft's installer is about 20 MB; refuse anything far larger.
const MAX_INSTALLER_BYTES: u64 = 64 * 1024 * 1024;

/// Microsoft's permalink for the Redistributable of this program's
/// architecture. ShellX Cut and its Python run as `arch`, and Microsoft's x64
/// package also carries the ARM64 files for an x64 program on an ARM64 PC.
pub fn download_url(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" => Some("https://aka.ms/vc14/vc_redist.x64.exe"),
        "aarch64" => Some("https://aka.ms/vc14/vc_redist.arm64.exe"),
        _ => None,
    }
}

/// This PC's state of the runtime.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    /// Windows only; elsewhere nothing is needed.
    pub applicable: bool,
    /// Every file in [`SYSTEM_FILES`] is in Windows' system folder.
    pub installed: bool,
    /// Microsoft's registered version, when readable.
    pub version: Option<String>,
    pub missing_files: Vec<String>,
}

pub fn status() -> Status {
    #[cfg(windows)]
    {
        windows::status()
    }
    #[cfg(not(windows))]
    {
        Status {
            applicable: false,
            installed: true,
            version: None,
            missing_files: Vec::new(),
        }
    }
}

/// What Install does, in plain words (shown beside the Install button).
pub fn about_text() -> String {
    format!(
        "Downloads Microsoft's Visual C++ Redistributable (about 20 MB) from Microsoft and installs it for all \
         of Windows. Licence: {LICENSE_NAME} ({LICENSE_URL}). Microsoft's installer shows its progress; Windows \
         asks for administrator permission. ShellX Cut checks Microsoft's signature before running it."
    )
}

/// The doctor card (Windows only).
pub fn doctor_card() -> Option<Card> {
    let state = status();
    if !state.applicable {
        return None;
    }
    Some(card_for(&state, std::env::consts::ARCH))
}

pub(crate) fn card_for(state: &Status, arch: &str) -> Card {
    let download = download_url(arch);
    Card {
        id: TOOL_ID.to_string(),
        // Listed with captions in Settings > AI & transcription: it is what
        // those features need first.
        kind: "perception".to_string(),
        status: if state.installed {
            CardStatus::Ok
        } else {
            CardStatus::Missing
        },
        source: None,
        version: state.version.clone(),
        hint: (!state.installed).then(|| {
            format!(
                "Needed for captions and transcription. Install it here (Microsoft's own installer; Windows asks \
                 for administrator permission), or from {OFFICIAL_PAGE}."
            )
        }),
        details: json!({
            "title": TITLE,
            "publisher": PUBLISHER,
            "installed": state.installed,
            "missing_files": state.missing_files,
            "needed_by": NEEDED_BY,
            "official_page": OFFICIAL_PAGE,
            "license_url": LICENSE_URL,
            "download_url": download,
            "fetch_available": download.is_some() && !state.installed,
            "fetch_action": {"verb": "system.fetch_tool", "args": {"tool": TOOL_ID}},
            "about": about_text(),
            "license_name": LICENSE_NAME,
            "approval": "Windows asks a person for administrator permission; an agent cannot approve it",
            "system_wide": true,
            "removed_by_uninstall": false,
        }),
    }
}

/// The refusal a feature gives when it needs the runtime and it is missing.
pub fn missing_error(feature: &str) -> CutError {
    CutError::new(
        error_codes::TOOL_MISSING,
        format!("{feature} cannot start: the Microsoft Visual C++ runtime is not installed on this PC"),
        "the Python packages behind this feature use Microsoft's Visual C++ Redistributable \
         (msvcp140.dll and related files), and Windows' system folder does not have it",
    )
    .with_requirement("vc_runtime")
    // A person reads this one text; an agent's route (system.fetch_tool
    // {"tool":"vc-runtime"}) is in `error.requirement.get` from the registry.
    .with_suggested_action(format!(
        "Install it first: Settings > AI & transcription > {TITLE} > Install (Microsoft's own installer; Windows \
         asks for administrator permission). Or install it from {OFFICIAL_PAGE}. Then choose this again; no \
         restart of ShellX Cut is needed"
    ))
}

/// `Ok` when `feature` can run as far as this runtime goes.
pub fn require_for(feature: &str) -> Result<(), CutError> {
    let state = status();
    if state.applicable && !state.installed {
        Err(missing_error(feature))
    } else {
        Ok(())
    }
}

/// What Microsoft's installer's exit code means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerExit {
    Installed {
        restart_needed: bool,
    },
    /// The same or a newer version is already installed.
    AlreadyPresent,
    /// The Windows administrator prompt was declined (or closed).
    Declined,
    Failed(u32),
}

/// Microsoft's installer reports Windows Installer codes, or an HRESULT.
pub fn classify_exit(code: u32) -> InstallerExit {
    match code {
        0 => InstallerExit::Installed {
            restart_needed: false,
        },
        3010 | 1641 => InstallerExit::Installed {
            restart_needed: true,
        },
        // ERROR_PRODUCT_VERSION (another version is installed), plain and as HRESULT.
        1638 | 0x8007_0666 => InstallerExit::AlreadyPresent,
        // ERROR_INSTALL_USEREXIT, ERROR_CANCELLED, both also as HRESULT.
        1602 | 1223 | 0x8007_0642 | 0x8007_04C7 => InstallerExit::Declined,
        other => InstallerExit::Failed(other),
    }
}

/// The refusal for an install that did not leave the runtime installed.
pub fn install_error(exit: InstallerExit, can_approve: bool) -> CutError {
    match exit {
        InstallerExit::Declined if can_approve => CutError::new(
            error_codes::JOB_FAILED,
            "The Windows administrator prompt was declined, so the Microsoft Visual C++ runtime was not installed",
            "Microsoft's installer needs administrator permission to install for all of Windows",
        )
        .with_suggested_action(format!(
            "Choose Install again and approve the Windows prompt, or install it from {OFFICIAL_PAGE}"
        )),
        InstallerExit::Declined => CutError::new(
            error_codes::JOB_FAILED,
            "This Windows account cannot approve installing the Microsoft Visual C++ runtime",
            "the account is not an administrator, and the administrator prompt was not approved",
        )
        .with_suggested_action(format!(
            "Ask an administrator to install the Microsoft Visual C++ Redistributable, or install it from \
             {OFFICIAL_PAGE}"
        )),
        InstallerExit::Failed(code) => CutError::new(
            error_codes::JOB_FAILED,
            format!("Microsoft's Visual C++ Redistributable installer stopped with code {code}"),
            format!("exit code {code} (0x{code:08X}); the runtime is still not installed"),
        )
        .with_suggested_action(format!(
            "Install it from {OFFICIAL_PAGE}, then choose Check again"
        )),
        InstallerExit::Installed { .. } | InstallerExit::AlreadyPresent => CutError::new(
            error_codes::JOB_FAILED,
            "Microsoft's installer finished, but Windows' system folder still lacks the Visual C++ runtime files",
            "the runtime files were not found after the install",
        )
        .with_suggested_action(format!(
            "Restart Windows and choose Check again, or install it from {OFFICIAL_PAGE}"
        )),
    }
}

pub fn download_error(cause: CutError) -> CutError {
    CutError::new(
        error_codes::JOB_FAILED,
        "The Microsoft Visual C++ runtime could not be downloaded from Microsoft",
        format!("{}: {}", cause.message, cause.cause),
    )
    .with_suggested_action(format!(
        "Check the internet connection and choose Install again, or get it from {OFFICIAL_PAGE} (also on \
         another PC) and run it here"
    ))
}

pub fn signature_error(reason: &str) -> CutError {
    CutError::new(
        error_codes::JOB_FAILED,
        "The downloaded Visual C++ Redistributable did not pass the Windows signature check, so it was not run",
        reason.to_string(),
    )
    .with_suggested_action(format!(
        "Choose Install again; if it keeps failing, install it from {OFFICIAL_PAGE}"
    ))
}

/// Download, verify and run Microsoft's installer. BLOCKING. `progress` gets
/// 0..1 and a plain message; `cancelled` is checked before the installer
/// starts (once it runs, Windows and Microsoft's installer own it: a cancel
/// then takes effect when it ends, and the result says what it did).
/// One install at a time: a second request is refused while one runs.
pub fn install(
    progress: &dyn Fn(f32, &str),
    cancelled: &dyn Fn() -> bool,
) -> Result<Value, CutError> {
    #[cfg(windows)]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static RUNNING: AtomicBool = AtomicBool::new(false);
        if RUNNING.swap(true, Ordering::SeqCst) {
            return Err(CutError::new(
                error_codes::JOB_FAILED,
                "Microsoft's Visual C++ Redistributable installer is already running",
                "another install of the runtime has not finished yet",
            )
            .with_suggested_action("Finish or close the Windows prompt of the running install, then choose Check again"));
        }
        struct Release;
        impl Drop for Release {
            fn drop(&mut self) {
                RUNNING.store(false, Ordering::SeqCst);
            }
        }
        let _release = Release;
        windows::install(progress, cancelled)
    }
    #[cfg(not(windows))]
    {
        let _ = (progress, cancelled);
        Err(CutError::new(
            error_codes::UNSUPPORTED_PLATFORM,
            "The Microsoft Visual C++ runtime is only needed on Windows",
            format!("this is {}", std::env::consts::OS),
        ))
    }
}

/// Shared by the Windows install: the private download location.
#[cfg_attr(not(windows), allow(dead_code))]
fn staging_dir() -> Result<tempfile::TempDir, CutError> {
    let tools = cut_media::toolpath::appdata_tools_dir().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "no app-data tools directory available (LOCALAPPDATA unset)",
            "cannot determine where to download Microsoft's installer",
        )
    })?;
    std::fs::create_dir_all(&tools)
        .map_err(|e| CutError::new(error_codes::IO, "create tools dir failed", e.to_string()))?;
    tempfile::Builder::new()
        .prefix(".vc-runtime-")
        .tempdir_in(&tools)
        .map_err(|e| {
            CutError::new(
                error_codes::IO,
                "create download folder failed",
                e.to_string(),
            )
        })
}

#[cfg(test)]
#[path = "vc_runtime_tests.rs"]
mod tests;
