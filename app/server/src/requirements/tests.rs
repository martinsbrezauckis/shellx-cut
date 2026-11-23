use super::state::EngineFacts;
use super::*;
use crate::doctor::{Card, CardStatus, DoctorReport};
use serde_json::json;

fn card(id: &str, status: CardStatus, details: Value) -> Card {
    Card {
        id: id.into(),
        kind: "tool".into(),
        status,
        source: None,
        version: None,
        hint: (status != CardStatus::Ok).then(|| format!("{id} hint")),
        details,
    }
}

fn report(cards: Vec<Card>) -> DoctorReport {
    DoctorReport {
        schema: "test".into(),
        scanned_at: "now".into(),
        os: "linux".into(),
        arch: "x86_64".into(),
        app_version: "test".into(),
        availability: None,
        addr: None,
        cards,
        essential_ok: true,
    }
}

fn facts(open_project: bool) -> EngineFacts {
    EngineFacts {
        open_project,
        export_folder: json!({"usable": true}),
        ui_clients: 0,
        os: "linux".into(),
    }
}

fn ready_machine() -> DoctorReport {
    report(vec![
        card(
            "ffmpeg",
            CardStatus::Ok,
            json!({"can_stabilize": true, "can_burn_captions": true, "can_color_manage": true}),
        ),
        card("ffprobe", CardStatus::Ok, json!({})),
        card(
            "gpu-encode",
            CardStatus::Ok,
            json!({"hardware_available": false, "ffmpeg_missing": false}),
        ),
        card("perception", CardStatus::Ok, json!({"tier": "full-stt"})),
    ])
}

#[test]
fn every_verb_has_exactly_one_requirements_declaration() {
    let registry = registry();
    assert!(
        !registry.features.is_empty(),
        "schema/requirements.json must parse"
    );
    let verbs = crate::registry::VerbRegistry::shared();
    for spec in &verbs.verbs {
        let owners: Vec<&str> = registry
            .features
            .iter()
            .filter(|feature| feature.verbs.contains(&spec.name))
            .map(|feature| feature.id.as_str())
            .collect();
        assert_eq!(
            owners.len(),
            1,
            "{} must be in exactly one feature of schema/requirements.json, found {owners:?}",
            spec.name
        );
    }
    for feature in &registry.features {
        for verb in &feature.verbs {
            assert!(
                verbs.get(verb).is_some(),
                "feature {} names unknown verb {verb}",
                feature.id
            );
        }
        let named = feature
            .requires
            .iter()
            .chain(feature.any_of.iter().flatten())
            .chain(feature.optional.iter().map(|item| &item.id))
            .chain(feature.verb_extra.values().flatten());
        for id in named {
            assert!(
                registry.requirement(id).is_some(),
                "feature {} names unknown requirement {id}",
                feature.id
            );
        }
    }
}

#[test]
fn every_verb_the_ffmpeg_gate_refuses_declares_ffmpeg() {
    for verb in crate::media_tool_gate::MEDIA_TOOL_VERBS {
        let needs = verb_needs(verb).unwrap_or_else(|| panic!("{verb} has no requirements"));
        let named = needs
            .requires
            .iter()
            .chain(needs.optional.iter().map(|(id, _)| id));
        assert!(
            named.into_iter().any(|id| id == "ffmpeg"),
            "{verb} is gated on ffmpeg but does not declare it"
        );
    }
}

#[test]
fn an_open_project_follows_the_verb_contract() {
    assert_eq!(
        verb_needs("media.transcribe")
            .unwrap()
            .requires
            .first()
            .map(String::as_str),
        Some("open_project")
    );
    assert!(!verb_needs("project.create")
        .unwrap()
        .requires
        .iter()
        .any(|id| id == "open_project"));
    assert!(!verb_needs("system.doctor")
        .unwrap()
        .requires
        .iter()
        .any(|id| id == "open_project"));
}

