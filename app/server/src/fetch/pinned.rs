//! pinned.rs — install an FFmpeg release whose files are pinned in the source
//! (cut_media::tool_setup: one per platform).
//!
//! Order of checks, all inside a private, randomly named folder the engine
//! creates in the tools folder (same volume, owner-only):
//!   1. each archive is downloaded over https (redirects too) with deadlines
//!      and an exact byte limit; a file the publisher no longer offers is a
//!      plain refusal, never retried;
//!   2. its size and SHA-256 equal the pinned values;
//!   3. a zip's every entry is safe (no absolute or `..` path, link or
//!      encryption) and holds each pinned program at its pinned size;
//!   4. only the pinned programs are unpacked (the system `tar`), and each
//!      unpacked program must be a regular file whose size and SHA-256 equal
//!      the pinned values;
//!   5. each program is the kind this machine runs (Mach-O arm64 with the
//!      publisher's Apple Developer ID; PE x64; ELF x86-64).
//! Only then are the programs moved into Cut's tools folder by rename. Any
//! failure returns a refusal naming the check and the platform's other ways,
//! and the private folder is removed with everything in it.

use super::macos_signature::verify_developer_id;
use super::program_header::{check_arm64_macho, check_linux_x64_elf, check_windows_x64_pe};
use super::system_programs::system_tar;
use super::zip_member::check_zip;
use super::*;
use cut_media::tool_setup::{ArchiveKind, PinnedAsset, PinnedMember, PinnedRelease, ProgramCheck};

/// A refused download: which check failed, the detail, and the platform's
/// other ways to get FFmpeg.
pub(super) fn refused(check: &str, detail: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::JOB_FAILED,
        format!("FFmpeg was not installed: {check}"),
        detail,
    )
    .with_suggested_action(cut_media::tool_setup::download_fallback_action(true))
}

#[cfg(test)]
static TEST_RELEASE: std::sync::OnceLock<PinnedRelease> = std::sync::OnceLock::new();

/// Let an isolated test process download a fixture release instead of the
/// production one (its hashes cannot match a local fixture).
#[cfg(all(test, target_os = "linux"))]
pub(crate) fn use_release_for_tests(release: PinnedRelease) {
    let _ = TEST_RELEASE.set(release);
}

/// The pinned release `system.fetch_tool` installs for `tool` here.
pub(super) fn release_for(tool: &str, os: &str, arch: &str) -> Option<&'static PinnedRelease> {
    #[cfg(test)]
    {
        if let Some(release) = TEST_RELEASE.get().filter(|_| tool == "ffmpeg") {
            return Some(release);
        }
    }
    cut_media::tool_setup::pinned_release(tool, os, arch)
}

pub(super) fn install(
    tool: &str,
    release: &'static PinnedRelease,
    progress: &ProgressFn,
) -> Result<InstallOutcome, CutError> {
    let tools_dir = cut_media::toolpath::appdata_tools_dir().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "no app-data tools directory available (HOME unset)",
            "cannot determine where to install the tool",
        )
    })?;
    install_into(tool, release, &base_url(release.base), &tools_dir, progress)
}

/// [`install`] with the tools folder and release base given explicitly.
fn install_into(
    tool: &str,
    release: &PinnedRelease,
    base: &str,
    tools_dir: &Path,
    progress: &ProgressFn,
) -> Result<InstallOutcome, CutError> {
    // The programs that unpack the archives must exist BEFORE 100+ MB are fetched.
    let kinds: Vec<ArchiveKind> = release.assets.iter().map(|asset| asset.kind).collect();
    let xz = super::unpack_tools::preflight(&kinds, &system_tar())?;
    std::fs::create_dir_all(tools_dir).map_err(io_err("create tools dir"))?;
    // Owner-only, unpredictable name, removed on every exit path (Drop).
    let staging = tempfile::Builder::new()
        .prefix(".download-")
        .tempdir_in(tools_dir)
        .map_err(io_err("create private download folder"))?;
    let bin = staging.path().join("bin");
    std::fs::create_dir(&bin).map_err(io_err("create staging bin dir"))?;

    let count = release.assets.len() as f32;
    let mut digests = Vec::new();
    for (index, asset) in release.assets.iter().enumerate() {
        let step = |fraction: f32, message: &str| {
            progress((index as f32 + fraction) / count * 0.9, message)
        };
        let url = format!("{base}/{}", asset.file);
        let archive = staging.path().join(asset.file);
        fetch_asset(asset, &url, &archive, &step)?;
        step(0.82, "checking the archive");
        let unpacked = staging.path().join(format!("unpacked-{index}"));
        unpack(asset, &archive, &unpacked, xz.as_deref())?;
        for member in asset.members {
            step(0.9, &format!("checking {}", member.install_as));
            let program = unpacked_member(&unpacked, member)?;
            verify_program(&program, release.check)?;
            std::fs::rename(&program, bin.join(member.install_as))
                .map_err(io_err("stage binary"))?;
        }
        // The archive is no longer needed; free its space before the next one.
        let _ = std::fs::remove_file(&archive);
        let _ = std::fs::remove_dir_all(&unpacked);
        digests.push(asset.sha256);
    }

    progress(0.92, "installing");
    let install_dir = tools_dir.join(tool);
    let names: Vec<&str> = release
        .members()
        .map(|m| m.install_as.trim_end_matches(".exe"))
        .collect();
    install_atomic(&bin, &install_dir, &names)?;
    progress(0.97, "probing version");
    let version = probe_version(&install_dir.join("bin").join(exe_name(names[0])));
    progress(1.0, "done");
    Ok(InstallOutcome {
        tool: tool.to_string(),
        installed_dir: install_dir.display().to_string(),
        version,
        // One SHA-256 per downloaded archive, space-separated, download order.
        sha256: digests.join(" "),
        source_url: base.to_string(),
        bytes: release.download_bytes(),
    })
}

