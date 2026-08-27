#![allow(improper_ctypes_definitions)]

use crate::macos::common::*;
use crate::rdevin::Event;
use crate::OwnedListenerJoinError;
use cocoa::base::nil;
use cocoa::foundation::NSAutoreleasePool;
use core_graphics::event::{CGEventTapLocation, CGEventType};
use lazy_static::lazy_static;
use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

type OwnedEventCallback = Box<dyn FnMut(Event) + Send>;

lazy_static! {
    static ref OWNED_CALLBACK: Mutex<Option<OwnedEventCallback>> = Mutex::new(None);
}

/// The listener thread releases these retained Core Foundation objects. The
/// controller only asks the retained run loop to stop and wake up.
struct NativeState {
    run_loop: usize,
    source: usize,
    tap: usize,
}

struct Control {
    state: Mutex<Option<NativeState>>,
    stop_requested: AtomicBool,
}

/// One passive `ListenOnly` event tap attached to the listener thread's current
/// run loop. It intentionally has no input interception or simulation path.
pub(crate) struct OwnedListener {
    control: Arc<Control>,
    thread: Option<JoinHandle<()>>,
    exited: mpsc::Receiver<Result<(), super::ListenError>>,
}

impl OwnedListener {
    pub(crate) fn start<T>(callback: T) -> Result<Self, super::ListenError>
    where
        T: FnMut(Event) + Send + 'static,
    {
        let control = Arc::new(Control {
            state: Mutex::new(None),
            stop_requested: AtomicBool::new(false),
        });
        let thread_control = control.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (exit_tx, exit_rx) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            let outcome = owned_listen(callback, thread_control, ready_tx);
            let _ = exit_tx.send(outcome);
        });
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                control,
                thread: Some(thread),
                exited: exit_rx,
            }),
            Ok(Err(())) | Err(_) => match thread.join() {
                Ok(()) => match exit_rx.recv() {
                    Ok(Err(error)) => Err(error),
                    Ok(Ok(())) | Err(_) => Err(super::ListenError::ListenerThread),
                },
                Err(_) => Err(super::ListenError::ListenerThread),
            },
        }
    }

    pub(crate) fn request_stop(&self) -> Result<(), super::ListenError> {
        self.control.stop_requested.store(true, Ordering::Release);
        let state = self
            .control
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = state.as_ref() else {
            return Ok(());
        };
        // SAFETY: `run_loop` was retained by the listener thread and remains
        // retained until that thread has returned from CFRunLoopRun and removed
        // the source/tap. Core Foundation permits stopping and waking this loop.
        unsafe {
            CFRunLoopStop(state.run_loop as CFRunLoopRef);
            CFRunLoopWakeUp(state.run_loop as CFRunLoopRef);
        }
        Ok(())
    }

    /// Wait no longer than `timeout` for the native listener to return. The
    /// final `join` happens only after its exit channel proves teardown ended.
    pub(crate) fn wait_for_exit(
        &mut self,
        timeout: Duration,
    ) -> Result<bool, OwnedListenerJoinError> {
        let Some(thread) = self.thread.take() else {
            return Ok(true);
        };
        match self.exited.recv_timeout(timeout) {
            Ok(result) => match thread.join() {
                Ok(()) => result.map(|()| true).map_err(OwnedListenerJoinError::from),
                Err(_) => Err(OwnedListenerJoinError::ThreadPanicked),
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.thread = Some(thread);
                Ok(false)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => match thread.join() {
                Ok(()) | Err(_) => Err(OwnedListenerJoinError::ThreadPanicked),
            },
        }
    }
}

