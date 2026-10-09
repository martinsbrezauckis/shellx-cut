//! Private lifecycle ownership for one in-process screen capture.
//!
//! The coordinator is deliberately observation-only in this slice: it records
//! preparation, backend start, and terminal stop without exposing pause/resume
//! controls or claiming that any selected stream can be sealed independently.

use super::audio_meters::{AudioMetersStatus, CaptureAudioMeters};
use record_capture::{
    CaptureClock, CaptureControllerPlacement, CaptureReadiness, CaptureReadinessStatus,
    CaptureSourceLifecycle, CaptureSourceLifecycleStatus, PauseStreamCoordinator,
    PrivateSceneCoordinator, PrivateSceneCoordinatorError, PrivateSceneProjection,
    RecordingSceneEngine, RecordingSceneProjection, RecordingSceneTimerAction,
    SelectedCaptureStreams, SessionPhase,
};
use record_core::scene_projection::SceneFixtureDescriptor;
use record_core::{AcceptedStartSnapshot, PresetRevision, SceneEventKind, SceneId};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Read-only internal lifecycle facts for server tests and future wiring.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // The status seam is deliberately private until server wiring consumes it.
pub(crate) struct CaptureSessionStatus {
    pub(crate) phase: SessionPhase,
    pub(crate) selected_streams: SelectedCaptureStreams,
    pub(crate) stop_requested: bool,
}

#[derive(Debug)]
struct CaptureSessionState {
    stop: Arc<AtomicBool>,
    active_preview: record_capture::active_capture_preview::ActiveCapturePreview,
    readiness: CaptureReadiness,
    controller_placement: CaptureControllerPlacement,
    source_lifecycle: CaptureSourceLifecycle,
    audio_meters: CaptureAudioMeters,
    native_launch: Mutex<NativeLaunch>,
    timeline: Mutex<CaptureSceneTimeline>,
    terminal_scene_error: Mutex<Option<PrivateSceneCoordinatorError>>,
    #[cfg(test)]
    scene_append_hook: Mutex<Option<SceneAppendTestHook>>,
    #[cfg(test)]
    terminal_stop_hook: Mutex<Option<TerminalStopTestHook>>,
}

/// One owner lock for logical timestamp issuance and its corresponding private
/// scene receipt append. A later caller can never reserve a timestamp, release
/// the logical clock, and then overtake an earlier receipt write.
#[derive(Debug)]
struct CaptureSceneTimeline {
    coordinator: PauseStreamCoordinator,
    scenes: Option<PrivateSceneCoordinator>,
    recording_scenes: Option<RecordingSceneEngine>,
    recording_scenes_camera_admitted: bool,
    backend_clock: Option<CaptureClock>,
    started_at: Option<Instant>,
    scene_started: bool,
    completed_scene: Option<PrivateSceneProjection>,
    completed_recording_scene: Option<RecordingSceneProjection>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct SceneAppendTestHook {
    pending: Arc<AtomicBool>,
    entered: Arc<std::sync::Barrier>,
    release: Arc<std::sync::Barrier>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct TerminalStopTestHook {
    pending: Arc<AtomicBool>,
    entered: Arc<std::sync::Barrier>,
    release: Arc<std::sync::Barrier>,
}

/// The one-way handoff from the ordinary server worker to a native backend.
///
/// `screen_record.stop` and the worker can race while a queued capture is still
/// preparing.  The lock makes that ordering explicit: a Stop that gets this
/// boundary first prevents the worker from opening a portal/WGC/SCK session or
/// starting its input/audio sidecars.  Once the worker has claimed the handoff,
/// Stop still owns the shared atomic signal used by every existing backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeLaunch {
    Pending,
    Claimed,
    Stopped,
}

/// One accepted screen-record request and its private physical stop signal.
///
/// The state begins in `Preparing`. Only the shared backend clock may advance it
/// to `Recording`; every terminal path transitions it to `Stopped` before waking
/// native workers through `stop`.
#[derive(Debug, Clone)]
pub(crate) struct CaptureSessionControl {
    state: Arc<CaptureSessionState>,
}

/// Keeps a worker-owned control terminal if its capture path unwinds early.
pub(crate) struct CaptureSessionTerminalGuard(CaptureSessionControl);

impl Drop for CaptureSessionTerminalGuard {
    fn drop(&mut self) {
        let _ = self.0.terminalize();
    }
}

impl CaptureSessionControl {
    pub(crate) fn new(
        duration_ms: Option<u64>,
        audio: bool,
        system_audio: bool,
        passive_input_capture_active: bool,
    ) -> Self {
        // Cursor, click, and scroll observation make `InputEvents` a selected
        // stream independently of the key-content permission carried only by
        // `CaptureConfig::capture_keys`. This control must never use key
        // permission to erase passive input from its immutable registration.
        let streams =
            SelectedCaptureStreams::new(audio, system_audio, false, passive_input_capture_active);
        let coordinator =
            PauseStreamCoordinator::new(streams, duration_ms.map(Duration::from_millis));
        let readiness = CaptureReadiness::default();
        let active_preview =
            record_capture::active_capture_preview::ActiveCapturePreview::default();
        #[cfg(not(any(windows, target_os = "macos")))]
        active_preview.refuse_backend_unavailable();
        Self {
            state: Arc::new(CaptureSessionState {
                stop: Arc::new(AtomicBool::new(false)),
                active_preview,
                source_lifecycle: CaptureSourceLifecycle::with_readiness(readiness.clone()),
                readiness,
                controller_placement: CaptureControllerPlacement::new(),
                audio_meters: CaptureAudioMeters::new(audio, system_audio),
                native_launch: Mutex::new(NativeLaunch::Pending),
                timeline: Mutex::new(CaptureSceneTimeline {
                    coordinator,
                    scenes: None,
                    recording_scenes: None,
                    recording_scenes_camera_admitted: false,
                    backend_clock: None,
                    started_at: None,
                    scene_started: false,
                    completed_scene: None,
                    completed_recording_scene: None,
                }),
                terminal_scene_error: Mutex::new(None),
                #[cfg(test)]
                scene_append_hook: Mutex::new(None),
                #[cfg(test)]
                terminal_stop_hook: Mutex::new(None),
            }),
        }
    }

