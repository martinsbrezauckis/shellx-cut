//! A pure logical capture-session clock for future pause and resume control.
//!
//! [`crate::CaptureClock`] remains the production backend origin. This type has no
//! worker, media, or input ownership: it only provides a deterministic logical
//! timebase and explicit control-state transitions for a future coordinator.

use serde::Serialize;
use std::time::{Duration, Instant};

/// Observable state of a logical recording session.
///
/// `Stopped` is terminal. In particular, a late pause/resume acknowledgement
/// cannot restart a session after the coordinator has stopped it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Preparing,
    Recording,
    Pausing,
    Paused,
    Resuming,
    Stopped,
}

impl SessionPhase {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped)
    }
}

/// One coordinator control edge for [`LogicalSessionClock`].
///
/// Pause and resume each have an acknowledgement edge so a future recorder can
/// stop timestamp issuance before its media workers have all observed the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTransition {
    Start,
    PauseRequested,
    PauseCompleted,
    ResumeRequested,
    ResumeCompleted,
    /// A selected stream declined resume. The coordinator returns to `Paused`
    /// without re-opening logical timestamp issuance.
    ResumeAborted,
    Stop,
}

/// Why a requested control edge did not change the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTransitionIgnored {
    /// The same edge already reached its intended state.
    AlreadyApplied,
    /// The edge is not valid from the current nonterminal state.
    NotReady,
    /// Stop is terminal and always wins over late control edges.
    Stopped,
}

/// The deterministic result of applying one [`SessionTransition`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTransitionResult {
    Applied {
        from: SessionPhase,
        to: SessionPhase,
    },
    Ignored {
        phase: SessionPhase,
        reason: SessionTransitionIgnored,
    },
}

impl SessionTransitionResult {
    pub fn changed(self) -> bool {
        matches!(self, Self::Applied { .. })
    }
}

/// A compact, coordinator-owned logical session clock.
///
/// Logical elapsed time advances only while [`SessionPhase::Recording`]. A pause
/// request freezes exactly one boundary before entering `Pausing`; no timestamp
/// is issued in `Pausing`, `Paused`, or `Resuming`. A resume acknowledgement
/// starts a fresh active interval from that frozen offset.
#[derive(Debug, Clone)]
pub struct LogicalSessionClock {
    phase: SessionPhase,
    frozen_elapsed: Duration,
    recording_started_at: Option<Instant>,
    active_duration_budget: Option<Duration>,
}

impl Default for LogicalSessionClock {
    fn default() -> Self {
        Self::new(None)
    }
}

impl LogicalSessionClock {
    /// Create a session in `Preparing`. A budget limits active logical time only;
    /// it never consumes wall time while the session is paused or transitioning.
    pub fn new(active_duration_budget: Option<Duration>) -> Self {
        Self {
            phase: SessionPhase::Preparing,
            frozen_elapsed: Duration::ZERO,
            recording_started_at: None,
            active_duration_budget,
        }
    }

    pub fn phase(&self) -> SessionPhase {
        self.phase
    }

    pub fn active_duration_budget(&self) -> Option<Duration> {
        self.active_duration_budget
    }

    /// Apply a control edge at a caller-supplied monotonic instant.
    ///
    /// Caller-supplied time keeps coordinator and unit-test ordering exact. `Stop`
    /// is checked before every nonterminal edge and is therefore terminal even if
    /// a late pause/resume completion arrives afterward.
    pub fn apply_at(
        &mut self,
        transition: SessionTransition,
        at: Instant,
    ) -> SessionTransitionResult {
        if self.phase.is_terminal() {
            return SessionTransitionResult::Ignored {
                phase: self.phase,
                reason: SessionTransitionIgnored::Stopped,
            };
        }

        if matches!(transition, SessionTransition::Stop) {
            return self.stop_at(at);
        }

        let from = self.phase;
        let to = match (self.phase, transition) {
            (SessionPhase::Preparing, SessionTransition::Start) => {
                self.recording_started_at = Some(at);
                SessionPhase::Recording
            }
            (SessionPhase::Recording, SessionTransition::PauseRequested) => {
                self.freeze_at(at);
                SessionPhase::Pausing
            }
            (SessionPhase::Pausing, SessionTransition::PauseCompleted) => SessionPhase::Paused,
            (SessionPhase::Paused, SessionTransition::ResumeRequested) => SessionPhase::Resuming,
            (SessionPhase::Resuming, SessionTransition::ResumeCompleted) => {
                self.recording_started_at = Some(at);
                SessionPhase::Recording
            }
            (SessionPhase::Resuming, SessionTransition::ResumeAborted) => SessionPhase::Paused,
            _ if is_repeated(self.phase, transition) => {
                return SessionTransitionResult::Ignored {
                    phase: self.phase,
                    reason: SessionTransitionIgnored::AlreadyApplied,
                };
            }
            _ => {
                return SessionTransitionResult::Ignored {
                    phase: self.phase,
                    reason: SessionTransitionIgnored::NotReady,
                };
            }
        };

        self.phase = to;
        SessionTransitionResult::Applied { from, to }
    }

