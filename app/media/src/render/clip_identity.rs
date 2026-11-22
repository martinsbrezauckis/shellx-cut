//! Render windows also depend on look fields kept on clips, outside the EDL.
use cut_core::{Edl, Project};
use sha2::{Digest, Sha256};

pub(super) fn update(hash: &mut Sha256, project: &Project, edl: &Edl) {
    hash.update(b"clip-records-v3-window-time-published-rect");
    let mut ids: Vec<_> = edl
        .segments
        .iter()
        .filter_map(|s| s.clip_id.as_deref())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        if let Some((track, index)) = project.find_clip(id) {
            // Serialize the entire record: transform.shape/radius/ring, all
            // keyframes, and future look fields must never be silently omitted.
            hash.update(serde_json::to_vec(&project.track(track).unwrap().clips[index]).unwrap());
        }
    }
}
