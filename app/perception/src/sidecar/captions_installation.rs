//! Passive installation evidence and bounded Python version probes, not analysis.
//! A configured interpreter that cannot start is damaged, never an optional miss.
use cut_core::{error_codes, CutError};
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Command, time::Duration};

const INSTALL_RECORD: &str = "captions-installed.json";
const REPAIR: &str =
    "Repair the captions tools in Settings > AI & transcription, then check again.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationState {
    NotInstalled,
    Ready,
    Damaged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionsInstallation {
    pub state: InstallationState,
    pub installation_id: Option<String>,
    pub python: Option<std::path::PathBuf>,
    pub version: Option<String>,
    pub error: Option<CutError>,
}

/// Inspect the same configured/prepared runtime as Doctor. Never falls back to a
/// bare PATH Python and never imports a model or starts media analysis.
pub fn inspect_captions_installation() -> CaptionsInstallation {
    let native = match super::native_runtime_context() {
        Ok(context) => context.is_some(),
        Err(error) => return rejected(error),
    };
    let selected = match super::configured_sidecar_python() {
        Ok(python) => python,
        Err(error) => return rejected(error),
    };
    // The ordinary resolver only returns usable paths. Retain evidence of an
    // installation whose interpreter disappeared, including after .venv removal.
    let selected = selected.or_else(|| {
        super::sidecar_base_dirs()
            .into_iter()
            .find(|base| {
                base.join(".venv").symlink_metadata().is_ok()
                    || base.join(INSTALL_RECORD).symlink_metadata().is_ok()
            })
            .map(|base| super::venv_python(&base))
    });
    probe_selected_python(selected.as_deref(), native, Duration::from_secs(8))
}

fn rejected(mut error: CutError) -> CaptionsInstallation {
    if error.suggested_action.is_none() {
        error.suggested_action = Some(REPAIR.into());
    }
    CaptionsInstallation {
        state: InstallationState::Damaged,
        installation_id: Some("prepared-captions-runtime".into()),
        python: None,
        version: None,
        error: Some(error),
    }
}

/// Explicit-path seam for deterministic tests and consumers already holding a
/// selection snapshot. `None` means no installation evidence, not a failed probe.
pub fn probe_selected_python(
    python: Option<&Path>,
    native: bool,
    timeout: Duration,
) -> CaptionsInstallation {
    let Some(python) = python else {
        return CaptionsInstallation {
            state: InstallationState::NotInstalled,
            installation_id: None,
            python: None,
            version: None,
            error: None,
        };
    };
    let id = format!("python:{:?}", python.as_os_str());
    let mut result = CaptionsInstallation {
        state: InstallationState::Damaged,
        installation_id: Some(id),
        python: Some(python.to_path_buf()),
        version: None,
        error: None,
    };
    let mut command = Command::new(python);
    super::apply_python_command_policy(&mut command, native);
    command.arg("--version");
    let control = cut_media::ffmpeg::OwnedProcessControl::bounded(timeout, || false);
    let probe =
        cut_media::ffmpeg::run_owned_command(&mut command, &control, "captions version probe");
    match probe {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let banner = stdout
                .lines()
                .chain(stderr.lines())
                .map(str::trim)
                .find(|s| s.starts_with("Python "));
            if output.status.success() && banner.is_some() {
                result.state = InstallationState::Ready;
                result.version = banner.map(str::to_owned);
            } else {
                result.error = Some(
                    CutError::new(
                        error_codes::SIDECAR,
                        "Captions tools could not run",
                        format!(
                            "{} --version exited with {}: {} {}",
                            python.display(),
                            output.status,
                            stdout.trim(),
                            stderr.trim()
                        ),
                    )
                    .with_suggested_action(REPAIR),
                );
            }
        }
        Err(mut error) => {
            // Keep the process owner's code, message and cause, including a
            // timeout or OS loader error. Only add the missing repair action.
            if error.suggested_action.is_none() {
                error.suggested_action = Some(REPAIR.into());
            }
            result.error = Some(error);
        }
    }
    result
}

/// Keep installation evidence outside .venv so a removed interpreter/environment
/// cannot silently turn a known installation into an optional-not-installed row.
/// The setup caller reports an error additively; this never invalidates its tools.
/// The caller has already admitted the tools directory; replace the record's name
/// atomically, never follow a planted link or write through a shared file.
pub fn remember_captions_installation(base: &Path, python_version: &str) -> Result<(), CutError> {
    let path = base.join(INSTALL_RECORD);
    let record = serde_json::json!({
        "schema": "shellx-cut/captions-installation/1",
        "python_version": python_version,
    });
    cut_core::store::replace_project_file(&path, record.to_string().as_bytes())
        .map_err(|error| record_error(&path, error))
}

fn record_error(path: &Path, error: std::io::Error) -> CutError {
    CutError::new(
        error_codes::IO,
        "Caption tools installed, but their installation record could not be saved",
        format!("{}: {error}", path.display()),
    )
    .with_suggested_action("Check the captions tools folder permissions, then run setup again.")
}