/// Download one archive, retrying only temporary network failures. A file
/// the publisher no longer serves (404/410) is refused at once.
fn fetch_asset(
    asset: &PinnedAsset,
    url: &str,
    archive: &Path,
    step: &dyn Fn(f32, &str),
) -> Result<(), CutError> {
    let mut delays = LATEST_FETCH_DELAYS.iter();
    loop {
        let failure =
            match download_hashing_with_limit(url, archive, &|f, m| step(f * 0.8, m), asset.size) {
                Ok((bytes, sha256)) => return check_download(asset, bytes, &sha256),
                Err(failure) => failure,
            };
        if matches!(failure.status, Some(404 | 410)) {
            return Err(gone(asset, url));
        }
        match delays.next() {
            Some(delay) if failure.transient => {
                step(0.0, "publisher temporarily unavailable; retrying");
                std::thread::sleep(*delay);
            }
            _ => {
                return Err(refused(
                    "the download did not complete",
                    format!(
                        "{}: {} ({})",
                        asset.file, failure.error.message, failure.error.cause
                    ),
                ))
            }
        }
    }
}

/// The pinned file is no longer offered at its address.
pub(super) fn gone(asset: &PinnedAsset, url: &str) -> CutError {
    CutError::new(
        error_codes::JOB_FAILED,
        "FFmpeg was not installed: the pinned FFmpeg build is no longer offered by its publisher",
        format!(
            "{} is gone from {url} (the publisher removed it); this version of ShellX Cut \
             only installs that exact file",
            asset.file
        ),
    )
    .with_suggested_action(cut_media::tool_setup::download_fallback_action(false))
}

/// The downloaded bytes must be exactly the pinned file.
pub(super) fn check_download(
    asset: &PinnedAsset,
    bytes: u64,
    sha256: &str,
) -> Result<(), CutError> {
    if bytes != asset.size {
        return Err(refused(
            "the file size differs from the pinned release",
            format!("{} is {bytes} bytes; expected {}", asset.file, asset.size),
        ));
    }
    if !sha256.eq_ignore_ascii_case(asset.sha256) {
        return Err(refused(
            "the SHA-256 differs from the pinned release",
            format!("{}: expected {}, got {sha256}", asset.file, asset.sha256),
        ));
    }
    Ok(())
}

/// Unpack only the pinned programs of `archive` into `dest`.
/// `xz` is the absolute decompressor the preflight admitted (`.tar.xz` only).
fn unpack(
    asset: &PinnedAsset,
    archive: &Path,
    dest: &Path,
    xz: Option<&Path>,
) -> Result<(), CutError> {
    if asset.kind == ArchiveKind::Zip {
        let bytes = std::fs::read(archive).map_err(io_err("read archive"))?;
        check_zip(&bytes, asset.members)
            .map_err(|e| refused("the archive is not the expected one", e))?;
    }
    std::fs::create_dir(dest).map_err(io_err("create unpack folder"))?;
    // tar starts xz by name: it may search only the admitted xz's folder.
    let mut command = super::system_programs::tar_command(xz);
    command.arg("-x");
    if asset.kind == ArchiveKind::TarXz {
        command.arg("-J");
    }
    command.arg("-f").arg(archive).arg("-C").arg(dest);
    for member in asset.members {
        command.arg(member.path);
    }
    let output = crate::dispatch::run_bounded_foreground_command(&mut command, "unpack archive")
        .map_err(|e| refused("the archive could not be unpacked", e.cause))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(super::unpack_tools::from_tar_failure(&stderr)
            .unwrap_or_else(|| refused("the archive could not be unpacked", stderr)))
    }
}

