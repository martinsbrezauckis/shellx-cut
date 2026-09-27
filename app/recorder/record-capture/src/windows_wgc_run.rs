//! Private WGC checkpoint ownership. It is intentionally not wired to server
//! Pause yet: physical checkpoint rotation is distinct from logical run state.

use std::path::{Path, PathBuf};

use record_core::{error_codes, RecordError, Result};
use record_recovery::{Checkpoint, CheckpointFacts};

#[allow(unused_imports)]
pub(crate) use crate::windows_wgc_run_types::{
    ScreenRunBoundary, ScreenRunIdentity, SealedScreenRun, WgcAcceptedCapture, WgcCaptureRange,
    WgcStartObservation, WgcStartedControl,
};

pub(crate) trait WgcNativeControl {
    /// Stop joins the native WGC worker, so a successful return means its encoder
    /// closed before the checkpoint verifier can observe the staging file.
    fn close(&mut self) -> Result<()>;
}

pub(crate) trait WgcControlFactory<T> {
    type Control: WgcNativeControl;

    /// Each call starts a new WGC control for an exact already-resolved target.
    /// It returns only after that control has accepted exact encoder settings
    /// and any stable monitor range available from the native backend.
    fn start(&mut self, target: &T, staging: &Path) -> Result<WgcStartedControl<Self::Control>>;
}

impl<T, F, C> WgcControlFactory<T> for F
where
    F: FnMut(&T, &Path) -> Result<WgcStartedControl<C>>,
    C: WgcNativeControl,
{
    type Control = C;

    fn start(&mut self, target: &T, staging: &Path) -> Result<WgcStartedControl<Self::Control>> {
        self(target, staging)
    }
}

pub(crate) trait WgcCheckpointPublisher {
    /// Reserve the only open WGC staging path for a new physical checkpoint.
    fn reserve(&mut self, start_ms: u64) -> Result<(u64, PathBuf)>;

    /// A native factory can begin encoding before it returns its control.
    /// Publishers that own that earlier capture clock may use the reserved
    /// boundary; the default preserves the post-open observation contract.
    fn capture_start_ms(&self, _reserved_start_ms: u64, observed_start_ms: u64) -> u64 {
        observed_start_ms
    }

    /// Verify a closed encoder result, then publish it with the no-replace
    /// checkpoint operation. The returned record is the immutable publication fact.
    fn verify_and_publish_new(
        &mut self,
        sequence: u64,
        staging: &Path,
        facts: CheckpointFacts,
    ) -> Result<Checkpoint>;
}

struct ActiveRun<C> {
    sequence: u64,
    staging: PathBuf,
    control: C,
    identity: ScreenRunIdentity,
}

/// Owns one WGC control at a time. One logical recording may rotate multiple
/// ordinary physical checkpoints; those checkpoint generations deliberately do
/// not mirror a later server Pause/Resume generation.
pub(crate) struct WgcRunOwner<T, F, P>
where
    F: WgcControlFactory<T>,
    P: WgcCheckpointPublisher,
{
    target: T,
    factory: F,
    publisher: P,
    active: Option<ActiveRun<F::Control>>,
    next_physical_generation: u64,
    stopped: bool,
}

