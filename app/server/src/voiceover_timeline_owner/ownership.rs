//! Retry and post-placement ownership transitions for Timeline voiceover.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
#[cfg(test)]
use cut_core::Project;
use cut_core::{error_codes, Actor, ActorKind, CutError};
use std::fmt;

#[cfg(test)]
use super::admit;
use super::{
    lifecycle, no_active_take, project_status, AcceptedVoiceoverTimeline, ActiveTake, Phase,
    VoiceoverCaptureControl, VoiceoverTakeSession, VoiceoverTimelineOwner,
    VoiceoverTimelineRequest, VoiceoverTimelineStatus,
};

/// Opaque, server-issued control claim for exactly one active voiceover take.
/// The capability remains memory-only: it is not serializable and never enters
/// a take journal, placement binding, request receipt, or project operation.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct VoiceoverOwnerClaim {
    pub(crate) session_id: String,
    pub(crate) capability: String,
}

impl fmt::Debug for VoiceoverOwnerClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VoiceoverOwnerClaim")
            .field("session_id", &self.session_id)
            .field("capability", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct VoiceoverOwnerActor {
    kind: ActorKind,
    name: String,
    via: String,
    request_id: Option<String>,
    request_fingerprint: Option<String>,
    request_caller: Option<String>,
}

impl VoiceoverOwnerActor {
    fn from_actor(actor: &Actor) -> Self {
        Self {
            kind: actor.kind,
            name: actor.name.clone(),
            via: actor.via.clone(),
            request_id: actor
                .request
                .as_ref()
                .map(|request| request.request_id.clone()),
            request_fingerprint: actor
                .request
                .as_ref()
                .map(|request| request.fingerprint.clone()),
            request_caller: actor.request.as_ref().map(|request| request.caller.clone()),
        }
    }

    fn same_surface(&self, actor: &Actor) -> bool {
        self.kind == actor.kind && self.name == actor.name && self.via == actor.via
    }

    fn same_start_request(&self, actor: &Actor, take: &AcceptedVoiceoverTimeline) -> bool {
        let Some(request) = actor.request.as_ref() else {
            return self.same_surface(actor)
                && self.request_id.is_none()
                && self.request_fingerprint.is_none()
                && self.request_caller.is_none();
        };
        self.same_surface(actor)
            && self.request_id.as_deref() == Some(request.request_id.as_str())
            && self.request_fingerprint.as_deref() == Some(request.fingerprint.as_str())
            && self.request_caller.as_deref() == Some(request.caller.as_str())
            && request.request_id == take.request_id
            && request.expected_revision.as_deref() == Some(take.revision.as_str())
    }
}

/// Private active-session owner. The actor's original controlled request is
/// retained only until terminal placement has written its ordinary durable
/// receipt; the secret never shares that storage boundary.
#[derive(Debug, Clone)]
pub(super) struct VoiceoverOwnerSession {
    claim: VoiceoverOwnerClaim,
    actor: VoiceoverOwnerActor,
}

impl VoiceoverOwnerSession {
    pub(super) fn issue(actor: &Actor, take: &AcceptedVoiceoverTimeline) -> Result<Self, CutError> {
        let owner = Self {
            claim: VoiceoverOwnerClaim {
                session_id: opaque_token(16)?,
                capability: opaque_token(32)?,
            },
            actor: VoiceoverOwnerActor::from_actor(actor),
        };
        // `take` carries the admitted request id/revision; this comparison
        // catches an accidentally unprepared actor before native admission.
        if owner
            .actor
            .request_id
            .as_deref()
            .is_some_and(|id| id != take.request_id)
        {
            return Err(owner_refused());
        }
        Ok(owner)
    }

    pub(super) fn claim(&self) -> VoiceoverOwnerClaim {
        self.claim.clone()
    }

    pub(super) fn authorize(
        &self,
        actor: &Actor,
        claim: &VoiceoverOwnerClaim,
    ) -> Result<(), CutError> {
        if self.actor.same_surface(actor)
            && constant_time_eq(&self.claim.session_id, &claim.session_id)
            && constant_time_eq(&self.claim.capability, &claim.capability)
        {
            return Ok(());
        }
        Err(owner_refused())
    }

    fn authorize_retry(
        &self,
        actor: &Actor,
        take: &AcceptedVoiceoverTimeline,
        owner_session_id: Option<&str>,
    ) -> Result<(), CutError> {
        if self.actor.same_start_request(actor, take)
            && owner_session_id.is_none_or(|id| constant_time_eq(id, &self.claim.session_id))
        {
            return Ok(());
        }
        Err(owner_refused())
    }
}

fn opaque_token(bytes: usize) -> Result<String, CutError> {
    let mut random = vec![0_u8; bytes];
    getrandom::fill(&mut random).map_err(|_| {
        CutError::new(
            error_codes::JOB_FAILED,
            "voiceover owner capability could not be issued",
            "the server cannot safely reserve a microphone session without secure entropy",
        )
    })?;
    Ok(URL_SAFE_NO_PAD.encode(random))
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let mut difference = left.len() ^ right.len();
    for (index, byte) in left.bytes().enumerate() {
        difference |= usize::from(byte ^ right.as_bytes().get(index).copied().unwrap_or_default());
    }
    difference == 0
}

fn owner_refused() -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        "voiceover session is not owned by this caller",
        "return to the Timeline tab that started this take and retry with its active session claim",
    )
}

