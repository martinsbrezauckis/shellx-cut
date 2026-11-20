//! Runner-pinned executable bundle admission for the desktop shell.
//!
//! This module owns generic bundle parsing and the Cut-specific ffmpeg/ffprobe
//! selection. `tools.rs` keeps normal discovery and engine override wiring.

use cut_native_runtime_context::{
    PinnedExecutableBundleContext, RuntimeContextSet, CONTEXT_CONTRACT, CONTEXT_ENV,
    PINNED_EXECUTABLE_BUNDLE_CONTEXT_CONTRACT, SET_CONTEXT_CONTRACT,
};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The desktop's independent observation of the generic Runner executable
/// bundle. Cut selects only its two product-owned names after the shared parser
/// has validated the sealed root, target host, entry metadata, and hashes.
#[derive(Debug, Clone, Default)]
pub enum PinnedExecutableBundleResolution {
    #[default]
    Absent,
    Accepted {
        locator: PathBuf,
        ffmpeg: PathBuf,
        ffprobe: PathBuf,
    },
    Rejected {
        reason: String,
    },
}

impl PinnedExecutableBundleResolution {
    pub(crate) fn error(&self) -> Option<&str> {
        match self {
            Self::Rejected { reason } => Some(reason),
            Self::Absent | Self::Accepted { .. } => None,
        }
    }

    pub(crate) fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Absent => serde_json::json!({"status":"absent"}),
            Self::Accepted {
                locator,
                ffmpeg,
                ffprobe,
            } => serde_json::json!({
                "status":"accepted",
                "locator": locator.display().to_string(),
                "executables": {
                    "ffmpeg": ffmpeg.display().to_string(),
                    "ffprobe": ffprobe.display().to_string(),
                },
            }),
            Self::Rejected { reason } => serde_json::json!({"status":"rejected", "reason":reason}),
        }
    }
}

const MEDIA_TOOLS_RUNTIME_MEMBER_ID: &str = "media-tools";
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const VERSION_PROBE_POLL: Duration = Duration::from_millis(25);

pub(super) fn resolve(
    context_schema: &Result<Option<String>, String>,
    context_set: Option<&Result<Option<RuntimeContextSet>, String>>,
) -> PinnedExecutableBundleResolution {
    match context_schema {
        Ok(None) => return PinnedExecutableBundleResolution::Absent,
        Ok(Some(schema)) if schema == CONTEXT_CONTRACT => {
            return PinnedExecutableBundleResolution::Absent
        }
        Err(reason) => {
            return PinnedExecutableBundleResolution::Rejected {
                reason: reason.clone(),
            };
        }
        Ok(Some(schema)) if schema == SET_CONTEXT_CONTRACT => {
            return match context_set {
                Some(Ok(Some(set))) => {
                    match set.pinned_executable_bundle(MEDIA_TOOLS_RUNTIME_MEMBER_ID) {
                        Ok(context) => accepted_context(context, set.locator()),
                        Err(reason) => PinnedExecutableBundleResolution::Rejected { reason },
                    }
                }
                Some(Ok(None)) => PinnedExecutableBundleResolution::Absent,
                Some(Err(reason)) => PinnedExecutableBundleResolution::Rejected {
                    reason: reason.clone(),
                },
                None => PinnedExecutableBundleResolution::Rejected {
                    reason: "native runtime context set was not loaded".into(),
                },
            };
        }
        Ok(Some(schema)) if schema != PINNED_EXECUTABLE_BUNDLE_CONTEXT_CONTRACT => {
            return PinnedExecutableBundleResolution::Rejected {
                reason: "Runner executable bundle context schema is not supported by ShellX Cut"
                    .into(),
            };
        }
        Ok(Some(_)) => {}
    }

    match PinnedExecutableBundleContext::from_env() {
        Ok(Some(context)) => accepted_context(
            &context,
            std::env::var_os(CONTEXT_ENV).as_deref().map(Path::new),
        ),
        Ok(None) => PinnedExecutableBundleResolution::Absent,
        Err(reason) => PinnedExecutableBundleResolution::Rejected { reason },
    }
}

