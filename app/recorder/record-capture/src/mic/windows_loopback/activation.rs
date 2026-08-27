use record_core::{error_codes, RecordError, Result};
use std::sync::{Arc, Mutex};
use windows::core::implement;
use windows::Win32::Media::Audio::{
    IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Impl,
};
use windows::Win32::System::Com::{IAgileObject, IAgileObject_Impl};

pub(super) struct ComApartment(bool);

impl ComApartment {
    pub(super) fn initialize() -> Result<Self> {
        use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

        // SAFETY: the capture worker owns this thread for its whole lifetime. A changed-mode
        // result means a caller already initialized COM, so COM is usable but not ours to undo.
        let status = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if status.is_ok() {
            Ok(Self(true))
        } else if status == RPC_E_CHANGED_MODE {
            Ok(Self(false))
        } else {
            Err(RecordError::new(
                error_codes::CAPTURE,
                "initialize system audio",
                format!("CoInitializeEx failed: {status:?}"),
            ))
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: paired with this thread's successful CoInitializeEx call.
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

pub(super) fn windows_error(stage: &'static str, error: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, stage, error.to_string())
}

pub(super) fn trace_windows_loopback(stage: impl std::fmt::Display) {
    if std::env::var_os("SHELLX_CAPTURE_TRACE").is_some() {
        eprintln!("windows-loopback: {stage}");
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
pub(super) struct ProcessLoopbackActivation {
    pub(super) result: Arc<(Mutex<Option<Result<MtaAudioClient>>>, std::sync::Condvar)>,
    pub(super) event: Arc<WindowsEvent>,
}

pub(super) struct MtaAudioClient(pub(super) windows::Win32::Media::Audio::IAudioClient);

// SAFETY: COM has one process-wide multithreaded apartment. ActivateCompleted runs in that MTA,
// and capture_process_loopback joins the MTA before requesting activation. The result mutex moves
// this interface exactly once to that worker, where all later calls and the final release occur.
unsafe impl Send for MtaAudioClient {}

pub(super) const PROCESS_LOOPBACK_CHANNELS: u16 = 2;
pub(super) const PROCESS_LOOPBACK_SAMPLE_RATE: u32 = 48_000;
pub(super) const PROCESS_LOOPBACK_BITS_PER_SAMPLE: u16 = 16;
pub(super) const PROCESS_LOOPBACK_BLOCK_ALIGN: u16 =
    PROCESS_LOOPBACK_CHANNELS * (PROCESS_LOOPBACK_BITS_PER_SAMPLE / 8);

pub(super) fn initialize_process_loopback(
    operation: &IActivateAudioInterfaceAsyncOperation,
    event: &WindowsEvent,
) -> Result<MtaAudioClient> {
    use windows::core::{Interface, HRESULT};
    use windows::Win32::Media::Audio::{
        IAudioClient, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX,
    };

    // Microsoft initializes the process-loopback client inside ActivateCompleted. Keeping
    // activation and initialization on that callback's MTA avoids driver-dependent hangs that
    // occur when Initialize is deferred to the thread waiting for the callback.
    unsafe {
        let mut activation_status = HRESULT(0);
        let mut activated = None;
        operation
            .GetActivateResult(&mut activation_status, &mut activated)
            .map_err(|e| windows_error("complete system audio activation", e))?;
        activation_status
            .ok()
            .map_err(|e| windows_error("complete system audio activation", e))?;
        let audio_client: IAudioClient = activated
            .ok_or_else(|| {
                RecordError::new(
                    error_codes::CAPTURE,
                    "complete system audio activation",
                    "Windows returned no process-loopback audio client",
                )
            })?
            .cast()
            .map_err(|e| windows_error("open system audio client", e))?;
        trace_windows_loopback("process-loopback callback opened audio client");

        let format = WAVEFORMATEX {
            wFormatTag: 1,
            nChannels: PROCESS_LOOPBACK_CHANNELS,
            nSamplesPerSec: PROCESS_LOOPBACK_SAMPLE_RATE,
            nAvgBytesPerSec: PROCESS_LOOPBACK_SAMPLE_RATE * u32::from(PROCESS_LOOPBACK_BLOCK_ALIGN),
            nBlockAlign: PROCESS_LOOPBACK_BLOCK_ALIGN,
            wBitsPerSample: PROCESS_LOOPBACK_BITS_PER_SAMPLE,
            cbSize: 0,
        };
        audio_client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK
                    | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                0,
                0,
                &format,
                None,
            )
            .map_err(|e| windows_error("initialize system audio loopback", e))?;
        let buffer_frames = audio_client
            .GetBufferSize()
            .map_err(|e| windows_error("read system audio buffer size", e))?;
        audio_client
            .SetEventHandle(event.0)
            .map_err(|e| windows_error("bind system audio event", e))?;
        trace_windows_loopback(format_args!(
            "process-loopback callback initialized audio client ({buffer_frames} frames)"
        ));

        Ok(MtaAudioClient(audio_client))
    }
}

impl IActivateAudioInterfaceCompletionHandler_Impl for ProcessLoopbackActivation_Impl {
    fn ActivateCompleted(
        &self,
        operation: windows::core::Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let result = operation
            .ok()
            .map_err(|e| windows_error("complete system audio activation", e))
            .and_then(|operation| initialize_process_loopback(operation, &self.event));
        match &result {
            Ok(_) => trace_windows_loopback("process-loopback callback completed"),
            Err(error) => {
                trace_windows_loopback(format_args!("process-loopback callback failed: {error}"))
            }
        }
        let mut slot = self
            .result
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = Some(result);
        self.result.1.notify_all();
        Ok(())
    }
}

impl IAgileObject_Impl for ProcessLoopbackActivation_Impl {}

pub(super) struct WindowsEvent(pub(super) windows::Win32::Foundation::HANDLE);

// SAFETY: this auto-reset kernel event supports concurrent SetEventHandle/WaitForSingleObject
// use, and Arc ownership prevents CloseHandle until every user releases it.
unsafe impl Send for WindowsEvent {}
// SAFETY: the event operation and lifetime invariants above also permit shared references.
unsafe impl Sync for WindowsEvent {}

impl WindowsEvent {
    pub(super) fn new() -> Result<Self> {
        // SAFETY: creates an unnamed auto-reset event owned by this RAII wrapper.
        let handle = unsafe {
            windows::Win32::System::Threading::CreateEventW(
                None,
                false,
                false,
                windows::core::PCWSTR::null(),
            )
        }
        .map_err(|e| windows_error("create system audio event", e))?;
        Ok(Self(handle))
    }
}

impl Drop for WindowsEvent {
    fn drop(&mut self) {
        // SAFETY: this handle was returned by CreateEventW and is closed exactly once.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}
