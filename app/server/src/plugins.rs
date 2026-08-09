//! plugins.rs — the tool/plugin API.
//!
//! A plugin is a PERMISSION-SCOPED SUBSET of the verb registry — NOT a parallel
//! API. A plugin manifest declares the verbs it PROVIDES (implements / exposes)
//! and CONSUMES (may call on the host); `plugins.call` is the SCOPED-DISPATCH
//! gateway: it runs a verb UNDER a plugin's identity only if the verb is within
//! that plugin's scope AND the plugin is enabled — so a plugin is "callable only
//! within its scope." The asset providers and the matte runtime are the
//! first capabilities re-expressed this way (each becomes a scoped plugin),
//! proving the model without inventing a second dispatch surface.
//!
//! Enabled state persists in the app-data dir (default: all enabled); disabling a
//! plugin makes every `plugins.call` under its name fail closed. A malformed or
//! unavailable persisted state blocks every plugin until an explicit recovery.

use serde::Serialize;

mod permissions;
#[cfg(test)]
mod permissions_tests;
pub use permissions::PermissionStateProblem;

/// A plugin's declared capability scope.
#[derive(Debug, Clone, Serialize)]
pub struct PluginManifest {
    /// Stable plugin id (the `plugins.call {plugin}` value).
    pub name: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    /// Verb-name patterns the plugin PROVIDES. A trailing `.*` matches a whole
    /// domain (e.g. `assets.*`); otherwise the pattern is an exact verb name.
    pub provides: &'static [&'static str],
    /// Verb-name patterns the plugin may CONSUME (call on the host).
    pub consumes: &'static [&'static str],
}

/// Built-in plugin manifests. The first re-expresses the asset providers as
/// a permission-scoped plugin (the model proof); the second scopes the matte
/// runtime. Future: load third-party manifests from an app-data plugins dir.
pub const BUILTIN_PLUGINS: &[PluginManifest] = &[
    PluginManifest {
        name: "openverse-assets",
        version: "1.0.0",
        description: "Search + fetch Creative-Commons / local media as project assets through a permission-scoped plugin.",
        provides: &["assets.providers", "assets.search", "assets.fetch"],
        consumes: &[],
    },
    PluginManifest {
        name: "matte-runtime",
        version: "1.0.0",
        description: "On-device AI background removal (RVM / MatAnyone2) — the matte capability scoped as a plugin.",
        provides: &["edit.matte", "system.setup_matte"],
        consumes: &[],
    },
];

/// Glob match: an exact verb name, or `domain.*` matching any verb in `domain`.
pub fn pattern_matches(pattern: &str, verb: &str) -> bool {
    if let Some(domain) = pattern.strip_suffix(".*") {
        verb.split('.').next() == Some(domain)
    } else {
        pattern == verb
    }
}

/// Look up a built-in plugin by name.
pub fn find(name: &str) -> Option<&'static PluginManifest> {
    BUILTIN_PLUGINS.iter().find(|p| p.name == name)
}

/// Is `verb` within the plugin's scope (provides ∪ consumes)?
pub fn in_scope(plugin: &PluginManifest, verb: &str) -> bool {
    plugin
        .provides
        .iter()
        .chain(plugin.consumes.iter())
        .any(|p| pattern_matches(p, verb))
}

/// Whether a plugin gateway call may proceed under the persisted permission state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginAccess {
    Enabled,
    Disabled,
    Blocked(PermissionStateProblem),
}

/// Return the current permission decision for one plugin. Corrupt or unavailable
/// state deliberately blocks every plugin instead of guessing a permissive default.
pub fn access(name: &str) -> PluginAccess {
    permissions::current_state().access(name)
}

/// Enable or disable a plugin and atomically persist the decision. When a corrupt
/// state is repaired, only this explicit grant is enabled; every other plugin
/// remains disabled until separately approved.
pub fn set_enabled(name: &str, enabled: bool) -> std::io::Result<permissions::SetEnabled> {
    permissions::set_enabled(name, enabled)
}

/// Manifest + live enabled state, plus a recovery status, as JSON for
/// `plugins.list`.
pub fn list_json() -> serde_json::Value {
    let state = permissions::current_state();
    list_json_for_state(&state)
}

fn list_json_for_state(state: &permissions::PermissionState) -> serde_json::Value {
    let plugins = BUILTIN_PLUGINS
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "version": p.version,
                "description": p.description,
                "provides": p.provides,
                "consumes": p.consumes,
                "enabled": state.access(p.name) == PluginAccess::Enabled,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "plugins": plugins,
        "permission_state": state.status_json(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_matching() {
        assert!(pattern_matches("assets.search", "assets.search"));
        assert!(!pattern_matches("assets.search", "assets.fetch"));
        assert!(pattern_matches("assets.*", "assets.fetch"));
        assert!(pattern_matches("assets.*", "assets.search"));
        assert!(!pattern_matches("assets.*", "project.create"));
        assert!(!pattern_matches("assets.*", "assetsx.search")); // domain boundary
    }

    #[test]
    fn scope_check() {
        let p = find("openverse-assets").expect("builtin present");
        assert!(in_scope(p, "assets.search"));
        assert!(in_scope(p, "assets.fetch"));
        assert!(!in_scope(p, "project.create")); // out of scope
        assert!(!in_scope(p, "edit.matte"));
        let m = find("matte-runtime").expect("builtin present");
        assert!(in_scope(m, "edit.matte"));
        assert!(!in_scope(m, "assets.search"));
        assert!(find("nope").is_none());
    }

    #[test]
    fn corrupt_state_is_visible_and_disables_every_listed_plugin() {
        let state = permissions::PermissionState::Blocked(PermissionStateProblem::Corrupt);
        let listed = list_json_for_state(&state);
        assert_eq!(listed["permission_state"]["status"], "corrupt");
        assert!(listed["permission_state"]["recovery"]
            .as_str()
            .unwrap()
            .contains("plugins.enable"));
        assert!(listed["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .all(|plugin| plugin["enabled"] == false));
    }
}