impl<C: VoiceoverCaptureControl> VoiceoverTimelineOwner<C> {
    #[cfg(test)]
    pub(super) fn accept_bound(
        &mut self,
        project: &Project,
        current_revision: &str,
        request: VoiceoverTimelineRequest,
        capture: C,
        placement_actor: Actor,
    ) -> Result<VoiceoverTimelineStatus, CutError> {
        self.ensure_inactive()?;
        let take = admit(project, current_revision, request)?;
        let owner = VoiceoverOwnerSession::issue(&placement_actor, &take)?;
        Ok(self.install(take, capture, None, owner, placement_actor))
    }

    pub(super) fn install(
        &mut self,
        take: AcceptedVoiceoverTimeline,
        capture: C,
        session: Option<VoiceoverTakeSession>,
        owner: VoiceoverOwnerSession,
        placement_actor: Actor,
    ) -> VoiceoverTimelineStatus {
        self.active = Some(ActiveTake {
            take,
            owner,
            placement_actor,
            capture,
            phase: Phase::AwaitingMicrophone,
            terminal_intent: None,
            session,
        });
        project_status(self.active.as_ref().expect("voiceover take installed"))
    }

    /// Return the opaque claim only for the active in-memory owner. It is
    /// deliberately not serializable or durable; the coordinator projects it
    /// only to the same authenticated caller that received it.
    pub(crate) fn owner_claim(&self) -> Result<VoiceoverOwnerClaim, CutError> {
        Ok(self
            .active
            .as_ref()
            .ok_or_else(no_active_take)?
            .owner
            .claim())
    }

    pub(crate) fn authorize(
        &self,
        actor: &Actor,
        claim: &VoiceoverOwnerClaim,
    ) -> Result<(), CutError> {
        let active = self.active.as_ref().ok_or_else(no_active_take)?;
        active.owner.authorize(actor, claim)
    }

    pub(crate) fn verify_preview_observation(
        &self,
        request_id: &str,
        request_fingerprint: &str,
        bridge_epoch: u64,
    ) -> Result<(), CutError> {
        let active = self.active.as_ref().ok_or_else(no_active_take)?;
        lifecycle::validate_preview_observation(
            active,
            request_id,
            request_fingerprint,
            bridge_epoch,
        )
    }

    /// A caller retry may observe the same live take, but it may never turn a
    /// reused request id into a second microphone reservation. The coordinator
    /// calls this before native admission, while its project/revision guard is
    /// still current.
    pub(crate) fn retry_status(
        &mut self,
        request: &VoiceoverTimelineRequest,
        actor: &Actor,
        owner_session_id: Option<&str>,
    ) -> Result<Option<VoiceoverTimelineStatus>, CutError> {
        let Some(active) = self.active.as_ref() else {
            return Ok(None);
        };
        if active.take.request_id != request.request_id
            || active.take.revision != request.expected_revision
            || active.take.audio_track != request.audio_track
            || active.take.start_ms != request.cue.start_ms()
            || active.take.out_ms != request.cue.out_ms()
        {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "a different voiceover take is already active",
                "wait for it to finish, Stop or Cancel it, then submit a new request id",
            ));
        }
        active
            .owner
            .authorize_retry(actor, &active.take, owner_session_id)?;
        self.status().map(Some)
    }

    /// A committed atomic insert is the terminal ownership boundary. The
    /// durable project op, rather than this volatile capture owner, remains the
    /// retry and Undo authority after the asset plus clip has landed.
    pub(crate) fn release_after_placement(&mut self) -> Result<(), CutError> {
        let active = self.active.as_ref().ok_or_else(no_active_take)?;
        if !matches!(active.phase, Phase::Finished(_)) {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "voiceover take cannot be released before finalization",
                "wait for the sealed WAV to finish before placing it",
            ));
        }
        self.active = None;
        Ok(())
    }

    pub(crate) fn placement_actor(&self) -> Result<Actor, CutError> {
        Ok(self
            .active
            .as_ref()
            .ok_or_else(no_active_take)?
            .placement_actor
            .clone())
    }
}
