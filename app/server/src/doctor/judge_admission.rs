//! Bounded render-judge admission probe for `system.doctor`.
//!
//! The Python ladder owns provider-specific admission rules. This module only
//! executes its cheap `detect` protocol once, under the doctor's owned-process
//! boundary, and rejects missing or malformed affirmative readiness.

use super::{run_doctor_command, CardStatus};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

const DETECT_TIMEOUT: Duration = Duration::from_secs(12);

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

/// Run the configured ladder's no-model `detect` protocol once. Its stdout and
/// stderr inherit the doctor's owned 512 KiB output cap, and its child tree is
/// reaped on the fixed timeout. The supplied PATH is the same augmented PATH
/// used by a real judge invocation, so off-PATH resolved CLIs do not disappear
/// between doctor and review.
pub(super) fn probe(
    adapter: &Path,
    python: &Path,
    judge_cli_path: Option<&OsStr>,
) -> JudgeAdmissions {
    let mut command = Command::new(python);
    command
        .arg(adapter)
        .arg("detect")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = judge_cli_path {
        command.env("PATH", path);
    }

    let Ok(output) = run_doctor_command(&mut command, DETECT_TIMEOUT, "judge admission probe")
    else {
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
mod tests {
    use super::*;

    #[test]
    fn parses_provider_admission_without_inferring_readiness() {
        let admissions = parse_detect_output(
            br#"{"rungs":[
                {"provider":"claude","found":true,"judge_ready":false,
                 "restricted_read_reason":"restricted Read capability is unavailable"},
                {"provider":"codex","found":true,"judge_ready":false,
                 "availability_reason":"render judge unavailable until restricted tool/file access is verified"},
                {"provider":"antigravity","found":true},
                {"provider":"grok","found":false,"judge_ready":true}
            ]}"#,
        )
        .expect("valid detect response");

        assert!(!admissions["claude"].judge_ready);
        assert_eq!(
            admissions["claude"].availability_reason.as_deref(),
            Some("restricted Read capability is unavailable")
        );
        assert!(!admissions["codex"].judge_ready);
        assert_eq!(
            admissions["codex"].availability_reason.as_deref(),
            Some("render judge unavailable until restricted tool/file access is verified")
        );
        assert!(!admissions["antigravity"].judge_ready);
        assert!(!admissions["grok"].judge_ready);
    }

    #[test]
    fn resolve_keeps_installed_but_unready_provider_out_of_ok() {
        let admissions = JudgeAdmissions::Verified(BTreeMap::from([(
            "codex".into(),
            ProviderAdmission {
                found: true,
                judge_ready: false,
                availability_reason: Some(
                    "render judge unavailable until restricted tool/file access is verified".into(),
                ),
            },
        )]));
        let codex = resolve("codex", true, true, &admissions);
        assert_eq!(codex.status, CardStatus::Degraded);
        assert!(!codex.judge_ready);
        assert_eq!(
            codex.availability_reason.as_deref(),
            Some("render judge unavailable until restricted tool/file access is verified")
        );

        let unverified = resolve("claude", true, true, &JudgeAdmissions::Unverified);
        assert_eq!(unverified.status, CardStatus::Unknown);
        assert!(!unverified.judge_ready);
    }

    #[test]
    fn rejects_missing_or_duplicate_provider_protocol_entries() {
        assert!(parse_detect_output(br#"{"rungs":[{"found":true}]}"#).is_err());
        assert!(parse_detect_output(
            br#"{"rungs":[
                {"provider":"claude","found":true,"judge_ready":false,
                 "restricted_read_reason":"restricted Read capability is unavailable"},
                {"provider":"claude","found":true,"judge_ready":true}
            ]}"#,
        )
        .is_err());
    }
}
