use super::*;

#[test]
fn schema_classifies_every_registry_verb_and_preserves_the_safe_94_168_split() {
    let registry = crate::registry::VerbRegistry::load();
    let mut allowed = 0;
    let mut denied = 0;
    for verb in &registry.verbs {
        match capability(verb) {
            AgentChatCapability::Inspect | AgentChatCapability::Edit => allowed += 1,
            AgentChatCapability::Deny => denied += 1,
        }
    }
    assert_eq!(allowed, 94, "schema-derived safe capability count");
    assert_eq!(denied, 168, "schema-derived denied capability count");
    assert_eq!(allowed + denied, registry.verbs.len());
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
    // the live media checks still stat registered asset paths and the project
    // reads load its current operation log.
    for name in [
        "project.state",
        "project.sequence_index",
        "project.ops",
        "project.diff",
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
            "captions.save_style",
            "edit.auto_balance",
            "edit.color_match",
            "media.bin_list",
            "media.check",
            "project.diff",
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
