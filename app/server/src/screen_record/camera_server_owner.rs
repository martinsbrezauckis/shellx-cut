//! Private project ownership for a reviewed Windows Camera Capture Engine run.
//!
//! This is deliberately not wired to `screen_record.start`, a server verb,
//! Doctor, schemas, or UI. It is the narrow composition that an installed
//! Windows proof may later admit: exact project/revision/capture reservation,
//! explicit opaque-device use, native first-frame admission, one Stop hand-off,
//! and a sealed camera asset plus separately editable hidden video track.

use std::path::PathBuf;

use cut_core::{error_codes, CutError, ProjectStore};
use record_capture::{
    private_windows_camera_owner::{
        PrivateWindowsCameraOwner, PrivateWindowsCameraReadiness, PrivateWindowsCameraUse,
    },
    CaptureClock,
};
use record_core::{CameraArtifact, CameraTerminalState};

use super::capture_registry::{
    active_camera_owner_control, camera_owner_claim_is_held, release_camera_owner,
    reserve_camera_owner, CameraOwnerClaim,
};
use super::capture_session_control::CaptureSessionControl;

const OWNER_SCHEMA: &str = "shellx-cut/private-windows-camera-owner/1";

mod binding;
mod lifecycle;
mod placement;
use binding::{current_revision, missing_capture, project_identity};
use placement::{place_sealed, CameraPlacement};

#[derive(Debug, Clone)]
enum CameraOwnerPhase {
    Reserved,
    Recording,
    /// Retained verbatim until the one durable placement commits. A failed
    /// placement never asks the native runtime to Stop a second time.
    Sealed(CameraArtifact),
    Placed(CameraPlacement),
    NoMedia,
    /// The native terminal outcome is retained for diagnosis, but no project
    /// placement retry remains after stale/cancel/unwind released its claim.
    Terminalized(CameraTerminalOutcome),
}

#[derive(Debug, Clone)]
enum CameraTerminalOutcome {
    NoMedia,
    Sealed(CameraArtifact),
    AbortFailed,
}

/// The normal screen Stop flag is an authorization to terminalize this exact
/// owner, not a stale reservation. A missing/replaced claim remains a separate
/// stale-or-cancelled error path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CameraReservationState {
    Live,
    StopDominant,
}

/// Exact private reservation retained from project admission through one
/// native Stop. It cannot be reconstructed from a late path/capture id.
pub(super) struct PrivateWindowsCameraProjectOwner {
    project_dir: PathBuf,
    project_identity: String,
    accepted_revision: String,
    capture_id: String,
    capture_dir: PathBuf,
    screen_control: CaptureSessionControl,
    screen_clock: CaptureClock,
    camera_claim: CameraOwnerClaim,
    native: PrivateWindowsCameraOwner,
    phase: CameraOwnerPhase,
}

impl PrivateWindowsCameraProjectOwner {
    /// Reserve only an already-live screen capture under the exact durable
    /// project revision. This opens neither the device nor a permission prompt.
    /// The per-capture claim is registered before the native runtime exists.
    pub(super) fn reserve(
        store: &ProjectStore,
        capture_id: &str,
        screen_clock: CaptureClock,
    ) -> Result<Self, CutError> {
        let capture_dir = super::existing_capture_dir(&store.dir, capture_id)?
            .ok_or_else(|| missing_capture(capture_id))?;
        let accepted_revision = current_revision(store)?;
        let project_identity = project_identity(&store.dir)?;
        let (mut camera_claim, screen_control) = reserve_camera_owner(capture_id)?;
        let native = match PrivateWindowsCameraOwner::reserve(&capture_dir, capture_id) {
            Ok(native) => native,
            Err(error) => {
                // No native runtime survived construction, so releasing makes
                // a future exact reservation safe rather than silently held.
                release_camera_owner(&mut camera_claim);
                return Err(super::record_err(error));
            }
        };
        Ok(Self {
            project_dir: store.dir.clone(),
            project_identity,
            accepted_revision,
            capture_id: capture_id.to_owned(),
            capture_dir,
            screen_control,
            screen_clock,
            camera_claim,
            native,
            phase: CameraOwnerPhase::Reserved,
        })
    }

    /// Passive status contains no label or raw Windows identifier and never
    /// calls the permission-capable `use_camera` operation.
    pub(super) fn readiness(
        &mut self,
        store: &ProjectStore,
        opaque_device_id: &str,
    ) -> Result<PrivateWindowsCameraReadiness, CutError> {
        if let Err(error) = self.ensure_project_binding(store) {
            return Err(self.reject_after_terminalization(error));
        }
        if let Err(error) = self.ensure_use_reservation() {
            return Err(self.reject_after_terminalization(error));
        }
        self.ensure_reservable_phase()?;
        Ok(self.native.readiness(opaque_device_id))
    }