    /// Private sidecars may prove that a registry lookup still names the same
    /// physical screen reservation before borrowing its Stop signal.
    #[allow(
        dead_code,
        reason = "the only consumer is the Windows-private camera owner"
    )]
    pub(crate) fn same_reservation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }

    /// Bind the normal live-capture path to a durable Screen-only scene receipt.
    ///
    /// This accepts no camera layout, no pause command, and no public selection
    /// input. The exact capture directory already exists when this runs.
    #[allow(clippy::too_many_arguments)]
    #[cfg(unix)]
    pub(crate) fn new_live(
        duration_ms: Option<u64>,
        audio: bool,
        system_audio: bool,
        passive_input_capture_active: bool,
        project_dir: &Path,
        capture_id: &str,
    ) -> Result<Self, PrivateSceneCoordinatorError> {
        let root = record_recovery::CaptureRoot::for_project(project_dir).map_err(|error| {
            PrivateSceneCoordinatorError::from_detail(format!(
                "could not admit private scene capture root: {error}"
            ))
        })?;
        let scenes = PrivateSceneCoordinator::create_default_screen_only(&root, capture_id)?;
        Ok(Self::with_scenes(
            duration_ms,
            audio,
            system_audio,
            passive_input_capture_active,
            scenes,
        ))
    }

    /// Bind the normal live-capture owner to a public named scene snapshot.
    /// The engine itself owns the durable header and only accepts timestamps
    /// derived from the shared backend CaptureClock below.
    pub(crate) fn new_recording_scenes_live(
        duration_ms: Option<u64>,
        audio: bool,
        system_audio: bool,
        passive_input_capture_active: bool,
        project_dir: &Path,
        capture_id: &str,
        snapshot: AcceptedStartSnapshot,
        camera_admitted: bool,
    ) -> Result<Self, PrivateSceneCoordinatorError> {
        let root = record_recovery::CaptureRoot::for_project(project_dir).map_err(|error| {
            PrivateSceneCoordinatorError::from_detail(format!(
                "could not admit recording-scenes capture root: {error}"
            ))
        })?;
        let scenes = RecordingSceneEngine::create(&root, capture_id, snapshot)
            .map_err(recording_scene_error)?;
        Ok(Self::with_recording_scenes(
            duration_ms,
            audio,
            system_audio,
            passive_input_capture_active,
            scenes,
            camera_admitted,
        ))
    }

    /// Start one observer for the capture clock that native backend setup owns.
    ///
    /// The observer only reacts to an actual clock origin. A stop while setup is
    /// pending wakes it through the same signal and leaves the control terminal.
    pub(crate) fn observe_backend_start(&self, clock: CaptureClock) -> std::io::Result<()> {
        self.bind_backend_clock(clock.clone())?;
        let control = self.clone();
        std::thread::Builder::new()
            .name("cut-capture-clock-observer".into())
            .spawn(move || {
                let stop = control.stop_signal();
                if let Some(started_at) = clock.wait_started(stop.as_ref()) {
                    control.backend_started_at(started_at);
                }
            })
            .map(|_| ())
    }

    /// Terminalize the lifecycle at a fixed Stop boundary before durable scene
    /// finalization delays physical backend shutdown.
    ///
    /// `PauseStreamCoordinator::stop_at` is idempotent and terminal. Recovering
    /// a poisoned lock preserves its actual coordinator value so neither a failed
    /// observer nor a prior panic can prevent a later stop from taking effect.
    pub(crate) fn terminalize(&self) -> Result<(), PrivateSceneCoordinatorError> {
        // Stop and all server-owned terminal paths are expected closure. A
        // backend callback that arrives after this boundary cannot be mistaken
        // for selected-source disappearance.
        self.state.source_lifecycle.expect_terminal_close();
        self.terminalize_at(Instant::now(), None)
    }

    /// Close the physical capture and seal any public Recording Scenes replay
    /// against the backend's authoritative EventTrack duration. A prior user
    /// Stop may already have closed the lifecycle; that Stop deliberately
    /// defers the public terminal event until this exact media duration exists.
    pub(crate) fn terminalize_after_capture(
        &self,
        media_duration_ms: u64,
    ) -> Result<(), PrivateSceneCoordinatorError> {
        self.state.source_lifecycle.expect_terminal_close();
        self.terminalize_at(Instant::now(), Some(media_duration_ms))
    }

    fn terminalize_at(
        &self,
        at: Instant,
        recording_scene_media_duration_ms: Option<u64>,
    ) -> Result<(), PrivateSceneCoordinatorError> {
        // Readiness is an admission fact, not a historical artifact claim. Close
        // it before exposing the physical Stop signal so a callback racing Stop
        // cannot make a terminal capture ready again.
        self.state.active_preview.terminate();
        self.state.readiness.mark_terminal();
        self.state.audio_meters.mark_terminal();
        // Linearize the native handoff first. A worker that has
        // not yet handed off to its native backend must observe `Stopped` and
        // publish the ordinary terminal failure instead of starting devices
        // after the user asked to stop.
        *self.native_launch() = NativeLaunch::Stopped;
        // This is the physical stop boundary. Do not retain an active capture
        // while private journal fsync runs below. A clock already open before
        // `at` is copied under the lifecycle lock and remains eligible for its
        // terminal projection; a later origin is rejected instead.
        self.state.stop.store(true, Ordering::Release);
        #[cfg(test)]
        self.pause_terminal_after_stop();
        let terminal_scene = {
            let mut timeline = self.timeline();
            // A backend can validly open its clock and return before the
            // detached observer is scheduled. Consume that already-open owner
            // clock while this lifecycle lock is held, before Stop wakes the
            // observer, so TimerStart/TimerEnd and the one projection remain a
            // single terminal transaction.
            let observed_clock_start = timeline
                .backend_clock
                .as_ref()
                .and_then(CaptureClock::started_at);
            if timeline
                .started_at
                .is_some_and(|started_at| started_at > at)
                || observed_clock_start.is_some_and(|started_at| started_at > at)
            {
                // Keep the ordinary lifecycle terminal even though this origin
                // is ineligible for private-scene admission. In particular, do
                // not derive a clamped zero-length TimerEnd from a source that
                // opened after the caller's Stop boundary.
                let _ = timeline.coordinator.stop_at(at);
                Err(PrivateSceneCoordinatorError::from_detail(
                    "backend capture clock opened after the terminal Stop boundary".into(),
                ))
            } else {
                let admitted_start = observed_clock_start
                    .map(|started_at| self.admit_backend_start(&mut timeline, started_at))
                    .transpose();
                if let Err(error) = admitted_start {
                    Err(error)
                } else {
                    let private_logical_media_time_ms =
                        timeline.coordinator.status_at(at).logical_elapsed_ms;
                    let stopped_now = timeline.coordinator.stop_at(at).changed();
                    if !timeline.scene_started {
                        Ok(())
                    } else if let Some(scenes) = timeline.scenes.as_mut() {
                        if !stopped_now {
                            Ok(())
                        } else {
                            let projection = scenes
                                .finish_at(private_logical_media_time_ms)
                                .and_then(|_| {
                                    scenes.completed_projection_at(private_logical_media_time_ms)
                                });
                            if let Ok(projection) = projection.as_ref() {
                                timeline.completed_scene = Some(projection.clone());
                            }
                            projection.map(|_| ())
                        }
                    } else if timeline.recording_scenes.is_some() {
                        if timeline.completed_recording_scene.is_some() {
                            Ok(())
                        } else if let Some(logical_media_time_ms) =
                            recording_scene_media_duration_ms
                        {
                            let observed_duration_ms = capture_clock_ms(&timeline, at)?;
                            if logical_media_time_ms > observed_duration_ms {
                                Err(PrivateSceneCoordinatorError::from_detail(
                                    "the backend media duration exceeds the shared CaptureClock stop boundary"
                                        .into(),
                                ))
                            } else {
                                let scenes = timeline
                                    .recording_scenes
                                    .as_mut()
                                    .expect("checked public scene engine is present");
                                let projection = scenes
                                    .finish_at(logical_media_time_ms)
                                    .and_then(|_| {
                                        scenes.completed_projection_at(logical_media_time_ms)
                                    })
                                    .map_err(recording_scene_error);
                                if let Ok(projection) = projection.as_ref() {
                                    timeline.completed_recording_scene = Some(projection.clone());
                                }
                                projection.map(|_| ())
                            }
                        } else {
                            // `screen_record.stop` owns the physical stop boundary,
                            // but only the backend can report the final EventTrack
                            // duration. Do not journal a later wall-clock TimerEnd
                            // that cannot be embedded in the resulting EditPlan.
                            Ok(())
                        }
                    } else {
                        Ok(())
                    }
                }
            }
        };
        if let Err(error) = terminal_scene {
            self.remember_terminal_scene_error(error);
        }
        self.terminal_scene_result()
    }

    #[cfg(all(test, unix))]
    pub(crate) fn terminalize_at_for_test(
        &self,
        at: Instant,
    ) -> Result<(), PrivateSceneCoordinatorError> {
        self.terminalize_at(at, None)
    }

    #[cfg(all(test, unix))]
    pub(crate) fn terminalize_after_capture_at_for_test(
        &self,
        at: Instant,
        media_duration_ms: u64,
    ) -> Result<(), PrivateSceneCoordinatorError> {
        self.terminalize_at(at, Some(media_duration_ms))
    }

    /// Claim the one native launch handoff for the normal capture worker.
    ///
    /// This is intentionally private: it is not Pause/Resume control.  It
    /// only ensures an already-terminal ordinary session cannot begin native
    /// screen, input, microphone, or system-audio work after Stop won the
    /// queueing race.
    pub(crate) fn claim_native_launch(&self) -> bool {
        let mut launch = self.native_launch();
        if *launch != NativeLaunch::Pending || self.state.stop.load(Ordering::Acquire) {
            return false;
        }
        *launch = NativeLaunch::Claimed;
        true
    }

    /// Ensure unexpected worker unwinding cannot release a nonterminal session.
    pub(crate) fn terminal_guard(&self) -> CaptureSessionTerminalGuard {
        CaptureSessionTerminalGuard(self.clone())
    }

    pub(crate) fn stop_signal(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.state.stop)
    }

    pub(crate) fn active_preview(
        &self,
    ) -> record_capture::active_capture_preview::ActiveCapturePreview {
        self.state.active_preview.clone()
    }

    /// Clone the private proof passed to the active native backend. It has no
    /// independent public writer: only a delivered screen frame may transition
    /// it to ready, and this owner terminalizes it with the Stop boundary.
    pub(crate) fn readiness(&self) -> CaptureReadiness {
        self.state.readiness.clone()
    }

    /// A capture backend receives only this observed-placement handle. It
    /// carries no native controller window or selected-source identity.
    pub(crate) fn controller_placement(&self) -> CaptureControllerPlacement {
        self.state.controller_placement.clone()
    }

    /// A native backend receives only this bounded source-lifecycle handle.
    /// It contains no selected identifier, window handle, or process identity.
    pub(crate) fn source_lifecycle(&self) -> CaptureSourceLifecycle {
        self.state.source_lifecycle.clone()
    }

    pub(crate) fn input_hook_startup(&self) -> record_capture::InputHookStartup {
        self.state.readiness.input_hook_startup()
    }

    pub(crate) fn readiness_status(&self) -> CaptureReadinessStatus {
        self.state.readiness.status()
    }

    pub(crate) fn source_lifecycle_status(&self) -> CaptureSourceLifecycleStatus {
        self.state.source_lifecycle.status()
    }

    pub(crate) fn microphone_meter(&self) -> Option<Arc<record_capture::RollingAudioLevel>> {
        self.state.audio_meters.microphone_meter()
    }

    pub(crate) fn system_audio_meter(&self) -> Option<Arc<record_capture::RollingAudioLevel>> {
        self.state.audio_meters.system_audio_meter()
    }

    pub(crate) fn audio_meters_status(&self) -> AudioMetersStatus {
        self.state.audio_meters.status()
    }

    /// Whether Stop has already become terminal after native launch was
    /// claimed. The ordinary worker checks this again before entering the
    /// backend, narrowing the handoff to the existing atomic-stop contract.
    pub(crate) fn stop_requested(&self) -> bool {
        self.state.stop.load(Ordering::Acquire)
    }

    #[allow(dead_code)] // Read-only internal status for tests and later server wiring.
    pub(crate) fn status(&self) -> CaptureSessionStatus {
        let timeline = self.timeline();
        CaptureSessionStatus {
            phase: timeline.coordinator.phase(),
            selected_streams: timeline.coordinator.selected_streams().clone(),
            stop_requested: self.state.stop.load(Ordering::Acquire),
        }
    }

    pub(crate) fn backend_started_at(&self, started_at: Instant) {
        let scene_start = {
            let mut timeline = self.timeline();
            if self.state.stop.load(Ordering::Acquire) || timeline.coordinator.phase().is_terminal()
            {
                return;
            }
            self.admit_backend_start(&mut timeline, started_at)
        };
        if let Err(error) = scene_start {
            // The live Screen-only receipt is part of this owner. If it cannot
            // durably record its initial logical event, no native backend may
            // continue under a misleading scene state.
            self.remember_terminal_scene_error(error);
            let _ = self.terminalize();
        }
    }

    /// Private future native-owner transaction: issue a logical timestamp and
    /// append its scene event while holding the same coordinator lock. This makes
    /// wall-time pause gaps and reverse concurrent receipt order unrepresentable.
    #[allow(dead_code)]
    pub(crate) fn append_scene_event_at(
        &self,
        kind: SceneEventKind,
        at: Instant,
    ) -> Result<Option<SceneFixtureDescriptor>, PrivateSceneCoordinatorError> {
        let mut timeline = self.timeline();
        if self.state.stop.load(Ordering::Acquire) || timeline.coordinator.phase().is_terminal() {
            return Ok(None);
        }
        let logical_media_time_ms = timeline
            .coordinator
            .timestamp_at(at)
            .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX));
        let Some(logical_media_time_ms) = logical_media_time_ms else {
            return Ok(None);
        };
        let Some(scenes) = timeline.scenes.as_mut() else {
            return Ok(None);
        };
        #[cfg(test)]
        self.pause_scene_append_after_timestamp();
        scenes.append_at(logical_media_time_ms, kind).map(Some)
    }

    /// Activate a public named layout at the current shared CaptureClock time.
    /// No caller-provided timestamp is accepted, so a late server request
    /// cannot invent wall-time or reorder a durable media timeline.
    pub(crate) fn activate_recording_scene(
        &self,
        scene_id: SceneId,
        preset_revision: PresetRevision,
    ) -> Result<(u64, SceneFixtureDescriptor), PrivateSceneCoordinatorError> {
        let mut timeline = self.timeline();
        if self.state.stop.load(Ordering::Acquire) || timeline.coordinator.phase().is_terminal() {
            return Err(PrivateSceneCoordinatorError::from_detail(
                "recording scene capture is no longer active".into(),
            ));
        }
        if !timeline.scene_started {
            return Err(PrivateSceneCoordinatorError::from_detail(
                "recording scene capture has not reached its CaptureClock start boundary".into(),
            ));
        }
        let logical_media_time_ms = capture_clock_ms(&timeline, Instant::now())?;
        let camera_admitted = timeline.recording_scenes_camera_admitted;
        let scenes = timeline.recording_scenes.as_mut().ok_or_else(|| {
            PrivateSceneCoordinatorError::from_detail(
                "this capture has no admitted public Recording Scenes engine".into(),
            )
        })?;
        if scenes
            .activation_requires_camera(&scene_id, preset_revision)
            .map_err(recording_scene_error)?
            && !camera_admitted
        {
            return Err(PrivateSceneCoordinatorError::from_detail(
                "Presenter PiP requires a currently admitted live camera before its scene event can be saved"
                    .into(),
            ));
        }
        let projection = scenes
            .activate_at(logical_media_time_ms, scene_id, preset_revision)
            .map_err(recording_scene_error)?;
        Ok((logical_media_time_ms, projection))
    }

    /// Apply a public timer action at the CaptureClock boundary while holding
    /// the same capture-owner lock as scene activation.
    pub(crate) fn recording_scene_timer_action(
        &self,
        action: RecordingSceneTimerAction,
    ) -> Result<(u64, SceneFixtureDescriptor), PrivateSceneCoordinatorError> {
        let mut timeline = self.timeline();
        if self.state.stop.load(Ordering::Acquire) || timeline.coordinator.phase().is_terminal() {
            return Err(PrivateSceneCoordinatorError::from_detail(
                "recording scene capture is no longer active".into(),
            ));
        }
        if !timeline.scene_started {
            return Err(PrivateSceneCoordinatorError::from_detail(
                "recording scene capture has not reached its CaptureClock start boundary".into(),
            ));
        }
        let logical_media_time_ms = capture_clock_ms(&timeline, Instant::now())?;
        let scenes = timeline.recording_scenes.as_mut().ok_or_else(|| {
            PrivateSceneCoordinatorError::from_detail(
                "this capture has no admitted public Recording Scenes engine".into(),
            )
        })?;
        let projection = scenes
            .timer_action_at(logical_media_time_ms, action)
            .map_err(recording_scene_error)?;
        Ok((logical_media_time_ms, projection))
    }

    /// Return the one terminal private scene projection after the ordinary
    /// capture owner has stopped. A missing value means this platform did not
    /// admit this private receipt or no backend start reached a scene timer.
    pub(crate) fn completed_scene_projection(
        &self,
    ) -> Result<Option<PrivateSceneProjection>, PrivateSceneCoordinatorError> {
        self.terminal_scene_result()?;
        let timeline = self.timeline();
        if timeline.scenes.is_some()
            && timeline.coordinator.phase().is_terminal()
            && timeline.completed_scene.is_none()
        {
            return Err(PrivateSceneCoordinatorError::from_detail(
                "a supported Screen-only capture reached terminalization without a durable scene projection"
                    .into(),
            ));
        }
        Ok(timeline.completed_scene.clone())
    }

    /// Return the terminal public scene replay for the projection owner.  It
    /// is a complete immutable snapshot/events value, never a journal path.
    pub(crate) fn completed_recording_scene_projection(
        &self,
    ) -> Result<Option<RecordingSceneProjection>, PrivateSceneCoordinatorError> {
        self.terminal_scene_result()?;
        let timeline = self.timeline();
        if timeline.recording_scenes.is_some()
            && timeline.coordinator.phase().is_terminal()
            && timeline.completed_recording_scene.is_none()
        {
            return Err(PrivateSceneCoordinatorError::from_detail(
                "a public Recording Scenes capture reached terminalization without a durable projection"
                    .into(),
            ));
        }
        Ok(timeline.completed_recording_scene.clone())
    }

    fn terminal_scene_result(&self) -> Result<(), PrivateSceneCoordinatorError> {
        self.state
            .terminal_scene_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .map_or(Ok(()), Err)
    }

    fn remember_terminal_scene_error(&self, error: PrivateSceneCoordinatorError) {
        let mut stored = self
            .state
            .terminal_scene_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if stored.is_none() {
            *stored = Some(error);
        }
    }

    /// Bind exactly one backend-owned clock before the capture worker can run.
    /// The same clock remains available to terminalization even if its observer
    /// has not received a scheduling slice yet.
    fn bind_backend_clock(&self, clock: CaptureClock) -> std::io::Result<()> {
        let mut timeline = self.timeline();
        if timeline.backend_clock.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "screen capture clock observer is already bound",
            ));
        }
        timeline.backend_clock = Some(clock);
        Ok(())
    }

    /// Start the reducer and its journal while the lifecycle owner lock is
    /// held. This must be used by both the observer and terminalization so a
    /// short capture cannot leave an admitted Screen-only owner without its
    /// initial durable timer event.
    fn admit_backend_start(
        &self,
        timeline: &mut CaptureSceneTimeline,
        started_at: Instant,
    ) -> Result<(), PrivateSceneCoordinatorError> {
        if !timeline.coordinator.start_at(started_at).changed() {
            return Ok(());
        }
        timeline.started_at = Some(started_at);
        let Some(scenes) = timeline.scenes.as_mut() else {
            if timeline.recording_scenes.is_none() {
                return Ok(());
            }
            let logical_media_time_ms = capture_clock_ms(timeline, started_at)?;
            let scenes = timeline
                .recording_scenes
                .as_mut()
                .expect("checked public scene engine is present");
            scenes
                .start_at(logical_media_time_ms)
                .map_err(recording_scene_error)?;
            timeline.scene_started = true;
            return Ok(());
        };
        #[cfg(test)]
        self.pause_scene_append_after_timestamp();
        scenes.append_at(0, SceneEventKind::TimerStart)?;
        timeline.scene_started = true;
        Ok(())
    }

    fn timeline(&self) -> std::sync::MutexGuard<'_, CaptureSceneTimeline> {
        self.state
            .timeline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(all(test, unix))]
    fn bind_backend_clock_for_test(&self, clock: CaptureClock) {
        self.bind_backend_clock(clock).unwrap();
    }

    #[cfg(unix)]
    fn with_scenes(
        duration_ms: Option<u64>,
        audio: bool,
        system_audio: bool,
        passive_input_capture_active: bool,
        scenes: PrivateSceneCoordinator,
    ) -> Self {
        let control = Self::new(
            duration_ms,
            audio,
            system_audio,
            passive_input_capture_active,
        );
        control.timeline().scenes = Some(scenes);
        control
    }

    fn with_recording_scenes(
        duration_ms: Option<u64>,
        audio: bool,
        system_audio: bool,
        passive_input_capture_active: bool,
        scenes: RecordingSceneEngine,
        camera_admitted: bool,
    ) -> Self {
        let control = Self::new(
            duration_ms,
            audio,
            system_audio,
            passive_input_capture_active,
        );
        {
            let mut timeline = control.timeline();
            timeline.recording_scenes = Some(scenes);
            timeline.recording_scenes_camera_admitted = camera_admitted;
        }
        control
    }

    #[cfg(all(test, unix))]
    fn install_scene_append_hook(
        &self,
        entered: Arc<std::sync::Barrier>,
        release: Arc<std::sync::Barrier>,
    ) {
        *self
            .state
            .scene_append_hook
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(SceneAppendTestHook {
            pending: Arc::new(AtomicBool::new(true)),
            entered,
            release,
        });
    }

    #[cfg(test)]
    fn pause_scene_append_after_timestamp(&self) {
        let hook = self
            .state
            .scene_append_hook
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            if hook.pending.swap(false, Ordering::AcqRel) {
                hook.entered.wait();
                hook.release.wait();
            }
        }
    }

    #[cfg(all(test, unix))]
    fn install_terminal_stop_hook(
        &self,
        entered: Arc<std::sync::Barrier>,
        release: Arc<std::sync::Barrier>,
    ) {
        *self
            .state
            .terminal_stop_hook
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(TerminalStopTestHook {
            pending: Arc::new(AtomicBool::new(true)),
            entered,
            release,
        });
    }

    #[cfg(test)]
    fn pause_terminal_after_stop(&self) {
        let hook = self
            .state
            .terminal_stop_hook
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            if hook.pending.swap(false, Ordering::AcqRel) {
                hook.entered.wait();
                hook.release.wait();
            }
        }
    }

    fn native_launch(&self) -> std::sync::MutexGuard<'_, NativeLaunch> {
        self.state
            .native_launch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn capture_clock_ms(
    timeline: &CaptureSceneTimeline,
    at: Instant,
) -> Result<u64, PrivateSceneCoordinatorError> {
    let elapsed = timeline
        .backend_clock
        .as_ref()
        .and_then(|clock| clock.elapsed_at(at))
        .ok_or_else(|| {
            PrivateSceneCoordinatorError::from_detail(
                "the shared CaptureClock has not started at this scene boundary".into(),
            )
        })?;
    u64::try_from(elapsed.as_millis()).map_err(|_| {
        PrivateSceneCoordinatorError::from_detail(
            "the shared CaptureClock timestamp exceeds recording scene limits".into(),
        )
    })
}

