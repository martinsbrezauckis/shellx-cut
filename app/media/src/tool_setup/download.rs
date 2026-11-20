//! download.rs — the built-in FFmpeg downloads Cut offers, and the consent
//! text shown before any of them starts.
//!
//! One place holds what a person agrees to (publisher, version, size, licence,
//! source) and the exact files Cut accepts on each platform: the https URL,
//! byte size and SHA-256 of every archive, and the size and SHA-256 of every
//! program installed from it. The server's download code reads the pinned
//! releases from here, so the consent text and the bytes that are verified
//! cannot drift apart. Updating a build means changing these constants and
//! nothing else; nothing the publisher's server says later changes them.

use serde::Serialize;

/// How an archive is packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    TarXz,
}

/// One program installed from an archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinnedMember {
    /// Path inside the archive (forward slashes).
    pub path: &'static str,
    /// Name in Cut's tools folder (`ffmpeg`, `ffprobe.exe`, …).
    pub install_as: &'static str,
    /// Exact unpacked size in bytes.
    pub size: u64,
    /// SHA-256 of the unpacked program, lowercase hex.
    pub sha256: &'static str,
}

/// One archive of a pinned release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinnedAsset {
    /// Archive file name under the release base URL.
    pub file: &'static str,
    pub kind: ArchiveKind,
    /// Exact archive size in bytes.
    pub size: u64,
    /// SHA-256 of the archive, lowercase hex.
    pub sha256: &'static str,
    /// The programs taken from it; nothing else is unpacked.
    pub members: &'static [PinnedMember],
}

/// What the unpacked programs must be on this platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramCheck {
    /// Thin arm64 Mach-O, signed with this Apple Developer ID team.
    MacArm64DeveloperId { team: &'static str },
    /// 64-bit x86 Windows PE executable.
    WindowsX64,
    /// 64-bit x86 Linux ELF executable.
    LinuxX64,
    /// No format check (test fixtures only; production releases never use it).
    None,
}

/// A release whose files are fixed in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinnedRelease {
    /// FFmpeg version as shown to the person.
    pub version: &'static str,
    /// The publisher's build name, when it differs from the version.
    pub build: &'static str,
    /// https base; every asset is `<base>/<file>`.
    pub base: &'static str,
    pub assets: &'static [PinnedAsset],
    pub check: ProgramCheck,
}

impl PinnedRelease {
    /// Total download size in bytes.
    pub fn download_bytes(&self) -> u64 {
        self.assets.iter().map(|a| a.size).sum()
    }

    /// Every program this release installs, in install order.
    pub fn members(&self) -> impl Iterator<Item = &PinnedMember> {
        self.assets.iter().flat_map(|a| a.members.iter())
    }
}

/// Martin Riedl's notarized FFmpeg 9.0.2 static build for Apple silicon
/// (https://ffmpeg.martin-riedl.de). ffmpeg and ffprobe are separate zips,
/// each holding one executable.
pub const MACOS_ARM64_FFMPEG: PinnedRelease = PinnedRelease {
    version: "9.0.2",
    build: "9.0.2",
    base: "https://ffmpeg.martin-riedl.de/download/macos/arm64/1789931890_9.0.2",
    assets: &[
        PinnedAsset {
            file: "ffmpeg.zip",
            kind: ArchiveKind::Zip,
            size: 28_395_699,
            sha256: "c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924",
            members: &[PinnedMember {
                path: "ffmpeg",
                install_as: "ffmpeg",
                size: 66_332_880,
                sha256: "2e11c6f90993cdb79fff84d3f90044d28316b310e75b3e030cfc9a54f2c9d384",
            }],
        },
        PinnedAsset {
            file: "ffprobe.zip",
            kind: ArchiveKind::Zip,
            size: 28_317_701,
            sha256: "fcbe839537485eaee7a7a8bc5cbc0f90d53617e80943e8a5b2e31cb851197ea6",
            members: &[PinnedMember {
                path: "ffprobe",
                install_as: "ffprobe",
                size: 66_158_064,
                sha256: "2738aa46a7f9acbc8ab09a6715554c90f7de603171ac403726765685df6a0059",
            }],
        },
    ],
    check: ProgramCheck::MacArm64DeveloperId { team: "KU3N25YGLU" },
};

