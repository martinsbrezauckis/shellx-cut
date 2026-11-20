//! report.rs — the `requirements` section of `system.doctor` and the answer
//! for one verb or feature (`system.doctor {feature}`).

use super::state::{evaluate, EngineFacts, State};
use super::{registry, verb_needs, Feature, Registry, Requirement};
use crate::doctor::DoctorReport;
use cut_core::{error_codes, CutError};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

fn state_json(requirement: &Requirement, state: &State) -> Value {
    let mut out = Map::new();
    out.insert("id".into(), json!(requirement.id));
    out.insert("name".into(), json!(requirement.name));
    out.insert("state".into(), json!(state.state));
    if let Some(reason) = &state.reason {
        out.insert("reason".into(), json!(reason));
    }
    if let Some(check) = &state.check {
        out.insert("check".into(), check.clone());
    }
    if !state.satisfied() {
        out.insert("get".into(), requirement.get.clone());
    }
    Value::Object(out)
}

struct Evaluated<'a> {
    registry: &'a Registry,
    states: BTreeMap<&'a str, State>,
}

impl<'a> Evaluated<'a> {
    fn new(report: &DoctorReport, facts: &EngineFacts) -> Self {
        let registry = registry();
        let states = registry
            .requirements
            .iter()
            .map(|item| (item.id.as_str(), evaluate(item, report, facts)))
            .collect();
        Self { registry, states }
    }

    fn entry(&self, id: &str) -> Option<Value> {
        let requirement = self.registry.requirement(id)?;
        Some(state_json(requirement, self.states.get(id)?))
    }

    fn state(&self, id: &str) -> Option<&State> {
        self.states.get(id)
    }

    /// can_run (yes | no | depends) plus what blocks and what is not known here.
    fn verdict(&self, requires: &[String], any_of: &[Vec<String>]) -> Value {
        let mut blocked = Vec::new();
        let mut depends = Vec::new();
        for id in requires {
            match self.state(id) {
                Some(state) if state.blocks() => blocked.extend(self.entry(id)),
                Some(state) if !state.satisfied() => depends.push(json!(id)),
                _ => {}
            }
        }
        if !any_of.is_empty() {
            let set_ok = |set: &Vec<String>| {
                set.iter()
                    .all(|id| self.state(id).is_some_and(State::satisfied))
            };
            let set_blocked = |set: &Vec<String>| {
                set.iter()
                    .any(|id| self.state(id).is_some_and(State::blocks))
            };
            if !any_of.iter().any(set_ok) {
                if any_of.iter().all(set_blocked) {
                    blocked.push(json!({
                        "any_of": any_of.iter().map(|set| set.iter().filter_map(|id| self.entry(id)).collect::<Vec<_>>()).collect::<Vec<_>>(),
                        "reason": "none of the alternatives is ready; one of them is enough",
                    }));
                } else {
                    depends.push(json!({"any_of": any_of}));
                }
            }
        }
        let can_run = if !blocked.is_empty() {
            "no"
        } else if !depends.is_empty() {
            "depends"
        } else {
            "yes"
        };
        json!({"can_run": can_run, "blocked_by": blocked, "depends_on": depends})
    }

    fn optional(&self, items: &[(String, String)]) -> Vec<Value> {
        items
            .iter()
            .filter_map(|(id, effect)| {
                let requirement = self.registry.requirement(id)?;
                let state = self.state(id)?;
                Some(json!({"id": id, "name": requirement.name, "state": state.state, "effect": effect}))
            })
            .collect()
    }
}

/// How many of the feature's verbs need an open project: all | some | none.
fn open_project_need(feature: &Feature) -> &'static str {
    let needs = feature
        .verbs
        .iter()
        .filter(|verb| {
            verb_needs(verb)
                .is_some_and(|needs| needs.requires.iter().any(|id| id == "open_project"))
        })
        .count();
    match needs {
        0 => "none",
        n if n == feature.verbs.len() => "all",
        _ => "some",
    }
}

