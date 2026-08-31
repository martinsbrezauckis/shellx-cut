//! Reviewed optional-perception lock selection and validation.
//!
//! The base transcription requirements stay separate from this module. Optional
//! instruments may only install from an exact, hash-checked lock for a target
//! Cut packages and tests from source; they must never resolve the unpinned
//! `requirements-full.txt` input on a user's machine.

use cut_core::{error_codes, CutError};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub(super) const PERCEPTION_LOCK_SCHEMA: &str = "shellx-cut/perception-lock@1";

/// CPU-only PyTorch wheel index for the optional extras. This avoids a CUDA
/// dependency graph on the ordinary desktop machines this best-effort phase
/// supports.
const TORCH_CPU_INDEX: &str = "https://download.pytorch.org/whl/cpu";

/// A lock file selected only for a target Cut actually packages today. This is
/// intentionally narrower than the `uv` bootstrap matrix: a `uv` binary does
/// not prove that the full optional wheel graph was reviewed for that target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PerceptionLockSpec {
    pub(super) platform: &'static str,
    pub(super) os: &'static str,
    pub(super) arch: &'static str,
    pub(super) file: &'static str,
}

pub(super) const PERCEPTION_LOCKS: &[PerceptionLockSpec] = &[
    PerceptionLockSpec {
        platform: "linux-x86_64",
        os: "linux",
        arch: "x86_64",
        file: "requirements-full.linux-x86_64.lock",
    },
    PerceptionLockSpec {
        platform: "windows-x86_64",
        os: "windows",
        arch: "x86_64",
        file: "requirements-full.windows-x86_64.lock",
    },
    PerceptionLockSpec {
        platform: "macos-aarch64",
        os: "macos",
        arch: "aarch64",
        file: "requirements-full.macos-aarch64.lock",
    },
];

/// User-facing optional-instrument inputs that every reviewed lock must carry,
/// in addition to its complete transitive graph.
pub(super) const REQUIRED_OPTIONAL_PERCEPTION_PACKAGES: &[&str] = &[
    "numpy",
    "whisperx",
    "torchaudio",
    "silero-vad",
    "soundfile",
    "transformers",
    "sentencepiece",
    "scenedetect",
    "torchvision",
    "supervision",
    "mediapipe",
    "rapidocr-onnxruntime",
];

/// Build the only permitted `uv pip install` arguments for optional extras.
pub(super) fn install_args<'a>(venv_python: &'a str, lock: &'a Path) -> Vec<&'a str> {
    vec![
        "pip",
        "install",
        "--python",
        venv_python,
        "--only-binary",
        ":all:",
        "--require-hashes",
        "--extra-index-url",
        TORCH_CPU_INDEX,
        "--index-strategy",
        "unsafe-best-match",
        "-r",
        lock.to_str().unwrap_or_default(),
    ]
}

/// Resolve and validate the optional perception lock for this exact runtime.
///
/// The source input is never a fallback: selecting an unreviewed platform or a
/// malformed packaged lock yields an honest base-only setup outcome instead of
/// silently resolving a new graph from the network.
pub(super) fn resolve(sidecar_dir: &Path, python_version: &str) -> Result<PathBuf, CutError> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let spec = perception_lock_spec(os, arch).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("no optional perception lock is packaged for {os}/{arch}"),
            "this target is not one of the reviewed ShellX Cut perception platform sets",
        )
        .with_suggested_action(
            "transcription remains available. Do not install the unpinned optional requirements manually; use a reviewed ShellX Cut build for this platform",
        )
    })?;
    let path = sidecar_dir.join(spec.file);
    let contents = std::fs::read_to_string(&path).map_err(super::io_err("read perception lock"))?;
    validate_contents(&contents, spec, python_version).map_err(|cause| {
        CutError::new(
            error_codes::IO,
            "packaged optional perception lock is invalid",
            format!("{}: {cause}", path.display()),
        )
        .with_suggested_action("reinstall this ShellX Cut build; do not bypass the reviewed lock")
    })?;
    Ok(path)
}

pub(super) fn perception_lock_spec(os: &str, arch: &str) -> Option<PerceptionLockSpec> {
    PERCEPTION_LOCKS
        .iter()
        .copied()
        .find(|spec| spec.os == os && spec.arch == arch)
}

/// Validate a lock manifest before giving it to `uv --require-hashes`.
pub(super) fn validate_contents(
    contents: &str,
    spec: PerceptionLockSpec,
    python_version: &str,
) -> Result<(), String> {
    let expected_header = format!(
        "# {PERCEPTION_LOCK_SCHEMA}\n# python={python_version}\n# platform={}",
        spec.platform
    );
    if !contents.starts_with(&expected_header) {
        return Err(format!(
            "expected manifest header for Python {python_version} on {}",
            spec.platform
        ));
    }

    let mut names = BTreeSet::new();
    let mut current_requirement: Option<String> = None;
    let mut requirements_with_hashes = BTreeSet::new();
    for (index, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(hash) = line.strip_prefix("--hash=sha256:") {
            let Some(requirement) = current_requirement.as_ref() else {
                return Err(format!("line {} hashes no requirement", index + 1));
            };
            let hash = hash.trim_end_matches('\\').trim();
            if !is_sha256_hex(hash) {
                return Err(format!("line {} has an invalid SHA-256 hash", index + 1));
            }
            requirements_with_hashes.insert(requirement.clone());
            continue;
        }
        let Some((name, version_with_continuation)) = line.split_once("==") else {
            return Err(format!("line {} is not exact-version locked", index + 1));
        };
        let version = version_with_continuation.trim_end_matches('\\').trim();
        if name.is_empty()
            || version.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || version.chars().any(char::is_whitespace)
        {
            return Err(format!(
                "line {} has an invalid locked requirement",
                index + 1
            ));
        }
        let name = name.to_ascii_lowercase();
        names.insert(name.clone());
        current_requirement = Some(name);
    }

    if names.is_empty() {
        return Err("lock has no installable requirements".to_string());
    }
    if let Some(requirement) = names.difference(&requirements_with_hashes).next() {
        return Err(format!(
            "locked requirement '{requirement}' has no artifact hash"
        ));
    }
    for package in REQUIRED_OPTIONAL_PERCEPTION_PACKAGES {
        if !names.contains(*package) {
            return Err(format!("missing required optional package '{package}'"));
        }
    }
    Ok(())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