/// An unpacked program must be a regular file of its pinned size and SHA-256.
pub(super) fn unpacked_member(dest: &Path, member: &PinnedMember) -> Result<PathBuf, CutError> {
    let path = member
        .path
        .split('/')
        .fold(dest.to_path_buf(), |p, part| p.join(part));
    let meta = std::fs::symlink_metadata(&path).map_err(|_| {
        refused(
            "the archive unpacked to something unexpected",
            format!("'{}' is missing after unpacking", member.path),
        )
    })?;
    if !meta.file_type().is_file() || meta.len() != member.size {
        return Err(refused(
            "the archive unpacked to something unexpected",
            format!(
                "'{}' is {} with {} bytes; expected a regular file of {} bytes",
                member.path,
                if meta.file_type().is_symlink() {
                    "a link"
                } else {
                    "a file"
                },
                meta.len(),
                member.size
            ),
        ));
    }
    let mut file = std::fs::File::open(&path).map_err(io_err("read unpacked program"))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(io_err("hash unpacked program"))?;
    let sha256 = hex::encode(hasher.finalize());
    if !sha256.eq_ignore_ascii_case(member.sha256) {
        return Err(refused(
            "an unpacked program differs from the pinned release",
            format!(
                "'{}': expected {}, got {sha256}",
                member.path, member.sha256
            ),
        ));
    }
    Ok(path)
}

/// The unpacked program must be the kind of program this machine runs.
fn verify_program(path: &Path, check: ProgramCheck) -> Result<(), CutError> {
    let mut header = vec![0u8; 4096];
    let read = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut header))
        .map_err(io_err("read program header"))?;
    header.truncate(read);
    let wrong = |e: String| refused("the program is not for this computer", e);
    match check {
        ProgramCheck::MacArm64DeveloperId { team } => {
            check_arm64_macho(&header).map_err(wrong)?;
            verify_developer_id(path, team)
                .map_err(|e| refused("the publisher's signature did not verify", e))
        }
        ProgramCheck::WindowsX64 => check_windows_x64_pe(&header).map_err(wrong),
        ProgramCheck::LinuxX64 => check_linux_x64_elf(&header).map_err(wrong),
        ProgramCheck::None => Ok(()),
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "pinned_busy_tests.rs"]
mod busy_tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "pinned_planted_xz_tests.rs"]
mod planted_xz_tests;

#[cfg(test)]
mod tests {
    use super::*;

    const ASSET: PinnedAsset = PinnedAsset {
        file: "ffmpeg.zip",
        kind: ArchiveKind::Zip,
        size: 5,
        sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        members: &[],
    };

    #[test]
    fn a_hash_or_size_mismatch_is_refused_with_the_way_forward() {
        assert!(check_download(&ASSET, 5, &ASSET.sha256.to_uppercase()).is_ok());
        let error = check_download(&ASSET, 5, &"0".repeat(64)).unwrap_err();
        assert!(
            error.message.contains("SHA-256 differs"),
            "{}",
            error.message
        );
        assert!(error
            .suggested_action
            .unwrap()
            .contains("Nothing was installed"));
        let error = check_download(&ASSET, 6, ASSET.sha256).unwrap_err();
        assert!(error.message.contains("size differs"), "{}", error.message);
    }

    #[test]
    fn a_file_the_publisher_removed_is_refused_plainly() {
        let error = gone(&ASSET, "https://example.invalid/ffmpeg.zip");
        assert!(
            error.message.contains("no longer offered by its publisher"),
            "{}",
            error.message
        );
        assert!(error.cause.contains("exact file"));
        let action = error.suggested_action.unwrap();
        assert!(action.contains("Nothing was installed"), "{action}");
        // The file is gone for good: retrying cannot help, so never say so.
        assert!(action.contains("trying again will not help"), "{action}");
        assert!(!action.contains("Try again later"), "{action}");
    }

    const MEMBER: PinnedMember = PinnedMember {
        path: "pkg/bin/ffmpeg",
        install_as: "ffmpeg",
        size: 5,
        sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    };