/// `system.doctor`'s `requirements` section: every tool and condition with
/// its state, where to get it and what uses it, and every feature with
/// whether it can run now and, if not, which requirement blocks it.
pub(crate) fn doctor_section(report: &DoctorReport, facts: &EngineFacts) -> Value {
    let evaluated = Evaluated::new(report, facts);
    let registry = evaluated.registry;
    let tools: Vec<Value> = registry
        .requirements
        .iter()
        .map(|item| {
            let mut entry = state_json(item, &evaluated.states[item.id.as_str()]);
            let needed_for: Vec<&str> = registry
                .features
                .iter()
                .filter(|feature| {
                    feature.requires.contains(&item.id)
                        || feature.any_of.iter().flatten().any(|id| *id == item.id)
                        || feature
                            .verb_extra
                            .values()
                            .flatten()
                            .any(|id| *id == item.id)
                })
                .map(|feature| feature.name.as_str())
                .collect();
            let mut needed_for = needed_for;
            if item.id == "open_project" {
                // Follows each verb's project_state, not the feature lists.
                needed_for.extend(
                    registry
                        .features
                        .iter()
                        .filter(|feature| open_project_need(feature) != "none")
                        .map(|feature| feature.name.as_str()),
                );
            }
            let improves: Vec<&str> = registry
                .features
                .iter()
                .filter(|feature| feature.optional.iter().any(|opt| opt.id == item.id))
                .map(|feature| feature.name.as_str())
                .filter(|name| !needed_for.contains(name))
                .collect();
            if let Value::Object(map) = &mut entry {
                map.insert("kind".into(), json!(item.kind));
                map.insert("what".into(), json!(item.what));
                map.insert("get".into(), item.get.clone());
                map.insert("needed_for".into(), json!(needed_for));
                map.insert("improves".into(), json!(improves));
                if let Some(card) = &item.state.doctor_card {
                    map.insert("doctor_card".into(), json!(card));
                }
                if let Some(prefix) = &item.state.doctor_cards_prefix {
                    map.insert("doctor_cards_prefix".into(), json!(prefix));
                }
                if let Some(license) = &item.license {
                    map.insert("license".into(), license.clone());
                }
                if let Some(platforms) = &item.platforms {
                    map.insert("platforms".into(), json!(platforms));
                }
                if item.per_asset {
                    map.insert("per_asset".into(), json!(true));
                }
            }
            entry
        })
        .collect();
    let features: Vec<Value> = registry
        .features
        .iter()
        .map(|feature| {
            let mut requires = feature.requires.clone();
            let open = open_project_need(feature);
            if open == "all" {
                requires.insert(0, "open_project".into());
            }
            let mut out = evaluated.verdict(&requires, &feature.any_of);
            if let Value::Object(map) = &mut out {
                map.insert("id".into(), json!(feature.id));
                map.insert("name".into(), json!(feature.name));
                map.insert("verbs".into(), json!(feature.verbs));
                map.insert("requires".into(), json!(requires));
                map.insert("open_project".into(), json!(open));
                if !feature.any_of.is_empty() {
                    map.insert("any_of".into(), json!(feature.any_of));
                }
                if !feature.verb_extra.is_empty() {
                    map.insert("verb_extra".into(), json!(feature.verb_extra));
                }
                let optional: Vec<(String, String)> = feature
                    .optional
                    .iter()
                    .map(|item| (item.id.clone(), item.effect.clone()))
                    .collect();
                map.insert("optional".into(), json!(evaluated.optional(&optional)));
                if let Some(note) = &feature.note {
                    map.insert("note".into(), json!(note));
                }
            }
            out
        })
        .collect();
    json!({
        "schema": "shellx-cut/requirements/1",
        "tools": tools,
        "features": features,
    })
}

/// The answer for one verb (`media.transcribe`) or feature id (`transcription`):
/// what it needs, the state of each requirement, whether it can run now and
/// what blocks it, without calling it.
pub(crate) fn feature_check(
    query: &str,
    report: &DoctorReport,
    facts: &EngineFacts,
) -> Result<Value, CutError> {
    let evaluated = Evaluated::new(report, facts);
    let registry = evaluated.registry;
    let (verb, needs) = if let Some(needs) = verb_needs(query) {
        (Some(query), needs)
    } else if let Some(feature) = registry.feature(query) {
        let mut requires = feature.requires.clone();
        if open_project_need(feature) == "all" {
            requires.insert(0, "open_project".into());
        }
        let needs = super::VerbNeeds {
            feature: feature.id.clone(),
            requires,
            any_of: feature.any_of.clone(),
            optional: feature
                .optional
                .iter()
                .map(|item| (item.id.clone(), item.effect.clone()))
                .collect(),
        };
        (None, needs)
    } else {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("'{query}' is neither a verb nor a feature"),
            "feature takes a verb name (e.g. media.transcribe) or a feature id",
        )
        .with_suggested_action(
            "pass a verb name from GET /api/verbs, or a feature id from system.doctor's requirements.features",
        ));
    };
    let feature = registry
        .feature(&needs.feature)
        .expect("verb_needs returns a registered feature");
    let mut out = evaluated.verdict(&needs.requires, &needs.any_of);
    if let Value::Object(map) = &mut out {
        if let Some(verb) = verb {
            map.insert("verb".into(), json!(verb));
        }
        map.insert("feature".into(), json!(feature.id));
        map.insert("feature_name".into(), json!(feature.name));
        map.insert(
            "requires".into(),
            json!(needs
                .requires
                .iter()
                .filter_map(|id| evaluated.entry(id))
                .collect::<Vec<_>>()),
        );
        if !needs.any_of.is_empty() {
            map.insert(
                "any_of".into(),
                json!(needs
                    .any_of
                    .iter()
                    .map(|set| set
                        .iter()
                        .filter_map(|id| evaluated.entry(id))
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>()),
            );
        }
        map.insert(
            "optional".into(),
            json!(evaluated.optional(&needs.optional)),
        );
        if let Some(note) = &feature.note {
            map.insert("note".into(), json!(note));
        }
    }
    Ok(out)
}
