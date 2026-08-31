//! Atomic project placement and typed Preview playback transport for voiceover.

#[cfg(test)]
use cut_core::MutationRequest;
use cut_core::{
    error_codes, Actor, Asset, AtomicMediaInsert, AtomicMediaInsertResult, CutError, ProjectStore,
    TrackKind,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{
    fingerprint, lifecycle::no_active_take, VoiceoverCaptureControl, VoiceoverMaterialization,
    VoiceoverPreviewBridgeClaim, VoiceoverTimelineOwner, VoiceoverTimelineStatus,
};

const PLACEMENT_DOMAIN: &[u8] = b"shellx-cut/voiceover-placement/1";
const PREVIEW_COMMAND: &str = "preview.voiceover.playback";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VoiceoverPlacement {
    pub(crate) asset_id: String,
    pub(crate) clip_id: String,
    pub(crate) op_id: String,
    pub(crate) already_applied: bool,
}

/// Typed request carried over the existing server-to-UI WebSocket bridge.
/// A future Preview handler must seek/start normal program playback and echo
/// the nested identity payload only after that action has applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VoiceoverPreviewPlaybackRequest {
    request_id: String,
    request_fingerprint: String,
    bridge_epoch: u64,
    accepted_revision: String,
    audio_track: String,
    start_ms: u64,
    out_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VoiceoverPreviewAcknowledgement {
    request_id: String,
    request_fingerprint: String,
    bridge_epoch: u64,
}

impl<C: VoiceoverCaptureControl> VoiceoverTimelineOwner<C> {
    /// Build a typed bridge request after reserving one durable epoch. The
    /// async UI round-trip runs outside the owner lock; the acknowledgement is
    /// later revalidated against this exact persisted claim.
    pub(crate) fn preview_playback_request(
        &mut self,
    ) -> Result<VoiceoverPreviewPlaybackRequest, CutError> {
        let take = self
            .active
            .as_ref()
            .ok_or_else(no_active_take)?
            .take
            .clone();
        let claim = self.claim_preview_bridge()?;
        Ok(VoiceoverPreviewPlaybackRequest::from_claim(take, claim))
    }

    pub(crate) fn playback_started_from_preview(
        &mut self,
        acknowledgement: VoiceoverPreviewAcknowledgement,
    ) -> Result<VoiceoverTimelineStatus, CutError> {
        self.playback_started(
            &acknowledgement.request_id,
            &acknowledgement.request_fingerprint,
            acknowledgement.bridge_epoch,
        )
    }

    /// Commit only the currently sealed, commit-eligible materialization. A
    /// successful retry is recognized before optimistic revision rejection;
    /// a first placement still requires the accepted revision to be current.
    pub(crate) fn place_materialization(
        &mut self,
        store: &mut ProjectStore,
    ) -> Result<VoiceoverPlacement, CutError> {
        let accepted_revision = self
            .active
            .as_ref()
            .ok_or_else(no_active_take)?
            .take
            .revision
            .clone();
        let materialization = self.materialization(&accepted_revision)?;
        let actor = self.placement_actor()?;
        commit_with_actor(store, &materialization, actor)
    }
}

impl VoiceoverPreviewPlaybackRequest {
    fn from_claim(
        take: super::AcceptedVoiceoverTimeline,
        claim: VoiceoverPreviewBridgeClaim,
    ) -> Self {
        Self {
            request_id: claim.request_id,
            request_fingerprint: claim.request_fingerprint,
            bridge_epoch: claim.bridge_epoch,
            accepted_revision: take.revision,
            audio_track: take.audio_track,
            start_ms: take.start_ms,
            out_ms: take.out_ms,
        }
    }

    /// Use the existing correlated UI bridge rather than accepting an
    /// in-process callback. No Preview UI handler is added by this source slice.
    pub(crate) async fn await_ack(
        &self,
        bridge: &crate::ui_bridge::UiBridge,
    ) -> Result<VoiceoverPreviewAcknowledgement, CutError> {
        let reply = bridge
            .request(json!({
                "type": "ui_command",
                "verb": PREVIEW_COMMAND,
                "args": {
                    "request_id": self.request_id,
                    "request_fingerprint": self.request_fingerprint,
                    "bridge_epoch": self.bridge_epoch,
                    "accepted_revision": self.accepted_revision,
                    "audio_track": self.audio_track,
                    "start_ms": self.start_ms,
                    "out_ms": self.out_ms,
                },
            }))
            .await?;
        acknowledgement_from_reply(self, &reply)
    }
}

#[cfg(test)]
/// Commit a sealed materialization once. Kept separate for recovery callers
/// that reopen an exact private session after a process loss.
pub(super) fn commit(
    store: &mut ProjectStore,
    materialization: &VoiceoverMaterialization,
) -> Result<VoiceoverPlacement, CutError> {
    let key = placement_fingerprint(materialization);
    commit_with_actor(
        store,
        materialization,
        placement_actor(materialization, &key),
    )
}

/// The coordinator supplies the original controlled actor for a live take.
/// Keeping that request metadata on the one atomic op makes the normal durable
/// receipt replay path recover a lost terminal response without admitting a
/// second microphone reservation.
pub(super) fn commit_with_actor(
    store: &mut ProjectStore,
    materialization: &VoiceoverMaterialization,
    actor: Actor,
) -> Result<VoiceoverPlacement, CutError> {
    let key = placement_fingerprint(materialization);
    let source = crate::dispatch::verify_attested_media_source(
        &materialization.artifact.path,
        &materialization.artifact.sha256,
        materialization.artifact.bytes,
    )?
    .0;
    let transaction = AtomicMediaInsert {
        idempotency_key: key,
        asset: voiceover_asset(&source, materialization),
        insert: json!({
            "track": materialization.take.audio_track,
            "at_ms": materialization.take.start_ms,
            "src_range_ms": [0, materialization.artifact.duration_ms],
            "ripple": false,
        }),
        binding: placement_binding(materialization),
        track: None,
    };
    if store.log.request_ops(&actor)?.is_none() {
        ensure_current_target(store, materialization)?;
    }
    let result = store.apply_atomic_media_insert(transaction, actor, None)?;
    Ok(placement_result(result))
}

fn ensure_current_target(
    store: &ProjectStore,
    materialization: &VoiceoverMaterialization,
) -> Result<(), CutError> {
    let current = store.log.current_revision()?.unwrap_or_default();
    if current != materialization.take.revision {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "voiceover placement target is stale",
            format!(
                "take accepted '{}' but the current project revision is '{current}'",
                materialization.take.revision
            ),
        )
        .with_suggested_action("discard this take and record again from the current timeline"));
    }
    let track = store
        .project
        .track(&materialization.take.audio_track)
        .ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                "voiceover target audio track no longer exists",
                "refresh the Timeline and record again",
            )
        })?;
    if track.kind != TrackKind::Audio || track.locked {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "voiceover target audio track is no longer eligible",
            "choose an unlocked audio track and record again",
        ));
    }
    Ok(())
}