fn accepted_context(
    context: &PinnedExecutableBundleContext,
    locator: Option<&Path>,
) -> PinnedExecutableBundleResolution {
    let selected = (|| -> Result<(PathBuf, PathBuf), String> {
        let ffmpeg = context
            .executable("ffmpeg")
            .ok_or("pinned executable bundle is missing Cut's ffmpeg entry")?;
        let ffprobe = context
            .executable("ffprobe")
            .ok_or("pinned executable bundle is missing Cut's ffprobe entry")?;
        context.verify_executable(ffmpeg)?;
        context.verify_executable(ffprobe)?;
        if !runnable_program(&ffmpeg.path, "-version") {
            return Err("pinned executable bundle ffmpeg is not runnable".into());
        }
        if !runnable_program(&ffprobe.path, "-version") {
            return Err("pinned executable bundle ffprobe is not runnable".into());
        }
        Ok((ffmpeg.path.clone(), ffprobe.path.clone()))
    })();
    match selected {
        Ok((ffmpeg, ffprobe)) => match locator {
            Some(locator) => PinnedExecutableBundleResolution::Accepted {
                locator: locator.to_path_buf(),
                ffmpeg,
                ffprobe,
            },
            None => PinnedExecutableBundleResolution::Rejected {
                reason: "native runtime context locator disappeared during bundle resolution"
                    .into(),
            },
        },
        Err(reason) => PinnedExecutableBundleResolution::Rejected { reason },
    }
}

fn runnable_program(program: &Path, version_flag: &str) -> bool {
    let mut command = Command::new(program);
    command
        .arg(version_flag)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // ffmpeg/ffprobe are console binaries. Keep the desktop's startup probe
    // silent just like its cutd child on Windows.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .spawn()
        .map(|mut child| probe_exits_successfully(&mut child, VERSION_PROBE_TIMEOUT))
        .unwrap_or(false)
}

