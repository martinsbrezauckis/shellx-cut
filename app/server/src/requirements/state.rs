//! state.rs — the state of one requirement on this machine, read from the
//! doctor cards and the engine's own state. Nothing here starts a process or
//! probes a tool: the doctor scan is the one detector.

use super::Requirement;
use crate::doctor::{Card, CardStatus, DoctorReport};
use serde_json::{json, Value};

/// Engine state the doctor cards do not hold.
#[derive(Debug, Clone, Default)]
pub(crate) struct EngineFacts {
    pub open_project: bool,
    /// `output_paths::output_folder_status()`.
    pub export_folder: Value,
    pub ui_clients: usize,
    pub os: String,
}

/// ready | missing | needs_attention | not_checked | per_asset | not_applicable
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct State {
    pub state: &'static str,
    pub reason: Option<String>,
    /// The verb that checks it when system.doctor does not (permissions,
    /// providers, per-asset facts).
    pub check: Option<Value>,
}

impl State {
    fn new(state: &'static str, reason: Option<String>) -> Self {
        Self {
            state,
            reason,
            check: None,
        }
    }

    /// Ready, or does not apply on this system.
    pub fn satisfied(&self) -> bool {
        matches!(self.state, "ready" | "not_applicable")
    }

    /// Known to block (as opposed to not checked here, or per asset).
    pub fn blocks(&self) -> bool {
        matches!(self.state, "missing" | "needs_attention")
    }
}

const TIERS: [&str; 3] = ["none", "instruments-capable", "full-stt"];

fn card_reason(card: &Card) -> Option<String> {
    card.hint.clone().filter(|hint| !hint.trim().is_empty())
}

fn from_card_status(card: &Card) -> State {
    match card.status {
        CardStatus::Ok => State::new("ready", None),
        CardStatus::Missing => State::new("missing", card_reason(card)),
        CardStatus::Degraded => State::new("needs_attention", card_reason(card)),
        CardStatus::Unknown => State::new(
            "not_checked",
            Some(card_reason(card).unwrap_or_else(|| {
                "the last check did not finish; re-scan with system.doctor {refresh:true}".into()
            })),
        ),
    }
}

fn card<'a>(report: &'a DoctorReport, id: &str) -> Option<&'a Card> {
    report.cards.iter().find(|card| card.id == id)
}

fn detail_flag(card: &Card, key: &str) -> Option<bool> {
    card.details.get(key).and_then(Value::as_bool)
}

fn one_card(requirement: &Requirement, card: &Card) -> State {
    let source = &requirement.state;
    if let Some(key) = &source.detail_true {
        if card.id == "ffmpeg" && card.status == CardStatus::Missing {
            return State::new("missing", Some("FFmpeg is not installed".into()));
        }
        if card.id == "gpu-encode" && detail_flag(card, "ffmpeg_missing") == Some(true) {
            return State::new("missing", Some("FFmpeg is not installed".into()));
        }
        return match detail_flag(card, key) {
            Some(true) => State::new("ready", None),
            Some(false) => State::new(
                "missing",
                card.details
                    .get("hardware_reason_text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| Some(format!("the {} check reports it is not available", card.id))),
            ),
            None => State::new(
                "not_checked",
                Some(format!("the {} check has no answer for this yet", card.id)),
            ),
        };
    }
    if let Some(level) = &source.tier_at_least {
        let tier = card
            .details
            .get("tier")
            .and_then(Value::as_str)
            .unwrap_or("none");
        let have = TIERS.iter().position(|item| *item == tier).unwrap_or(0);
        let want = TIERS
            .iter()
            .position(|item| item == level)
            .unwrap_or(TIERS.len() - 1);
        if have >= want {
            return State::new("ready", None);
        }
        if card.status == CardStatus::Unknown {
            return from_card_status(card);
        }
        let reason = card_reason(card)
            .or_else(|| Some("Captions and transcription are not installed".into()));
        return State::new(
            if have == 0 {
                "missing"
            } else {
                "needs_attention"
            },
            reason,
        );
    }
    from_card_status(card)
}

fn cards_with_prefix(requirement: &Requirement, report: &DoctorReport, prefix: &str) -> State {
    let cards: Vec<&Card> = report
        .cards
        .iter()
        .filter(|card| card.id.starts_with(prefix))
        .collect();
    if cards.is_empty() {
        return State::new("not_checked", Some("no provider was checked".into()));
    }
    let ready = cards.iter().any(|card| {
        if requirement.state.chat_wired {
            card.details.pointer("/chat/wired").and_then(Value::as_bool) == Some(true)
        } else if let Some(key) = &requirement.state.detail_true {
            detail_flag(card, key) == Some(true)
        } else {
            card.status == CardStatus::Ok
        }
    });
    if ready {
        return State::new("ready", None);
    }
    let reasons: Vec<String> = cards
        .iter()
        .map(|card| {
            let why = card
                .details
                .get("availability_reason")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| card_reason(card))
                .unwrap_or_else(|| "not ready".into());
            format!("{}: {why}", card.id.trim_start_matches(prefix))
        })
        .collect();
    State::new("missing", Some(reasons.join("; ")))
}

fn engine_state(kind: &str, facts: &EngineFacts) -> State {
    match kind {
        "open_project" if facts.open_project => State::new("ready", None),
        "open_project" => State::new("missing", Some("no project is open".into())),
        "ui_client" if facts.ui_clients > 0 => State::new("ready", None),
        "ui_client" => State::new("missing", Some("no editor window is connected".into())),
        "export_folder" => {
            if facts.export_folder.get("usable").and_then(Value::as_bool) == Some(false) {
                State::new(
                    "missing",
                    facts
                        .export_folder
                        .get("problem")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                )
            } else {
                State::new("ready", None)
            }
        }
        other => State::new(
            "not_checked",
            Some(format!("unknown engine state '{other}'")),
        ),
    }
}

/// The state of `requirement` on this machine now.
pub(crate) fn evaluate(
    requirement: &Requirement,
    report: &DoctorReport,
    facts: &EngineFacts,
) -> State {
    if let Some(platforms) = &requirement.platforms {
        if !platforms.contains(&facts.os) {
            return State::new(
                "not_applicable",
                Some(format!("only on {}", platforms.join(", "))),
            );
        }
    }
    let source = &requirement.state;
    if let Some(kind) = &source.engine {
        return engine_state(kind, facts);
    }
    if let Some(prefix) = &source.doctor_cards_prefix {
        return cards_with_prefix(requirement, report, prefix);
    }
    if let Some(id) = &source.doctor_card {
        if let Some(found) = card(report, id) {
            return one_card(requirement, found);
        }
        if let Some(owner) = &source.pending_owner {
            return State::new(
                "not_checked",
                Some(format!(
                    "this check is not in this build yet (it comes with {owner})"
                )),
            );
        }
        return State::new(
            "not_checked",
            Some(format!("the doctor has no '{id}' card")),
        );
    }
    if let Some(fact) = &source.per_asset {
        return State {
            state: "per_asset",
            reason: Some(format!("depends on the asset: read {fact}")),
            check: Some(json!({"verb": "project.state"})),
        };
    }
    if let Some(verb) = &source.check_verb {
        return State {
            state: "not_checked",
            reason: source.reason.clone(),
            check: Some(
                json!({"verb": verb, "args": source.check_args.clone().unwrap_or_else(|| json!({}))}),
            ),
        };
    }
    State::new(
        "not_checked",
        source.not_checked.clone().or_else(|| source.reason.clone()),
    )
}
