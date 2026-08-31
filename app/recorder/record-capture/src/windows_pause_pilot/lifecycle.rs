//! Owned thread lifecycle for the private Windows pause-pilot worker.
//!
//! This module deliberately owns only native command/event transport and the
//! thread that owns WGC. It does not own a Cut server session, journal, or
//! projection. The server keeps its existing command adapter and event
//! translator; a future internal session owner must retain this handle beside
//! those two halves so owner drop cannot leave a native worker detached.

use std::sync::mpsc;
#[cfg(test)]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::audio_run::WindowsPauseAudioFactory;
use super::input_run::WindowsPauseInputFactory;
use super::worker::WindowsPausePilotWorker;
use super::{
    channel, WindowsPausePilotChannelError, WindowsPausePilotCommand,
    WindowsPausePilotCommandReceiver, WindowsPausePilotCommandSender,
    WindowsPausePilotEventReceiver, WindowsPausePilotEventSender, WindowsPausePilotProfile,
    WindowsPausePilotStartError,
};
use crate::windows_wgc_run::{
    WgcCheckpointPublisher, WgcControlFactory, WgcRunOwner, WgcStartObservation,
};

/// This bounds only how long the caller waits for the initial `Started` fact.
/// If it expires, cleanup still requests Stop and joins the worker; that is
/// ownership, not force-cancellation of a native start that refuses to return.
const DEFAULT_STARTUP_WAIT: Duration = Duration::from_secs(15);
const DROP_STOP_EPOCH: u64 = u64::MAX;

/// The one owner for a spawned private WGC worker. The command clone is kept
/// solely to make Drop terminal; server code gets a separate clone through
/// [`Self::command_sender`], and can move the unique event receiver into its
/// existing `WindowsPauseEventTranslator` through [`Self::take_event_receiver`].
///
/// It is intentionally not a public capture mode. It cannot be constructed
/// from a record verb or UI request, and production construction stays in the
/// Windows backend beside the real WGC factory and checkpoint publisher.
pub struct WindowsPausePilotThread {
    terminal_commands: Option<WindowsPausePilotCommandSender>,
    events: Option<WindowsPausePilotEventReceiver>,
    join: Option<JoinHandle<Result<(), WindowsPausePilotChannelError>>>,
    #[cfg(test)]
    worker_done: Arc<AtomicBool>,
}

impl WindowsPausePilotThread {
    /// A clone for the existing server-private dispatch adapter. The lifecycle
    /// owner always retains its own terminal clone until it joins the worker.
    pub fn command_sender(&self) -> WindowsPausePilotCommandSender {
        self.terminal_commands
            .as_ref()
            .expect("pause-pilot command sender is retained until join")
            .clone()
    }

    /// Move the unique event receiver into the existing private server event
    /// translator. It may be taken exactly once; the thread owner remains
    /// responsible for terminal Stop plus join regardless.
    pub fn take_event_receiver(&mut self) -> Option<WindowsPausePilotEventReceiver> {
        self.events.take()
    }

    /// Make the native worker terminal and wait for its owned thread. A Stop is
    /// enqueued even if a server Stop may already be queued; WGC's worker drains
    /// Stop-dominantly, so the duplicate cannot restart or revive a run. Native
    /// close itself is not force-cancellable: ownership means this call joins
    /// rather than detaching a potentially stuck native thread.
    pub fn shutdown_and_join(&mut self) -> Result<(), WindowsPausePilotChannelError> {
        let send_result = self.terminal_commands.take().map(|commands| {
            commands.send(WindowsPausePilotCommand::Stop {
                epoch: DROP_STOP_EPOCH,
            })
        });
        let join_result = self.join_owned();
        match (send_result, join_result) {
            (_, Some(result)) => result,
            (Some(Err(error)), None) => Err(error),
            _ => Ok(()),
        }
    }

    /// Join after the real terminal event has already been durably accepted by
    /// the future server owner. Unlike [`Self::shutdown_and_join`], this sends
    /// no synthetic command or epoch: it only drops this owner's sender clone
    /// and joins the worker that has already stopped from its actual `Stop`.
    /// Calling it before an accepted terminal event can wait indefinitely, so
    /// it is deliberately not a substitute for abnormal cleanup.
    pub fn join_after_terminal(&mut self) -> Result<(), WindowsPausePilotChannelError> {
        self.terminal_commands.take();
        self.join_owned().unwrap_or(Ok(()))
    }

    fn join_owned(&mut self) -> Option<Result<(), WindowsPausePilotChannelError>> {
        self.join.take().map(|join| match join.join() {
            Ok(result) => result,
            Err(_) => Err(WindowsPausePilotChannelError::NativeTerminal),
        })
    }

    #[cfg(test)]
    fn completion_probe(&self) -> Arc<AtomicBool> {
        self.worker_done.clone()
    }
}

impl Drop for WindowsPausePilotThread {
    fn drop(&mut self) {
        // There is deliberately no detach path. A failed native close remains
        // terminal inside `WgcRunOwner`; join makes the thread lifetime owned
        // even when a server/controller disappears mid-boundary.
        let _ = self.shutdown_and_join();
    }
}

