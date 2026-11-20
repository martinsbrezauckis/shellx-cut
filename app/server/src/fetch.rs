//! fetch.rs — `system.fetch_tool` consented tool download + install.
//!
//! ROLE
//!   The packaging design downloads ffmpeg on first
//!   run, user-consented" — it keeps our installer ~3 MB and our license open
//!   (the GPL libx264 binary lands on the USER's disk by the user's action,
//!   never in our distributed artifact). This module implements the download:
//!   a pinned, verified FFmpeg build fetched from its publisher into the
//!   app-data tools dir — the exact rung-3 directory the cut-media toolpath
//!   resolver checks, so a successful fetch flips the doctor's ffmpeg card
//!   missing → bundled-or-appdata with zero other code change.
//!
//! SECURITY (HARD — this is a network-write primitive; treat it like the output-fencing contract)
//!   1. NO caller-supplied URL. The tool id ("ffmpeg") indexes a BUILT-IN
//!      REGISTRY of https-pinned URLs. A caller can pick WHICH known tool, never
//!      WHERE from. This is the difference between a feature and an arbitrary-
//!      download RCE.
//!   2. https only (redirects included), with deadlines.
//!   3. FFmpeg: every platform's release is pinned in the source (exact URL,
//!      size and SHA-256 of each archive and of each program installed from
//!      it); see `pinned.rs`. uv: the publisher's per-asset checksum is read
//!      first and the archive is verified against it.
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
//!   FFmpeg: cut_media::tool_setup::pinned_release picks the build for
//!   `std::env::consts::OS`/`ARCH` — Martin Riedl's notarized build on Apple
//!   silicon, an FFmpeg-Builds monthly 9.0 build on Windows x64 and Linux
//!   x86_64. Other platforms have no download (`unsupported_platform`).
//!
//! CONSENT
//!   The verb refuses unless the caller passes `accept_license:true` after the
//!   person agreed to the download offer (publisher, version, size, licence,
//!   source) that the doctor's ffmpeg card carries.
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
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

mod durable_install;
mod macos_signature;
mod pinned;
#[cfg(all(test, target_os = "linux"))]
pub(crate) use pinned::use_release_for_tests;
mod program_header;
mod system_programs;
mod unpack_tools;
mod zip_member;

/// Env override for the registry base URL — TEST/operator seam only (see header
/// TESTABILITY). Never a verb arg.
pub const ENV_FETCH_BASE_URL: &str = "SHELLX_CUT_FETCH_BASE_URL";

/// astral-sh's maintained latest-release alias for the uv provisioner.
const UV_BASE: &str = "https://github.com/astral-sh/uv/releases/latest/download";

/// Bound both decoded checksum metadata and staged archive bytes. The archive
/// allowance is well above the current registry assets, including FFmpeg.
const MAX_CHECKSUM_BYTES: u64 = 1024 * 1024;
const MAX_TOOL_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;

/// How a tool's checksum is published.
#[derive(Clone, Copy)]
enum ChecksumSource {
    /// A sibling `<asset>.sha256` next to each asset (astral-sh/uv convention).
    PerAsset,
}

