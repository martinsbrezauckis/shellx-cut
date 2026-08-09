//! fetch.rs — `system.fetch_tool` consented tool download + install.
//!
//! ROLE
//!   The packaging design downloads ffmpeg on first
//!   run, user-consented" — it keeps our installer ~3 MB and our license open
//!   (the GPL libx264 binary lands on the USER's disk by the user's action,
//!   never in our distributed artifact). This module implements the download:
//!   a verified BtbN FFmpeg build fetched into the app-data tools dir — the
//!   exact rung-3 directory the cut-media toolpath resolver checks, so a
//!   successful fetch flips the doctor's ffmpeg card missing → bundled-or-
//!   appdata with zero other code change.
//!
//! SECURITY (HARD — this is a network-write primitive; treat it like the output-fencing contract)
//!   1. NO caller-supplied URL. The tool id ("ffmpeg") indexes a BUILT-IN
//!      REGISTRY of https-pinned URLs. A caller can pick WHICH known tool, never
//!      WHERE from. This is the difference between a feature and an arbitrary-
//!      download RCE.
//!   2. https only, host-pinned to github.com/BtbN releases.
//!   3. sha256 of the downloaded archive is verified against the release's
//!      `checksums.sha256` BEFORE a single byte is extracted/installed. A
//!      mismatch aborts — nothing touches the install dir.
//!   4. Staged-then-atomic: download + verify + extract happen in a temp dir;
//!      the verified payload is moved into place by a single rename (atomic on
//!      the same volume; a copy+swap fallback otherwise). A failure mid-way
//!      never leaves a half-installed tool the resolver would trust.
//!   5. Extraction is of ALREADY-VERIFIED bytes, via the platform archive tool
//!      (tar on unix, tar.exe/Expand-Archive on Windows) — we do not parse
//!      untrusted archive bytes in-process, and we contain extraction to the
//!      temp dir.
//!
//! OS-AWARENESS
//!   The registry selects the asset by `std::env::consts::OS`/`ARCH`: Linux ⇒
//!   `*-linux64-gpl.tar.xz` (verified here), Windows ⇒ `*-win64-gpl.zip`
//!   (selected by OS, install path documented for the cold-notebook session).
//!
//! TESTABILITY
//!   `SHELLX_CUT_FETCH_BASE_URL` overrides the registry BASE (the github.com
//!   release URL) so the full path — download, checksum-verify, extract,
//!   atomic-install, doctor re-scan — is provable against a LOCAL https/http
//!   fixture server with a test checksum, with no network. The override changes
//!   only the host, never the "no caller URL" property (it is operator/test
//!   config, not a verb arg).
//!
//! Dependencies: ureq (blocking https), sha2/hex (checksum), std::process
//! (extract), cut-media::toolpath (install dir), jobs.rs (progress), doctor.rs
//! (re-scan). Primary caller: dispatch.rs (system.fetch_tool).

