use crate::linux::common::{convert, FALSE, KEYBOARD};
use crate::linux::keyboard::Keyboard;
use crate::rdevin::Event;
use crate::OwnedListenerJoinError;
use lazy_static::lazy_static;
use std::convert::TryInto;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_uchar, c_uint, c_ulong};
use std::ptr::null;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use x11::xlib;
use x11::xrecord;

type OwnedCallback = Box<dyn FnMut(Event) + Send>;

lazy_static! {
    static ref OWNED_CALLBACK: Mutex<Option<OwnedCallback>> = Mutex::new(None);
}

/// Both display pointers and the RECORD context are protected together. The
/// stop path holds this lock while disabling/flushing, and the listener thread
/// takes it only after `XRecordEnableContext` returns to free/close resources.
struct NativeState {
    control_display: usize,
    data_display: usize,
    context: c_ulong,
}

struct Control {
    state: Mutex<Option<NativeState>>,
    stop_requested: AtomicBool,
}

/// One owned passive X RECORD listener. The data connection receives events;
/// the independent control connection disables the context to unblock it.
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
        // SAFETY: `state` remains locked through the disable/flush calls. The
        // listener only frees it after its blocking data call has returned.
        unsafe {
            if xrecord::XRecordDisableContext(
                state.control_display as *mut xlib::Display,
                state.context,
            ) == 0
            {
                return Err(super::ListenError::DisableRecordContext);
            }
            xlib::XFlush(state.control_display as *mut xlib::Display);
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
    let keyboard = match Keyboard::new() {
        Some(keyboard) => keyboard,
        None => {
            let _ = ready.send(Err(()));
            return Err(super::ListenError::NoDisplays);
        }
    };
    unsafe {
        KEYBOARD = Some(keyboard);
    }
    *OWNED_CALLBACK.lock().unwrap() = Some(Box::new(callback));

    let control_display = unsafe { xlib::XOpenDisplay(null()) };
    let data_display = unsafe { xlib::XOpenDisplay(null()) };
    if control_display.is_null() || data_display.is_null() {
        unsafe {
            if !data_display.is_null() {
                xlib::XCloseDisplay(data_display);
            }
            if !control_display.is_null() {
                xlib::XCloseDisplay(control_display);
            }
        }
        clear_owned_state();
        let _ = ready.send(Err(()));
        return Err(super::ListenError::NoDisplays);
    }
    let extension_name = CStr::from_bytes_with_nul(b"RECORD\0").unwrap();
    if unsafe { xlib::XInitExtension(control_display, extension_name.as_ptr()) }.is_null() {
        close_displays(control_display, data_display);
        clear_owned_state();
        let _ = ready.send(Err(()));
        return Err(super::ListenError::InitExtension);
    }

    let range = unsafe { xrecord::XRecordAllocRange() };
    if range.is_null() {
        close_displays(control_display, data_display);
        clear_owned_state();
        let _ = ready.send(Err(()));
        return Err(super::ListenError::AllocateRecordRange);
    }
    unsafe {
        (*range).device_events.first = xlib::KeyPress as c_uchar;
        (*range).device_events.last = if crate::keyboard_only() {
            xlib::KeyRelease
        } else {
            xlib::MotionNotify
        } as c_uchar;
    }
    let mut clients = xrecord::XRecordAllClients;
    let mut range_ptr = range;
    let context = unsafe {
        xrecord::XRecordCreateContext(control_display, 0, &mut clients, 1, &mut range_ptr, 1)
    };
    unsafe { xlib::XFree(range as *mut _) };
    if context == 0 {
        close_displays(control_display, data_display);
        clear_owned_state();
        let _ = ready.send(Err(()));
        return Err(super::ListenError::CreateRecordContext);
    }

    unsafe { xlib::XSync(control_display, FALSE) };
    {
        let mut state = control.state.lock().unwrap();
        *state = Some(NativeState {
            control_display: control_display as usize,
            data_display: data_display as usize,
            context,
        });
    }
    if ready.send(Ok(())).is_err() {
        let _ = stop(&control);
        free_state(&control);
        clear_owned_state();
        return Err(super::ListenError::ListenerThread);
    }

    // A seal may arrive as soon as startup is acknowledged. Do not enter the
    // blocking data path after its control display has already disabled this
    // context; release everything deterministically instead.
    if control.stop_requested.load(Ordering::Acquire) {
        free_state(&control);
        clear_owned_state();
        return Ok(());
    }

    let enabled = unsafe {
        xrecord::XRecordEnableContext(data_display, context, Some(record_callback), &mut 0)
    };
    let stopped = control.stop_requested.load(Ordering::Acquire);
    free_state(&control);
    clear_owned_state();
    if enabled == 0 && !stopped {
        Err(super::ListenError::EnableRecordContext)
    } else {
        Ok(())
    }
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
        if xrecord::XRecordDisableContext(
            state.control_display as *mut xlib::Display,
            state.context,
        ) == 0
        {
            return Err(super::ListenError::DisableRecordContext);
        }
        xlib::XFlush(state.control_display as *mut xlib::Display);
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
            xrecord::XRecordFreeContext(state.control_display as *mut xlib::Display, state.context);
            xlib::XCloseDisplay(state.data_display as *mut xlib::Display);
            xlib::XCloseDisplay(state.control_display as *mut xlib::Display);
        }
    }
}

fn close_displays(control_display: *mut xlib::Display, data_display: *mut xlib::Display) {
    unsafe {
        xlib::XCloseDisplay(data_display);
        xlib::XCloseDisplay(control_display);
    }
}

fn clear_owned_state() {
    *OWNED_CALLBACK.lock().unwrap() = None;
    // X RECORD callbacks have stopped before every call site that reaches this
    // cleanup helper, so dropping the converter cannot race with `convert`.
    unsafe {
        KEYBOARD = None;
    }
}

// Matches the upstream RECORD datum layout used by `listen.rs`.
#[repr(C)]
struct XRecordDatum {
    type_: u8,
    code: u8,
    _rest: u64,
    _1: bool,
    _2: bool,
    _3: bool,
    root_x: i16,
    root_y: i16,
    event_x: i16,
    event_y: i16,
    state: u16,
}

unsafe extern "C" fn record_callback(
    _unused: *mut c_char,
    raw_data: *mut xrecord::XRecordInterceptData,
) {
    let Some(data) = raw_data.as_ref() else {
        return;
    };
    if data.category == xrecord::XRecordFromServer
        && data.data_len * 4 >= std::mem::size_of::<XRecordDatum>().try_into().unwrap()
    {
        #[allow(clippy::cast_ptr_alignment)]
        if let Some(datum) = (data.data as *const XRecordDatum).as_ref() {
            let code: c_uint = datum.code.into();
            let type_: c_int = datum.type_.into();
            if let Some(event) = convert(
                &mut KEYBOARD,
                code,
                type_,
                datum.root_x as f64,
                datum.root_y as f64,
            ) {
                if let Some(callback) = OWNED_CALLBACK.lock().unwrap().as_mut() {
                    callback(event);
                }
            }
        }
    }
    xrecord::XRecordFreeData(raw_data);
}