/// One built-in tool the user may consent to download. NO field is caller-
/// supplied; the caller only names the `id`. The download host is PINNED per
/// tool here (never caller-influenced) — this struct IS the allow-list.
struct ToolSpec {
    /// Registry id (the verb's `tool` arg must equal this).
    id: &'static str,
    /// Built-in publisher release base URL; every asset is `<base>/<asset>`.
    /// Overridable ONLY by the test/operator env seam, never a verb argument.
    base: &'static str,
    /// Archive file name for THIS os/arch under the base URL.
    asset: &'static str,
    /// Where this tool's current publisher SHA-256 is published.
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
// ≥3.10). The maintained latest-release alias keeps this provisioner current;
// its publisher-provided per-asset `.sha256` is verified before use.

/// Resolve the built-in spec for `(tool, os, arch)`. Returns None for an unknown
/// tool id OR an unsupported platform (the verb turns that into an honest
/// error). This is the ENTIRE allow-list — there is no other way to reach a URL.
fn tool_spec(tool: &str, os: &str, arch: &str) -> Option<ToolSpec> {
    match (tool, os, arch) {
        // ffmpeg is not in this registry: every platform's FFmpeg release is
        // pinned in cut_media::tool_setup (see pinned.rs).
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

/// The refusal when the registry has no download for `tool` on this platform.
/// The tool id is valid; the platform is the reason, and the action says how
/// to get ffmpeg here instead.
pub(crate) fn no_builtin_download(tool: &str, os: &str, arch: &str) -> CutError {
    let error = CutError::new(
        error_codes::UNSUPPORTED_PLATFORM,
        format!("ShellX Cut has no built-in {tool} download for {os}/{arch}"),
        "built-in downloads exist only where a verified publisher build is registered \
         (ffmpeg: Windows and Linux x86_64, macOS on Apple silicon; uv: Windows, Linux and macOS)",
    );
    if tool == "ffmpeg" {
        let guide = cut_media::tool_setup::ffmpeg_setup_guide_for(os, arch);
        error.with_suggested_action(cut_media::tool_setup::ffmpeg_setup_action_for(&guide))
    } else {
        error
    }
}

/// The refusal when `system.fetch_tool` is called without the person's
/// agreement. The cause carries the exact consent text to show them.
pub(crate) fn consent_required(offer: Option<&cut_media::tool_setup::DownloadOffer>) -> CutError {
    let consent = offer
        .map(|o| o.consent.clone())
        .unwrap_or_else(|| "Download FFmpeg, a separate GPL program, from its publisher.".into());
    CutError::new(
        error_codes::INVALID_ARGS,
        "downloading FFmpeg needs the person's agreement",
        consent,
    )
    .with_suggested_action(
        "Show the person the text in cause. Only when they agree, call system.fetch_tool \
         {\"tool\":\"ffmpeg\",\"accept_license\":true}. Do not agree on their behalf.",
    )
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

/// Waits between attempts after a temporary publisher failure (pinned FFmpeg).
const LATEST_FETCH_DELAYS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(15),
];

struct FetchAttemptError {
    error: CutError,
    transient: bool,
    /// The HTTP status when the server answered with an error status.
    status: Option<u16>,
}

impl FetchAttemptError {
    fn permanent(error: CutError) -> Self {
        Self {
            error,
            transient: false,
            status: None,
        }
    }
    fn network(op: &str, url: &str, error: ureq::Error) -> Self {
        let transient = matches!(
            error,
            ureq::Error::StatusCode(404 | 429 | 500..=599)
                | ureq::Error::Io(_)
                | ureq::Error::Timeout(_)
                | ureq::Error::HostNotFound
                | ureq::Error::ConnectionFailed
                | ureq::Error::Protocol(_)
        );
        let status = match error {
            ureq::Error::StatusCode(code) => Some(code),
            _ => None,
        };
        Self {
            error: net_err(op, url, error),
            transient,
            status,
        }
    }
}

/// Download, verify, and install a built-in tool. BLOCKING — call from a
/// spawn_blocking task (it does sync HTTPS I/O + process spawns). Every error
/// is actionable. On success the tool is in the app-data tools dir the resolver
/// checks; the caller re-scans the doctor.
///
/// `tool` is the verb's (registry-validated) id. `progress` reports stages.
pub fn install_tool(tool: &str, progress: &ProgressFn) -> Result<InstallOutcome, CutError> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    if let Some(release) = pinned::release_for(tool, os, arch) {
        return pinned::install(tool, release, progress);
    }
    let spec = tool_spec(tool, os, arch).ok_or_else(|| no_builtin_download(tool, os, arch))?;

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

    // ---- 1. load the expected sha256 -----------------------------------------
    // Every production and explicit fixture request reads its publisher's
    // matching checksum metadata before archive bytes are accepted.
    let archive_path = staging.join(spec.asset);
    let (bytes, got) = fetch_verified_archive(
        &spec,
        &asset_url,
        &archive_path,
        progress,
        &[],
        std::thread::sleep,
    )?;

    // ---- 4. extract (verified bytes) into the staging dir -------------------
    progress(0.85, "extracting");
    let extract_root = staging.join("x");
    std::fs::create_dir_all(&extract_root).map_err(io_err("create extract dir"))?;
    extract(spec.kind, &archive_path, &extract_root)?;

    // Archives may nest the binaries in a top-level folder; find the dir that
    // actually contains them.
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
    let primary = install_dir.join("bin").join(exe_name(spec.binaries[0]));
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

/// Retry only a temporary failure to fetch the current release. Each attempt
/// starts with a new publisher manifest; no archive is ever trusted across attempts.
fn fetch_verified_archive(
    spec: &ToolSpec,
    asset_url: &str,
    archive_path: &Path,
    progress: &ProgressFn,
    delays: &[Duration],
    pause: impl Fn(Duration),
) -> Result<(u64, String), CutError> {
    let highest_progress = AtomicU32::new(0);
    let report = |fraction: f32, message: &str| {
        let highest = highest_progress.fetch_max(fraction.to_bits(), Ordering::Relaxed);
        progress(f32::from_bits(highest.max(fraction.to_bits())), message);
    };
    for attempt in 0..=delays.len() {
        report(0.02, "loading checksum");
        let result = (|| {
            let expected = expected_sha256(spec, asset_url)?;
            report(0.05, "downloading");
            let (bytes, got) = download_hashing(asset_url, archive_path, &|frac, msg| {
                report(0.05 + frac * 0.75, msg);
            })?;
            report(0.82, "verifying sha256");
            if !got.eq_ignore_ascii_case(&expected) {
                return Err(FetchAttemptError::permanent(CutError::new(
                    error_codes::JOB_FAILED,
                    "sha256 mismatch — download differs from the publisher checksum",
                    format!("expected {expected}, got {got}"),
                ).with_suggested_action(
                    "nothing was installed; the file was discarded. Retry to load the current publisher checksum. If it persists, report it",
                )));
            }
            Ok((bytes, got))
        })();
        match result {
            Ok(verified) => return Ok(verified),
            Err(failure) if failure.transient && attempt < delays.len() => {
                report(0.02, "current release temporarily unavailable; retrying");
                pause(delays[attempt]);
            }
            Err(failure) => return Err(failure.error),
        }
    }
    unreachable!("bounded release attempts always return")
}

// ---------------------------------------------------------------------------
// HTTP (ureq, blocking)
// ---------------------------------------------------------------------------

/// Resolve the checksum accepted for this download.
///
/// The trusted publisher's current release metadata is authoritative for every
/// request, including the local-fixture/operator override seam.
fn expected_sha256(spec: &ToolSpec, asset_url: &str) -> Result<String, FetchAttemptError> {
    let expected = match spec.checksum {
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
    expected.ok_or_else(|| {
        FetchAttemptError::permanent(
            CutError::new(
                error_codes::JOB_FAILED,
                format!("no sha256 found for asset '{}'", spec.asset),
                "the selected registry does not list the requested asset",
            )
            .with_suggested_action("report this; do not bypass checksum verification"),
        )
    })
}

/// GET a small text resource (the checksum manifest). Errors are actionable.
fn http_get_string(url: &str) -> Result<String, FetchAttemptError> {
    http_get_string_with_limit(url, MAX_CHECKSUM_BYTES)
}

fn http_get_string_with_limit(url: &str, limit: u64) -> Result<String, FetchAttemptError> {
    require_https(url).map_err(FetchAttemptError::permanent)?;
    let resp = fetch_agent(url)
        .get(url)
        .call()
        .map_err(|e| FetchAttemptError::network("fetch checksum manifest", url, e))?;
    reject_oversized_length(&resp, limit, "checksum manifest")?;
    let mut bytes = Vec::new();
    resp.into_body()
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| FetchAttemptError {
            error: net_err("read checksum manifest", url, e),
            transient: true,
            status: None,
        })?;
    if bytes.len() as u64 > limit {
        return Err(size_limit_error("checksum manifest", limit));
    }
    String::from_utf8(bytes).map_err(|e| {
        FetchAttemptError::permanent(CutError::new(
            error_codes::JOB_FAILED,
            "checksum manifest is not UTF-8",
            e.to_string(),
        ))
    })
}

/// Stream-download `url` to `dest`, computing sha256 as bytes arrive (never
/// buffering the whole archive in memory). Reports download fraction when the
/// server sends a Content-Length. Returns (byte_count, hex_sha256).
/// Download `url` to `dest` over https (redirects stay on https) with the
/// same deadlines as every tool download, at most `limit` bytes. Returns
/// (bytes, sha256). Used for downloads whose trust is checked another way
/// (Microsoft's signed Visual C++ Redistributable, vc_runtime.rs).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn download_file(
    url: &str,
    dest: &Path,
    limit: u64,
    progress: &dyn Fn(f32, &str),
) -> Result<(u64, String), CutError> {
    download_hashing_with_limit(url, dest, progress, limit).map_err(|failure| failure.error)
}

fn download_hashing(
    url: &str,
    dest: &Path,
    progress: &dyn Fn(f32, &str),
) -> Result<(u64, String), FetchAttemptError> {
    download_hashing_with_limit(url, dest, progress, MAX_TOOL_ARCHIVE_BYTES)
}

fn download_hashing_with_limit(
    url: &str,
    dest: &Path,
    progress: &dyn Fn(f32, &str),
    limit: u64,
) -> Result<(u64, String), FetchAttemptError> {
    require_https(url).map_err(FetchAttemptError::permanent)?;
    let resp = fetch_agent(url)
        .get(url)
        .call()
        .map_err(|e| FetchAttemptError::network("download", url, e))?;
    reject_oversized_length(&resp, limit, "tool archive")?;
    let total: Option<u64> = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    let mut reader = resp.into_body().into_reader();
    let mut file = std::fs::File::create(dest)
        .map_err(|e| FetchAttemptError::permanent(io_err("create archive file")(e)))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut written: u64 = 0;
    let mut last_report = 0.0f32;
    loop {
        let n = reader.read(&mut buf).map_err(|e| FetchAttemptError {
            error: net_err("read download stream", url, e),
            transient: true,
            status: None,
        })?;
        if n == 0 {
            break;
        }
        if n as u64 > limit.saturating_sub(written) {
            return Err(size_limit_error("tool archive", limit));
        }
        hasher.update(&buf[..n]);
        std::io::Write::write_all(&mut file, &buf[..n])
            .map_err(|e| FetchAttemptError::permanent(io_err("write archive file")(e)))?;
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
    std::io::Write::flush(&mut file)
        .map_err(|e| FetchAttemptError::permanent(io_err("flush archive file")(e)))?;
    let hex = hex::encode(hasher.finalize());
    Ok((written, hex))
}

/// The HTTP client for every tool download: redirects stay on https (plain
/// http is only ever a loopback test fixture), connects and first bytes are
/// bounded, and one overall deadline stops a server that trickles forever.
fn fetch_agent(url: &str) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .https_only(url.starts_with("https://"))
        .max_redirects(10)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_global(Some(Duration::from_secs(60 * 60)))
        .build();
    ureq::Agent::new_with_config(config)
}

fn reject_oversized_length(
    resp: &ureq::http::Response<ureq::Body>,
    limit: u64,
    label: &str,
) -> Result<(), FetchAttemptError> {
    let length = resp
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if length.is_some_and(|length| length > limit) {
        return Err(size_limit_error(label, limit));
    }
    Ok(())
}

fn size_limit_error(label: &str, limit: u64) -> FetchAttemptError {
    FetchAttemptError::permanent(CutError::new(
        error_codes::JOB_FAILED,
        format!("{label} exceeds the download size limit"),
        format!("maximum accepted size: {}", human(limit)),
    ))
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

/// Find the hex sha256 for `asset` in GNU coreutils `sha256sum` output (a
/// publisher's `<asset>.sha256` file): `<hex>  <filename>` per line
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
    let mut tar = system_programs::tar_command(None);
    tar.arg("-xf").arg(archive).arg("-C").arg(dest);
    let tar_try = crate::dispatch::run_bounded_foreground_command(&mut tar, "archive tar");
    if matches!(tar_try, Ok(output) if output.status.success()) {
        return Ok(());
    }
    let mut powershell = std::process::Command::new(system_programs::windows_powershell());
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

/// Run the system tar (absolute path; its decompressor search sees only
/// absolute PATH folders), mapping failure to an actionable error.
fn run_extract(args: &[&std::ffi::OsStr], archive: &Path) -> Result<(), CutError> {
    let prog = "tar";
    let mut command = system_programs::tar_command(None);
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
    // The programs reach the disk before they get their final names.
    for stem in binaries {
        durable_install::sync_file(&bin_src.join(exe_name(stem)))
            .map_err(io_err("flush verified binary"))?;
    }
    // Remove any prior bin dir (re-fetch / upgrade) — best effort.
    let _ = std::fs::remove_dir_all(&final_bin);
    // Same-volume rename is atomic; cross-volume falls back to a named copy.
    match std::fs::rename(bin_src, &final_bin) {
        Ok(()) => durable_install::sync_dir(install_dir).map_err(io_err("flush install dir")),
        Err(_) => copy_named_binaries(bin_src, &final_bin, binaries),
    }
}

/// Cross-volume install fallback: copy ONLY the named binaries (allow-list of
/// fixed leaf filenames — `ffmpeg[.exe]`, `ffprobe[.exe]`) from the verified
/// `src` bin dir into `dst`. No directory walk: the set of files is determined
/// entirely by our own `binaries` constant, so nothing from the archive's
/// directory listing can influence which paths are written. Each program is
/// copied under a temporary name, made executable, flushed and closed, then
/// renamed, so no half-copied program is ever under its final name.
fn copy_named_binaries(src: &Path, dst: &Path, binaries: &[&str]) -> Result<(), CutError> {
    std::fs::create_dir_all(dst).map_err(io_err("create install bin dir"))?;
    for stem in binaries {
        let name = exe_name(stem); // fixed, code-derived leaf name
        durable_install::copy_into_place(&src.join(&name), &dst.join(&name))
            .map_err(io_err("copy binary into install dir"))?;
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
#[path = "fetch_latest_tests.rs"]
mod fetch_latest_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_an_allowlist() {
        // ffmpeg is never in this floating registry: every platform pins it.
        for (os, arch) in [
            ("linux", "x86_64"),
            ("windows", "x86_64"),
            ("macos", "aarch64"),
        ] {
            assert!(tool_spec("ffmpeg", os, arch).is_none(), "{os}");
            assert!(cut_media::tool_setup::pinned_release("ffmpeg", os, arch).is_some());
        }
        // uv resolves across the supported os/arch matrix.
        assert!(tool_spec("uv", "windows", "x86_64").is_some());
        assert!(tool_spec("uv", "linux", "x86_64").is_some());
        assert!(tool_spec("uv", "macos", "aarch64").is_some());
        assert!(tool_spec("uv", "macos", "x86_64").is_some());
        // Unknown tool id → None (no URL reachable).
        assert!(tool_spec("curl", "linux", "x86_64").is_none());
        assert!(tool_spec("../../etc/passwd", "linux", "x86_64").is_none());
        assert!(tool_spec("uv", "freebsd", "x86_64").is_none());
    }

    /// The setup guidance shown to people and agents must agree with this
    /// registry about where a built-in FFmpeg download exists.
    #[test]
    fn setup_guidance_matches_the_ffmpeg_registry() {
        for os in ["linux", "windows", "macos", "freebsd"] {
            for arch in ["x86_64", "aarch64"] {
                assert_eq!(
                    tool_spec("ffmpeg", os, arch).is_some()
                        || cut_media::tool_setup::pinned_release("ffmpeg", os, arch).is_some(),
                    cut_media::tool_setup::builtin_ffmpeg_download_for(os, arch),
                    "{os}/{arch}"
                );
            }
        }
    }

    #[test]
    fn a_platform_without_a_download_is_refused_with_the_way_forward() {
        let error = no_builtin_download("ffmpeg", "linux", "aarch64");
        assert_eq!(error.code, error_codes::UNSUPPORTED_PLATFORM);
        assert!(error.message.contains("linux/aarch64"));
        let action = error.suggested_action.expect("an action");
        assert!(!action.contains("check the tool id"));
        assert!(action.contains("system.set_ffmpeg"));
        // Linux arm64: the distribution's packages, never a download.
        for needle in ["sudo apt install ffmpeg", "sudo dnf install ffmpeg-free"] {
            assert!(action.contains(needle), "{needle}: {action}");
        }
        assert!(!action.contains("fetch_tool"), "{action}");
        // An arm64 build of Cut for Windows: the publisher's page, never apt.
        let action = no_builtin_download("ffmpeg", "windows", "aarch64")
            .suggested_action
            .expect("an action");
        assert!(
            action.contains(cut_media::tool_setup::FFMPEG_BUILDS_RELEASES),
            "{action}"
        );
        assert!(
            !action.contains("apt") && !action.contains("fetch_tool"),
            "{action}"
        );
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
