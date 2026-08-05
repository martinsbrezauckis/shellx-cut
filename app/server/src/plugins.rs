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
//! plugin makes every `plugins.call` under its name fail closed.

use serde::Serialize;
use std::path::PathBuf;

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

// ---------------------------------------------------------------------------
// Enabled-state persistence (default: all enabled)
// ---------------------------------------------------------------------------

fn state_path() -> Option<PathBuf> {
    // Beside the perception app-data dir (one shellx-cut app-data root).
    cut_perception::appdata_sidecar_dir().and_then(|p| p.parent().map(|d| d.join("plugins.json")))
}

/// Names of DISABLED plugins (absent file / parse error → none disabled).
pub fn read_disabled() -> Vec<String> {
    let Some(p) = state_path() else {
        return Vec::new();
    };
    let Ok(txt) = std::fs::read_to_string(p) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else {
        return Vec::new();
    };
    v.get("disabled")
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// True unless the plugin is in the disabled set.
pub fn is_enabled(name: &str) -> bool {
    !read_disabled().iter().any(|n| n == name)
}

/// Enable or disable a plugin (persisted). No-op for an unknown name handled by
/// the caller.
pub fn set_enabled(name: &str, enabled: bool) -> std::io::Result<()> {
    let mut disabled = read_disabled();
    if enabled {
        disabled.retain(|n| n != name);
    } else if !disabled.iter().any(|n| n == name) {
        disabled.push(name.to_string());
    }
    let path = state_path().ok_or_else(|| std::io::Error::other("cannot resolve app-data dir"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = serde_json::json!({ "disabled": disabled });
    std::fs::write(
        path,
        serde_json::to_string_pretty(&body).unwrap_or_default(),
    )
}

/// Manifest + live enabled state, as JSON for `plugins.list`.
pub fn list_json() -> Vec<serde_json::Value> {
    BUILTIN_PLUGINS
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "version": p.version,
                "description": p.description,
                "provides": p.provides,
                "consumes": p.consumes,
                "enabled": is_enabled(p.name),
            })
        })
        .collect()
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
    fn list_has_builtins_enabled_by_default() {
        let l = list_json();
        assert!(l
            .iter()
            .any(|p| p["name"] == "openverse-assets" && p["enabled"] == true));
        assert!(l.len() >= 2);
    }
}
