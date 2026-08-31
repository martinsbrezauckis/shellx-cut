//! Terminalization and claim-release ordering for the private camera owner.

use super::*;

impl PrivateWindowsCameraProjectOwner {
    pub(super) fn reject_after_terminalization(&mut self, error: CutError) -> CutError {
        self.terminalize_and_release();
        error
    }

    /// Every normal or unwind release passes through this boundary. The only
    /// live-native branch calls the bounded native Cancelled Stop first; its
    /// exact claim is released only after that abort/join attempt returns.
    pub(super) fn terminalize_and_release(&mut self) {
        let terminal = match self.phase.clone() {
            CameraOwnerPhase::Recording => match self.native.abort_for_unwind() {
                Ok(Some(artifact)) => CameraTerminalOutcome::Sealed(artifact),
                Ok(None) => CameraTerminalOutcome::NoMedia,
                Err(_) => CameraTerminalOutcome::AbortFailed,
            },
            CameraOwnerPhase::Reserved | CameraOwnerPhase::NoMedia => {
                CameraTerminalOutcome::NoMedia
            }
            // A normal placement retry already owns a terminal native result.
            CameraOwnerPhase::Sealed(artifact) => CameraTerminalOutcome::Sealed(artifact),
            CameraOwnerPhase::Placed(_) | CameraOwnerPhase::Terminalized(_) => {
                self.release_claim_after_terminalization();
                return;
            }
        };
        // `Terminalized` retains the truthful sealed/no-media result but does
        // not retain a claim: stale/cancel/unwind makes placement retry
        // unavailable, unlike the owner-held `Sealed` placement-retry phase.
        self.phase = CameraOwnerPhase::Terminalized(terminal);
        self.release_claim_after_terminalization();
    }

    pub(super) fn finish_no_media(&mut self) {
        // Reserved means explicit use never admitted a native run, so this is
        // already terminal before releasing the exact claim.
        self.phase = CameraOwnerPhase::NoMedia;
        self.release_claim_after_terminalization();
    }

    pub(super) fn ensure_reservable_phase(&self) -> Result<(), CutError> {
        if matches!(self.phase, CameraOwnerPhase::Reserved) {
            return Ok(());
        }
        Err(CutError::new(
            error_codes::CONFLICT,
            "private camera owner is already terminal or recording",
            "wait for its terminal placement or reserve a new screen capture",
        ))
    }

    pub(super) fn ensure_use_reservation(&self) -> Result<(), CutError> {
        match self.reservation_state()? {
            CameraReservationState::Live => Ok(()),
            CameraReservationState::StopDominant => Err(CutError::new(
                error_codes::CONFLICT,
                "private camera use is blocked by the screen Stop request",
                "allow the exact camera owner to terminalize; do not start a new camera run",
            )),
        }
    }

    /// Unlike `ensure_use_reservation`, Stop accepts the normal stop-dominant
    /// state. Missing/replaced claims still fail as stale/cancelled rather than
    /// being mistaken for a normal terminal authorization.
    pub(super) fn ensure_terminal_reservation(&self) -> Result<CameraReservationState, CutError> {
        self.reservation_state()
    }

    pub(super) fn ensure_claim_held(&self) -> Result<(), CutError> {
        if camera_owner_claim_is_held(&self.camera_claim) {
            return Ok(());
        }
        Err(CutError::new(
            error_codes::CONFLICT,
            "private camera sealed artifact claim is no longer held",
            "do not place a camera artifact after its exact owner was released or replaced",
        ))
    }

    fn reservation_state(&self) -> Result<CameraReservationState, CutError> {
        let current = active_camera_owner_control(&self.camera_claim).ok_or_else(|| {
            CutError::new(
                error_codes::CONFLICT,
                "private camera reservation is stale or cancelled",
                "do not attach a camera sidecar to a missing or replaced screen capture",
            )
        })?;
        if !self.screen_control.same_reservation(&current) {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "private camera reservation is stale or cancelled",
                "do not attach a camera sidecar to a replaced screen capture",
            ));
        }
        if current.stop_requested() {
            Ok(CameraReservationState::StopDominant)
        } else {
            Ok(CameraReservationState::Live)
        }
    }

    fn release_claim_after_terminalization(&mut self) {
        release_camera_owner(&mut self.camera_claim);
    }

    pub(super) fn released_terminal_error(&self) -> CutError {
        let detail = match &self.phase {
            CameraOwnerPhase::Terminalized(CameraTerminalOutcome::Sealed(_)) => {
                "a sealed camera artifact was retained, but its placement claim was released"
            }
            CameraOwnerPhase::Terminalized(CameraTerminalOutcome::NoMedia) => {
                "the camera terminal had no media and its claim was released"
            }
            CameraOwnerPhase::Terminalized(CameraTerminalOutcome::AbortFailed) => {
                "bounded native abort returned an error and its claim was released"
            }
            _ => "the private camera owner is already terminal",
        };
        CutError::new(
            error_codes::CONFLICT,
            "private camera placement retry is no longer admitted",
            detail,
        )
    }
}

impl Drop for PrivateWindowsCameraProjectOwner {
    fn drop(&mut self) {
        // Unwind/drop cannot delegate a live native sidecar to an orphaned
        // claim. `terminalize_and_release` waits through the bounded native
        // abort path before releasing that exact per-capture reservation.
        self.terminalize_and_release();
    }
}