/// The FFmpeg-Builds monthly build of the FFmpeg 9.0 release branch from
/// 2026-10-31 (FFmpeg 9.0.2 plus 39 branch fixes). The publisher keeps "the
/// last build of each month … for two years"
/// (https://github.com/FFmpeg-Builds/FFmpeg-Builds#release-retention-policy),
/// so this URL is expected to stay until about the end of October 2028.
const FFMPEG_BUILDS_BASE: &str =
    "https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/download/autobuild-2026-10-31-12-59";

/// Windows x64: FFmpeg-Builds `win64-gpl-9.0` (one zip with ffmpeg + ffprobe).
pub const WINDOWS_X64_FFMPEG: PinnedRelease = PinnedRelease {
    version: "9.0.2",
    build: "n9.0.2-39-g7adfbe40dc",
    base: FFMPEG_BUILDS_BASE,
    assets: &[PinnedAsset {
        file: "ffmpeg-n9.0.2-39-g7adfbe40dc-win64-gpl-9.0.zip",
        kind: ArchiveKind::Zip,
        size: 195_110_184,
        sha256: "693f3e224bc184b40524a62ca47526cfebaf2504414021b92fe75f677ad868b9",
        members: &[
            PinnedMember {
                path: "ffmpeg-n9.0.2-39-g7adfbe40dc-win64-gpl-9.0/bin/ffmpeg.exe",
                install_as: "ffmpeg.exe",
                size: 165_984_256,
                sha256: "c930b63cb1ee8b7e2eaf9c15161160edd5c9acad34f0303b5855c8eb142a7e8d",
            },
            PinnedMember {
                path: "ffmpeg-n9.0.2-39-g7adfbe40dc-win64-gpl-9.0/bin/ffprobe.exe",
                install_as: "ffprobe.exe",
                size: 165_783_040,
                sha256: "075848c1bbb8b4e2e0b0ecba06e8ad06c889a8902ed219d6a9f5758c417ca108",
            },
        ],
    }],
    check: ProgramCheck::WindowsX64,
};

/// Linux x86_64: FFmpeg-Builds `linux64-gpl-9.0` (one tar.xz).
pub const LINUX_X64_FFMPEG: PinnedRelease = PinnedRelease {
    version: "9.0.2",
    build: "n9.0.2-39-g7adfbe40dc",
    base: FFMPEG_BUILDS_BASE,
    assets: &[PinnedAsset {
        file: "ffmpeg-n9.0.2-39-g7adfbe40dc-linux64-gpl-9.0.tar.xz",
        kind: ArchiveKind::TarXz,
        size: 153_175_100,
        sha256: "73e29a56974ae5a651133a04584c7483b21afe941e17dc91d79814a758b608d3",
        members: &[
            PinnedMember {
                path: "ffmpeg-n9.0.2-39-g7adfbe40dc-linux64-gpl-9.0/bin/ffmpeg",
                install_as: "ffmpeg",
                size: 173_534_088,
                sha256: "384742b75419584d065bbf372613f9f7e8ccf22525b421c16a2ee679a78ee748",
            },
            PinnedMember {
                path: "ffmpeg-n9.0.2-39-g7adfbe40dc-linux64-gpl-9.0/bin/ffprobe",
                install_as: "ffprobe",
                size: 173_300_552,
                sha256: "ae9051af68f52e7ad5706b442577689c74c02da618ca1590caf1ac7199f2931c",
            },
        ],
    }],
    check: ProgramCheck::LinuxX64,
};