#[test]
fn a_missing_ffmpeg_blocks_transcription_and_names_the_way_forward() {
    let machine = report(vec![
        card("ffmpeg", CardStatus::Missing, json!({})),
        card("ffprobe", CardStatus::Missing, json!({})),
        card("perception", CardStatus::Missing, json!({"tier": "none"})),
    ]);
    let answer = feature_check("media.transcribe", &machine, &facts(true)).expect("known verb");
    assert_eq!(answer["can_run"], "no");
    let blocked: Vec<&str> = answer["blocked_by"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert_eq!(blocked, ["ffmpeg", "captions_runtime", "speech_model"]);
    assert_eq!(answer["blocked_by"][0]["get"]["verb"], "system.fetch_tool");
    assert_eq!(
        answer["blocked_by"][1]["get"]["verb"],
        "system.setup_perception"
    );
}

#[test]
fn a_ready_machine_runs_transcription_and_windows_only_runtime_does_not_apply_on_linux() {
    let answer =
        feature_check("media.transcribe", &ready_machine(), &facts(true)).expect("known verb");
    assert_eq!(answer["can_run"], "yes", "{answer}");
    let vc = answer["requires"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "vc_runtime")
        .unwrap();
    assert_eq!(vc["state"], "not_applicable");
}

#[test]
fn no_open_project_blocks_project_verbs_and_per_asset_needs_are_not_guessed() {
    let answer =
        feature_check("transcript.cut_words", &ready_machine(), &facts(false)).expect("known verb");
    assert_eq!(answer["can_run"], "no");
    assert_eq!(answer["blocked_by"][0]["id"], "open_project");
    let answer =
        feature_check("transcript.cut_words", &ready_machine(), &facts(true)).expect("known verb");
    assert_eq!(
        answer["can_run"], "depends",
        "a transcript is per asset: {answer}"
    );
    assert_eq!(answer["depends_on"], json!(["transcript"]));
}

#[test]
fn a_degraded_ffmpeg_without_vid_stab_blocks_stabilize_only() {
    let mut machine = ready_machine();
    machine.cards[0] = card(
        "ffmpeg",
        CardStatus::Degraded,
        json!({"can_stabilize": false, "can_burn_captions": true, "can_color_manage": true}),
    );
    let stabilize = feature_check("edit.stabilize", &machine, &facts(true)).unwrap();
    assert_eq!(stabilize["can_run"], "no");
    assert_eq!(stabilize["blocked_by"][0]["id"], "ffmpeg_vidstab");
    let split = feature_check("edit.split", &machine, &facts(true)).unwrap();
    assert_eq!(split["can_run"], "yes");
}

#[test]
fn optional_requirements_say_their_effect_and_do_not_block() {
    let answer = feature_check("render.final", &ready_machine(), &facts(true)).unwrap();
    assert_eq!(answer["can_run"], "yes");
    let gpu = answer["optional"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "gpu_encoder")
        .unwrap();
    assert_eq!(gpu["state"], "missing");
    assert!(gpu["effect"].as_str().unwrap().contains("software encoder"));
}

#[test]
fn an_unusable_export_folder_blocks_export_with_its_problem() {
    let mut engine = facts(true);
    engine.export_folder = json!({"usable": false, "problem": "export folder /x does not exist"});
    let answer = feature_check("render.final", &ready_machine(), &engine).unwrap();
    assert_eq!(answer["can_run"], "no");
    let folder = answer["blocked_by"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "export_folder")
        .unwrap();
    assert_eq!(folder["reason"], "export folder /x does not exist");
    assert_eq!(folder["get"]["verb"], "project.set_output_dir");
}

#[test]
fn translation_needs_one_of_its_alternatives() {
    let mut machine = report(vec![card(
        "perception",
        CardStatus::Missing,
        json!({"tier": "none"}),
    )]);
    let answer = feature_check("captions.translate", &machine, &facts(true)).unwrap();
    assert_eq!(
        answer["can_run"], "depends",
        "no chat CLI was checked, so it is not known: {answer}"
    );
    machine.cards.push(card(
        "judge.codex",
        CardStatus::Missing,
        json!({"chat": {"wired": false}}),
    ));
    let answer = feature_check("captions.translate", &machine, &facts(true)).unwrap();
    assert_eq!(
        answer["can_run"], "no",
        "no chat CLI and no captions runtime: {answer}"
    );
    let answer = feature_check("captions.translate", &ready_machine(), &facts(true)).unwrap();
    assert_eq!(
        answer["can_run"], "yes",
        "the local model is enough: {answer}"
    );
}

#[test]
fn signed_in_providers_are_read_from_the_judge_cards() {
    let mut machine = ready_machine();
    machine.cards.push(card("judge.claude", CardStatus::Missing, json!({"judge_ready": false, "availability_reason": "not signed in", "chat": {"wired": false}})));
    let answer = feature_check("verify.judge", &machine, &facts(true)).unwrap();
    assert_eq!(answer["can_run"], "no");
    let judge = answer["blocked_by"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "review_judge")
        .unwrap();
    assert_eq!(judge["reason"], "claude: not signed in");
    machine.cards.last_mut().unwrap().details =
        json!({"judge_ready": true, "chat": {"wired": true}});
    assert_eq!(
        feature_check("agent.chat", &machine, &facts(true)).unwrap()["can_run"],
        "yes"
    );
}

#[test]
fn an_unknown_query_is_refused_with_where_to_find_names() {
    let error = feature_check("no.such", &ready_machine(), &facts(true)).expect_err("unknown");
    assert_eq!(error.code, cut_core::error_codes::INVALID_ARGS);
    assert!(error
        .suggested_action
        .unwrap()
        .contains("requirements.features"));
    assert_eq!(
        feature_check("transcription", &ready_machine(), &facts(true)).unwrap()["feature"],
        "transcription"
    );
}

#[test]
fn the_doctor_section_lists_every_requirement_with_what_uses_it() {
    let section = doctor_section(&ready_machine(), &facts(true));
    let tools = section["tools"].as_array().unwrap();
    assert_eq!(tools.len(), registry().requirements.len());
    let ffmpeg = tools.iter().find(|item| item["id"] == "ffmpeg").unwrap();
    assert_eq!(ffmpeg["state"], "ready");
    assert!(ffmpeg["needed_for"]
        .as_array()
        .unwrap()
        .iter()
        .any(|name| name == "Export and render"));
    let features = section["features"].as_array().unwrap();
    assert_eq!(features.len(), registry().features.len());
    let projects = features
        .iter()
        .find(|item| item["id"] == "projects")
        .unwrap();
    assert_eq!(projects["can_run"], "yes");
    assert_eq!(projects["open_project"], "none");
}

#[tokio::test]
async fn system_doctor_carries_the_section_and_answers_for_one_verb() {
    let state = crate::state::AppState::new();
    let actor = cut_core::Actor::system();
    let full = crate::dispatch::dispatch(&state, "system.doctor", json!({}), actor.clone()).await;
    assert!(full.ok, "{:?}", full.error);
    let section = &full.result.as_ref().unwrap()["requirements"];
    assert_eq!(section["schema"], "shellx-cut/requirements/1");
    assert_eq!(
        section["features"].as_array().unwrap().len(),
        registry().features.len()
    );

    let one = crate::dispatch::dispatch(
        &state,
        "system.doctor",
        json!({"feature": "media.transcribe"}),
        actor.clone(),
    )
    .await;
    let answer = one.result.expect("feature answer");
    assert_eq!(answer["verb"], "media.transcribe");
    assert_eq!(answer["can_run"], "no", "no project is open: {answer}");
    assert_eq!(answer["blocked_by"][0]["id"], "open_project");
    assert_eq!(answer["blocked_by"][0]["get"]["verb"], "project.open");

    let unknown = crate::dispatch::dispatch(
        &state,
        "system.doctor",
        json!({"feature": "no.such"}),
        actor,
    )
    .await;
    assert_eq!(
        unknown.error.expect("refused").code,
        cut_core::error_codes::INVALID_ARGS
    );
}
