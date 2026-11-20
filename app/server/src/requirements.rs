//! requirements.rs — what each verb and feature needs, read from one registry.
//!
//! `schema/requirements.json` lists the tools Cut uses and the conditions of
//! the machine and project (FFmpeg, the captions runtime, a signed-in agent
//! CLI, screen permission, an open project, …) and maps every verb to the ones
//! it needs. The engine embeds the same file the documents are generated from,
//! so `system.doctor`'s `requirements` section, the refusals and the agent and
//! person documents cannot drift apart. A verb missing from the registry fails
//! the completeness tests (Node: scripts/public-tests/requirements-contract;
//! Rust: requirements/tests.rs).
//!
//! The state of each requirement is READ, never probed again here: tool
//! states come from the doctor cards (the one detector), engine conditions
//! from the engine's own state, and the rest name the verb that checks them.

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

mod refusal;
#[cfg(test)]
mod refusal_tests;
mod report;
mod state;
#[cfg(test)]
mod tests;

pub(crate) use refusal::{complete, complete_result, refusal};
pub(crate) use report::{doctor_section, feature_check};
pub(crate) use state::EngineFacts;

pub const REQUIREMENTS_JSON: &str = include_str!("../../../schema/requirements.json");

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Registry {
    pub requirements: Vec<Requirement>,
    pub features: Vec<Feature>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Requirement {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub what: String,
    /// The licence of a downloadable tool, {name, url?}, for the agent to tell
    /// the person before it downloads (no acceptance machinery: the agent asks).
    #[serde(default)]
    pub license: Option<Value>,
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
    #[serde(default)]
    pub per_asset: bool,
    pub state: StateSource,
    pub get: Value,
}

/// Where a requirement's state is read (see the registry's `state` keys).
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct StateSource {
    #[serde(default)]
    pub doctor_card: Option<String>,
    #[serde(default)]
    pub doctor_cards_prefix: Option<String>,
    #[serde(default)]
    pub detail_true: Option<String>,
    #[serde(default)]
    pub tier_at_least: Option<String>,
    #[serde(default)]
    pub chat_wired: bool,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub check_verb: Option<String>,
    #[serde(default)]
    pub check_args: Option<Value>,
    #[serde(default)]
    pub per_asset: Option<String>,
    #[serde(default)]
    pub not_checked: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub pending_owner: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Feature {
    pub id: String,
    pub name: String,
    pub verbs: Vec<String>,
    pub requires: Vec<String>,
    #[serde(default)]
    pub any_of: Vec<Vec<String>>,
    #[serde(default)]
    pub optional: Vec<Optional>,
    #[serde(default)]
    pub verb_extra: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Optional {
    pub id: String,
    pub effect: String,
    #[serde(default)]
    pub verbs: Option<Vec<String>>,
}

/// The registry, parsed once. A malformed file is a build-time mistake the
/// tests catch; at run time it degrades to an empty registry rather than
/// taking the engine down.
pub(crate) fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        serde_json::from_str(REQUIREMENTS_JSON).unwrap_or_else(|error| {
            tracing::error!("schema/requirements.json does not parse: {error}");
            Registry {
                requirements: Vec::new(),
                features: Vec::new(),
            }
        })
    })
}

impl Registry {
    pub fn requirement(&self, id: &str) -> Option<&Requirement> {
        self.requirements.iter().find(|item| item.id == id)
    }

    pub fn feature_of_verb(&self, verb: &str) -> Option<&Feature> {
        self.features
            .iter()
            .find(|feature| feature.verbs.iter().any(|name| name == verb))
    }

    pub fn feature(&self, id: &str) -> Option<&Feature> {
        self.features.iter().find(|feature| feature.id == id)
    }
}

/// Everything one verb needs. An open project follows the verb contract
/// (`behavior.project_state: "required"`), not the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerbNeeds {
    pub feature: String,
    pub requires: Vec<String>,
    pub any_of: Vec<Vec<String>>,
    pub optional: Vec<(String, String)>,
}

pub(crate) fn verb_needs(verb: &str) -> Option<VerbNeeds> {
    let registry = registry();
    let feature = registry.feature_of_verb(verb)?;
    let mut requires = Vec::new();
    let project_required =
        cut_core::verb_contract::contract_for_verb(verb).is_some_and(|contract| {
            contract.project_state == cut_core::verb_contract::ProjectState::Required
        });
    if project_required {
        requires.push("open_project".to_string());
    }
    for id in feature
        .requires
        .iter()
        .chain(feature.verb_extra.get(verb).into_iter().flatten())
    {
        if !requires.contains(id) {
            requires.push(id.clone());
        }
    }
    let optional = feature
        .optional
        .iter()
        .filter(|item| {
            item.verbs
                .as_ref()
                .is_none_or(|verbs| verbs.iter().any(|name| name == verb))
        })
        .map(|item| (item.id.clone(), item.effect.clone()))
        .collect();
    Some(VerbNeeds {
        feature: feature.id.clone(),
        requires,
        any_of: feature.any_of.clone(),
        optional,
    })
}