fn probe_exits_successfully(child: &mut Child, timeout: Duration) -> bool {
    let started = Instant::now();
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Err(_) => {
                // A failed wait must not leave this probe's child running.
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) if !remaining.is_zero() => {
                std::thread::sleep(VERSION_PROBE_POLL.min(remaining));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

// Every test here drives a unix shell script; nothing in this module compiles on Windows.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    use super::super::{ToolResolution, TEST_ENV_LOCK};

    #[cfg(unix)]
    #[test]
    fn version_probe_retires_a_hung_child_and_preserves_exit_verdicts() {
        let mut hung = Command::new("sleep").arg("30").spawn().unwrap();
        let started = Instant::now();
        assert!(!probe_exits_successfully(
            &mut hung,
            Duration::from_millis(100)
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            hung.try_wait().unwrap().is_some(),
            "timed-out probe was not reaped"
        );

        for (exit_code, expected) in [(0, true), (7, false)] {
            let mut child = Command::new("sh")
                .arg("-c")
                .arg(format!("exit {exit_code}"))
                .spawn()
                .unwrap();
            assert_eq!(
                probe_exits_successfully(&mut child, Duration::from_secs(2)),
                expected
            );
            assert!(child.try_wait().unwrap().is_some());
        }
    }

    struct EnvRestore(Option<std::ffi::OsString>);

    impl EnvRestore {
        fn capture() -> Self {
            Self(std::env::var_os(CONTEXT_ENV))
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match &self.0 {
                Some(value) => std::env::set_var(CONTEXT_ENV, value),
                None => std::env::remove_var(CONTEXT_ENV),
            }
        }
    }

    fn sha256(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    #[cfg(unix)]
    #[test]
    fn runtime_context_set_resolves_the_verified_python_and_tool_pair() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = TEST_ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();
        let temp =
            std::env::temp_dir().join(format!("scut-pinned-tool-bundle-{}", std::process::id()));
        let bin = temp.join("media-tools").join("bin");
        let python_root = temp.join("python");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&python_root).unwrap();
        let script = b"#!/bin/sh\nexit 0\n";
        for path in [bin.join("ffmpeg"), bin.join("ffprobe")] {
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(python_root.join("python"), b"python").unwrap();
        std::fs::write(python_root.join("onnx_asr.py"), b"import").unwrap();
        let media_root = std::fs::canonicalize(temp.join("media-tools")).unwrap();
        let python_root = std::fs::canonicalize(python_root).unwrap();
        let ffmpeg = media_root.join("bin").join("ffmpeg");
        let ffprobe = media_root.join("bin").join("ffprobe");
        let interpreter = python_root.join("python");
        let import = python_root.join("onnx_asr.py");
        let bundle = serde_json::json!({
            "schema": PINNED_EXECUTABLE_BUNDLE_CONTEXT_CONTRACT,
            "root": media_root,
            "manifestSha256": "a".repeat(64),
            "receiptSha256": "b".repeat(64),
            "platform": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
            "executables": [
                {"name":"ffmpeg","path":ffmpeg,"sha256":sha256(script),"bytes":script.len(),"mode":493},
                {"name":"ffprobe","path":ffprobe,"sha256":sha256(script),"bytes":script.len(),"mode":493}
            ],
            "files": 2,
            "totalBytes": script.len() * 2,
        });
        let python = serde_json::json!({
            "schema": CONTEXT_CONTRACT,
            "root": python_root,
            "manifestSha256": "c".repeat(64),
            "receiptSha256": "d".repeat(64),
            "interpreter":{"path":interpreter,"sha256":sha256(b"python"),"version":"3.12.13"},
            "imports":[{"module":"onnx_asr","path":import,"sha256":sha256(b"import")}],
            "models":[],
            "files":2,
            "totalBytes":12,
        });
        let mut context = serde_json::json!({
            "schema": "release-runner.native-runtime-context-set/v1",
            "runtimes": [
                {"id":"media-tools","context":bundle},
                {"id":"python","context":python}
            ]
        });
        let locator = temp.join("native-runtime-context.json");
        std::fs::write(&locator, serde_json::to_vec(&context).unwrap()).unwrap();
        std::env::set_var(CONTEXT_ENV, &locator);

        let resolution = ToolResolution::detect(&temp.join("empty"));
        assert!(resolution.ffmpeg_ok);
        assert_eq!(resolution.ffmpeg_source, "runner-pinned-bundle");
        assert!(matches!(
            &resolution.native_runtime,
            super::super::NativeRuntimeResolution::Accepted { interpreter: selected, .. }
                if selected == &python_root.join("python")
        ));
        assert!(matches!(
            &resolution.pinned_executable_bundle,
            PinnedExecutableBundleResolution::Accepted { ffmpeg: selected_ffmpeg, ffprobe: selected_ffprobe, .. }
                if selected_ffmpeg == &ffmpeg && selected_ffprobe == &ffprobe
        ));
        assert_eq!(
            resolution.to_json()["pinnedExecutableBundle"]["executables"]["ffprobe"],
            serde_json::json!(ffprobe.display().to_string())
        );

        context["runtimes"][0]["context"]["executables"][1]["sha256"] =
            serde_json::json!("0".repeat(64));
        std::fs::write(&locator, serde_json::to_vec(&context).unwrap()).unwrap();
        let rejected = ToolResolution::detect(&temp.join("empty"));
        assert!(
            !rejected.ffmpeg_ok,
            "a rejected set bundle must not use ambient ffmpeg"
        );
        assert!(matches!(
            rejected.pinned_executable_bundle,
            PinnedExecutableBundleResolution::Rejected { .. }
        ));

        let _ = std::fs::remove_dir_all(temp);
    }
}
