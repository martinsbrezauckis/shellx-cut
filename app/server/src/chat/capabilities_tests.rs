use super::*;

#[test]
fn restricted_mcp_marker_requires_the_agent_chat_actor() {
    let marker = Some(RESTRICTED_MCP_MARKER_VALUE);
    assert!(restricted_mcp_environment(
        marker,
        Some("agent:turn-7:agent.chat")
    ));
    assert!(!restricted_mcp_environment(
        None,
        Some("agent:turn-7:agent.chat")
    ));
    assert!(!restricted_mcp_environment(marker, None));
    assert!(!restricted_mcp_environment(marker, Some("human:turn-7")));
    assert!(!restricted_mcp_environment(
        marker,
        Some("agent:turn-7:judge")
    ));
}

#[test]
fn schema_classifies_every_registry_verb_and_requires_review_for_new_safe_capabilities() {
    let registry = crate::registry::VerbRegistry::load();
    let mut allowed = 0;
    let mut denied = 0;
    for verb in &registry.verbs {
        match capability(verb) {
            AgentChatCapability::Inspect | AgentChatCapability::Edit => allowed += 1,
            AgentChatCapability::Deny => denied += 1,
        }
    }
    // The contained broker fails closed for newly registered verbs: a change
    // to this reviewed safe-capability budget requires an explicit policy
    // decision, while new schema verbs classified as deny need no stale count
    // update. The current 308-verb registry therefore remains 103 safe / 205
    // denied without making the growth of the denied surface a magic number.
    assert_eq!(allowed, 103, "reviewed safe capability budget");
    assert_eq!(allowed + denied, registry.verbs.len());
    assert_eq!(denied, registry.verbs.len() - allowed);
}

#[test]
fn prohibited_cut_tools_are_denied_but_marker_edits_are_available() {
    let registry = crate::registry::VerbRegistry::load();
    for denied in [
        "project.open",
        "media.import",
        "assets.search",
        "assets.fetch",
        "export.frame",
        "system.fetch_tool",
        "agent.chat",
        "project.revert",
        // Preset replacement is off the undo cursor; Step back cannot restore it.
        "captions.save_style",
        // This is a Human Record-workspace preference that persists an
        // app-local microphone selection, not a project edit an agent may make.
        "screen_record.microphone_selection",
        // Native preview/rehearsal owns temporary capture bytes and/or a live
        // native source outside the contained agent's reversible project scope.
        "screen_record.preview_capability",
        "screen_record.preview_frame",
        "screen_record.preview_hide",
        "screen_record.preview_pause",
        "screen_record.preview_resume",
        "screen_record.preview_start",
        "screen_record.preview_status",
        "screen_record.preview_stop",
        "screen_record.rehearsal_discard",
        "screen_record.rehearsal_start",
        // Connector capability is host/provider state, not an open-project
        // inspection surface exposed to contained Agent Chat.
        "system.motion_status",
    ] {
        let spec = registry.get(denied).expect("registered denied verb");
        assert_eq!(capability(spec), AgentChatCapability::Deny);
        assert!(!allows(spec));
    }
    let marker = registry
        .get("edit.add_marker")
        .expect("registered marker edit");
    assert_eq!(capability(marker), AgentChatCapability::Edit);
    assert!(allows(marker));
}

#[test]
fn bounded_engine_interactions_stay_truthful_and_independent_of_agent_capability() {
    let registry = crate::registry::VerbRegistry::load();
    // Agent Chat exposure means that the broker may call this constrained Cut
    // handler. It does not mean the handler is pure: both colour helpers
    // resolve registered assets and spawn ffmpeg for a one-frame sample.
    for name in ["edit.color_match", "edit.auto_balance"] {
        let spec = registry.get(name).expect("registered colour helper");
        assert_eq!(capability(spec), AgentChatCapability::Edit);
        assert!(spec.behavior.side_effects.filesystem, "{name}");
        assert!(spec.behavior.side_effects.process, "{name}");
        assert!(!spec.behavior.side_effects.network, "{name}");
    }
    // Inspection is similarly allowed only through the open-project handler;
    // the live media checks and current-index evidence readers still inspect
    // registered project state while project reads load the current op log.
    for name in [
        "project.state",
        "project.sequence_index",
        "project.ops",
        "project.diff",
        "project.cache_preview",
        "project.group_preview",
        "media.check",
        "media.bin_list",
    ] {
        let spec = registry.get(name).expect("registered bounded inspection");
        assert_eq!(capability(spec), AgentChatCapability::Inspect);
        assert!(spec.behavior.side_effects.filesystem, "{name}");
        assert!(!spec.behavior.side_effects.network, "{name}");
    }
    let mut interacting_safe_verbs: Vec<_> = registry
        .verbs
        .iter()
        .filter(|spec| allows(spec))
        .filter(|spec| {
            let side_effects = spec.behavior.side_effects;
            side_effects.filesystem
                || side_effects.process
                || side_effects.network
                || side_effects.ui
        })
        .map(|spec| spec.name.as_str())
        .collect();
    interacting_safe_verbs.sort_unstable();
    assert_eq!(
        interacting_safe_verbs,
        [
            "edit.auto_balance",
            "edit.color_match",
            "inspect.media",
            "inspect.range",
            "media.bin_list",
            "media.check",
            "media.intelligence_search",
            "media.intelligence_status",
            "project.cache_preview",
            "project.diff",
            "project.group_preview",
            "project.health",
            "project.ops",
            "project.sequence_index",
            "project.state",
        ],
        "the Agent Chat allow-list is audited for every direct engine interaction",
    );
    assert!(registry
        .verbs
        .iter()
        .filter(|spec| allows(spec))
        .all(|spec| !spec.behavior.side_effects.network));
}

#[test]
fn workspace_navigation_and_reconciliation_are_not_represented_as_pure_reads() {
    let registry = crate::registry::VerbRegistry::load();
    let open = registry.get("project.open").expect("project.open");
    assert_eq!(capability(open), AgentChatCapability::Deny);
    assert_eq!(
        open.behavior.mutation_class,
        cut_core::MutationClass::Navigation
    );
    assert!(open.behavior.side_effects.filesystem);
    assert_eq!(
        open.behavior.replayability,
        cut_core::Replayability::NotReplayable
    );

    let list = registry.get("project.list").expect("project.list");
    assert_eq!(capability(list), AgentChatCapability::Deny);
    assert_eq!(list.behavior.mutation_class, cut_core::MutationClass::Read);
    assert!(list.behavior.side_effects.filesystem);
    assert_eq!(list.behavior.idempotency, cut_core::Idempotency::Natural);
    assert_eq!(list.behavior.risk, cut_core::VerbRisk::None);
}
