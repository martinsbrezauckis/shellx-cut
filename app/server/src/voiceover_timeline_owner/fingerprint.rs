//! Canonical private identity for one already-admitted voiceover request.

use sha2::{Digest, Sha256};

use super::admission::AcceptedVoiceoverTimeline;

const DOMAIN: &[u8] = b"shellx-cut/voiceover-request-fingerprint/1";

/// Hash exact admitted fields with length delimiters, so caller input can never
/// select a session directory independently of the accepted Timeline request.
pub(super) fn for_accepted(take: &AcceptedVoiceoverTimeline) -> String {
    let mut digest = Sha256::new();
    digest.update(DOMAIN);
    hash_text(&mut digest, &take.request_id);
    hash_text(&mut digest, &take.revision);
    hash_text(&mut digest, &take.audio_track);
    digest.update(take.start_ms.to_be_bytes());
    match take.out_ms {
        Some(out_ms) => {
            digest.update([1]);
            digest.update(out_ms.to_be_bytes());
        }
        None => digest.update([0]),
    }
    format!("{:x}", digest.finalize())
}

fn hash_text(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value.as_bytes());
}
