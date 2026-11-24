//! The export check list: every catalog check with its state, built from facts the
//! engine already holds (pregate risks, cached analysis, recorded attempts, the
//! receipt's check rows, earlier on-request results, the installation state).
//!
//! Pure: no I/O, no process. The list is information only; nothing here feeds an
//! export refusal. A state is never guessed: an empty array is not a pass, a
//! missing report is not a missing tool, and an old report without provenance is
//! `unknown_older_analysis`.

use crate::analysis_attempts::{AnalysisAttempt, ExecutionState};
use crate::captions_installation::{CaptionsInstallation, InstallationState};
use crate::checks::PregateReport;
use cut_core::export_checks::*;
use cut_core::{CheckResult, CutError};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// What the engine knows about one analysis report of a placed asset.
#[derive(Debug, Clone)]
pub enum ReportEvidence {
    /// No report was ever written for this asset.
    Missing,
    /// A report exists but could not be read.
    Unreadable(Box<CutError>),
    Loaded {
        instruments_run: Vec<String>,
        attempt: Option<Box<AnalysisAttempt>>,
    },
}

#[derive(Debug, Clone)]
pub struct AssetEvidence {
    pub id: String,
    pub has_video: bool,
    pub has_audio: bool,
    pub report: ReportEvidence,
    /// Durable attempt history for this asset's current file.
    pub attempts: Vec<AnalysisAttempt>,
}