/// Spawn the generic owned lifecycle. Production passes the actual WGC target
/// resolver/factory/publisher from the Windows-only `native` module; tests
/// inject deterministic equivalents without requiring a desktop session.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_owned<T, F, P, R, B, RS, OS, OB>(
    profile: WindowsPausePilotProfile,
    resolve: R,
    make_owner: B,
    input_factory: Box<dyn WindowsPauseInputFactory>,
    audio_factory: Box<dyn WindowsPauseAudioFactory>,
    reserve_start_ms: RS,
    observe_started: OS,
    observe_boundary: OB,
    rollover_interval: Duration,
) -> Result<WindowsPausePilotThread, WindowsPausePilotStartError>
where
    T: Send + 'static,
    F: WgcControlFactory<T> + Send + 'static,
    F::Control: Send + 'static,
    P: WgcCheckpointPublisher + Send + 'static,
    R: FnMut(&str) -> Option<T> + Send + 'static,
    B: FnOnce(T) -> record_core::Result<WgcRunOwner<T, F, P>> + Send + 'static,
    RS: FnMut() -> u64 + Send + 'static,
    OS: FnMut() -> record_core::Result<WgcStartObservation> + Send + 'static,
    OB: FnMut() -> (u64, Instant) + Send + 'static,
{
    if rollover_interval.is_zero() {
        return Err(WindowsPausePilotStartError::NativeStartFailed);
    }
    let (commands, command_receiver, event_sender, events) = channel();
    let terminal_commands = commands.clone();
    let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
    #[cfg(test)]
    let worker_done = Arc::new(AtomicBool::new(false));
    #[cfg(test)]
    let done_on_exit = worker_done.clone();

    let join = match thread::Builder::new()
        .name("shellx-cut-pause-pilot".into())
        .spawn(move || {
            let result = run_owned(
                profile,
                resolve,
                make_owner,
                input_factory,
                audio_factory,
                command_receiver,
                event_sender,
                reserve_start_ms,
                observe_started,
                observe_boundary,
                rollover_interval,
                startup_sender,
            );
            #[cfg(test)]
            done_on_exit.store(true, Ordering::Release);
            result
        }) {
        Ok(join) => join,
        Err(_) => return Err(WindowsPausePilotStartError::NativeStartFailed),
    };
    let mut lifecycle = WindowsPausePilotThread {
        terminal_commands: Some(terminal_commands),
        events: Some(events),
        join: Some(join),
        #[cfg(test)]
        worker_done,
    };

    let startup = startup_receiver.recv_timeout(DEFAULT_STARTUP_WAIT);
    match startup {
        Ok(Ok(())) => Ok(lifecycle),
        Ok(Err(error)) => {
            let _ = lifecycle.shutdown_and_join();
            Err(error)
        }
        Err(_) => {
            // Do not return a handle without a physical Started fact. The
            // cleanup path sends a terminal Stop and joins before reporting the
            // startup failure. It never turns a potentially stuck native close
            // into a detached worker.
            let _ = lifecycle.shutdown_and_join();
            Err(WindowsPausePilotStartError::NativeStartFailed)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_owned<T, F, P, R, B, RS, OS, OB>(
    profile: WindowsPausePilotProfile,
    resolve: R,
    make_owner: B,
    input_factory: Box<dyn WindowsPauseInputFactory>,
    audio_factory: Box<dyn WindowsPauseAudioFactory>,
    command_receiver: WindowsPausePilotCommandReceiver,
    event_sender: WindowsPausePilotEventSender,
    mut reserve_start_ms: RS,
    mut observe_started: OS,
    mut observe_boundary: OB,
    rollover_interval: Duration,
    startup_sender: mpsc::SyncSender<Result<(), WindowsPausePilotStartError>>,
) -> Result<(), WindowsPausePilotChannelError>
where
    F: WgcControlFactory<T>,
    P: WgcCheckpointPublisher,
    R: FnMut(&str) -> Option<T>,
    B: FnOnce(T) -> record_core::Result<WgcRunOwner<T, F, P>>,
    RS: FnMut() -> u64,
    OS: FnMut() -> record_core::Result<WgcStartObservation>,
    OB: FnMut() -> (u64, Instant),
{
    // `spawn_owned` rejects this before a thread exists. Keep the invariant at
    // the worker boundary too so a future internal caller cannot turn the WGC
    // recovery loop into an unbounded zero-duration spin.
    if rollover_interval.is_zero() {
        let _ = startup_sender.send(Err(WindowsPausePilotStartError::NativeStartFailed));
        return Ok(());
    }
    let (mut worker, started) = match WindowsPausePilotWorker::start(
        profile,
        resolve,
        make_owner,
        input_factory,
        audio_factory,
        &mut reserve_start_ms,
        &mut observe_started,
    ) {
        Ok(started) => started,
        Err(error) => {
            let _ = startup_sender.send(Err(error));
            return Ok(());
        }
    };
    if event_sender.send(started).is_err() {
        worker.close_and_reap(&mut observe_boundary);
        let _ = startup_sender.send(Err(WindowsPausePilotStartError::NativeStartFailed));
        return Err(WindowsPausePilotChannelError::Disconnected);
    }
    if startup_sender.send(Ok(())).is_err() {
        worker.close_and_reap(&mut observe_boundary);
        return Err(WindowsPausePilotChannelError::Disconnected);
    }
    worker.run_until_terminal(
        &command_receiver,
        &event_sender,
        rollover_interval,
        reserve_start_ms,
        observe_started,
        observe_boundary,
    )
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
