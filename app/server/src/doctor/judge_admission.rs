//! Bounded render-judge admission probe for `system.doctor`.
//!
//! The Python ladder owns provider-specific admission rules. This module only
//! executes its cheap `detect` protocol once, under the doctor's owned-process
//! boundary, and rejects missing or malformed affirmative readiness.

use super::{run_doctor_command, CardStatus};
use cut_media::ffmpeg::{run_owned_command_with_input, OwnedProcessControl};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

const DETECT_TIMEOUT: Duration = Duration::from_secs(12);
const JUDGE_PROVIDERS: &[&str] = &["claude", "codex", "antigravity", "grok"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderAdmission {
    pub(super) found: bool,
    pub(super) judge_ready: bool,
    pub(super) availability_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum JudgeAdmissions {
    Verified(BTreeMap<String, ProviderAdmission>),
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedAdmission {
    pub(super) judge_ready: bool,
    pub(super) availability_reason: Option<String>,
    pub(super) status: CardStatus,
}

/// Turn a provider entry into doctor-card state. Binary presence is retained
/// separately from admission: a found CLI may remain degraded, and an
/// uncompleted/malformed detect probe is Unknown rather than optimistic.
pub(super) fn resolve(
    provider: &str,
    installed: bool,
    adapter_runtime_ready: bool,
    admissions: &JudgeAdmissions,
) -> ResolvedAdmission {
    if !installed {
        return ResolvedAdmission {
            judge_ready: false,
            availability_reason: Some("CLI not found on the resolved judge path".into()),
            status: CardStatus::Missing,
        };
    }
    if !adapter_runtime_ready {
        return ResolvedAdmission {
            judge_ready: false,
            availability_reason: Some(
                "render-judge adapter and Python runtime are required before admission can be checked"
                    .into(),
            ),
            status: CardStatus::Degraded,
        };
    }
    let JudgeAdmissions::Verified(admissions) = admissions else {
        return ResolvedAdmission {
            judge_ready: false,
            availability_reason: Some(
                "render-judge admission could not be verified; re-scan before running review"
                    .into(),
            ),
            status: CardStatus::Unknown,
        };
    };
    let Some(admission) = admissions.get(provider) else {
        return ResolvedAdmission {
            judge_ready: false,
            availability_reason: Some(
                "render-judge adapter did not affirmatively report this provider ready".into(),
            ),
            status: CardStatus::Degraded,
        };
    };
    if !admission.found {
        return ResolvedAdmission {
            judge_ready: false,
            availability_reason: admission.availability_reason.clone().or_else(|| {
                Some(
                    "render-judge adapter could not find this provider on the invocation path"
                        .into(),
                )
            }),
            status: CardStatus::Degraded,
        };
    }
    if !admission.judge_ready {
        return ResolvedAdmission {
            judge_ready: false,
            availability_reason: admission.availability_reason.clone().or_else(|| {
                Some("render-judge adapter did not affirmatively report this provider ready".into())
            }),
            status: CardStatus::Degraded,
        };
    }
    ResolvedAdmission {
        judge_ready: true,
        availability_reason: None,
        status: CardStatus::Ok,
    }
}

fn admitted_probe_input(
    launches: &BTreeMap<String, crate::provider_runtime::ProviderChildLaunch>,
) -> String {
    crate::provider_runtime::python_child_launches_value(launches, JUDGE_PROVIDERS).to_string()
}

/// Run the configured ladder's no-model `detect` protocol once. Its stdout and
/// stderr inherit the doctor's owned 512 KiB output cap, and its child tree is
/// reaped on the fixed timeout. With selected Runner context, Python receives
/// only its validated handoff and never discovers a provider through PATH.
/// Without context, preserve the ordinary augmented-PATH route.
pub(super) fn probe(
    adapter: &Path,
    python: &Path,
    judge_cli_path: Option<&OsStr>,
    native_context: bool,
) -> JudgeAdmissions {
    let provider_launches_input =
        match crate::provider_runtime::provider_launches_from_process_environment() {
            Ok(Some(launches)) => Some(admitted_probe_input(&launches)),
            Ok(None) => None,
            Err(_) => return JudgeAdmissions::Unverified,
        };
    let mut command = Command::new(python);
    cut_perception::apply_python_command_policy(&mut command, native_context);
    command.arg(adapter).arg("detect");
    let output = if let Some(input) = provider_launches_input {
        command.arg("--provider-launches-stdin");
        let control = OwnedProcessControl::bounded(DETECT_TIMEOUT, || false);
        run_owned_command_with_input(
            &mut command,
            input.as_bytes(),
            &control,
            "judge admission probe",
            None,
        )
    } else {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(path) = judge_cli_path {
            command.env("PATH", path);
        }
        run_doctor_command(&mut command, DETECT_TIMEOUT, "judge admission probe")
    };
    let Ok(output) = output else {
        return JudgeAdmissions::Unverified;
    };
    if !output.status.success() {
        return JudgeAdmissions::Unverified;
    }
    parse_detect_output(&output.stdout)
        .map(JudgeAdmissions::Verified)
        .unwrap_or(JudgeAdmissions::Unverified)
}

fn parse_detect_output(bytes: &[u8]) -> Result<BTreeMap<String, ProviderAdmission>, ()> {
    let payload: Value = serde_json::from_slice(bytes).map_err(|_| ())?;
    let rungs = payload.get("rungs").and_then(Value::as_array).ok_or(())?;
    let mut admissions = BTreeMap::new();
    for rung in rungs {
        let provider = rung
            .get("provider")
            .and_then(Value::as_str)
            .filter(|provider| !provider.is_empty())
            .ok_or(())?;
        let found = rung.get("found").and_then(Value::as_bool).unwrap_or(false);
        // Admission is affirmative: an old/custom adapter that does not publish
        // judge_ready remains unavailable instead of inheriting binary presence.
        let judge_ready = found && rung.get("judge_ready").and_then(Value::as_bool) == Some(true);
        let availability_reason = rung
            .get("availability_reason")
            .or_else(|| rung.get("restricted_read_reason"))
            .and_then(Value::as_str)
            .map(bounded_reason);
        if admissions
            .insert(
                provider.to_owned(),
                ProviderAdmission {
                    found,
                    judge_ready,
                    availability_reason,
                },
            )
            .is_some()
        {
            return Err(());
        }
    }
    Ok(admissions)
}

fn bounded_reason(reason: &str) -> String {
    const MAX_REASON_CHARS: usize = 512;
    let mut chars = reason.chars();
    let mut bounded: String = chars.by_ref().take(MAX_REASON_CHARS).collect();
    if chars.next().is_some() {
        bounded.push('…');
    }
    bounded
}

#[cfg(test)]
#[path = "judge_admission_tests.rs"]
mod tests;