/// A result of a check the person asked for earlier (not run by the export).
#[derive(Debug, Clone)]
pub enum OnRequestFact {
    Ran {
        findings: Vec<ExportCheckFinding>,
        at_op: String,
    },
    Failed {
        error: CutError,
        at_op: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct OnRequestEvidence {
    pub fact: Option<OnRequestFact>,
    /// A fully resolved command that can run this check now; `None` when the
    /// check has no standalone command or its context is not available.
    pub command: Option<ExportCheckAction>,
}

/// A check computed from the project alone (caption timing, brand rules).
#[derive(Debug, Clone)]
pub enum DirectResult {
    Clear,
    Found(Vec<ExportCheckFinding>),
    NotApplicable(String),
    Failed(CutError),
}

#[derive(Debug, Clone)]
pub struct ToolFact {
    pub status: ExportCheckProviderStatus,
    pub version: Option<String>,
    pub version_reason: Option<String>,
    pub error: Option<CutError>,
}

impl ToolFact {
    pub fn ready(version: Option<String>) -> Self {
        let version_reason = version.is_none().then(|| "version not read".to_string());
        Self {
            status: ExportCheckProviderStatus::Ready,
            version,
            version_reason,
            error: None,
        }
    }
    pub fn unknown(reason: &str) -> Self {
        Self {
            status: ExportCheckProviderStatus::Unknown,
            version: None,
            version_reason: Some(reason.into()),
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Tools {
    pub captions: ToolFact,
    pub captions_installation_id: Option<String>,
    pub ffmpeg: ToolFact,
    pub ffprobe: ToolFact,
    pub judge: ToolFact,
}

impl Tools {
    pub fn from_installation(install: &CaptionsInstallation) -> ToolFact {
        match install.state {
            InstallationState::NotInstalled => ToolFact {
                status: ExportCheckProviderStatus::Missing,
                version: None,
                version_reason: Some("not installed".into()),
                error: None,
            },
            InstallationState::Ready => ToolFact {
                status: ExportCheckProviderStatus::Ready,
                version: install.version.clone(),
                version_reason: install
                    .version
                    .is_none()
                    .then(|| "version not read".to_string()),
                error: None,
            },
            InstallationState::Damaged => ToolFact {
                status: ExportCheckProviderStatus::Damaged,
                version: install.version.clone(),
                version_reason: install
                    .version
                    .is_none()
                    .then(|| "version probe failed".to_string()),
                error: install.error.clone(),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct Env {
    pub app_version: String,
    pub os: String,
    pub arch: String,
}

/// Facts of the timeline itself.
#[derive(Debug, Clone, Default)]
pub struct TimelineFacts {
    pub has_picture: bool,
    pub media_clip_count: usize,
}

/// Memory/disk estimate of the render plan, when one exists.
#[derive(Debug, Clone)]
pub enum ResourceFact {
    /// Estimated at the start of an export; no estimate exists before it.
    NotEstimated,
    Clear,
    Found(Vec<ExportCheckFinding>),
}

pub struct ListInput<'a> {
    pub phase: ExportCheckPhase,
    pub at_op: String,
    pub render_id: Option<String>,
    pub env: Env,
    pub tools: Tools,
    pub timeline: TimelineFacts,
    pub assets: &'a [AssetEvidence],
    pub pregate: Option<&'a PregateReport>,
    /// The before-export list captured for this export (finished phase only).
    pub before_snapshot: Option<&'a ExportChecks>,
    pub receipt_checks: &'a [CheckResult],
    /// Placed sources the engine could not probe for this export, by asset.
    pub source_errors: &'a BTreeMap<String, CutError>,
    pub direct: &'a BTreeMap<String, DirectResult>,
    pub on_request: &'a BTreeMap<String, OnRequestEvidence>,
    pub memory: ResourceFact,
    pub disk: ResourceFact,
}

// ---------------------------------------------------------------- helpers

fn action(kind: ExportCheckActionKind, label: &str) -> ExportCheckAction {
    ExportCheckAction {
        kind,
        label: label.into(),
        at_ms: None,
        tool_id: None,
        text: None,
        verb: None,
        args: None,
    }
}

pub fn seek_action(at_ms: u64) -> ExportCheckAction {
    ExportCheckAction {
        at_ms: Some(at_ms),
        ..action(ExportCheckActionKind::Seek, "Go to place")
    }
}

pub fn command_action(verb: &str, args: Value) -> ExportCheckAction {
    ExportCheckAction {
        verb: Some(verb.into()),
        args: Some(args),
        ..action(ExportCheckActionKind::Command, "Check now")
    }
}

fn open_tools_action() -> ExportCheckAction {
    ExportCheckAction {
        tool_id: Some("perception".into()),
        ..action(ExportCheckActionKind::OpenTools, "Install captions")
    }
}

fn error_text(e: &CutError) -> String {
    let mut out = format!("{}: {}", e.code, e.message);
    if !e.cause.is_empty() {
        out.push('\n');
        out.push_str(&e.cause);
    }
    if let Some(s) = &e.suggested_action {
        out.push('\n');
        out.push_str(s);
    }
    out
}

fn words(s: &str) -> usize {
    s.split_whitespace().count()
}

fn is_captions_missing(e: &CutError) -> bool {
    crate::is_captions_not_installed(e)
}

struct Ctx<'a> {
    input: &'a ListInput<'a>,
}

impl Ctx<'_> {
    fn tool_for(&self, provider: &str) -> ToolFact {
        let t = &self.input.tools;
        if provider.starts_with("perception.") || provider == "perception" {
            t.captions.clone()
        } else if provider == "ffprobe" {
            t.ffprobe.clone()
        } else if provider.starts_with("ffmpeg") {
            t.ffmpeg.clone()
        } else if provider == "judge" {
            t.judge.clone()
        } else {
            ToolFact {
                status: ExportCheckProviderStatus::Ready,
                version: None,
                version_reason: Some("measured on this computer, no tool".into()),
                error: None,
            }
        }
    }

    fn providers(&self, def: &ExportCheckDefinition, attempted: bool) -> Vec<ExportCheckProvider> {
        def.providers
            .iter()
            .map(|id| {
                let tool = self.tool_for(id);
                let captions = id.starts_with("perception.");
                ExportCheckProvider {
                    id: id.clone(),
                    installation_id: captions
                        .then(|| self.input.tools.captions_installation_id.clone())
                        .flatten(),
                    configured: tool.status != ExportCheckProviderStatus::Missing,
                    status: tool.status,
                    version: tool.version.clone(),
                    version_reason: if tool.version.is_none() {
                        Some(
                            tool.version_reason
                                .clone()
                                .unwrap_or_else(|| "version not read".into()),
                        )
                    } else {
                        None
                    },
                    attempted,
                    started_at: None,
                    finished_at: None,
                    error: tool
                        .error
                        .as_ref()
                        .and_then(|e| serde_json::to_value(e).ok()),
                }
            })
            .collect()
    }

    fn base(
        &self,
        def: &ExportCheckDefinition,
        state: ExportCheckState,
        reason_code: ExportCheckReason,
        result: &str,
        reason: String,
        suggested_action: &str,
    ) -> ExportCheckEntry {
        debug_assert!(words(result) <= 5 && words(&def.name) <= 4, "{}", def.id);
        ExportCheckEntry {
            id: def.id.clone(),
            name: def.name.clone(),
            when: def.when,
            mode: def.mode,
            state,
            reason_code,
            result: result.into(),
            reason,
            suggested_action: suggested_action.into(),
            action: None,
            findings: vec![],
            coverage: ExportCheckCoverage {
                applicable_assets: vec![],
                measured_assets: vec![],
                unchecked_assets: vec![],
                errors: vec![],
            },
            providers: self.providers(def, false),
            report_details: None,
        }
    }

    fn checked(&self, def: &ExportCheckDefinition, what: &str) -> ExportCheckEntry {
        self.base(
            def,
            ExportCheckState::Checked,
            ExportCheckReason::Clear,
            "fine",
            what.into(),
            "No action needed.",
        )
    }

    fn not_applicable(&self, def: &ExportCheckDefinition, why: &str) -> ExportCheckEntry {
        self.base(
            def,
            ExportCheckState::NotApplicable,
            ExportCheckReason::NotApplicable,
            "not applicable",
            why.into(),
            "No action needed.",
        )
    }

    fn found(
        &self,
        def: &ExportCheckDefinition,
        findings: Vec<ExportCheckFinding>,
    ) -> ExportCheckEntry {
        let headline = findings
            .first()
            .map(|f| f.reason.clone())
            .unwrap_or_else(|| "A problem was found.".into());
        let result = if findings.len() > 1 {
            "found several problems"
        } else {
            "found a problem"
        };
        let seek = findings
            .iter()
            .find_map(|f| f.range_ms.as_ref().and_then(|r| r.first().copied()));
        let mut e = self.base(
            def,
            ExportCheckState::Found,
            ExportCheckReason::Finding,
            result,
            headline,
            if seek.is_some() {
                "Go to the place and look."
            } else {
                "Look at the finding and decide."
            },
        );
        e.action = seek.map(seek_action);
        e.findings = findings;
        e
    }

    fn not_checked(
        &self,
        def: &ExportCheckDefinition,
        code: ExportCheckReason,
        result: &str,
        reason: &str,
        suggestion: &str,
        act: Option<ExportCheckAction>,
    ) -> ExportCheckEntry {
        let mut e = self.base(
            def,
            ExportCheckState::NotChecked,
            code,
            result,
            reason.into(),
            suggestion,
        );
        e.action = act;
        e
    }

    fn tool_missing(&self, def: &ExportCheckDefinition) -> ExportCheckEntry {
        self.not_checked(
            def,
            ExportCheckReason::OptionalToolMissing,
            "not checked, tools not installed",
            "The optional captions tools that measure this are not installed.",
            "Install captions",
            Some(open_tools_action()),
        )
    }

    /// The run said "the captions tools are not installed". What the person is
    /// told depends on what is on the computer now: nothing installed is a quiet
    /// suggestion, an installation that cannot start is an error to report.
    fn missing_or_damaged(
        &self,
        def: &ExportCheckDefinition,
        runtime: CutError,
    ) -> ExportCheckEntry {
        let captions = &self.input.tools.captions;
        match captions.status {
            ExportCheckProviderStatus::Damaged => {
                let error = captions.error.clone().unwrap_or(runtime);
                self.failed(def, &error, true)
            }
            ExportCheckProviderStatus::Ready => self.not_checked(
                def,
                ExportCheckReason::NotRunYet,
                "not checked yet",
                "The captions tools were not installed when this ran; they are now.",
                "Check now",
                None,
            ),
            _ => self.tool_missing(def),
        }
    }

    fn failed(
        &self,
        def: &ExportCheckDefinition,
        error: &CutError,
        attempted: bool,
    ) -> ExportCheckEntry {
        let mut e = self.base(
            def,
            ExportCheckState::Error,
            ExportCheckReason::CheckFailed,
            "could not run",
            if error.cause.is_empty() {
                error.message.clone()
            } else {
                error.cause.clone()
            },
            "Copy details",
        );
        e.providers = self.providers(def, attempted);
        e.coverage.errors.push(json!({
            "provider_id": def.providers.first().cloned().unwrap_or_else(|| "engine".into()),
            "reason": error.message,
            "error": error,
        }));
        let err_json = serde_json::to_value(error).unwrap_or(Value::Null);
        let tool = def
            .providers
            .first()
            .cloned()
            .unwrap_or_else(|| "engine".into());
        let version = e
            .providers
            .first()
            .map(|p| {
                p.version.clone().unwrap_or_else(|| {
                    format!(
                        "version unavailable ({})",
                        p.version_reason.clone().unwrap_or_default()
                    )
                })
            })
            .unwrap_or_else(|| "no tool".into());
        let env = &self.input.env;
        let text = format!(
            "{}\nTool: {}; {}\n{}\nCut: {}\nSystem: {} {}",
            def.id,
            tool,
            version,
            error_text(error),
            env.app_version,
            env.os,
            env.arch
        );
        e.report_details = Some(ExportCheckReportDetails {
            check_id: def.id.clone(),
            providers: e.providers.clone(),
            error: err_json,
            app_version: env.app_version.clone(),
            os: env.os.clone(),
            arch: env.arch.clone(),
            at_op: self.input.at_op.clone(),
            render_id: self.input.render_id.clone(),
        });
        e.action = Some(ExportCheckAction {
            text: Some(text),
            ..action(ExportCheckActionKind::CopyDetails, "Copy details")
        });
        e
    }
}

// ------------------------------------------------- source-fact derivation

enum AssetState {
    Skip,
    Measured,
    ToolMissing,
    NotRun,
    Legacy,
    Failed(CutError),
}

fn failure_of(attempt: &AnalysisAttempt, instrument: &str) -> Option<CutError> {
    if let Some(e) = &attempt.error {
        return Some(e.clone());
    }
    attempt
        .instruments
        .iter()
        .find(|i| i.instrument == instrument && i.state == ExecutionState::Failed)
        .map(|i| {
            let raw = i.error.clone().unwrap_or(Value::Null);
            serde_json::from_value::<CutError>(raw.clone()).unwrap_or_else(|_| {
                CutError::new(
                    cut_core::error_codes::SIDECAR,
                    format!("{instrument} analysis failed"),
                    if raw.is_null() {
                        "the analysis tool reported a failure".to_string()
                    } else {
                        raw.to_string()
                    },
                )
            })
        })
}

fn asset_state(instrument: &str, a: &AssetEvidence, captions: &ToolFact) -> AssetState {
    let applicable = match instrument {
        "scenes" => a.has_video,
        "silence" => a.has_audio,
        _ => true,
    };
    if !applicable {
        return AssetState::Skip;
    }
    let (mut attempts, ran): (Vec<&AnalysisAttempt>, bool) = match &a.report {
        ReportEvidence::Loaded {
            instruments_run,
            attempt,
        } => (
            a.attempts
                .iter()
                .chain(attempt.iter().map(|b| &**b))
                .collect(),
            instruments_run.iter().any(|i| i == instrument),
        ),
        _ => (a.attempts.iter().collect(), false),
    };
    if ran {
        return AssetState::Measured;
    }
    attempts.sort_by_key(|t| t.asked_at_unix_ms);
    if let Some(last) = attempts.last() {
        if last
            .instruments
            .iter()
            .any(|i| i.instrument == instrument && i.state == ExecutionState::NotApplicable)
        {
            return AssetState::Skip;
        }
        if let Some(err) = failure_of(last, instrument) {
            return not_installed_or(err, captions);
        }
    }
    match &a.report {
        ReportEvidence::Unreadable(e) => AssetState::Failed((**e).clone()),
        ReportEvidence::Loaded { attempt, .. } => {
            if attempt.is_none() && attempts.is_empty() {
                AssetState::Legacy
            } else {
                AssetState::NotRun
            }
        }
        ReportEvidence::Missing => match captions.status {
            ExportCheckProviderStatus::Missing => AssetState::ToolMissing,
            ExportCheckProviderStatus::Damaged => {
                AssetState::Failed(captions.error.clone().unwrap_or_else(|| {
                    CutError::new(
                        cut_core::error_codes::SIDECAR,
                        "The captions tools could not start",
                        "the installation is present but its probe failed",
                    )
                }))
            }
            _ => AssetState::NotRun,
        },
    }
}

fn not_installed_or(err: CutError, captions: &ToolFact) -> AssetState {
    if is_captions_missing(&err) {
        return match captions.status {
            ExportCheckProviderStatus::Missing => AssetState::ToolMissing,
            ExportCheckProviderStatus::Damaged => {
                AssetState::Failed(captions.error.clone().unwrap_or(err))
            }
            _ => AssetState::NotRun,
        };
    }
    AssetState::Failed(err)
}

fn risk_findings(pregate: Option<&PregateReport>, kind: &str) -> Vec<ExportCheckFinding> {
    pregate
        .map(|p| {
            p.risks
                .iter()
                .filter(|r| r.kind == kind)
                .map(|r| ExportCheckFinding {
                    reason: r.detail.clone(),
                    asset_id: None,
                    clip_id: None,
                    range_ms: r.range_ms.map(|r| vec![r[0], r[1]]),
                    evidence: json!({"severity": r.severity, "suggested_action": r.suggested_action}),
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Ctx<'_> {
    fn source_fact(
        &self,
        def: &ExportCheckDefinition,
        instrument: &str,
        risk_kind: &str,
    ) -> ExportCheckEntry {
        let findings = risk_findings(self.input.pregate, risk_kind);
        let mut applicable = vec![];
        let mut measured = vec![];
        let mut unchecked = vec![];
        let mut errors: Vec<(String, CutError)> = vec![];
        let (mut missing, mut not_run, mut legacy, mut attempted) = (0, 0, 0, false);
        for a in self.input.assets {
            let state = asset_state(instrument, a, &self.input.tools.captions);
            if matches!(state, AssetState::Skip) {
                continue;
            }
            applicable.push(a.id.clone());
            attempted |= !a.attempts.is_empty();
            match state {
                AssetState::Measured => measured.push(a.id.clone()),
                AssetState::ToolMissing => {
                    missing += 1;
                    unchecked.push(a.id.clone());
                }
                AssetState::NotRun => {
                    not_run += 1;
                    unchecked.push(a.id.clone());
                }
                AssetState::Legacy => {
                    legacy += 1;
                    unchecked.push(a.id.clone());
                }
                AssetState::Failed(e) => {
                    unchecked.push(a.id.clone());
                    errors.push((a.id.clone(), e));
                }
                AssetState::Skip => {}
            }
        }
        let first_unchecked = unchecked.first().cloned();
        let mut entry = if !findings.is_empty() {
            self.found(def, findings)
        } else if applicable.is_empty() {
            self.not_applicable(def, "No placed file has this kind of picture or sound.")
        } else if let Some((_, e)) = errors.first() {
            self.failed(def, e, attempted)
        } else if missing > 0 {
            self.tool_missing(def)
        } else if not_run + legacy > 0 {
            let (code, why) = if not_run > 0 {
                (
                    ExportCheckReason::NotRunYet,
                    "The analysis of the placed files has not been run.",
                )
            } else {
                (
                    ExportCheckReason::UnknownOlderAnalysis,
                    "The analysis was made by an older version and does not say what ran.",
                )
            };
            self.not_checked(
                def,
                code,
                "not checked yet",
                why,
                "Check now",
                first_unchecked
                    .as_ref()
                    .map(|id| command_action("media.perception", json!({ "asset": id }))),
            )
        } else {
            self.checked(def, "Every placed file was measured and nothing was found.")
        };
        entry.coverage = ExportCheckCoverage {
            applicable_assets: applicable,
            measured_assets: measured,
            unchecked_assets: unchecked,
            errors: errors
                .iter()
                .map(|(id, e)| {
                    json!({
                        "asset_id": id,
                        "provider_id": def.providers.first(),
                        "reason": e.message,
                        "error": e,
                    })
                })
                .collect(),
        };
        entry.providers = self.providers(def, attempted);
        if entry.state == ExportCheckState::Error {
            if let Some(d) = entry.report_details.as_mut() {
                d.providers = entry.providers.clone();
            }
        }
        entry
    }

    fn edl_check(
        &self,
        def: &ExportCheckDefinition,
        kind: &str,
        applicable: bool,
        na: &str,
    ) -> ExportCheckEntry {
        let findings = risk_findings(self.input.pregate, kind);
        if !findings.is_empty() {
            self.found(def, findings)
        } else if !applicable {
            self.not_applicable(def, na)
        } else {
            self.checked(def, "Calculated from the timeline; nothing was found.")
        }
    }

    fn direct(&self, def: &ExportCheckDefinition) -> ExportCheckEntry {
        match self.input.direct.get(&def.id) {
            Some(DirectResult::Clear) => {
                self.checked(def, "Calculated from the project; nothing was found.")
            }
            Some(DirectResult::Found(f)) => self.found(def, f.clone()),
            Some(DirectResult::NotApplicable(why)) => self.not_applicable(def, why),
            Some(DirectResult::Failed(e)) => self.failed(def, e, true),
            None => self.not_checked(
                def,
                ExportCheckReason::NotRunYet,
                "not checked yet",
                "This check was not run for this list.",
                "No action available.",
                None,
            ),
        }
    }

    fn after_export(&self, def: &ExportCheckDefinition) -> ExportCheckEntry {
        self.not_checked(
            def,
            ExportCheckReason::AfterExport,
            "after the export",
            "This is checked on the finished file.",
            "No action needed.",
            None,
        )
    }

    fn on_request(&self, def: &ExportCheckDefinition) -> ExportCheckEntry {
        let ev = self.input.on_request.get(&def.id);
        match ev.and_then(|e| e.fact.as_ref()) {
            Some(OnRequestFact::Ran { findings, .. }) if findings.is_empty() => {
                self.checked(def, "You asked for this check and nothing was found.")
            }
            Some(OnRequestFact::Ran { findings, .. }) => self.found(def, findings.clone()),
            Some(OnRequestFact::Failed { error, .. }) => {
                if is_captions_missing(error) {
                    self.missing_or_damaged(def, error.clone())
                } else {
                    self.failed(def, error, true)
                }
            }
            None => self.not_checked(
                def,
                ExportCheckReason::OnRequest,
                "on request",
                "This check runs only when you ask for it.",
                if ev.and_then(|e| e.command.as_ref()).is_some() {
                    "Check now"
                } else {
                    "No standalone command."
                },
                ev.and_then(|e| e.command.clone()),
            ),
        }
    }

    fn resources(&self, def: &ExportCheckDefinition, fact: &ResourceFact) -> ExportCheckEntry {
        match fact {
            ResourceFact::NotEstimated => self.not_checked(
                def,
                ExportCheckReason::NotRunYet,
                "estimated at export",
                "The estimate is made when the export starts, from its size and folder.",
                "No action needed.",
                None,
            ),
            ResourceFact::Clear => self.checked(def, "The estimate fits this computer."),
            ResourceFact::Found(f) => self.found(def, f.clone()),
        }
    }

    // ---- finished-file rows from the receipt's own checks
    /// The receipt's own row, except that a placed source the engine could not
    /// read means "source files" cannot be called fine.
    fn receipt_row(&self, def: &ExportCheckDefinition) -> ExportCheckEntry {
        let entry = self.receipt_row_plain(def);
        let errors = self.input.source_errors;
        if def.id != "sources_decodable"
            || errors.is_empty()
            || entry.state == ExportCheckState::Found
        {
            return entry;
        }
        let first = errors.values().next().cloned().unwrap_or_else(|| {
            CutError::new(
                cut_core::error_codes::IO,
                "A placed source could not be read",
                "the source file could not be probed",
            )
        });
        let mut failed = self.failed(def, &first, true);
        failed.coverage.applicable_assets = errors.keys().cloned().collect();
        failed.coverage.unchecked_assets = errors.keys().cloned().collect();
        failed.coverage.errors = errors
            .iter()
            .map(|(id, e)| json!({"asset_id": id, "provider_id": "ffprobe", "reason": e.message, "error": e}))
            .collect();
        failed
    }

    fn receipt_row_plain(&self, def: &ExportCheckDefinition) -> ExportCheckEntry {
        let row = self.input.receipt_checks.iter().find(|c| c.name == def.id);
        let Some(row) = row else {
            return match def.id.as_str() {
                "j_l_cut" | "bed_duck_under_speech" | "crossfade_smoothness" => self
                    .not_applicable(
                        def,
                        "The edit has no such transition; this check is only made when it does.",
                    ),
                "cut_on_beat" => self.not_applicable(
                    def,
                    "No placed music has a measured beat grid; cuts are not compared to beats.",
                ),
                "sources_decodable" => self.not_checked(
                    def,
                    ExportCheckReason::NotRunYet,
                    "not checked yet",
                    "No source decode result was recorded for this export.",
                    "No action available.",
                    None,
                ),
                _ => self.not_checked(
                    def,
                    ExportCheckReason::UnknownOlderAnalysis,
                    "not recorded",
                    "This receipt does not list this check.",
                    "No action available.",
                    None,
                ),
            };
        };
        if cut_core::is_not_applicable(row) {
            let why = row
                .details
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("This check does not apply to this export.");
            return self.not_applicable(def, why);
        }
        let status = row.details.get("status").and_then(Value::as_str);
        let measured_false = row.details.get("measured").and_then(Value::as_bool) == Some(false);
        if status == Some("unmeasured") || measured_false {
            let runtime = row
                .details
                .get("runtime_error")
                .and_then(|v| serde_json::from_value::<CutError>(v.clone()).ok());
            return match runtime {
                Some(e) if is_captions_missing(&e) => self.missing_or_damaged(def, e),
                Some(e) => self.failed(def, &e, true),
                None => {
                    let why = row
                        .details
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("The check could not measure the finished file.");
                    let e = CutError::new(
                        cut_core::error_codes::IO,
                        "The check could not measure the file",
                        why.to_string(),
                    );
                    self.failed(def, &e, true)
                }
            };
        }
        if row.pass {
            return self.checked(def, "The finished export was measured; nothing was found.");
        }
        let at_ms = cut_core::fix_action(row).and_then(|f| f.targets.iter().find_map(|t| t.at_ms));
        let reason = row
            .details
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{} did not pass.", def.name));
        self.found(
            def,
            vec![ExportCheckFinding {
                reason,
                asset_id: None,
                clip_id: None,
                range_ms: at_ms.map(|t| vec![t, t]),
                evidence: row.evidence.clone(),
            }],
        )
    }
}

/// Build the whole list. One entry per catalog check, in catalog order.
pub fn build_export_checks(input: &ListInput<'_>) -> ExportChecks {
    let ctx = Ctx { input };
    let finished = input.phase == ExportCheckPhase::FinishedFile;
    let entries = export_check_catalog()
        .iter()
        .map(|def| {
            let id = def.id.as_str();
            // Checks that describe the export input are taken from the list
            // captured for this export, not from the now-edited project.
            if finished && def.when == ExportCheckWhen::BeforeExport {
                return match input
                    .before_snapshot
                    .and_then(|b| b.entries.iter().find(|e| e.id == id))
                {
                    Some(e) => e.clone(),
                    None => ctx.not_checked(
                        def,
                        ExportCheckReason::UnknownOlderAnalysis,
                        "not recorded",
                        "No list was captured before this export.",
                        "No action available.",
                        None,
                    ),
                };
            }
            match id {
                "empty_tail" => ctx.edl_check(
                    def,
                    id,
                    input.timeline.has_picture,
                    "The timeline has no picture.",
                ),
                "slideshow_risk" => ctx.edl_check(
                    def,
                    id,
                    input.timeline.has_picture,
                    "The timeline has no picture.",
                ),
                "tiny_or_zero_clips" => ctx.edl_check(
                    def,
                    id,
                    input.timeline.media_clip_count > 0,
                    "The timeline has no media clips.",
                ),
                "black_or_frozen" => ctx.source_fact(def, "scenes", id),
                "silent_output" => ctx.source_fact(def, "silence", id),
                "uniform_border" if !finished => ctx.source_fact(def, "scenes", id),
                "memory" => ctx.resources(def, &input.memory),
                "disk" => ctx.resources(def, &input.disk),
                "caption_qc" | "brand" => ctx.direct(def),
                "judge"
                | "signal_scopes"
                | "source_loudness"
                | "delivery"
                | "reframe_composition"
                | "output_geometry" => {
                    if !finished && def.when == ExportCheckWhen::FinishedFile {
                        ctx.after_export(def)
                    } else {
                        ctx.on_request(def)
                    }
                }
                _ if !finished => ctx.after_export(def),
                _ => ctx.receipt_row(def),
            }
        })
        .collect();
    ExportChecks {
        schema_version: 1,
        catalog_version: 1,
        phase: input.phase,
        at_op: input.at_op.clone(),
        render_id: input.render_id.clone(),
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_attempts::{AnalysisAttempt, InstrumentAttempt};

    fn tools(status: ExportCheckProviderStatus, error: Option<CutError>) -> Tools {
        Tools {
            captions: ToolFact {
                status,
                version: Some("3.11".into()),
                version_reason: None,
                error,
            },
            captions_installation_id: Some("venv".into()),
            ffmpeg: ToolFact::ready(Some("7".into())),
            ffprobe: ToolFact::ready(Some("7".into())),
            judge: ToolFact::unknown("not asked"),
        }
    }

    fn asset(id: &str, report: ReportEvidence, attempts: Vec<AnalysisAttempt>) -> AssetEvidence {
        AssetEvidence {
            id: id.into(),
            has_video: true,
            has_audio: true,
            report,
            attempts,
        }
    }

    fn attempt(error: Option<CutError>, instruments: Vec<InstrumentAttempt>) -> AnalysisAttempt {
        AnalysisAttempt {
            schema_version: 1,
            attempt_id: "a".into(),
            asset_id: "a1".into(),
            asset_hash: "h".into(),
            asked_at_unix_ms: 1,
            finished_at_unix_ms: Some(2),
            instruments,
            error,
            storage_error: None,
            protocol_error: None,
        }
    }

    fn build(
        status: ExportCheckProviderStatus,
        terr: Option<CutError>,
        assets: Vec<AssetEvidence>,
        phase: ExportCheckPhase,
    ) -> ExportChecks {
        let direct = BTreeMap::new();
        let on_request = BTreeMap::new();
        build_export_checks(&ListInput {
            phase,
            at_op: "op_000007".into(),
            render_id: None,
            env: Env {
                app_version: "0.6.122".into(),
                os: "linux".into(),
                arch: "x86_64".into(),
            },
            tools: tools(status, terr),
            timeline: TimelineFacts {
                has_picture: true,
                media_clip_count: 2,
            },
            assets: &assets,
            pregate: None,
            before_snapshot: None,
            receipt_checks: &[],
            source_errors: &BTreeMap::new(),
            direct: &direct,
            on_request: &on_request,
            memory: ResourceFact::NotEstimated,
            disk: ResourceFact::NotEstimated,
        })
    }

    fn entry<'a>(l: &'a ExportChecks, id: &str) -> &'a ExportCheckEntry {
        l.entries.iter().find(|e| e.id == id).unwrap()
    }

    fn missing_err() -> CutError {
        crate::captions_not_installed_error(&crate::sidecar::InterpreterProblem::NotFound {
            what: "no python".into(),
        })
    }

    #[test]
    fn the_list_names_every_catalog_check_once() {
        let l = build(
            ExportCheckProviderStatus::Ready,
            None,
            vec![],
            ExportCheckPhase::BeforeExport,
        );
        let ids: Vec<_> = l.entries.iter().map(|e| e.id.as_str()).collect();
        let catalog: Vec<_> = export_check_catalog().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, catalog.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(l.entries.len(), 27);
        for e in &l.entries {
            assert!(words(&e.name) <= 4 && words(&e.result) <= 5, "{}", e.id);
        }
    }

    #[test]
    fn a_missing_tool_is_a_quiet_suggestion_not_an_error() {
        let l = build(
            ExportCheckProviderStatus::Missing,
            None,
            vec![asset("a1", ReportEvidence::Missing, vec![])],
            ExportCheckPhase::BeforeExport,
        );
        let e = entry(&l, "black_or_frozen");
        assert_eq!(e.state, ExportCheckState::NotChecked);
        assert_eq!(e.reason_code, ExportCheckReason::OptionalToolMissing);
        assert_eq!(
            e.action.as_ref().unwrap().kind,
            ExportCheckActionKind::OpenTools
        );
        assert_eq!(e.coverage.unchecked_assets, vec!["a1".to_string()]);
        // Calculations from the timeline never wait for an optional tool.
        assert_eq!(entry(&l, "empty_tail").state, ExportCheckState::Checked);
    }

    #[test]
    fn a_damaged_installation_is_an_error_with_copyable_details() {
        let err = CutError::new(
            cut_core::error_codes::SIDECAR,
            "Captions tools could not start",
            "Configured interpreter exited with code 1",
        );
        let l = build(
            ExportCheckProviderStatus::Damaged,
            Some(err),
            vec![asset("a1", ReportEvidence::Missing, vec![])],
            ExportCheckPhase::BeforeExport,
        );
        let e = entry(&l, "silent_output");
        assert_eq!(e.state, ExportCheckState::Error);
        assert_eq!(e.reason_code, ExportCheckReason::CheckFailed);
        let act = e.action.as_ref().unwrap();
        assert_eq!(act.kind, ExportCheckActionKind::CopyDetails);
        let text = act.text.as_ref().unwrap();
        assert!(text.contains("silent_output") && text.contains("exited with code 1"));
        assert!(text.contains("Cut: 0.6.122") && text.contains("linux x86_64"));
        assert!(e.report_details.is_some());
    }

    #[test]
    fn a_failed_recorded_attempt_is_an_error_even_when_the_tools_look_ready() {
        let err = CutError::new(cut_core::error_codes::SIDECAR, "analysis crashed", "exit 9");
        let l = build(
            ExportCheckProviderStatus::Ready,
            None,
            vec![asset(
                "a1",
                ReportEvidence::Missing,
                vec![attempt(Some(err), vec![])],
            )],
            ExportCheckPhase::BeforeExport,
        );
        assert_eq!(entry(&l, "black_or_frozen").state, ExportCheckState::Error);
    }

    #[test]
    fn ready_tools_without_a_request_are_not_run_yet_with_check_now() {
        let l = build(
            ExportCheckProviderStatus::Ready,
            None,
            vec![asset("a1", ReportEvidence::Missing, vec![])],
            ExportCheckPhase::BeforeExport,
        );
        let e = entry(&l, "silent_output");
        assert_eq!(e.reason_code, ExportCheckReason::NotRunYet);
        let a = e.action.as_ref().unwrap();
        assert_eq!(a.verb.as_deref(), Some("media.perception"));
        assert_eq!(a.args.as_ref().unwrap()["asset"], "a1");
    }

    #[test]
    fn an_old_report_without_provenance_is_unknown_never_fine() {
        let l = build(
            ExportCheckProviderStatus::Ready,
            None,
            vec![asset(
                "a1",
                ReportEvidence::Loaded {
                    instruments_run: vec![],
                    attempt: None,
                },
                vec![],
            )],
            ExportCheckPhase::BeforeExport,
        );
        assert_eq!(
            entry(&l, "black_or_frozen").reason_code,
            ExportCheckReason::UnknownOlderAnalysis
        );
    }

    #[test]
    fn a_measured_asset_is_checked_and_a_stale_missing_error_does_not_apply() {
        let l = build(
            ExportCheckProviderStatus::Ready,
            None,
            vec![asset(
                "a1",
                ReportEvidence::Loaded {
                    instruments_run: vec!["scenes".into(), "silence".into()],
                    attempt: None,
                },
                vec![attempt(Some(missing_err()), vec![])],
            )],
            ExportCheckPhase::BeforeExport,
        );
        assert_eq!(
            entry(&l, "black_or_frozen").state,
            ExportCheckState::Checked
        );
        assert_eq!(entry(&l, "silent_output").state, ExportCheckState::Checked);
    }

    #[test]
    fn a_finding_wins_the_row_but_the_gap_stays_in_coverage() {
        let direct = BTreeMap::new();
        let on_request = BTreeMap::new();
        let pregate = PregateReport {
            pass: false,
            risks: vec![crate::checks::PregateRisk {
                kind: "black_or_frozen".into(),
                severity: "high".into(),
                detail: "Black from 0:12 to 0:15.".into(),
                suggested_action: String::new(),
                range_ms: Some([12_000, 15_000]),
            }],
            summary: String::new(),
            thresholds: Value::Null,
            perception_assets: 1,
            uninstrumented_assets: vec!["a2".into()],
        };
        let assets = vec![
            asset(
                "a1",
                ReportEvidence::Loaded {
                    instruments_run: vec!["scenes".into()],
                    attempt: None,
                },
                vec![],
            ),
            asset("a2", ReportEvidence::Missing, vec![]),
        ];
        let l = build_export_checks(&ListInput {
            phase: ExportCheckPhase::BeforeExport,
            at_op: "op_1".into(),
            render_id: None,
            env: Env {
                app_version: "x".into(),
                os: "linux".into(),
                arch: "x86_64".into(),
            },
            tools: tools(ExportCheckProviderStatus::Ready, None),
            timeline: TimelineFacts {
                has_picture: true,
                media_clip_count: 2,
            },
            assets: &assets,
            pregate: Some(&pregate),
            before_snapshot: None,
            receipt_checks: &[],
            source_errors: &BTreeMap::new(),
            direct: &direct,
            on_request: &on_request,
            memory: ResourceFact::NotEstimated,
            disk: ResourceFact::NotEstimated,
        });
        let e = entry(&l, "black_or_frozen");
        assert_eq!(e.state, ExportCheckState::Found);
        assert_eq!(e.action.as_ref().unwrap().at_ms, Some(12_000));
        assert_eq!(e.coverage.unchecked_assets, vec!["a2".to_string()]);
        assert_eq!(e.coverage.measured_assets, vec!["a1".to_string()]);
    }

    #[test]
    fn output_checks_wait_for_the_file_before_export_and_use_receipt_rows_after() {
        let before = build(
            ExportCheckProviderStatus::Ready,
            None,
            vec![],
            ExportCheckPhase::BeforeExport,
        );
        for id in ["lufs", "duration_matches_edl", "silence_at_edges"] {
            assert_eq!(
                entry(&before, id).reason_code,
                ExportCheckReason::AfterExport,
                "{id}"
            );
        }
        let rows = vec![
            CheckResult {
                name: "duration_matches_edl".into(),
                pass: true,
                details: json!({}),
                evidence: json!({}),
            },
            CheckResult {
                name: "lufs".into(),
                pass: false,
                details: json!({"status": "unmeasured", "runtime_error": missing_err()}),
                evidence: json!({}),
            },
            CheckResult {
                name: "silence_at_edges".into(),
                pass: false,
                details: json!({"status": "unmeasured", "runtime_error": {
                    "code": "sidecar", "message": "silence run failed", "cause": "exit 3"}}),
                evidence: json!({}),
            },
        ];
        let direct = BTreeMap::new();
        let on_request = BTreeMap::new();
        let l = build_export_checks(&ListInput {
            phase: ExportCheckPhase::FinishedFile,
            at_op: "op_000007".into(),
            render_id: Some("render_001".into()),
            env: Env {
                app_version: "x".into(),
                os: "linux".into(),
                arch: "x86_64".into(),
            },
            tools: tools(ExportCheckProviderStatus::Missing, None),
            timeline: TimelineFacts::default(),
            assets: &[],
            pregate: None,
            before_snapshot: Some(&before),
            receipt_checks: &rows,
            source_errors: &BTreeMap::new(),
            direct: &direct,
            on_request: &on_request,
            memory: ResourceFact::NotEstimated,
            disk: ResourceFact::NotEstimated,
        });
        assert_eq!(
            entry(&l, "duration_matches_edl").state,
            ExportCheckState::Checked
        );
        assert_eq!(
            entry(&l, "lufs").reason_code,
            ExportCheckReason::OptionalToolMissing
        );
        let silence = entry(&l, "silence_at_edges");
        assert_eq!(silence.state, ExportCheckState::Error);
        assert!(silence
            .action
            .as_ref()
            .unwrap()
            .text
            .as_ref()
            .unwrap()
            .contains("exit 3"));
        assert_eq!(entry(&l, "empty_tail").state, ExportCheckState::Checked);
        assert_eq!(l.render_id.as_deref(), Some("render_001"));
        // An export-input check keeps the state captured before the export.
        assert_eq!(entry(&l, "empty_tail"), entry(&before, "empty_tail"));
    }

    #[test]
    fn a_not_installed_run_on_a_damaged_installation_is_an_error_not_a_suggestion() {
        let rows = vec![CheckResult {
            name: "lufs".into(),
            pass: false,
            details: json!({"status": "unmeasured", "runtime_error": missing_err()}),
            evidence: json!({}),
        }];
        let damaged = CutError::new(
            cut_core::error_codes::SIDECAR,
            "Captions tools could not start",
            "Configured interpreter is gone",
        );
        let direct = BTreeMap::new();
        let on_request = BTreeMap::new();
        for (status, error, want) in [
            (
                ExportCheckProviderStatus::Damaged,
                Some(damaged),
                ExportCheckState::Error,
            ),
            (
                ExportCheckProviderStatus::Missing,
                None,
                ExportCheckState::NotChecked,
            ),
            (
                ExportCheckProviderStatus::Ready,
                None,
                ExportCheckState::NotChecked,
            ),
        ] {
            let l = build_export_checks(&ListInput {
                phase: ExportCheckPhase::FinishedFile,
                at_op: "op_1".into(),
                render_id: Some("render_001".into()),
                env: Env {
                    app_version: "x".into(),
                    os: "linux".into(),
                    arch: "x86_64".into(),
                },
                tools: tools(status, error),
                timeline: TimelineFacts::default(),
                assets: &[],
                pregate: None,
                before_snapshot: None,
                receipt_checks: &rows,
                source_errors: &BTreeMap::new(),
                direct: &direct,
                on_request: &on_request,
                memory: ResourceFact::NotEstimated,
                disk: ResourceFact::NotEstimated,
            });
            let e = entry(&l, "lufs");
            assert_eq!(e.state, want, "{status:?}");
            match status {
                ExportCheckProviderStatus::Damaged => {
                    assert!(e
                        .action
                        .as_ref()
                        .unwrap()
                        .text
                        .as_ref()
                        .unwrap()
                        .contains("Configured interpreter is gone"));
                }
                ExportCheckProviderStatus::Missing => {
                    assert_eq!(e.reason_code, ExportCheckReason::OptionalToolMissing)
                }
                _ => assert_eq!(e.reason_code, ExportCheckReason::NotRunYet),
            }
        }
    }

    #[test]
    fn a_placed_source_that_could_not_be_read_is_never_reported_as_fine() {
        let rows = vec![CheckResult {
            name: "sources_decodable".into(),
            pass: true,
            details: json!({}),
            evidence: json!({}),
        }];
        let mut errors = BTreeMap::new();
        errors.insert(
            "a2".to_string(),
            CutError::new(
                cut_core::error_codes::IO,
                "could not read the file",
                "/media/a2.mp4 is gone",
            ),
        );
        let direct = BTreeMap::new();
        let on_request = BTreeMap::new();
        let l = build_export_checks(&ListInput {
            phase: ExportCheckPhase::FinishedFile,
            at_op: "op_1".into(),
            render_id: Some("render_001".into()),
            env: Env {
                app_version: "x".into(),
                os: "linux".into(),
                arch: "x86_64".into(),
            },
            tools: tools(ExportCheckProviderStatus::Ready, None),
            timeline: TimelineFacts::default(),
            assets: &[],
            pregate: None,
            before_snapshot: None,
            receipt_checks: &rows,
            source_errors: &errors,
            direct: &direct,
            on_request: &on_request,
            memory: ResourceFact::NotEstimated,
            disk: ResourceFact::NotEstimated,
        });
        let e = entry(&l, "sources_decodable");
        assert_eq!(e.state, ExportCheckState::Error);
        assert_eq!(e.coverage.unchecked_assets, vec!["a2".to_string()]);
        assert!(e
            .action
            .as_ref()
            .unwrap()
            .text
            .as_ref()
            .unwrap()
            .contains("/media/a2.mp4 is gone"));
    }
}
