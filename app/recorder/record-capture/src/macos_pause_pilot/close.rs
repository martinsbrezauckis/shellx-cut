//! Resumable terminal-close state for one native ScreenCaptureKit generation.
//!
//! A close acknowledgement can fail after an earlier native operation has
//! already succeeded. Keep that exact prefix so a retry never sends Stop or
//! detaches an output twice just because a later callback failed.

const MAX_ATTEMPTS_PER_STEP: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeClosePhase {
    Open,
    StopSent,
    StopAcknowledged,
    OutputDetached,
    CallbackComplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeCloseStep {
    Stop,
    DetachOutput,
    WaitCallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NativeCloseFailure {
    step: NativeCloseStep,
    retry_exhausted: bool,
}

impl NativeCloseFailure {
    pub(super) fn step(self) -> NativeCloseStep {
        self.step
    }

    pub(super) fn retry_exhausted(self) -> bool {
        self.retry_exhausted
    }
}

/// State retained beside the live native output until its callback has
/// completed. `StopSent` is deliberately distinct from `StopAcknowledged`:
/// an error from the synchronous native call has no acknowledgement and can
/// consume the bounded retry budget, whereas acknowledged work is never sent
/// again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NativeCloseState {
    phase: NativeClosePhase,
    attempts: [u8; 3],
    last_error: Option<NativeCloseFailure>,
}

impl Default for NativeCloseState {
    fn default() -> Self {
        Self {
            phase: NativeClosePhase::Open,
            attempts: [0; 3],
            last_error: None,
        }
    }
}

impl NativeCloseState {
    pub(super) fn phase(&self) -> NativeClosePhase {
        self.phase
    }

    pub(super) fn last_error(&self) -> Option<NativeCloseFailure> {
        self.last_error
    }

    pub(super) fn close<Stop, Detach, Callback>(
        &mut self,
        stop: Stop,
        detach: Detach,
        callback: Callback,
    ) -> Result<(), NativeCloseFailure>
    where
        Stop: FnOnce() -> Result<(), ()>,
        Detach: FnOnce() -> Result<(), ()>,
        Callback: FnOnce() -> Result<(), ()>,
    {
        if matches!(
            self.phase,
            NativeClosePhase::Open | NativeClosePhase::StopSent
        ) {
            self.phase = NativeClosePhase::StopSent;
            self.attempt(NativeCloseStep::Stop, stop)?;
            self.phase = NativeClosePhase::StopAcknowledged;
        }
        if self.phase == NativeClosePhase::StopAcknowledged {
            self.attempt(NativeCloseStep::DetachOutput, detach)?;
            self.phase = NativeClosePhase::OutputDetached;
        }
        if self.phase == NativeClosePhase::OutputDetached {
            self.attempt(NativeCloseStep::WaitCallback, callback)?;
            self.phase = NativeClosePhase::CallbackComplete;
        }
        debug_assert_eq!(self.phase, NativeClosePhase::CallbackComplete);
        self.last_error = None;
        Ok(())
    }

    fn attempt<Action>(
        &mut self,
        step: NativeCloseStep,
        action: Action,
    ) -> Result<(), NativeCloseFailure>
    where
        Action: FnOnce() -> Result<(), ()>,
    {
        let index = match step {
            NativeCloseStep::Stop => 0,
            NativeCloseStep::DetachOutput => 1,
            NativeCloseStep::WaitCallback => 2,
        };
        if self.attempts[index] >= MAX_ATTEMPTS_PER_STEP {
            let failure = NativeCloseFailure {
                step,
                retry_exhausted: true,
            };
            self.last_error = Some(failure);
            return Err(failure);
        }
        self.attempts[index] += 1;
        if action().is_err() {
            let failure = NativeCloseFailure {
                step,
                retry_exhausted: false,
            };
            self.last_error = Some(failure);
            return Err(failure);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{NativeClosePhase, NativeCloseState, NativeCloseStep};

    #[test]
    fn stop_failure_retries_only_stop_then_runs_the_remaining_steps_once() {
        let mut close = NativeCloseState::default();
        let mut stop_calls = 0;
        let mut detach_calls = 0;
        let mut callback_calls = 0;
        let first = close.close(
            || {
                stop_calls += 1;
                Err(())
            },
            || {
                detach_calls += 1;
                Ok(())
            },
            || {
                callback_calls += 1;
                Ok(())
            },
        );
        assert_eq!(first.unwrap_err().step(), NativeCloseStep::Stop);
        assert_eq!(close.phase(), NativeClosePhase::StopSent);
        assert_eq!((stop_calls, detach_calls, callback_calls), (1, 0, 0));

        close
            .close(
                || {
                    stop_calls += 1;
                    Ok(())
                },
                || {
                    detach_calls += 1;
                    Ok(())
                },
                || {
                    callback_calls += 1;
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(close.phase(), NativeClosePhase::CallbackComplete);
        assert_eq!((stop_calls, detach_calls, callback_calls), (2, 1, 1));
    }

    #[test]
    fn detach_failure_does_not_repeat_acknowledged_stop() {
        let mut close = NativeCloseState::default();
        let mut stop_calls = 0;
        let mut detach_calls = 0;
        let mut callback_calls = 0;
        let first = close.close(
            || {
                stop_calls += 1;
                Ok(())
            },
            || {
                detach_calls += 1;
                Err(())
            },
            || {
                callback_calls += 1;
                Ok(())
            },
        );
        assert_eq!(first.unwrap_err().step(), NativeCloseStep::DetachOutput);
        assert_eq!(close.phase(), NativeClosePhase::StopAcknowledged);
        assert_eq!((stop_calls, detach_calls, callback_calls), (1, 1, 0));

        close
            .close(
                || {
                    stop_calls += 1;
                    Ok(())
                },
                || {
                    detach_calls += 1;
                    Ok(())
                },
                || {
                    callback_calls += 1;
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!((stop_calls, detach_calls, callback_calls), (1, 2, 1));
    }

    #[test]
    fn callback_failure_does_not_repeat_stop_or_detach() {
        let mut close = NativeCloseState::default();
        let mut stop_calls = 0;
        let mut detach_calls = 0;
        let mut callback_calls = 0;
        let first = close.close(
            || {
                stop_calls += 1;
                Ok(())
            },
            || {
                detach_calls += 1;
                Ok(())
            },
            || {
                callback_calls += 1;
                Err(())
            },
        );
        assert_eq!(first.unwrap_err().step(), NativeCloseStep::WaitCallback);
        assert_eq!(close.phase(), NativeClosePhase::OutputDetached);
        assert_eq!((stop_calls, detach_calls, callback_calls), (1, 1, 1));

        close
            .close(
                || {
                    stop_calls += 1;
                    Ok(())
                },
                || {
                    detach_calls += 1;
                    Ok(())
                },
                || {
                    callback_calls += 1;
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!((stop_calls, detach_calls, callback_calls), (1, 1, 2));
    }

    #[test]
    fn a_persistently_failed_step_exhausts_its_bounded_retry_without_replaying_prior_steps() {
        let mut close = NativeCloseState::default();
        let mut stop_calls = 0;
        for attempt in 0..3 {
            let error = close
                .close(
                    || {
                        stop_calls += 1;
                        Err(())
                    },
                    || Ok(()),
                    || Ok(()),
                )
                .unwrap_err();
            assert_eq!(error.step(), NativeCloseStep::Stop);
            assert_eq!(error.retry_exhausted(), attempt == 2);
        }
        assert_eq!(stop_calls, 2);
        assert_eq!(close.phase(), NativeClosePhase::StopSent);
        assert!(close.last_error().unwrap().retry_exhausted());
    }
}