use cut_core::{error_codes, CutError};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Env override for the registry base URL — TEST/operator seam only (see header
/// TESTABILITY). Never a verb arg.
pub const ENV_FETCH_BASE_URL: &str = "SHELLX_CUT_FETCH_BASE_URL";

/// The pinned production base: the BtbN FFmpeg-Builds "latest" release. Every
/// registry asset is `<BASE>/<asset>`; the checksum surface is the release-wide
/// `<BASE>/checksums.sha256` file rather than per-asset checksum files.
const BTBN_BASE: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest";

/// Pinned uv release base (keep the version in sync with UV_VERSION below).
const UV_BASE: &str = "https://github.com/astral-sh/uv/releases/download/0.11.21";

/// How a tool's checksum is published — the two release conventions we support.
#[derive(Clone, Copy)]
enum ChecksumSource {
    /// One manifest file under the base listing every asset (BtbN
    /// `checksums.sha256`, GNU `sha256sum` lines). The &str is its file name.
    Manifest(&'static str),
    /// A sibling `<asset>.sha256` next to each asset (astral-sh/uv convention).
    PerAsset,
}

/// One built-in tool the user may consent to download. NO field is caller-
/// supplied; the caller only names the `id`. The download host is PINNED per
/// tool here (never caller-influenced) — this struct IS the allow-list.
struct ToolSpec {
    /// Registry id (the verb's `tool` arg must equal this).
    id: &'static str,
    /// Pinned release base URL; every asset is `<base>/<asset>`. Overridable
    /// ONLY by the test/operator env seam (base_url), never a verb arg.
    base: &'static str,
    /// Archive file name for THIS os/arch under the base URL.
    asset: &'static str,
    /// Where this tool's sha256 lives (manifest vs per-asset sibling).
    checksum: ChecksumSource,
    /// Archive kind, picks the extractor: "tar.xz" | "tar.gz" (unix) | "zip".
    kind: &'static str,
    /// The executables this archive provides (so we can locate them after
    /// extraction and report a version). Stems; platform ext added at use.
    binaries: &'static [&'static str],
}

// uv (astral-sh/uv) is the standalone Python provisioner used by
// `system.setup_perception` to build the sidecar venv on a modern CPython
// (system python is too old on real desktops: macOS ships 3.9, onnx-asr needs
// ≥3.10). The version is pinned in UV_BASE; bump deliberately — the per-asset
// `.sha256` is verified before use.

/// Resolve the built-in spec for `(tool, os, arch)`. Returns None for an unknown
/// tool id OR an unsupported platform (the verb turns that into an honest
/// error). This is the ENTIRE allow-list — there is no other way to reach a URL.
fn tool_spec(tool: &str, os: &str, arch: &str) -> Option<ToolSpec> {
    match (tool, os, arch) {
        // ── ffmpeg (BtbN, GPL static; linux64 + win64 only) ──────────────────
        ("ffmpeg", "linux", "x86_64") => Some(ToolSpec {
            id: "ffmpeg",
            base: BTBN_BASE,
            asset: "ffmpeg-master-latest-linux64-gpl.tar.xz",
            checksum: ChecksumSource::Manifest("checksums.sha256"),
            kind: "tar.xz",
            binaries: &["ffmpeg", "ffprobe"],
        }),
        ("ffmpeg", "windows", "x86_64") => Some(ToolSpec {
            id: "ffmpeg",
            base: BTBN_BASE,
            asset: "ffmpeg-master-latest-win64-gpl.zip",
            checksum: ChecksumSource::Manifest("checksums.sha256"),
            kind: "zip",
            binaries: &["ffmpeg", "ffprobe"],
        }),
        // macOS ffmpeg: BtbN ships no mac build — follow-on (evermeet/martin-riedl).
        // ── uv (astral-sh/uv; the perception-venv provisioner) ───────────────
        ("uv", os, arch) => {
            let asset = match (os, arch) {
                ("windows", "x86_64") => "uv-x86_64-pc-windows-msvc.zip",
                ("windows", "aarch64") => "uv-aarch64-pc-windows-msvc.zip",
                ("linux", "x86_64") => "uv-x86_64-unknown-linux-gnu.tar.gz",
                ("linux", "aarch64") => "uv-aarch64-unknown-linux-gnu.tar.gz",
                ("macos", "aarch64") => "uv-aarch64-apple-darwin.tar.gz",
                ("macos", "x86_64") => "uv-x86_64-apple-darwin.tar.gz",
                _ => return None,
            };
            let kind = if os == "windows" { "zip" } else { "tar.gz" };
            Some(ToolSpec {
                id: "uv",
                // Leaked &'static via Box: the version is a const but the URL is
                // composed; the registry stays a compile-time allow-list of hosts.
                base: UV_BASE,
                asset,
                checksum: ChecksumSource::PerAsset,
                kind,
                binaries: &["uv"],
            })
        }
        _ => None,
    }
}

/// The base URL in effect for a tool: the test/operator override if set, else
/// the tool's pinned `spec.base`. The override changes only the host, never the
/// "no caller-supplied URL" property (it is operator/test config, not a verb arg).
fn base_url(spec_base: &str) -> String {
    match std::env::var(ENV_FETCH_BASE_URL) {
        Ok(v) if !v.is_empty() => v.trim_end_matches('/').to_string(),
        _ => spec_base.trim_end_matches('/').to_string(),
    }
}

/// Outcome of a successful install — becomes the job result + drives the
/// doctor re-scan.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InstallOutcome {
    pub tool: String,
    pub installed_dir: String,
    pub version: Option<String>,
    pub sha256: String,
    pub source_url: String,
    pub bytes: u64,
}

/// Progress callback: (fraction 0..1, human message). The job verb wires this
/// to JobManager::progress so the status bar + WS see download → verify →
/// install. Kept as a plain Fn so this module has no dependency on jobs.rs.
pub type ProgressFn<'a> = dyn Fn(f32, &str) + Send + Sync + 'a;