/// The pinned release for `(tool, os, arch)`, if Cut downloads one there.
/// Windows on ARM runs the x64 build of Cut, which reports x86_64 and so
/// gets the x64 download; no arm64 build of Cut is shipped for Windows or
/// Linux, and no arm64 download is offered.
pub fn pinned_release(tool: &str, os: &str, arch: &str) -> Option<&'static PinnedRelease> {
    match (tool, os, arch) {
        ("ffmpeg", "macos", "aarch64") => Some(&MACOS_ARM64_FFMPEG),
        ("ffmpeg", "windows", "x86_64") => Some(&WINDOWS_X64_FFMPEG),
        ("ffmpeg", "linux", "x86_64") => Some(&LINUX_X64_FFMPEG),
        _ => None,
    }
}

/// What a built-in download fetches, shown to the person before they agree
/// and returned to an agent that calls `system.fetch_tool` without consent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DownloadOffer {
    pub publisher: String,
    pub publisher_url: String,
    pub version: String,
    /// Download size in bytes: the sum of the pinned archives.
    pub size_bytes: u64,
    pub size_is_exact: bool,
    pub license: String,
    pub license_url: String,
    /// Where the corresponding source code is published.
    pub source_url: String,
    /// What this build cannot do, in one plain sentence (None = complete).
    pub limitation: Option<String>,
    /// The sentence the person agrees to by choosing Download.
    pub consent: String,
}

const GPL3: &str = "GNU GPL version 3";
const GPL3_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";

/// Whole megabytes, rounded, for consent text.
fn megabytes(bytes: u64) -> u64 {
    (bytes + 500_000) / 1_000_000
}