fn recording_scene_error(
    error: record_capture::RecordingSceneEngineError,
) -> PrivateSceneCoordinatorError {
    PrivateSceneCoordinatorError::from_detail(format!(
        "public Recording Scenes engine rejected the operation: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::CaptureSessionControl;
    #[cfg(unix)]
    use record_capture::PrivateSceneCoordinator;
    use record_capture::{CaptureClock, RecordingStream, SessionPhase};
    #[cfg(unix)]
    use record_core::scene_projection::{
        EditableSceneTimeline, SceneFixtureDescriptor, SceneTimerFixture,
    };
    #[cfg(unix)]
    use record_core::{EditPlan, SceneComposition, SceneEventKind, SceneId, TimerPhase};
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::sync::{mpsc, Arc, Barrier};
    use std::time::{Duration, Instant};

    fn wait_for_phase(control: &CaptureSessionControl, expected: SessionPhase) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while control.status().phase != expected {
            assert!(
                Instant::now() < deadline,
                "lifecycle did not reach {expected:?}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn capture_session_control_waits_for_the_shared_backend_clock() {
        let control = CaptureSessionControl::new(Some(1_000), true, true, true);
        let clock = CaptureClock::new();
        control.observe_backend_start(clock.clone()).unwrap();

        let preparing = control.status();
        assert_eq!(preparing.phase, SessionPhase::Preparing);
        assert_eq!(
            preparing.selected_streams.streams(),
            &[
                RecordingStream::ScreenVideo,
                RecordingStream::MicrophoneAudio,
                RecordingStream::SystemAudio,
                RecordingStream::InputEvents,
            ]
        );

        clock.start();
        wait_for_phase(&control, SessionPhase::Recording);
    }

    #[test]
    fn capture_session_control_selects_the_explicit_passive_input_stream() {
        let control = CaptureSessionControl::new(None, false, false, true);

        assert_eq!(
            control.status().selected_streams.streams(),
            &[RecordingStream::ScreenVideo, RecordingStream::InputEvents]
        );
    }

    #[test]
    fn capture_session_control_does_not_claim_input_when_passive_capture_is_disabled() {
        let control = CaptureSessionControl::new(None, false, false, false);

        assert_eq!(
            control.status().selected_streams.streams(),
            &[RecordingStream::ScreenVideo]
        );
    }

    #[test]
    fn capture_session_control_stop_is_terminal_and_idempotent_before_late_start() {
        let control = CaptureSessionControl::new(None, false, false, false);
        let clock = CaptureClock::new();
        control.observe_backend_start(clock.clone()).unwrap();

        control.terminalize().unwrap();
        control.terminalize().unwrap();
        clock.start();
        std::thread::sleep(Duration::from_millis(30));

        let status = control.status();
        assert_eq!(status.phase, SessionPhase::Stopped);
        assert!(status.stop_requested);
        assert_eq!(
            status.selected_streams.streams(),
            &[RecordingStream::ScreenVideo]
        );
    }

    #[test]
    fn capture_session_control_stop_wins_before_native_launch_is_claimed() {
        let control = CaptureSessionControl::new(None, true, true, true);

        control.terminalize().unwrap();

        assert!(control.stop_requested());
        assert!(
            !control.claim_native_launch(),
            "a terminal session must never open the normal native capture path"
        );
        let status = control.status();
        assert_eq!(status.phase, SessionPhase::Stopped);
        assert_eq!(
            status.selected_streams.streams(),
            &[
                RecordingStream::ScreenVideo,
                RecordingStream::MicrophoneAudio,
                RecordingStream::SystemAudio,
                RecordingStream::InputEvents,
            ]
        );
    }

    #[test]
    fn capture_session_control_stop_after_native_launch_keeps_stop_dominant() {
        let control = CaptureSessionControl::new(None, false, false, false);

        assert!(control.claim_native_launch());
        control.terminalize().unwrap();

        assert!(control.stop_requested());
        assert_eq!(control.status().phase, SessionPhase::Stopped);
        assert!(
            !control.claim_native_launch(),
            "a terminal session cannot be re-opened by a later worker handoff"
        );
    }

    #[test]
    fn capture_session_control_terminalizes_after_recording_for_every_completion_path() {
        let control = CaptureSessionControl::new(None, false, false, false);
        let clock = CaptureClock::new();
        control.observe_backend_start(clock.clone()).unwrap();
        clock.start();
        wait_for_phase(&control, SessionPhase::Recording);

        // The worker invokes this same terminal operation after either a normal
        // backend return or a backend error, before finalization continues.
        control.terminalize().unwrap();
        assert_eq!(control.status().phase, SessionPhase::Stopped);
        assert!(control.status().stop_requested);
    }

    #[test]
    fn capture_session_control_stop_survives_a_poisoned_lifecycle_lock() {
        let control = CaptureSessionControl::new(None, false, false, false);
        let state = control.state.clone();
        let _ = std::panic::catch_unwind(move || {
            let _timeline = state.timeline.lock().unwrap();
            panic!("deliberately poison the lifecycle mutex");
        });

        control.terminalize().unwrap();
        let status = control.status();
        assert_eq!(status.phase, SessionPhase::Stopped);
        assert!(status.stop_requested);
    }

    #[cfg(unix)]
    #[test]
    fn live_control_binds_the_private_screen_scene_receipt_to_backend_time() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project.cutproj");
        fs::create_dir(&project).unwrap();
        let root = record_recovery::CaptureRoot::for_project(&project).unwrap();
        root.create_capture_dir("scene-live-control").unwrap();
        let control = CaptureSessionControl::new_live(
            None,
            false,
            false,
            false,
            &project,
            "scene-live-control",
        )
        .unwrap();
        let origin = Instant::now();

        control.backend_started_at(origin);
        assert_eq!(control.status().phase, SessionPhase::Recording);
        control.terminalize_at_for_test(origin).unwrap();
        assert_eq!(control.status().phase, SessionPhase::Stopped);
        drop(control);

        let reopened = PrivateSceneCoordinator::reopen(&root, "scene-live-control").unwrap();
        assert_eq!(
            reopened.project_at(0).unwrap(),
            SceneFixtureDescriptor {
                active_scene_id: SceneId::parse("screen-only").unwrap(),
                composition: SceneComposition::ScreenOnly,
                timer: SceneTimerFixture::Elapsed {
                    phase: TimerPhase::Ended,
                    elapsed_ms: 0,
                },
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn public_presenter_activation_refuses_before_journal_append_without_camera_admission() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project.cutproj");
        fs::create_dir(&project).unwrap();
        let root = record_recovery::CaptureRoot::for_project(&project).unwrap();
        root.create_capture_dir("scene-presenter-refusal").unwrap();
        let config: record_capture::RecordingSceneConfig = serde_json::from_value(
            serde_json::json!({
                "catalog_revision": 1,
                "initial_scene_id": "screen",
                "presets": [{
                    "id": "screen", "name": "Screen", "preset_revision": 1,
                    "layout": {"kind": "screen"}
                }, {
                    "id": "presenter", "name": "Presenter", "preset_revision": 2,
                    "layout": {"kind":"presenter_pip","corner":"top_right","size_percent":25,"shape":"circle"}
                }],
                "timer": {"kind": "elapsed"}
            }),
        )
        .unwrap();
        let control = CaptureSessionControl::new_recording_scenes_live(
            None,
            false,
            false,
            false,
            &project,
            "scene-presenter-refusal",
            config.accepted_snapshot().unwrap(),
            false,
        )
        .unwrap();
        let clock = CaptureClock::new();
        let origin = clock.start();
        control.bind_backend_clock_for_test(clock);
        control.backend_started_at(origin);

        let error = control
            .activate_recording_scene(
                SceneId::parse("presenter").unwrap(),
                record_core::PresetRevision::new(2).unwrap(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("camera"));
        control
            .terminalize_after_capture_at_for_test(origin, 0)
            .unwrap();
        let projection = control
            .completed_recording_scene_projection()
            .unwrap()
            .unwrap();
        assert_eq!(
            projection.events().len(),
            2,
            "only start and end are durable"
        );
    }

    #[cfg(unix)]
    #[test]
    fn public_scene_terminal_waits_for_the_backend_media_duration() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project.cutproj");
        fs::create_dir(&project).unwrap();
        let root = record_recovery::CaptureRoot::for_project(&project).unwrap();
        root.create_capture_dir("scene-media-duration").unwrap();
        let control = CaptureSessionControl::new_recording_scenes_live(
            None,
            false,
            false,
            false,
            &project,
            "scene-media-duration",
            record_capture::RecordingSceneConfig::default_screen()
                .accepted_snapshot()
                .unwrap(),
            false,
        )
        .unwrap();
        let clock = CaptureClock::new();
        let origin = clock.start();
        control.bind_backend_clock_for_test(clock);
        control.backend_started_at(origin);

        control
            .terminalize_at_for_test(origin + Duration::from_millis(75))
            .unwrap();
        assert!(
            control.completed_recording_scene_projection().is_err(),
            "a user Stop cannot invent the final backend media duration"
        );

        control
            .terminalize_after_capture_at_for_test(origin + Duration::from_millis(100), 40)
            .unwrap();
        let projection = control
            .completed_recording_scene_projection()
            .unwrap()
            .unwrap();
        assert_eq!(projection.logical_media_time_ms(), 40);
        assert_eq!(
            projection.events().last().unwrap().logical_media_time_ms,
            40
        );
        let timeline = EditableSceneTimeline::from_replay(
            projection.snapshot(),
            projection.events(),
            projection.logical_media_time_ms(),
            projection.journal_sha256(),
        )
        .unwrap();
        let mut plan = EditPlan::empty(1920, 1080, 40, 30.0);
        timeline.apply_to_edit_plan(&mut plan, None, None).unwrap();
        plan.validate().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn terminalization_consumes_a_short_capture_clock_before_a_delayed_observer() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project.cutproj");
        fs::create_dir(&project).unwrap();
        let root = record_recovery::CaptureRoot::for_project(&project).unwrap();
        root.create_capture_dir("scene-delayed-observer").unwrap();
        let control = CaptureSessionControl::new_live(
            None,
            false,
            false,
            false,
            &project,
            "scene-delayed-observer",
        )
        .unwrap();
        let clock = CaptureClock::new();
        // Deliberately bind without spawning the normal observer. This models a
        // valid backend that opens and returns before that detached thread gets
        // scheduled; terminalization must admit the same clock synchronously.
        control.bind_backend_clock_for_test(clock.clone());
        let origin = clock.start();

        control
            .terminalize_at_for_test(origin + Duration::from_millis(5))
            .unwrap();
        assert_eq!(control.status().phase, SessionPhase::Stopped);
        let projection =
            serde_json::to_value(control.completed_scene_projection().unwrap().unwrap()).unwrap();
        assert_eq!(projection["logical_media_time_ms"], 5);
        assert_eq!(projection["scene"]["timer"]["Elapsed"]["phase"], "Ended");
        drop(control);

        // Reopen replays the durable 0ms TimerStart before the 5ms TimerEnd.
        // Project at the terminal boundary: the reducer deliberately refuses a
        // projection earlier than its latest durable logical-media event.
        let reopened = PrivateSceneCoordinator::reopen(&root, "scene-delayed-observer").unwrap();
        assert_eq!(
            reopened.project_at(5).unwrap().timer,
            SceneTimerFixture::Elapsed {
                phase: TimerPhase::Ended,
                elapsed_ms: 5,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn terminalization_rejects_a_clock_opened_after_its_stop_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project.cutproj");
        fs::create_dir(&project).unwrap();
        let root = record_recovery::CaptureRoot::for_project(&project).unwrap();
        let capture_dir = root.create_capture_dir("scene-stop-before-clock").unwrap();
        let control = CaptureSessionControl::new_live(
            None,
            false,
            false,
            false,
            &project,
            "scene-stop-before-clock",
        )
        .unwrap();
        let clock = CaptureClock::new();
        control.bind_backend_clock_for_test(clock.clone());
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        control.install_terminal_stop_hook(entered.clone(), release.clone());
        let stop_at = Instant::now();
        let terminal_control = control.clone();
        let terminal =
            std::thread::spawn(move || terminal_control.terminalize_at_for_test(stop_at));
        // The terminal owner has set its physical stop signal but has not yet
        // inspected the clock. Open it now: this forces the exact historical
        // scheduler race where a late backend origin would otherwise produce
        // a clamped zero-length TimerEnd while its source is nonempty.
        entered.wait();
        assert!(control.stop_requested());
        let origin = clock.start();
        assert!(origin > stop_at);
        release.wait();

        let error = terminal.join().unwrap().unwrap_err();
        assert!(error
            .to_string()
            .contains("opened after the terminal Stop boundary"));
        assert_eq!(control.status().phase, SessionPhase::Stopped);
        assert!(control.stop_requested());
        assert!(control.completed_scene_projection().is_err());
        let journal =
            fs::read_to_string(capture_dir.join("recorder-scenes.journal.jsonl")).unwrap();
        assert!(!journal.contains("TimerStart"));
        assert!(!journal.contains("TimerEnd"));
    }

    #[cfg(unix)]
    #[test]
    fn scene_timer_start_commits_before_a_later_concurrent_scene_action() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project.cutproj");
        fs::create_dir(&project).unwrap();
        let root = record_recovery::CaptureRoot::for_project(&project).unwrap();
        root.create_capture_dir("scene-start-transaction").unwrap();
        let control = Arc::new(
            CaptureSessionControl::new_live(
                None,
                false,
                false,
                false,
                &project,
                "scene-start-transaction",
            )
            .unwrap(),
        );
        let origin = Instant::now();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        control.install_scene_append_hook(entered.clone(), release.clone());

        let start_control = control.clone();
        let start = std::thread::spawn(move || start_control.backend_started_at(origin));
        entered.wait();
        let (attempted_send, attempted_receive) = mpsc::channel();
        let action_control = control.clone();
        let action = std::thread::spawn(move || {
            attempted_send.send(()).unwrap();
            action_control.append_scene_event_at(
                SceneEventKind::TimerPause,
                origin + Duration::from_millis(100),
            )
        });
        attempted_receive.recv().unwrap();
        release.wait();

        start.join().unwrap();
        let fixture = action.join().unwrap().unwrap().unwrap();
        assert_eq!(
            fixture.timer,
            SceneTimerFixture::Elapsed {
                phase: TimerPhase::Paused,
                elapsed_ms: 100,
            }
        );
        drop(control);

        let reopened = PrivateSceneCoordinator::reopen(&root, "scene-start-transaction").unwrap();
        assert_eq!(
            reopened.project_at(100).unwrap().timer,
            SceneTimerFixture::Elapsed {
                phase: TimerPhase::Paused,
                elapsed_ms: 100,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_scene_actions_cannot_overtake_their_issued_logical_times() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project.cutproj");
        fs::create_dir(&project).unwrap();
        let root = record_recovery::CaptureRoot::for_project(&project).unwrap();
        root.create_capture_dir("scene-action-transaction").unwrap();
        let control = Arc::new(
            CaptureSessionControl::new_live(
                None,
                false,
                false,
                false,
                &project,
                "scene-action-transaction",
            )
            .unwrap(),
        );
        let origin = Instant::now();
        control.backend_started_at(origin);
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        control.install_scene_append_hook(entered.clone(), release.clone());

        let first_control = control.clone();
        let first = std::thread::spawn(move || {
            first_control.append_scene_event_at(
                SceneEventKind::TimerPause,
                origin + Duration::from_millis(100),
            )
        });
        entered.wait();
        let (attempted_send, attempted_receive) = mpsc::channel();
        let second_control = control.clone();
        let second = std::thread::spawn(move || {
            attempted_send.send(()).unwrap();
            second_control.append_scene_event_at(
                SceneEventKind::TimerEnd,
                origin + Duration::from_millis(200),
            )
        });
        attempted_receive.recv().unwrap();
        release.wait();

        let paused = first.join().unwrap().unwrap().unwrap();
        assert_eq!(
            paused.timer,
            SceneTimerFixture::Elapsed {
                phase: TimerPhase::Paused,
                elapsed_ms: 100,
            }
        );
        let ended = second.join().unwrap().unwrap().unwrap();
        assert_eq!(
            ended.timer,
            SceneTimerFixture::Elapsed {
                phase: TimerPhase::Ended,
                elapsed_ms: 100,
            }
        );
        drop(control);

        let reopened = PrivateSceneCoordinator::reopen(&root, "scene-action-transaction").unwrap();
        assert_eq!(
            reopened.project_at(200).unwrap().timer,
            SceneTimerFixture::Elapsed {
                phase: TimerPhase::Ended,
                elapsed_ms: 100,
            }
        );
    }
}