/// Download, verify, and install a built-in tool. BLOCKING — call from a
/// spawn_blocking task (it does sync HTTPS I/O + process spawns). Every error
/// is actionable. On success the tool is in the app-data tools dir the resolver
/// checks; the caller re-scans the doctor.
///
/// `tool` is the verb's (registry-validated) id. `progress` reports stages.
pub fn install_tool(tool: &str, progress: &ProgressFn) -> Result<InstallOutcome, CutError> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let spec = tool_spec(tool, os, arch).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("no built-in download for tool '{tool}' on {os}/{arch}"),
            "the fetch registry is an allow-list (ffmpeg on linux/windows x86_64; uv on win/linux/mac); there is deliberately no caller-supplied URL",
        )
        .with_suggested_action("on macOS run `brew install ffmpeg-full`; Cut detects the keg-only path after restart. Otherwise check the tool id")
    })?;

    let base = base_url(spec.base);
    let asset_url = format!("{base}/{}", spec.asset);

    // The app-data tools dir (resolver rung 3) and the per-tool subdir.
    let tools_dir = cut_media::toolpath::appdata_tools_dir().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "no app-data tools directory available (HOME/LOCALAPPDATA unset)",
            "cannot determine where to install the tool",
        )
    })?;
    let install_dir = tools_dir.join(spec.id);

    // ---- stage in a temp dir on the SAME volume (atomic move target) --------
    std::fs::create_dir_all(&tools_dir).map_err(io_err("create tools dir"))?;
    let staging = tools_dir.join(format!(".staging-{}-{}", spec.id, std::process::id()));
    // Clean any leftover from a previous crashed attempt.
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(io_err("create staging dir"))?;
    // RAII-ish cleanup: ensure the staging dir is removed on every exit path.
    let _guard = StagingGuard(staging.clone());

    // ---- 1. fetch the expected sha256 (manifest line, or per-asset sibling) -
    progress(0.02, "fetching checksum");
    let expected = match spec.checksum {
        ChecksumSource::Manifest(name) => {
            let manifest = http_get_string(&format!("{base}/{name}"))?;
            checksum_for(&manifest, spec.asset)
        }
        ChecksumSource::PerAsset => {
            // `<asset>.sha256` holds a single GNU `sha256sum` line for the asset.
            let text = http_get_string(&format!("{asset_url}.sha256"))?;
            checksum_for(&text, spec.asset).or_else(|| {
                // Some per-asset files are just the bare 64-hex digest.
                let first = text.split_whitespace().next().unwrap_or("");
                is_hex_sha256(first).then(|| first.to_lowercase())
            })
        }
    };
    let expected = expected.ok_or_else(|| {
        CutError::new(
            error_codes::JOB_FAILED,
            format!("no sha256 found for asset '{}'", spec.asset),
            "the pinned release does not list our asset — the upstream layout may have changed",
        )
        .with_suggested_action("report this; do not bypass checksum verification")
    })?;

    // ---- 2. stream-download the archive while hashing -----------------------
    progress(0.05, "downloading");
    let archive_path = staging.join(spec.asset);
    let (bytes, got) = download_hashing(&asset_url, &archive_path, &|frac, msg| {
        // Map download to the 0.05..0.80 band of overall progress.
        progress(0.05 + frac * 0.75, msg);
    })?;

    // ---- 3. VERIFY before doing anything with the bytes ---------------------
    progress(0.82, "verifying sha256");
    if !got.eq_ignore_ascii_case(&expected) {
        return Err(CutError::new(
            error_codes::JOB_FAILED,
            "sha256 mismatch — the download is NOT the pinned artifact",
            format!("expected {expected}, got {got}"),
        )
        .with_suggested_action(
            "nothing was installed; the file was discarded. Re-run; if it persists the release or the network is compromised — do not install",
        ));
    }

    // ---- 4. extract (verified bytes) into the staging dir -------------------
    progress(0.85, "extracting");
    let extract_root = staging.join("x");
    std::fs::create_dir_all(&extract_root).map_err(io_err("create extract dir"))?;
    extract(spec.kind, &archive_path, &extract_root)?;

    // BtbN archives extract to a single top-level `ffmpeg-.../bin/` dir; find
    // the dir that actually contains the binaries.
    let bin_src = find_bin_dir(&extract_root, spec.binaries).ok_or_else(|| {
        CutError::new(
            error_codes::JOB_FAILED,
            "extracted archive did not contain the expected binaries",
            format!(
                "looked for {:?} under {}",
                spec.binaries,
                extract_root.display()
            ),
        )
    })?;

    // ---- 5. atomic install: swap the verified bin dir into place ------------
    progress(0.92, "installing");
    install_atomic(&bin_src, &install_dir, spec.binaries)?;

    // ---- 6. report version from the freshly-installed binary ----------------
    progress(0.97, "probing version");
    let primary = install_dir.join(exe_name(spec.binaries[0]));
    let version = probe_version(&primary);

    progress(1.0, "done");
    Ok(InstallOutcome {
        tool: spec.id.to_string(),
        installed_dir: install_dir.display().to_string(),
        version,
        sha256: got,
        source_url: asset_url,
        bytes,
    })
}