#[cfg(test)]
fn placement_actor(materialization: &VoiceoverMaterialization, key: &str) -> Actor {
    Actor::system().with_request(MutationRequest {
        caller: "shellx-cut/voiceover-placement/1".into(),
        request_id: materialization.take.request_id.clone(),
        fingerprint: format!("sha256:{key}"),
        expected_revision: Some(materialization.take.revision.clone()),
    })
}

fn voiceover_asset(source: &std::path::Path, materialization: &VoiceoverMaterialization) -> Asset {
    Asset {
        path: source.display().to_string(),
        hash: format!("sha256:{}", materialization.artifact.sha256),
        probe: Some(json!({
            "kind": "audio",
            "duration_ms": materialization.artifact.duration_ms,
            "sample_rate_hz": materialization.artifact.sample_rate_hz,
            "channels": materialization.artifact.channels,
            "has_audio": true,
        })),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    }
}

fn placement_binding(materialization: &VoiceoverMaterialization) -> Value {
    json!({
        "schema": "shellx-cut/voiceover-placement/1",
        "request_id": materialization.take.request_id,
        "request_fingerprint": fingerprint::for_accepted(&materialization.take),
        "accepted_revision": materialization.take.revision,
        "audio_track": materialization.take.audio_track,
        "start_ms": materialization.take.start_ms,
        "out_ms": materialization.take.out_ms,
        "sealed": {
            "sha256": materialization.artifact.sha256,
            "bytes": materialization.artifact.bytes,
            "duration_ms": materialization.artifact.duration_ms,
            "sample_rate_hz": materialization.artifact.sample_rate_hz,
            "channels": materialization.artifact.channels,
            "device_lost_after_prefix": materialization.device_lost_after_prefix,
        },
    })
}

fn placement_fingerprint(materialization: &VoiceoverMaterialization) -> String {
    let mut digest = Sha256::new();
    digest.update(PLACEMENT_DOMAIN);
    hash_text(
        &mut digest,
        &fingerprint::for_accepted(&materialization.take),
    );
    hash_text(&mut digest, &materialization.artifact.sha256);
    digest.update(materialization.artifact.bytes.to_be_bytes());
    digest.update(materialization.artifact.duration_ms.to_be_bytes());
    digest.update(materialization.artifact.sample_rate_hz.to_be_bytes());
    digest.update(materialization.artifact.channels.to_be_bytes());
    digest.update([u8::from(materialization.device_lost_after_prefix)]);
    format!("{:x}", digest.finalize())
}

fn hash_text(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value.as_bytes());
}

fn placement_result(result: AtomicMediaInsertResult) -> VoiceoverPlacement {
    VoiceoverPlacement {
        asset_id: result.asset_id,
        clip_id: result.clip_id,
        op_id: result.op.op_id,
        already_applied: result.already_applied,
    }
}

fn acknowledgement_from_reply(
    request: &VoiceoverPreviewPlaybackRequest,
    reply: &Value,
) -> Result<VoiceoverPreviewAcknowledgement, CutError> {
    if reply.get("applied").and_then(Value::as_bool) != Some(true) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "Preview did not start voiceover playback",
            "recording remains blocked until Preview confirms the exact request",
        ));
    }
    let acknowledgement = reply
        .get("voiceover_playback")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            CutError::new(
                error_codes::CONFLICT,
                "Preview acknowledgement is missing voiceover identity",
                "echo the exact request_id, request_fingerprint, and bridge_epoch",
            )
        })?;
    let request_id = acknowledgement.get("request_id").and_then(Value::as_str);
    let request_fingerprint = acknowledgement
        .get("request_fingerprint")
        .and_then(Value::as_str);
    let bridge_epoch = acknowledgement.get("bridge_epoch").and_then(Value::as_u64);
    if request_id != Some(&request.request_id)
        || request_fingerprint != Some(&request.request_fingerprint)
        || bridge_epoch != Some(request.bridge_epoch)
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "Preview acknowledgement is stale or mismatched",
            "acknowledge the exact latest durable request, fingerprint, and bridge epoch",
        ));
    }
    Ok(VoiceoverPreviewAcknowledgement {
        request_id: request.request_id.clone(),
        request_fingerprint: request.request_fingerprint.clone(),
        bridge_epoch: request.bridge_epoch,
    })
}

#[cfg(test)]
#[path = "placement_tests.rs"]
mod tests;