    #[test]
    fn an_unpacked_program_must_match_its_pinned_size_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("pkg/bin")).unwrap();
        std::fs::write(dir.path().join("pkg/bin/ffmpeg"), b"hello").unwrap();
        assert!(unpacked_member(dir.path(), &MEMBER).is_ok());
        std::fs::write(dir.path().join("pkg/bin/ffmpeg"), b"jello").unwrap();
        let error = unpacked_member(dir.path(), &MEMBER).unwrap_err();
        assert!(
            error.message.contains("differs from the pinned release"),
            "{}",
            error.message
        );
        std::fs::write(dir.path().join("pkg/bin/ffmpeg"), b"hello!").unwrap();
        assert!(
            unpacked_member(dir.path(), &MEMBER).is_err(),
            "size must match"
        );
        std::fs::remove_file(dir.path().join("pkg/bin/ffmpeg")).unwrap();
        assert!(unpacked_member(dir.path(), &MEMBER)
            .unwrap_err()
            .cause
            .contains("missing"));
    }

    #[cfg(unix)]
    #[test]
    fn an_unpacked_link_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("pkg/bin")).unwrap();
        std::os::unix::fs::symlink("/bin/sh", dir.path().join("pkg/bin/ffmpeg")).unwrap();
        let error = unpacked_member(dir.path(), &MEMBER).unwrap_err();
        assert!(error.cause.contains("a link"), "{}", error.cause);
    }

    /// Serve fixed responses over loopback http (the download code allows
    /// plain http only to an exact loopback host).
    fn serve(
        responses: Vec<(&'static str, u16, Vec<u8>)>,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for (path, status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let size = stream.read(&mut request).unwrap();
                assert!(
                    String::from_utf8_lossy(&request[..size]).starts_with(&format!("GET /{path} "))
                );
                write!(
                    stream,
                    "HTTP/1.1 {status} fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        (base, server)
    }

    fn fixture_release(sha256: &'static str, size: u64) -> PinnedRelease {
        let assets: &'static [PinnedAsset] = Box::leak(Box::new([PinnedAsset {
            file: "ffmpeg.zip",
            kind: ArchiveKind::Zip,
            size,
            sha256,
            members: &[],
        }]));
        PinnedRelease {
            version: "test",
            build: "test",
            base: "https://example.invalid",
            assets,
            check: ProgramCheck::None,
        }
    }

    fn left_in(tools: &Path) -> Vec<std::ffi::OsString> {
        std::fs::read_dir(tools)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect()
    }

    /// The full route against a loopback server: a tampered archive is
    /// refused at the hash check and the tools folder gains nothing.
    #[test]
    fn a_tampered_download_installs_nothing() {
        let tools = tempfile::tempdir().unwrap();
        let (base, server) = serve(vec![("ffmpeg.zip", 200, b"tampered".to_vec())]);
        let release = fixture_release(
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
            8,
        );
        let error = install_into("ffmpeg", &release, &base, tools.path(), &|_, _| {}).unwrap_err();
        server.join().unwrap();
        assert!(
            error.message.contains("SHA-256 differs"),
            "{}",
            error.message
        );
        assert!(left_in(tools.path()).is_empty(), "nothing may remain");
    }

    /// A pinned file that is gone (404) is refused at once, without retries,
    /// and nothing is installed.
    #[test]
    fn a_removed_pinned_file_is_refused_without_retrying() {
        let tools = tempfile::tempdir().unwrap();
        let (base, server) = serve(vec![("ffmpeg.zip", 404, Vec::new())]);
        let release = fixture_release(
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
            5,
        );
        let started = std::time::Instant::now();
        let error = install_into("ffmpeg", &release, &base, tools.path(), &|_, _| {}).unwrap_err();
        server.join().unwrap();
        assert!(
            error.message.contains("no longer offered"),
            "{}",
            error.message
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "no retry delays"
        );
        assert!(left_in(tools.path()).is_empty(), "nothing may remain");
    }

    /// The real route end to end on this platform: download from the
    /// publisher, every check, install into a temporary tools folder, run
    /// `-version`. Run by hand on Apple silicon, Windows x64 or Linux x86_64:
    /// `cargo test -p server real_pinned_download -- --ignored --nocapture`.
    #[test]
    #[ignore = "downloads 57-195 MB from the publisher"]
    fn real_pinned_download_installs_a_verified_build() {
        let release = cut_media::tool_setup::pinned_release(
            "ffmpeg",
            std::env::consts::OS,
            std::env::consts::ARCH,
        )
        .expect("this platform has a pinned download");
        let tools = tempfile::tempdir().unwrap();
        let outcome = install_into("ffmpeg", release, release.base, tools.path(), &|f, m| {
            eprintln!("{:>3.0}% {m}", f * 100.0)
        })
        .expect("the publisher's build passes every check");
        eprintln!("{outcome:?}");
        let bin = tools.path().join("ffmpeg").join("bin");
        for member in release.members() {
            let out = std::process::Command::new(bin.join(member.install_as))
                .arg("-version")
                .output()
                .unwrap();
            assert!(out.status.success(), "{} runs", member.install_as);
            let banner = String::from_utf8_lossy(&out.stdout);
            assert!(banner.contains("9.0.2"), "{}: {banner}", member.install_as);
        }
        let left: Vec<_> = left_in(tools.path())
            .into_iter()
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["ffmpeg"], "the private download folder is gone");
    }
}
