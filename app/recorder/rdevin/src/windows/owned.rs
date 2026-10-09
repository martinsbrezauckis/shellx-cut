use crate::rdevin::{Event, NativePointerSample};
use crate::windows::common::{convert, get_scan_code};
use crate::OwnedListenerJoinError;
use lazy_static::lazy_static;
use std::os::raw::c_int;
use std::ptr::null_mut;
use std::sync::{mpsc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::time::SystemTime;
use winapi::shared::basetsd::ULONG_PTR;
use winapi::shared::minwindef::{DWORD, LPARAM, LRESULT, WPARAM};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::processthreadsapi::GetCurrentThreadId;
use winapi::um::winuser::{
    CallNextHookEx, DispatchMessageW, GetMessageW, GetPhysicalCursorPos, PeekMessageW,
    PostThreadMessageW, SetWindowsHookExA, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, MSG,
    MSLLHOOKSTRUCT, PKBDLLHOOKSTRUCT, PM_NOREMOVE, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP,
};

const OWNED_STOP_MESSAGE: u32 = WM_APP + 0x412;
type OwnedEventCallback = Box<dyn FnMut(Event) + Send>;

lazy_static! {
    static ref OWNED_CALLBACK: Mutex<Option<OwnedEventCallback>> = Mutex::new(None);
}

/// The listener thread owns hook handles and unhooks both after it receives this
/// private message. The controller only ever posts that message to its queue.
pub(crate) struct OwnedListener {
    thread_id: DWORD,
    thread: Option<JoinHandle<()>>,
    exited: mpsc::Receiver<Result<(), super::ListenError>>,
}

impl OwnedListener {
    pub(crate) fn start<T>(callback: T) -> Result<Self, super::ListenError>
    where
        T: FnMut(Event) + Send + 'static,
    {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (exit_tx, exit_rx) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            let outcome = owned_listen(callback, ready_tx);
            let _ = exit_tx.send(outcome);
        });
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
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
        // SAFETY: the message queue is created before the start acknowledgement;
        // the private message is handled only by this listener thread.
        if unsafe { PostThreadMessageW(self.thread_id, OWNED_STOP_MESSAGE, 0, 0) } == 0 {
            return Err(super::ListenError::StopMessage(unsafe { GetLastError() }));
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
    ready: mpsc::SyncSender<Result<DWORD, ()>>,
) -> Result<(), super::ListenError>
where
    T: FnMut(Event) + Send + 'static,
{
    // `PostThreadMessageW` requires a queue. A non-removing peek creates it
    // before the controller receives the thread id.
    let mut message: MSG = unsafe { std::mem::zeroed() };
    unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE) };

    *OWNED_CALLBACK.lock().unwrap() = Some(Box::new(callback));
    let keyboard_hook =
        unsafe { SetWindowsHookExA(WH_KEYBOARD_LL, Some(keyboard_callback), null_mut(), 0) };
    if keyboard_hook.is_null() {
        *OWNED_CALLBACK.lock().unwrap() = None;
        let error = unsafe { GetLastError() };
        let _ = ready.send(Err(()));
        return Err(super::ListenError::Hook(super::common::HookError::Key(
            error,
        )));
    }
    let mouse_hook = if crate::keyboard_only() {
        null_mut()
    } else {
        unsafe { SetWindowsHookExA(WH_MOUSE_LL, Some(mouse_callback), null_mut(), 0) }
    };
    if !crate::keyboard_only() && mouse_hook.is_null() {
        unsafe { UnhookWindowsHookEx(keyboard_hook) };
        *OWNED_CALLBACK.lock().unwrap() = None;
        let error = unsafe { GetLastError() };
        let _ = ready.send(Err(()));
        return Err(super::ListenError::Hook(super::common::HookError::Mouse(
            error,
        )));
    }
    if ready.send(Ok(unsafe { GetCurrentThreadId() })).is_err() {
        unsafe {
            if !mouse_hook.is_null() {
                UnhookWindowsHookEx(mouse_hook);
            }
            UnhookWindowsHookEx(keyboard_hook);
        }
        *OWNED_CALLBACK.lock().unwrap() = None;
        return Err(super::ListenError::ListenerThread);
    }

    let loop_result = loop {
        let status = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if status == -1 {
            break Err(super::ListenError::MessageLoop(unsafe { GetLastError() }));
        }
        if status == 0 || message.message == OWNED_STOP_MESSAGE {
            break Ok(());
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    };
    unsafe {
        if !mouse_hook.is_null() {
            UnhookWindowsHookEx(mouse_hook);
        }
        UnhookWindowsHookEx(keyboard_hook);
    }
    *OWNED_CALLBACK.lock().unwrap() = None;
    loop_result
}

unsafe fn dispatch(
    code: c_int,
    param: WPARAM,
    lpdata: LPARAM,
    extra_data: impl FnOnce(isize) -> ULONG_PTR,
    native_pointer: Option<(i32, i32, u32)>,
) -> LRESULT {
    if code == HC_ACTION {
        let (event_type, platform_code) = convert(param, lpdata);
        if let Some(event_type) = event_type {
            let mut event = Event {
                native_pointer: None,
                event_type,
                time: SystemTime::now(),
                unicode: None,
                platform_code: platform_code as _,
                position_code: get_scan_code(lpdata),
                usb_hid: 0,
                extra_data: extra_data(lpdata),
            };
            if let Some(callback) = OWNED_CALLBACK.lock().unwrap().as_mut() {
                // Compute age at delivery after conversion and callback admission,
                // so any queue/lock delay remains part of the freshness bound.
                event.native_pointer = native_pointer.map(|(x, y, time)| {
                    NativePointerSample::windows_payload(x, y, time, GetTickCount())
                });
                callback(event);
            }
        }
    }
    // This is a passive low-level hook: every callback forwards unmodified input.
    CallNextHookEx(null_mut(), code, param, lpdata)
}

unsafe extern "system" fn mouse_callback(code: i32, param: usize, lpdata: isize) -> isize {
    // The callback payload is MSLLHOOKSTRUCT (not the ordinary mouse-hook struct).
    let point = (code == HC_ACTION).then(|| {
        let mouse = unsafe { &*(lpdata as *const MSLLHOOKSTRUCT) };
        (mouse.pt.x, mouse.pt.y, mouse.time)
    });
    dispatch(
        code,
        param,
        lpdata,
        |data| unsafe { (*(data as *const MSLLHOOKSTRUCT)).dwExtraInfo },
        point,
    )
}

unsafe extern "system" fn keyboard_callback(code: i32, param: usize, lpdata: isize) -> isize {
    dispatch(
        code,
        param,
        lpdata,
        |data| unsafe { (*(data as PKBDLLHOOKSTRUCT)).dwExtraInfo },
        None,
    )
}

#[link(name = "kernel32")]
extern "system" {
    fn GetTickCount() -> u32;
}

/// Passive physical desktop position, not an injected MouseMove or cursor setter.
pub(crate) fn current_pointer_position() -> Option<(f64, f64)> {
    let mut point = winapi::shared::windef::POINT { x: 0, y: 0 };
    (unsafe { GetPhysicalCursorPos(&mut point) } != 0)
        .then_some((f64::from(point.x), f64::from(point.y)))
}