/// The download offer for `(os, arch)`, or None where Cut has no download.
pub fn download_offer_for(os: &str, arch: &str) -> Option<DownloadOffer> {
    match (os, arch) {
        ("macos", "aarch64") => {
            let release = &MACOS_ARM64_FFMPEG;
            let size = release.download_bytes();
            let source_url = format!(
                "https://ffmpeg.org/releases/ffmpeg-{}.tar.xz",
                release.version
            );
            Some(DownloadOffer {
                publisher: "Martin Riedl".into(),
                publisher_url: "https://ffmpeg.martin-riedl.de".into(),
                version: release.version.into(),
                size_bytes: size,
                size_is_exact: true,
                license: GPL3.into(),
                license_url: GPL3_URL.into(),
                consent: format!(
                    "Download FFmpeg {} for Apple silicon ({} MB) from its publisher Martin \
                     Riedl (ffmpeg.martin-riedl.de). FFmpeg is free software under the {GPL3}, \
                     separate from ShellX Cut. Source code: {source_url}",
                    release.version,
                    megabytes(size)
                ),
                source_url,
                limitation: Some(
                    "This build cannot stabilize video. For Stabilize, install the complete \
                     Homebrew build instead (steps below)."
                        .into(),
                ),
            })
        }
        ("windows" | "linux", "x86_64") => {
            let (system, release) = if os == "windows" {
                ("Windows", &WINDOWS_X64_FFMPEG)
            } else {
                ("Linux", &LINUX_X64_FFMPEG)
            };
            let size = release.download_bytes();
            // The FFmpeg commit this build was made from (the hash in its name).
            let commit = release.build.rsplit("-g").next().unwrap_or(release.build);
            let source_url = format!("https://github.com/FFmpeg/FFmpeg/commit/{commit}");
            Some(DownloadOffer {
                publisher: "FFmpeg-Builds".into(),
                publisher_url: "https://github.com/FFmpeg-Builds/FFmpeg-Builds".into(),
                version: format!("{} ({})", release.version, release.build),
                size_bytes: size,
                size_is_exact: true,
                license: GPL3.into(),
                license_url: GPL3_URL.into(),
                consent: format!(
                    "Download FFmpeg {} (build {}) for {system} ({} MB) from its publisher \
                     FFmpeg-Builds (github.com/FFmpeg-Builds). FFmpeg is free software under the \
                     {GPL3}, separate from ShellX Cut. Source code: {source_url}",
                    release.version,
                    release.build,
                    megabytes(size)
                ),
                source_url,
                limitation: None,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_sha256(s: &str) -> bool {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }

    #[test]
    fn every_download_is_pinned_to_exact_https_files_and_programs() {
        for (os, arch) in [
            ("macos", "aarch64"),
            ("windows", "x86_64"),
            ("linux", "x86_64"),
        ] {
            let release = pinned_release("ffmpeg", os, arch).expect("a download");
            assert!(release.base.starts_with("https://"), "{os}");
            assert_eq!(
                release.version, "9.0.2",
                "one FFmpeg version on all three systems"
            );
            assert_ne!(
                release.check,
                ProgramCheck::None,
                "{os}: production releases check programs"
            );
            let installed: Vec<_> = release.members().map(|m| m.install_as).collect();
            let exe = if os == "windows" { ".exe" } else { "" };
            assert_eq!(
                installed,
                [format!("ffmpeg{exe}"), format!("ffprobe{exe}")],
                "{os}"
            );
            for asset in release.assets {
                assert!(is_sha256(asset.sha256), "{}", asset.file);
                assert!(asset.size > 0);
                for member in asset.members {
                    assert!(is_sha256(member.sha256), "{}", member.path);
                    assert!(member.size > 0);
                    assert!(!member.path.starts_with('/') && !member.path.contains(".."));
                    assert!(!member.path.contains('\\'));
                }
            }
        }
        assert_eq!(
            MACOS_ARM64_FFMPEG.check,
            ProgramCheck::MacArm64DeveloperId { team: "KU3N25YGLU" }
        );
        assert_eq!(WINDOWS_X64_FFMPEG.check, ProgramCheck::WindowsX64);
        assert_eq!(LINUX_X64_FFMPEG.check, ProgramCheck::LinuxX64);
        // The monthly build the publisher keeps for two years, not a floating alias.
        for release in [&WINDOWS_X64_FFMPEG, &LINUX_X64_FFMPEG] {
            assert!(release.base.ends_with("/autobuild-2026-10-31-12-59"));
            assert!(!release.base.contains("latest"));
        }
        for (os, arch) in [
            ("macos", "x86_64"),
            ("linux", "aarch64"),
            ("windows", "aarch64"),
        ] {
            assert!(pinned_release("ffmpeg", os, arch).is_none(), "{os}/{arch}");
        }
    }

    #[test]
    fn the_macos_consent_names_publisher_version_size_licence_and_source() {
        let offer = download_offer_for("macos", "aarch64").unwrap();
        for needle in [
            "Martin Riedl",
            "ffmpeg.martin-riedl.de",
            "9.0.2",
            "57 MB",
            "GNU GPL version 3",
            "https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz",
        ] {
            assert!(
                offer.consent.contains(needle),
                "{needle}: {}",
                offer.consent
            );
        }
        assert!(offer.limitation.unwrap().contains("Stabilize"));
    }

    #[test]
    fn windows_and_linux_consent_names_the_exact_build() {
        for (os, size) in [("windows", "195 MB"), ("linux", "153 MB")] {
            let offer = download_offer_for(os, "x86_64").unwrap();
            for needle in [
                "FFmpeg 9.0.2",
                "build n9.0.2-39-g7adfbe40dc",
                size,
                "FFmpeg-Builds",
                "GNU GPL version 3",
                "https://github.com/FFmpeg/FFmpeg/commit/7adfbe40dc",
            ] {
                assert!(
                    offer.consent.contains(needle),
                    "{os} {needle}: {}",
                    offer.consent
                );
            }
            assert!(
                !offer.consent.contains("current"),
                "{os}: no floating build"
            );
            assert!(offer.size_is_exact && offer.limitation.is_none());
        }
        assert!(download_offer_for("linux", "aarch64").is_none());
        assert!(download_offer_for("windows", "aarch64").is_none());
    }
}