impl<T, F, P> WgcRunOwner<T, F, P>
where
    F: WgcControlFactory<T>,
    P: WgcCheckpointPublisher,
{
    pub(crate) fn new(target: T, factory: F, publisher: P) -> Self {
        Self {
            target,
            factory,
            publisher,
            active: None,
            next_physical_generation: 0,
            stopped: false,
        }
    }

    /// The observation runs only after the fresh native control accepted the
    /// exact target, settings, and available monitor range.
    pub(crate) fn begin(
        &mut self,
        reserved_start_ms: u64,
        observe_started: impl FnOnce() -> Result<WgcStartObservation>,
    ) -> Result<ScreenRunIdentity> {
        self.begin_current(reserved_start_ms, observe_started)
    }

    /// Resume against a freshly re-resolved target. The caller must obtain it
    /// from an exact native identity; this owner deliberately has no ordinal,
    /// title, primary-display, or geometry fallback.
    #[allow(dead_code)] // Consumed by the private Windows pause-pilot worker.
    pub(crate) fn resume_with_target(
        &mut self,
        target: T,
        reserved_start_ms: u64,
        observe_started: impl FnOnce() -> Result<WgcStartObservation>,
    ) -> Result<ScreenRunIdentity> {
        self.ensure_can_start()?;
        self.target = target;
        self.begin_current(reserved_start_ms, observe_started)
    }

    /// Close, verify, publish, and immediately reopen the next physical WGC
    /// checkpoint. This is ordinary recovery/cadence rotation inside one
    /// logical active recording, not a Pause acknowledgement.
    pub(crate) fn rollover_checkpoint(
        &mut self,
        observe_closed_at: impl FnOnce() -> u64,
        reserve_start_ms: impl FnOnce() -> u64,
        observe_started: impl FnOnce() -> Result<WgcStartObservation>,
    ) -> Result<(SealedScreenRun, ScreenRunIdentity)> {
        let sealed = self.seal_active(observe_closed_at)?;
        // The next native control can encode during start. Reserve its capture
        // boundary only after the previous control has closed and published;
        // a timestamp sampled before close makes the checkpoint facts overlap.
        let reserved_start_ms = reserve_start_ms();
        if reserved_start_ms < sealed.boundary.end_ms {
            self.stopped = true;
            return Err(state_error(
                "next WGC checkpoint starts before the previous one ended",
            ));
        }
        match self.begin(reserved_start_ms, observe_started) {
            Ok(next) => Ok((sealed, next)),
            Err(error) => {
                self.stopped = true;
                Err(error)
            }
        }
    }

    /// Close and immutably seal the active physical checkpoint while the caller
    /// decides whether the logical recording is paused or terminal. This has no
    /// server-generation semantics.
    pub(crate) fn seal_active_checkpoint(
        &mut self,
        observe_closed_at: impl FnOnce() -> u64,
    ) -> Result<SealedScreenRun> {
        if self.stopped {
            return Err(state_error("screen run is terminal after stop"));
        }
        self.seal_active(observe_closed_at)
    }

    /// Close the just-opened native WGC worker without turning its unverified
    /// staging path into a checkpoint. Used when the pilot cannot establish its
    /// required native acceptance evidence after start.
    pub(crate) fn abort_unpublished_start(&mut self) -> Result<()> {
        self.stopped = true;
        let Some(mut active) = self.active.take() else {
            return Ok(());
        };
        active.control.close()
    }

    /// Stop becomes terminal before issuing the physical native close. A failed
    /// close or publication remains terminal, so no later command can replace it.
    pub(crate) fn stop(
        &mut self,
        observe_closed_at: impl FnOnce() -> u64,
    ) -> Result<Option<SealedScreenRun>> {
        if self.stopped {
            return Ok(None);
        }
        self.stopped = true;
        self.active
            .is_some()
            .then(|| self.seal_active(observe_closed_at))
            .transpose()
    }

    #[allow(dead_code)] // Test seam for terminal-state assertions before server wiring.
    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped
    }

    #[cfg_attr(not(all(windows, feature = "capture-windows")), allow(dead_code))]
    pub(crate) fn into_publisher(self) -> P {
        self.publisher
    }

    fn ensure_can_start(&self) -> Result<()> {
        if self.stopped {
            return Err(state_error("screen run is terminal after stop"));
        }
        if self.active.is_some() {
            return Err(state_error("screen run is already active"));
        }
        Ok(())
    }

    fn begin_current(
        &mut self,
        reserved_start_ms: u64,
        observe_started: impl FnOnce() -> Result<WgcStartObservation>,
    ) -> Result<ScreenRunIdentity> {
        self.ensure_can_start()?;
        let physical_generation = self
            .next_physical_generation
            .checked_add(1)
            .ok_or_else(|| state_error("physical WGC checkpoint generation overflowed"))?;
        let (sequence, staging) = self.publisher.reserve(reserved_start_ms)?;
        let mut started = self.factory.start(&self.target, &staging)?;
        let observation = match observe_started() {
            Ok(observation) => observation,
            Err(error) => {
                self.stopped = true;
                let _ = started.control.close();
                return Err(error);
            }
        };
        let start_ms = self
            .publisher
            .capture_start_ms(reserved_start_ms, observation.start_ms);
        if start_ms > observation.start_ms {
            self.stopped = true;
            let _ = started.control.close();
            return Err(state_error(
                "WGC capture boundary follows native start observation",
            ));
        }
        let identity = ScreenRunIdentity {
            physical_generation,
            start_ms,
            started: observation,
            accepted: started.accepted,
        };
        self.active = Some(ActiveRun {
            sequence,
            staging,
            control: started.control,
            identity: identity.clone(),
        });
        self.next_physical_generation = physical_generation;
        Ok(identity)
    }

    fn seal_active(&mut self, observe_closed_at: impl FnOnce() -> u64) -> Result<SealedScreenRun> {
        let start_ms = self
            .active
            .as_ref()
            .ok_or_else(|| state_error("screen run is not active"))?
            .identity
            .start_ms;
        let mut active = self.active.take().expect("active screen run was checked");
        if let Err(error) = active.control.close() {
            self.stopped = true;
            return Err(error);
        }
        // The observer is deliberately invoked only after `close` has joined
        // the native worker. A caller cannot smuggle in a pre-close endpoint.
        let end_ms = observe_closed_at();
        if end_ms <= start_ms {
            self.stopped = true;
            return Err(state_error("screen run boundary is not increasing"));
        }
        let boundary = ScreenRunBoundary {
            physical_generation: active.identity.physical_generation,
            start_ms,
            end_ms,
            event_offset_ms: start_ms,
        };
        let facts = CheckpointFacts {
            start_ms: boundary.start_ms,
            end_ms: boundary.end_ms,
            event_offset_ms: boundary.event_offset_ms,
            audio_offset_ms: None,
        };
        match self
            .publisher
            .verify_and_publish_new(active.sequence, &active.staging, facts)
        {
            Ok(checkpoint) => Ok(SealedScreenRun {
                boundary,
                accepted: active.identity.accepted,
                checkpoint,
            }),
            Err(error) => {
                self.stopped = true;
                Err(error)
            }
        }
    }
}

fn state_error(detail: &str) -> RecordError {
    RecordError::new(
        error_codes::CAPTURE,
        "invalid WGC screen run transition",
        detail,
    )
    .with_action("stop the recording and start a fresh capture")
}