// ---------------------------------------------------------------------------
// HTTP (ureq, blocking)
// ---------------------------------------------------------------------------

/// GET a small text resource (the checksum manifest). Errors are actionable.
fn http_get_string(url: &str) -> Result<String, CutError> {
    require_https(url)?;
    let resp = ureq::get(url)
        .call()
        .map_err(|e| net_err("fetch checksum manifest", url, e))?;
    let body = resp
        .into_body()
        .read_to_string()
        .map_err(|e| net_err("read checksum manifest", url, e))?;
    Ok(body)
}

/// Stream-download `url` to `dest`, computing sha256 as bytes arrive (never
/// buffering the whole archive in memory). Reports download fraction when the
/// server sends a Content-Length. Returns (byte_count, hex_sha256).
fn download_hashing(
    url: &str,
    dest: &Path,
    progress: &dyn Fn(f32, &str),
) -> Result<(u64, String), CutError> {
    require_https(url)?;
    let resp = ureq::get(url)
        .call()
        .map_err(|e| net_err("download", url, e))?;
    let total: Option<u64> = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    let mut reader = resp.into_body().into_reader();
    let mut file = std::fs::File::create(dest).map_err(io_err("create archive file"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut written: u64 = 0;
    let mut last_report = 0.0f32;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(io_err("read download stream"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        std::io::Write::write_all(&mut file, &buf[..n]).map_err(io_err("write archive file"))?;
        written += n as u64;
        if let Some(t) = total {
            if t > 0 {
                let frac = (written as f32 / t as f32).clamp(0.0, 1.0);
                // Throttle progress callbacks to ~1% steps.
                if frac - last_report >= 0.01 {
                    last_report = frac;
                    progress(
                        frac,
                        &format!("downloading {}/{}", human(written), human(t)),
                    );
                }
            }
        }
    }
    std::io::Write::flush(&mut file).map_err(io_err("flush archive file"))?;
    let hex = hex::encode(hasher.finalize());
    Ok((written, hex))
}

/// Reject any non-https URL up front (the base override could in theory be
/// http for a LOCAL fixture test — we allow loopback http explicitly so the
/// live-proof fixture works, but never plain http to a remote host).
fn require_https(url: &str) -> Result<(), CutError> {
    if url.starts_with("https://") {
        return Ok(());
    }
    // Allow http ONLY to an EXACT loopback host (the test fixture server). The
    // host is PARSED, not prefix-matched (S1): `http://127.0.0.1.evil.com` and
    // `http://localhost.evil.example` start with the old prefixes but are remote
    // — authority_is_loopback rejects them by parsing the host as an IP.
    if let Some(rest) = url.strip_prefix("http://") {
        if crate::http::authority_is_loopback(rest) {
            return Ok(());
        }
    }
    Err(CutError::new(
        error_codes::INVALID_ARGS,
        "refusing a non-https download URL",
        format!("url must be https (or http to loopback for tests): {url}"),
    ))
}

// ---------------------------------------------------------------------------
// Checksum manifest parsing
// ---------------------------------------------------------------------------

/// Find the hex sha256 for `asset` in a `checksums.sha256` manifest. The BtbN
/// format is GNU coreutils `sha256sum` output: `<hex>  <filename>` per line
/// (two spaces; the name may or may not have a leading `*`). We match on the
/// basename so a manifest that lists `ffmpeg-...tar.xz` matches our asset.
fn checksum_for(manifest: &str, asset: &str) -> Option<String> {
    for line in manifest.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Split into hash + the rest (filename, possibly with a `*` binary mark).
        let mut it = line.splitn(2, char::is_whitespace);
        let hash = it.next()?.trim();
        let name = it.next()?.trim().trim_start_matches('*').trim();
        if !is_hex_sha256(hash) {
            continue;
        }
        // Match the asset by basename (manifest may path-qualify the name).
        let name_base = Path::new(name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(name);
        if name_base == asset {
            return Some(hash.to_lowercase());
        }
    }
    None
}

/// True for a 64-char lowercase/uppercase hex string (a sha256 digest).
fn is_hex_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

// ---------------------------------------------------------------------------
// Extraction (verified bytes → temp dir, via the platform archive tool)
// ---------------------------------------------------------------------------

/// Extract `archive` (already sha256-verified) into `dest`. Uses the platform
/// archive tool — we never parse untrusted archive bytes in-process. The bytes
/// are trusted at this point (checksum passed); extraction is contained to the
/// temp `dest`.
fn extract(kind: &str, archive: &Path, dest: &Path) -> Result<(), CutError> {
    match kind {
        // tar handles .tar.xz via -J (xz) or -a (auto); GNU + bsdtar both do.
        "tar.xz" => run_extract(
            "tar",
            &[
                "-xJf".as_ref(),
                archive.as_os_str(),
                "-C".as_ref(),
                dest.as_os_str(),
            ],
            archive,
        ),
        // .tar.gz via -z (gzip); used by the uv release archives.
        "tar.gz" => run_extract(
            "tar",
            &[
                "-xzf".as_ref(),
                archive.as_os_str(),
                "-C".as_ref(),
                dest.as_os_str(),
            ],
            archive,
        ),
        // Windows: tar.exe (bsdtar, present Win10 1803+) extracts zips too;
        // fall back to PowerShell Expand-Archive if tar is missing.
        "zip" => extract_zip(archive, dest),
        other => Err(CutError::new(
            error_codes::JOB_FAILED,
            format!("unknown archive kind '{other}'"),
            "the fetch registry and the extractor disagree — a code bug",
        )),
    }
}

/// Zip extraction with a tar.exe-then-Expand-Archive fallback (Windows).
fn extract_zip(archive: &Path, dest: &Path) -> Result<(), CutError> {
    // Try bsdtar first (handles zip; present on Win10 1803+, mac, many linux).
    let mut tar = std::process::Command::new("tar");
    tar.arg("-xf").arg(archive).arg("-C").arg(dest);
    let tar_try = crate::dispatch::run_bounded_foreground_command(&mut tar, "archive tar");
    if matches!(tar_try, Ok(output) if output.status.success()) {
        return Ok(());
    }
    let mut powershell = std::process::Command::new("powershell");
    powershell.args(powershell_expand_archive_args(archive, dest));
    let status = crate::dispatch::run_bounded_foreground_command(
        &mut powershell,
        "PowerShell archive extraction",
    )
    .map_err(|e| {
        CutError::new(
            error_codes::JOB_FAILED,
            "could not extract the downloaded zip",
            format!("neither tar nor PowerShell Expand-Archive ran: {e}"),
        )
    })?;
    if status.status.success() {
        Ok(())
    } else {
        Err(CutError::new(
            error_codes::JOB_FAILED,
            "zip extraction failed",
            format!("Expand-Archive exited {}", status.status),
        ))
    }
}

fn powershell_expand_archive_args(archive: &Path, dest: &Path) -> Vec<OsString> {
    vec![
        OsString::from("-NoProfile"),
        OsString::from("-NonInteractive"),
        OsString::from("-Command"),
        OsString::from("Expand-Archive -LiteralPath $args[0] -DestinationPath $args[1] -Force"),
        archive.as_os_str().to_owned(),
        dest.as_os_str().to_owned(),
    ]
}

/// Run an extraction command, mapping failure to an actionable error.
fn run_extract(prog: &str, args: &[&std::ffi::OsStr], archive: &Path) -> Result<(), CutError> {
    let mut command = std::process::Command::new(prog);
    command.args(args);
    let status =
        crate::dispatch::run_bounded_foreground_command(&mut command, "archive extraction")
            .map_err(|e| {
                CutError::new(
                    error_codes::JOB_FAILED,
                    format!("could not run '{prog}' to extract the archive"),
                    format!("{e} (archive: {})", archive.display()),
                )
                .with_suggested_action("ensure the platform archive tool (tar) is installed")
            })?;
    if status.status.success() {
        Ok(())
    } else {
        Err(CutError::new(
            error_codes::JOB_FAILED,
            "archive extraction failed",
            format!("'{prog}' exited {} on {}", status.status, archive.display()),
        ))
    }
}

/// Find the directory under `root` that contains all of `binaries` (BtbN nests
/// them in `ffmpeg-.../bin/`). Bounded BFS so a deep archive can't loop us.
fn find_bin_dir(root: &Path, binaries: &[&str]) -> Option<PathBuf> {
    let wanted: Vec<String> = binaries.iter().map(|b| exe_name(b)).collect();
    let mut queue = vec![root.to_path_buf()];
    let mut visited = 0usize;
    while let Some(dir) = queue.pop() {
        visited += 1;
        if visited > 10_000 {
            break; // safety bound
        }
        let has_all = wanted.iter().all(|w| dir.join(w).is_file());
        if has_all {
            return Some(dir);
        }
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.path().is_dir() {
                    queue.push(e.path());
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Atomic install
// ---------------------------------------------------------------------------

/// Move the verified bin dir into `install_dir`, atomically where the
/// filesystem allows. We install into `<tools>/ffmpeg/bin` (matching the
/// resolver's accepted `ffmpeg/` and `ffmpeg/bin/` shapes). An existing install
/// is replaced via a swap so a re-fetch is safe.
///
/// SECURITY: the cross-volume fallback copies ONLY the explicitly-named
/// `binaries` (an allow-list of leaf filenames built in code — never derived
/// from a directory walk of untrusted/extracted names). Same-volume installs
/// use one atomic rename of the whole already-verified, temp-contained dir.
fn install_atomic(bin_src: &Path, install_dir: &Path, binaries: &[&str]) -> Result<(), CutError> {
    let final_bin = install_dir.join("bin");
    std::fs::create_dir_all(install_dir).map_err(io_err("create install dir"))?;
    // Remove any prior bin dir (re-fetch / upgrade) — best effort.
    let _ = std::fs::remove_dir_all(&final_bin);
    // Same-volume rename is atomic; cross-volume falls back to a named copy.
    match std::fs::rename(bin_src, &final_bin) {
        Ok(()) => Ok(()),
        Err(_) => copy_named_binaries(bin_src, &final_bin, binaries),
    }
}

/// Cross-volume install fallback: copy ONLY the named binaries (allow-list of
/// fixed leaf filenames — `ffmpeg[.exe]`, `ffprobe[.exe]`) from the verified
/// `src` bin dir into `dst`. No directory walk: the set of files is determined
/// entirely by our own `binaries` constant, so nothing from the archive's
/// directory listing can influence which paths are written. Preserves the
/// executable bit on unix.
fn copy_named_binaries(src: &Path, dst: &Path, binaries: &[&str]) -> Result<(), CutError> {
    std::fs::create_dir_all(dst).map_err(io_err("create install bin dir"))?;
    for stem in binaries {
        let name = exe_name(stem); // fixed, code-derived leaf name
        let from = src.join(&name);
        let to = dst.join(&name);
        std::fs::copy(&from, &to).map_err(io_err("copy binary into install dir"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // ffmpeg/ffprobe must be executable.
            let _ = std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o755));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

/// Platform exe name for a stem.
fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// Best-effort `<bin> -version` first line (post-install confirmation).
fn probe_version(bin: &Path) -> Option<String> {
    let mut command = std::process::Command::new(bin);
    command.arg("-version");
    let out =
        crate::dispatch::run_bounded_foreground_command(&mut command, "tool version probe").ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .next()
        .map(|l| l.trim().replace("ffmpeg version ", ""))
}

/// IO error → actionable CutError, with the operation named.
fn io_err(op: &'static str) -> impl Fn(std::io::Error) -> CutError {
    move |e| CutError::new(error_codes::IO, format!("{op} failed"), e.to_string())
}

/// Network/ureq error → actionable CutError.
fn net_err(op: &str, url: &str, e: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::JOB_FAILED,
        format!("{op} failed"),
        format!("{e} (url: {url})"),
    )
    .with_suggested_action("check your network connection and retry; nothing was installed")
}

/// Human byte count (decimal SI-ish, for progress messages).
fn human(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n}B")
    } else {
        format!("{v:.1}{}", U[i])
    }
}

/// Removes the staging dir on drop (every exit path), so a failed/aborted fetch
/// never leaves a partial download behind.
struct StagingGuard(PathBuf);
impl Drop for StagingGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_an_allowlist() {
        // Known tool + supported platform resolves.
        assert!(tool_spec("ffmpeg", "linux", "x86_64").is_some());
        assert!(tool_spec("ffmpeg", "windows", "x86_64").is_some());
        // uv resolves across the supported os/arch matrix.
        assert!(tool_spec("uv", "windows", "x86_64").is_some());
        assert!(tool_spec("uv", "linux", "x86_64").is_some());
        assert!(tool_spec("uv", "macos", "aarch64").is_some());
        assert!(tool_spec("uv", "macos", "x86_64").is_some());
        // Unknown tool id → None (no URL reachable).
        assert!(tool_spec("curl", "linux", "x86_64").is_none());
        assert!(tool_spec("../../etc/passwd", "linux", "x86_64").is_none());
        // Unsupported platform → None.
        assert!(tool_spec("ffmpeg", "macos", "aarch64").is_none());
        assert!(tool_spec("uv", "freebsd", "x86_64").is_none());
    }

    #[test]
    fn parses_sha256sum_manifest() {
        let h = "a".repeat(64);
        let manifest = format!(
            "# header\n{h}  ffmpeg-master-latest-linux64-gpl.tar.xz\n\
             {b}  some-other-asset.zip\n",
            b = "b".repeat(64)
        );
        let got = checksum_for(&manifest, "ffmpeg-master-latest-linux64-gpl.tar.xz");
        assert_eq!(got.as_deref(), Some(h.as_str()));
        // Asset not present → None (we never guess).
        assert!(checksum_for(&manifest, "nope.tar.xz").is_none());
    }

    #[test]
    fn checksum_matches_basename_when_path_qualified() {
        let h = "c".repeat(64);
        let manifest = format!("{h} *bin/ffmpeg-master-latest-win64-gpl.zip\n");
        assert_eq!(
            checksum_for(&manifest, "ffmpeg-master-latest-win64-gpl.zip").as_deref(),
            Some(h.as_str())
        );
    }

    #[test]
    fn require_https_rejects_remote_http() {
        assert!(require_https("https://github.com/x").is_ok());
        assert!(require_https("http://127.0.0.1:9000/x").is_ok()); // loopback fixture
        assert!(require_https("http://localhost:9000/x").is_ok());
        assert!(require_https("http://[::1]:9000/x").is_ok());
        assert!(require_https("http://evil.example.com/x").is_err());
        assert!(require_https("ftp://x").is_err());
        // S1: prefix-lookalike hosts that the old starts_with() check accepted
        // must now be REJECTED (parsed host, not a substring).
        assert!(require_https("http://127.0.0.1.evil.com/x").is_err());
        assert!(require_https("http://localhost.evil.example/x").is_err());
        assert!(require_https("http://127.0.0.1evil.com/x").is_err());
    }

    #[test]
    fn is_hex_sha256_validates() {
        assert!(is_hex_sha256(&"0".repeat(64)));
        // A 64-char mixed-case hex string.
        let mixed: String = "aBcDeF0123456789".chars().cycle().take(64).collect();
        assert_eq!(mixed.len(), 64);
        assert!(is_hex_sha256(&mixed));
        assert!(!is_hex_sha256(&"0".repeat(63))); // wrong length
        assert!(!is_hex_sha256(&"g".repeat(64))); // non-hex char
        assert!(!is_hex_sha256("xyz"));
    }

    #[test]
    fn powershell_expand_archive_args_do_not_interpolate_paths() {
        let archive = Path::new(r"C:\tmp\bad'; Remove-Item C:\x; '.zip");
        let dest = Path::new(r"C:\tmp\out'; Write-Host nope; '");
        let args = powershell_expand_archive_args(archive, dest);
        let script = args[3].to_string_lossy();

        assert!(script.contains("$args[0]"));
        assert!(script.contains("$args[1]"));
        assert!(!script.contains("Remove-Item"));
        assert!(!script.contains("Write-Host"));
        assert_eq!(args[4], archive.as_os_str());
        assert_eq!(args[5], dest.as_os_str());
    }
}