    /// Apply a control edge using the local monotonic clock.
    pub fn apply(&mut self, transition: SessionTransition) -> SessionTransitionResult {
        self.apply_at(transition, Instant::now())
    }

    /// Return the logical elapsed duration, including a live active interval only
    /// in `Recording`. This is an observation method, not permission to emit a
    /// media or event timestamp while paused or transitioning.
    pub fn logical_elapsed_at(&self, at: Instant) -> Duration {
        match self.recording_started_at {
            Some(started_at) if self.phase == SessionPhase::Recording => add_duration(
                self.frozen_elapsed,
                at.saturating_duration_since(started_at),
            ),
            _ => self.frozen_elapsed,
        }
    }

    /// Return a logical event timestamp only while actively recording.
    pub fn timestamp_at(&self, at: Instant) -> Option<Duration> {
        (self.phase == SessionPhase::Recording).then(|| self.logical_elapsed_at(at))
    }

    /// The unused portion of the active-duration budget, if one was configured.
    pub fn remaining_active_duration_at(&self, at: Instant) -> Option<Duration> {
        self.active_duration_budget
            .map(|budget| budget.saturating_sub(self.logical_elapsed_at(at)))
    }

    /// Whether the active-duration budget has been consumed.
    ///
    /// This deliberately does not stop a session. A future coordinator owns the
    /// worker shutdown edge and should apply `Stop` after observing this result.
    pub fn active_duration_exhausted_at(&self, at: Instant) -> bool {
        self.active_duration_budget
            .map(|budget| self.logical_elapsed_at(at) >= budget)
            .unwrap_or(false)
    }

    /// Accept the exact logical endpoint from a post-close sealed artifact.
    ///
    /// A pause or stop command suppresses timestamp issuance immediately, but
    /// it is not evidence that a native encoder has closed at that instant. A
    /// higher-level owner may advance the frozen logical endpoint only after it
    /// has verified and durably accepted that post-close fact. It can never
    /// rewind a command's timestamp floor or reopen timestamp issuance.
    pub(crate) fn accept_post_close_elapsed(&mut self, elapsed: Duration) -> bool {
        if !matches!(self.phase, SessionPhase::Paused | SessionPhase::Stopped)
            || self.recording_started_at.is_some()
            || elapsed < self.frozen_elapsed
        {
            return false;
        }
        self.frozen_elapsed = elapsed;
        true
    }

    fn stop_at(&mut self, at: Instant) -> SessionTransitionResult {
        let from = self.phase;
        self.freeze_at(at);
        self.phase = SessionPhase::Stopped;
        SessionTransitionResult::Applied {
            from,
            to: SessionPhase::Stopped,
        }
    }

    fn freeze_at(&mut self, at: Instant) {
        self.frozen_elapsed = self.logical_elapsed_at(at);
        self.recording_started_at = None;
    }
}

fn add_duration(left: Duration, right: Duration) -> Duration {
    left.checked_add(right).unwrap_or(Duration::MAX)
}

fn is_repeated(phase: SessionPhase, transition: SessionTransition) -> bool {
    matches!(
        (phase, transition),
        (SessionPhase::Recording, SessionTransition::Start)
            | (SessionPhase::Pausing, SessionTransition::PauseRequested)
            | (SessionPhase::Paused, SessionTransition::PauseRequested)
            | (SessionPhase::Paused, SessionTransition::PauseCompleted)
            | (SessionPhase::Resuming, SessionTransition::ResumeRequested)
            | (SessionPhase::Recording, SessionTransition::ResumeCompleted)
            | (SessionPhase::Paused, SessionTransition::ResumeAborted)
    )
}
