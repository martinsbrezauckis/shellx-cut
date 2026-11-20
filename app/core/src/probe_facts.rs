//! Which media facts and derived pointers of an opened project this engine has
//! established itself.
//!
//! A project can come from someone else. Its `project.json` cache and its
//! `receipts/<id>.probe.json` files carry an asset's probe (kind, sound,
//! picture size, frame rate) and pointers to derived files; a project open
//! accepts the cache when its edit history matches, so those values are
//! whatever the sender wrote. They stay on the asset for display and editing,
//! but a decision that must not be fooled (whether a quality check applies,
//! how much memory a render needs) asks [`Project::verified_probe`]: it answers
//! only for a probe this engine read from the asset's own source in this
//! session (an import, `media.probe`, an asset the engine made, or the re-read
//! after a project open). The record is runtime-only: never saved, not part of
//! project equality, and bound to the asset's content hash and the probe
//! itself, so a later change of either makes the probe unknown again.

use crate::types::{Asset, Project};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Asset id -> digest of (content hash, probe) this engine established.
#[derive(Debug, Clone, Default)]
pub struct ProbeFacts(BTreeMap<String, String>);

/// Runtime state, not project content: two projects that differ only here are equal.
impl PartialEq for ProbeFacts {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl ProbeFacts {
    /// Keep the entries of an earlier state (they still count only while
    /// their asset's hash and probe are unchanged).
    pub fn inherit(&mut self, earlier: &ProbeFacts) {
        for (id, digest) in &earlier.0 {
            self.0.entry(id.clone()).or_insert_with(|| digest.clone());
        }
    }
}

fn digest(asset: &Asset) -> Option<String> {
    let probe = asset.probe.as_ref()?;
    let mut h = Sha256::new();
    h.update(asset.hash.as_bytes());
    h.update(b"\n");
    h.update(serde_json::to_vec(probe).ok()?);
    Some(format!("{:x}", h.finalize()))
}

impl Project {
    /// The asset's probe when this engine established it for the asset's
    /// recorded source content in this session; `None` means unknown (never
    /// waive a check or size memory from it).
    pub fn verified_probe(&self, asset_id: &str) -> Option<&serde_json::Value> {
        let asset = self.assets.get(asset_id)?;
        let recorded = self.probe_facts.0.get(asset_id)?;
        (digest(asset).as_deref() == Some(recorded.as_str()))
            .then_some(asset.probe.as_ref())
            .flatten()
    }

    /// Whether [`Project::verified_probe`] has an answer for this asset.
    pub fn probe_is_verified(&self, asset_id: &str) -> bool {
        self.verified_probe(asset_id).is_some()
    }

    /// The binding of the asset's current probe (its content hash and the
    /// probe itself), for an engine-origin record of having read it.
    pub fn probe_binding(&self, asset_id: &str) -> Option<String> {
        self.assets.get(asset_id).and_then(digest)
    }

    /// Record that this engine has just read the asset's probe from its own
    /// source (or made the asset itself). Engine code only.
    pub fn mark_probe_verified(&mut self, asset_id: &str) {
        if let Some(digest) = self.assets.get(asset_id).and_then(digest) {
            self.probe_facts.0.insert(asset_id.to_string(), digest);
        }
    }
}

/// The only derived pointers an open keeps from a received cache: the
/// deterministic names the engine itself gives an asset's proxy and timeline
/// strip in the project's own folders. Anything else (another asset's file, a
/// path outside, a host-relative URL such as `/host/x.png`) is dropped; the
/// engine makes the file again.
pub(crate) fn keep_own_derived_pointers(project: &mut Project) {
    for (id, asset) in project.assets.iter_mut() {
        if asset.proxy.as_deref() != Some(format!("proxies/{id}.mp4").as_str()) {
            asset.proxy = None;
        }
        if asset.filmstrip.as_deref() != Some(format!("filmstrip/{id}.jpg").as_str()) {
            asset.filmstrip = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ProjectSettings;
    use serde_json::json;

    fn project(probe: serde_json::Value) -> Project {
        let mut project = Project::new("p", ProjectSettings::default());
        project.assets.insert(
            "a1".into(),
            Asset {
                path: "/m/a.mp4".into(),
                hash: "sha256:aa".into(),
                probe: Some(probe),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        project
    }

    #[test]
    fn a_probe_counts_only_when_this_engine_established_it() {
        let mut p = project(json!({"kind": "video", "has_audio": false}));
        assert_eq!(p.verified_probe("a1"), None, "a received probe is unknown");
        p.mark_probe_verified("a1");
        assert!(p.probe_is_verified("a1"));
        // Saved and read back (what an open does): unknown again.
        let reread: Project = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert!(!reread.probe_is_verified("a1"));
        // A changed probe or content hash is unknown again.
        let mut changed = p.clone();
        changed.assets.get_mut("a1").unwrap().probe = Some(json!({"kind": "image"}));
        assert!(!changed.probe_is_verified("a1"));
        let mut rehashed = p.clone();
        rehashed.assets.get_mut("a1").unwrap().hash = "sha256:bb".into();
        assert!(!rehashed.probe_is_verified("a1"));
        // Not part of the project's content.
        assert_eq!(p, reread);
    }

    #[test]
    fn only_the_assets_own_derived_names_are_kept() {
        let mut p = project(json!({}));
        let a = p.assets.get_mut("a1").unwrap();
        a.proxy = Some("proxies/a2.mp4".into());
        a.filmstrip = Some("/attacker.example/seen.png".into());
        keep_own_derived_pointers(&mut p);
        assert_eq!(p.assets["a1"].proxy, None, "another asset's proxy");
        assert_eq!(p.assets["a1"].filmstrip, None, "a host-relative URL");
        let a = p.assets.get_mut("a1").unwrap();
        a.proxy = Some("proxies/a1.mp4".into());
        a.filmstrip = Some("filmstrip/a1.jpg".into());
        keep_own_derived_pointers(&mut p);
        assert_eq!(p.assets["a1"].proxy.as_deref(), Some("proxies/a1.mp4"));
        assert_eq!(
            p.assets["a1"].filmstrip.as_deref(),
            Some("filmstrip/a1.jpg")
        );
    }
}