fn owned_listen<T>(
    callback: T,
    control: Arc<Control>,
    ready: mpsc::SyncSender<Result<(), ()>>,
) -> Result<(), super::ListenError>
where
    T: FnMut(Event) + Send + 'static,
{
    *OWNED_CALLBACK.lock().unwrap() = Some(Box::new(callback));
    // The owned listener runs off the app main thread; key-layout translation
    // is dispatched back to that thread by the maintained converter.
    set_is_main_thread(false);
    let _pool = unsafe { NSAutoreleasePool::new(nil) };
    let event_mask = if crate::keyboard_only() {
        (1 << CGEventType::KeyDown as u64)
            + (1 << CGEventType::KeyUp as u64)
            + (1 << CGEventType::FlagsChanged as u64)
    } else {
        kCGEventMaskForAllEvents
    };
    let tap = unsafe {
        CGEventTapCreate(
            CGEventTapLocation::HID,
            kCGHeadInsertEventTap,
            CGEventTapOption::ListenOnly,
            event_mask,
            event_callback,
            nil,
        )
    };
    if tap.is_null() {
        *OWNED_CALLBACK.lock().unwrap() = None;
        let _ = ready.send(Err(()));
        return Err(super::ListenError::EventTapError);
    }
    let source = unsafe { CFMachPortCreateRunLoopSource(nil, tap, 0) };
    if source.is_null() {
        unsafe { CFRelease(tap) };
        *OWNED_CALLBACK.lock().unwrap() = None;
        let _ = ready.send(Err(()));
        return Err(super::ListenError::LoopSourceError);
    }
    let run_loop = unsafe { CFRunLoopGetCurrent() };
    if run_loop.is_null() {
        unsafe {
            CFRelease(source as *const c_void);
            CFRelease(tap);
        }
        *OWNED_CALLBACK.lock().unwrap() = None;
        let _ = ready.send(Err(()));
        return Err(super::ListenError::RunLoopError);
    }
    unsafe {
        CFRetain(run_loop as *const c_void);
        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
    }
    {
        let mut state = control.state.lock().unwrap();
        *state = Some(NativeState {
            run_loop: run_loop as usize,
            source: source as usize,
            tap: tap as usize,
        });
    }
    if ready.send(Ok(())).is_err() {
        let _ = stop(&control);
        free_state(&control);
        *OWNED_CALLBACK.lock().unwrap() = None;
        return Err(super::ListenError::ListenerThread);
    }

    // `request_stop` can arrive immediately after the start acknowledgement.
    // Check before entering the run loop so a pre-run stop cannot strand a tap.
    if control.stop_requested.load(Ordering::Acquire) {
        free_state(&control);
        *OWNED_CALLBACK.lock().unwrap() = None;
        return Ok(());
    }

    unsafe { CFRunLoopRun() };
    free_state(&control);
    *OWNED_CALLBACK.lock().unwrap() = None;
    Ok(())
}

fn stop(control: &Control) -> Result<(), super::ListenError> {
    control.stop_requested.store(true, Ordering::Release);
    let state = control
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(state) = state.as_ref() else {
        return Ok(());
    };
    unsafe {
        CFRunLoopStop(state.run_loop as CFRunLoopRef);
        CFRunLoopWakeUp(state.run_loop as CFRunLoopRef);
    }
    Ok(())
}

fn free_state(control: &Control) {
    let state = control
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if let Some(state) = state {
        unsafe {
            let run_loop = state.run_loop as CFRunLoopRef;
            let source = state.source as CFRunLoopSourceRef;
            let tap = state.tap as CFMachPortRef;
            CGEventTapEnable(tap, false);
            CFRunLoopRemoveSource(run_loop, source, kCFRunLoopCommonModes);
            CFRelease(source as *const c_void);
            CFRelease(tap);
            CFRelease(run_loop as *const c_void);
        }
    }
}

unsafe extern "C" fn event_callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    if let Ok(mut state) = KEYBOARD_STATE.lock() {
        if let Some(keyboard) = state.as_mut() {
            if let Some(event) = convert(event_type, &event, keyboard) {
                if let Some(callback) = OWNED_CALLBACK.lock().unwrap().as_mut() {
                    callback(event);
                }
            }
        }
    }
    // ListenOnly means preserve and return the original event without change.
    event
}