    /// Call only from a future explicit, native-proof-qualified Use-camera
    /// action. `Started` means Media Foundation delivered the bounded first
    /// frame; a refusal performs no synthetic capture or retry.
    pub(super) fn use_camera(
        &mut self,
        store: &ProjectStore,
        opaque_device_id: &str,
    ) -> Result<PrivateWindowsCameraUse, CutError> {
        if let Err(error) = self.ensure_project_binding(store) {
            return Err(self.reject_after_terminalization(error));
        }
        if let Err(error) = self.ensure_use_reservation() {
            return Err(self.reject_after_terminalization(error));
        }
        self.ensure_reservable_phase()?;
        let outcome = self
            .native
            .use_camera(
                opaque_device_id,
                &self.screen_clock,
                self.screen_control.stop_signal().as_ref(),
            )
            .map_err(super::record_err)?;
        if outcome == PrivateWindowsCameraUse::Started {
            self.phase = CameraOwnerPhase::Recording;
        }
        Ok(outcome)
    }

    /// Terminalize once under the same project/capture claim. A normal screen
    /// Stop is stop-dominant and is explicitly accepted here; it blocks only
    /// new readiness/use work. A sealed artifact stays retained across a
    /// placement conflict so retry neither reopens the device nor creates a
    /// second receipt. Device loss remains owned by the native terminal state.
    fn stop_and_place(
        &mut self,
        store: &mut ProjectStore,
        requested_terminal: CameraTerminalState,
    ) -> Result<Option<CameraPlacement>, CutError> {
        match &self.phase {
            CameraOwnerPhase::Placed(placement) => return Ok(Some(placement.clone())),
            CameraOwnerPhase::NoMedia => return Ok(None),
            CameraOwnerPhase::Terminalized(_) => return Err(self.released_terminal_error()),
            CameraOwnerPhase::Sealed(_) => {
                if let Err(error) = self.ensure_project_binding(store) {
                    return Err(self.reject_after_terminalization(error));
                }
                if let Err(error) = self.ensure_claim_held() {
                    return Err(self.reject_after_terminalization(error));
                }
                return self.place_retained_artifact(store);
            }
            CameraOwnerPhase::Reserved | CameraOwnerPhase::Recording => {}
        }

        if let Err(error) = self.ensure_project_binding(store) {
            return Err(self.reject_after_terminalization(error));
        }
        let terminal = match self.ensure_terminal_reservation() {
            Ok(terminal) => terminal,
            Err(error) => return Err(self.reject_after_terminalization(error)),
        };
        match terminal {
            CameraReservationState::Live | CameraReservationState::StopDominant => {}
        }
        if matches!(self.phase, CameraOwnerPhase::Reserved) {
            // No explicit-use camera run exists to seal. This is a truthful,
            // idempotent normal Stop outcome, not a native Stop error.
            self.finish_no_media();
            return Ok(None);
        }

        let artifact = match self.native.stop(requested_terminal) {
            Ok(artifact) => artifact,
            Err(error) => return Err(self.reject_after_terminalization(super::record_err(error))),
        };
        let Some(artifact) = artifact else {
            self.finish_no_media();
            return Ok(None);
        };
        self.phase = CameraOwnerPhase::Sealed(artifact);
        let artifact = match &self.phase {
            CameraOwnerPhase::Sealed(artifact) => artifact,
            _ => unreachable!("sealed artifact is retained before validation"),
        };
        if let Err(error) = artifact.validate() {
            return Err(self.reject_after_terminalization(super::record_err(error)));
        }
        self.place_retained_artifact(store)
    }

    fn place_retained_artifact(
        &mut self,
        store: &mut ProjectStore,
    ) -> Result<Option<CameraPlacement>, CutError> {
        let artifact = match &self.phase {
            CameraOwnerPhase::Sealed(artifact) => artifact.clone(),
            _ => unreachable!("sealed placement is only entered with a retained artifact"),
        };
        let placement = place_sealed(self, store, artifact)?;
        self.phase = CameraOwnerPhase::Placed(placement.clone());
        self.terminalize_and_release();
        Ok(Some(placement))
    }

    fn ensure_project_binding(&self, store: &ProjectStore) -> Result<(), CutError> {
        if store.dir != self.project_dir || project_identity(&store.dir)? != self.project_identity {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "private camera project changed after reservation",
                "keep the originally reserved project open until the camera terminal settles",
            ));
        }
        let current = current_revision(store)?;
        if current != self.accepted_revision {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "private camera project revision changed after reservation",
                format!(
                    "camera accepted '{}' but the current project revision is '{current}'",
                    self.accepted_revision
                ),
            ));
        }
        Ok(())
    }
}
