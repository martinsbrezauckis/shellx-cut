//! mic.rs — microphone / voice capture via cpal (pure-Rust, lightweight) → WAV.
//!
//! Compiled under the `mic` feature (on for capture-windows / capture-macos).
//! Records the DEFAULT input device for the recording, on its own thread, synced
//! to the same capture clock as the screen (both start together; stopped by the
//! same `stop` flag). Written as 16-bit WAV; the renderer muxes it into the output.
//! This is the AUDIO (you hear it); Cut's Parakeet STT separately turns this same
//! WAV into caption text — capture and transcription are distinct, chained layers.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use record_core::{error_codes, RecordError, Result};
#[cfg(windows)]
use windows::core::implement;
#[cfg(windows)]
use windows::Win32::Media::Audio::{
    IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Impl,
};
#[cfg(windows)]
use windows::Win32::System::Com::{IAgileObject, IAgileObject_Impl};

const MIC_JOIN_GRACE: Duration = Duration::from_millis(750);
static MIC_WARM_ACTIVE: AtomicBool = AtomicBool::new(false);
static MIC_WARM_SEQ: AtomicU64 = AtomicU64::new(0);

fn wait_for_thread<T>(handle: &JoinHandle<T>, max_wait: Duration) -> bool {
    let started = std::time::Instant::now();
    while !handle.is_finished() && started.elapsed() < max_wait {
        thread::sleep(Duration::from_millis(20));
    }
    handle.is_finished()
}

/// Join a microphone worker without allowing a stuck native audio driver to
/// block screen-capture finalization forever. A timed-out handle is detached;
/// its shared stop flag has already been set by every caller.
pub(crate) fn join_bounded(
    handle: JoinHandle<Result<String>>,
    max_wait: Duration,
) -> Option<Result<String>> {
    if wait_for_thread(&handle, max_wait) {
        handle.join().ok()
    } else {
        None
    }
}

/// Spawn default-mic capture on a thread; streams a 16-bit WAV to `path` until
/// `stop` is set, and returns the path. `ready` is set true on the FIRST audio
/// callback — i.e. once samples actually flow (after any permission prompt) — so
/// the caller can wait for a live mic before starting the synced capture window.
pub fn spawn_mic(
    path: String,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
) -> JoinHandle<Result<String>> {
    // Do not open a second cpal stream while the pre-warm worker is still
    // shutting down. On affected Windows drivers that worker can remain inside
    // native device initialization after its deadline; video capture must still
    // start and finish, honestly without microphone audio.
    if MIC_WARM_ACTIVE.load(Ordering::Acquire) {
        return thread::spawn(|| {
            Err(RecordError::new(
                error_codes::CAPTURE,
                "microphone warm-up is still stopping",
                "the native microphone driver did not finish its bounded warm-up",
            )
            .with_action(
                "record without microphone audio or retry after the audio device becomes responsive",
            ))
        });
    }
    thread::spawn(move || run(path, stop, ready))
}

/// Warm the default microphone for up to `max_ms`. Opens it via the SAME cpal
/// path the recorder uses (so the OS permission prompt is answered NOW and the stream
/// spins up), waits for the first audio callback, then stops. Returns
/// `(went_live, device_name)`; the throwaway WAV is removed. Called on entering the
/// Record surface so a short FIRST recording doesn't finish before the just-granted
/// mic starts flowing — the capture's own in-line 8 s warm loses that race for very
/// short clips when the user is still answering the permission dialog.
pub fn warm(max_ms: u64) -> (bool, Option<String>) {
    // Only one warm worker may own the default device. Re-entering the Record
    // surface while a native driver is stuck must return immediately rather
    // than accumulate unkillable cpal threads.
    if MIC_WARM_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return (false, None);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(AtomicBool::new(false));
    let device = Arc::new(Mutex::new(None));
    let seq = MIC_WARM_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!(
        "shellx_mic_warm_{}_{}.wav",
        std::process::id(),
        seq
    ));
    let worker_tmp = tmp.clone();
    let worker_stop = stop.clone();
    let worker_ready = ready.clone();
    let worker_device = device.clone();
    let handle = thread::spawn(move || {
        let name = cpal::default_host()
            .default_input_device()
            .and_then(|d| d.name().ok());
        *worker_device
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = name;
        run(
            worker_tmp.to_string_lossy().into_owned(),
            worker_stop,
            worker_ready,
        )
    });
    let started = std::time::Instant::now();
    let budget = Duration::from_millis(max_ms.max(200));
    while !ready.load(Ordering::Relaxed) && started.elapsed() < budget {
        thread::sleep(Duration::from_millis(20));
    }
    let live = ready.load(Ordering::Relaxed);
    stop.store(true, Ordering::Relaxed);
    let name = device
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if wait_for_thread(&handle, MIC_JOIN_GRACE) {
        let _ = handle.join();
        let _ = std::fs::remove_file(&tmp);
        MIC_WARM_ACTIVE.store(false, Ordering::Release);
    } else {
        // Joining a cpal worker is itself unbounded on some Windows drivers.
        // Reap and clean it asynchronously if it ever returns, while the API
        // answers on time and the single-flight guard prevents another warm.
        let _ = thread::Builder::new()
            .name("cut-mic-warm-reaper".into())
            .spawn(move || {
                let _ = handle.join();
                let _ = std::fs::remove_file(&tmp);
                MIC_WARM_ACTIVE.store(false, Ordering::Release);
            });
    }
    (live, name)
}

fn run(path: String, stop: Arc<AtomicBool>, ready: Arc<AtomicBool>) -> Result<String> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| {
        RecordError::new(
            error_codes::CAPTURE,
            "no input device",
            "no default microphone found",
        )
        .with_action("connect/enable a microphone and grant mic permission")
    })?;
    let supported = device
        .default_input_config()
        .map_err(|e| RecordError::new(error_codes::CAPTURE, "mic config", e.to_string()))?;
    run_stream(&device, supported, &path, None, &stop, &ready)?;
    Ok(path)
}

#[cfg(any(windows, test))]
fn decode_process_loopback_packet(
    data: Option<&[u8]>,
    frames: u32,
    silent: bool,
) -> Result<Vec<i16>> {
    const CHANNELS: usize = 2;
    const BLOCK_ALIGN: usize = CHANNELS * std::mem::size_of::<i16>();
    let sample_count = usize::try_from(frames)
        .ok()
        .and_then(|count| count.checked_mul(CHANNELS))
        .ok_or_else(|| {
            RecordError::new(
                error_codes::CAPTURE,
                "decode system audio",
                "WASAPI packet size overflowed",
            )
        })?;
    if silent {
        return Ok(vec![0; sample_count]);
    }
    let expected = usize::try_from(frames)
        .ok()
        .and_then(|count| count.checked_mul(BLOCK_ALIGN))
        .ok_or_else(|| {
            RecordError::new(
                error_codes::CAPTURE,
                "decode system audio",
                "WASAPI packet byte size overflowed",
            )
        })?;
    let data = data
        .filter(|bytes| bytes.len() >= expected)
        .ok_or_else(|| {
            RecordError::new(
                error_codes::CAPTURE,
                "decode system audio",
                "WASAPI returned a truncated audio packet",
            )
        })?;

    Ok(data[..expected]
        .chunks_exact(2)
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
        .collect())
}

#[cfg(any(windows, test))]
fn process_loopback_expected_samples(
    elapsed: Duration,
    channels: u16,
    sample_rate: u32,
) -> Result<usize> {
    elapsed
        .as_nanos()
        .checked_mul(u128::from(sample_rate))
        .and_then(|frames| frames.checked_div(1_000_000_000))
        .and_then(|frames| frames.checked_mul(u128::from(channels)))
        .and_then(|count| usize::try_from(count).ok())
        .ok_or_else(|| {
            RecordError::new(
                error_codes::CAPTURE,
                "pad system audio",
                "system-audio capture duration overflowed",
            )
        })
}

// Leave room for the RIFF/WAVE headers before hound's u32 byte counter reaches its limit.
// The limit is frame-aligned so every published WAV remains valid for multichannel input.
const WAV_HEADER_MARGIN_BYTES: u64 = 4_096;
const MIC_PENDING_SECONDS: u64 = 2;

#[derive(Default)]
struct PendingMicSamples {
    samples: Vec<i16>,
    overflowed: bool,
}

impl PendingMicSamples {
    fn extend<I>(&mut self, samples: I, incoming_len: usize, limit: usize)
    where
        I: IntoIterator<Item = i16>,
    {
        let remaining = limit.saturating_sub(self.samples.len());
        if incoming_len > remaining {
            self.overflowed = true;
        }
        self.samples.extend(samples.into_iter().take(remaining));
    }
}

fn wav_i16_sample_capacity(channels: u16) -> Result<u64> {
    let frame_bytes = u64::from(channels).checked_mul(2).ok_or_else(|| {
        RecordError::new(
            error_codes::CAPTURE,
            "microphone format",
            "microphone frame size overflowed",
        )
    })?;
    if frame_bytes == 0 {
        return Err(RecordError::new(
            error_codes::CAPTURE,
            "microphone format",
            "microphone reported zero channels",
        ));
    }
    let data_bytes = (u64::from(u32::MAX) - WAV_HEADER_MARGIN_BYTES) / frame_bytes * frame_bytes;
    Ok(data_bytes / 2)
}

fn remove_file_if_exists(path: &std::path::Path, stage: &'static str) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RecordError::new(error_codes::IO, stage, error.to_string())),
    }
}

#[cfg(windows)]
struct ComApartment(bool);

#[cfg(windows)]
impl ComApartment {
    fn initialize() -> Result<Self> {
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

#[cfg(windows)]
impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: paired with this thread's successful CoInitializeEx call.
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

#[cfg(windows)]
fn windows_error(stage: &'static str, error: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, stage, error.to_string())
}

#[cfg(windows)]
fn trace_windows_loopback(stage: impl std::fmt::Display) {
    if std::env::var_os("SHELLX_CAPTURE_TRACE").is_some() {
        eprintln!("windows-loopback: {stage}");
    }
}

#[cfg(windows)]
#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct ProcessLoopbackActivation {
    result: Arc<(Mutex<Option<Result<MtaAudioClient>>>, std::sync::Condvar)>,
    event: Arc<WindowsEvent>,
}

#[cfg(windows)]
struct MtaAudioClient(windows::Win32::Media::Audio::IAudioClient);

// SAFETY: COM has one process-wide multithreaded apartment. ActivateCompleted runs in that MTA,
// and capture_process_loopback joins the MTA before requesting activation. The result mutex moves
// this interface exactly once to that worker, where all later calls and the final release occur.
#[cfg(windows)]
unsafe impl Send for MtaAudioClient {}

#[cfg(windows)]
const PROCESS_LOOPBACK_CHANNELS: u16 = 2;
#[cfg(windows)]
const PROCESS_LOOPBACK_SAMPLE_RATE: u32 = 48_000;
#[cfg(windows)]
const PROCESS_LOOPBACK_BITS_PER_SAMPLE: u16 = 16;
#[cfg(windows)]
const PROCESS_LOOPBACK_BLOCK_ALIGN: u16 =
    PROCESS_LOOPBACK_CHANNELS * (PROCESS_LOOPBACK_BITS_PER_SAMPLE / 8);

#[cfg(windows)]
fn initialize_process_loopback(
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

#[cfg(windows)]
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

#[cfg(windows)]
impl IAgileObject_Impl for ProcessLoopbackActivation_Impl {}

#[cfg(windows)]
struct WindowsEvent(windows::Win32::Foundation::HANDLE);

// SAFETY: this auto-reset kernel event supports concurrent SetEventHandle/WaitForSingleObject
// use, and Arc ownership prevents CloseHandle until every user releases it.
#[cfg(windows)]
unsafe impl Send for WindowsEvent {}
// SAFETY: the event operation and lifetime invariants above also permit shared references.
#[cfg(windows)]
unsafe impl Sync for WindowsEvent {}

#[cfg(windows)]
impl WindowsEvent {
    fn new() -> Result<Self> {
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

#[cfg(windows)]
impl Drop for WindowsEvent {
    fn drop(&mut self) {
        // SAFETY: this handle was returned by CreateEventW and is closed exactly once.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// Capture desktop audio through Windows' endpoint-independent process-loopback device.
/// Excluding Cut's daemon process tree yields the rest of the system mix without opening a
/// physical render driver. Requires Windows build 20348+; older builds fail cleanly.
#[cfg(windows)]
pub fn capture_system_loopback(
    path: &str,
    max_ms: Option<u64>,
    stop: Arc<AtomicBool>,
) -> Result<String> {
    capture_process_loopback(path, max_ms, stop)
}

/// Process-loopback implementation. Writes a fixed 48 kHz stereo 16-bit WAV.
#[cfg(windows)]
fn capture_process_loopback(
    path: &str,
    max_ms: Option<u64>,
    stop: Arc<AtomicBool>,
) -> Result<String> {
    use std::mem::{size_of, ManuallyDrop};
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{
        ActivateAudioInterfaceAsync, IAudioCaptureClient, IAudioClient, AUDCLNT_BUFFERFLAGS_SILENT,
        AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
        AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
        PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    };
    use windows::Win32::System::Com::StructuredStorage::{
        PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
    };
    use windows::Win32::System::Com::BLOB;
    use windows::Win32::System::Threading::WaitForSingleObject;
    use windows::Win32::System::Variant::VT_BLOB;

    // Classic WAV uses 32-bit chunk lengths. Six hours of 48 kHz stereo i16 is
    // 4,147,200,000 data bytes, leaving enough header/packet margin below 4 GiB.
    const LOOPBACK_CEILING_MS: u64 = 6 * 60 * 60 * 1000;
    let limit =
        Duration::from_millis(max_ms.map_or(LOOPBACK_CEILING_MS, |ms| ms.min(LOOPBACK_CEILING_MS)));
    let _apartment = ComApartment::initialize()?;
    trace_windows_loopback("process-loopback COM initialized");

    let mut activation = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: std::process::id(),
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let activation_blob = BLOB {
        cbSize: u32::try_from(size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>()).unwrap(),
        pBlobData: (&mut activation as *mut AUDIOCLIENT_ACTIVATION_PARAMS).cast(),
    };
    // This VT_BLOB borrows `activation`; PROPVARIANT's generated Drop calls
    // PropVariantClear and would otherwise try to free that stack-owned pointer.
    let activation_variant = ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: activation_blob,
                },
            }),
        },
    });
    let event = Arc::new(WindowsEvent::new()?);
    let activation_result = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
    let handler: IActivateAudioInterfaceCompletionHandler = ProcessLoopbackActivation {
        result: activation_result.clone(),
        event: event.clone(),
    }
    .into();

    // SAFETY: COM is initialized; activation parameters remain valid for the async-call entry;
    // the callback owns the shared event/result even after a waiter timeout; and every acquired
    // capture packet is released before the next GetBuffer call.
    unsafe {
        let _operation = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &<IAudioClient as Interface>::IID,
            Some(&*activation_variant),
            &handler,
        )
        .map_err(|e| windows_error("activate system audio loopback", e))?;
        trace_windows_loopback("process-loopback activation requested");

        let result = activation_result
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut result, wait) = activation_result
            .1
            .wait_timeout_while(result, Duration::from_secs(10), |value| value.is_none())
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if result.is_none() && wait.timed_out() {
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "activate system audio loopback",
                "Windows did not complete process-loopback activation within 10 seconds",
            ));
        }
        trace_windows_loopback("process-loopback activation completed");
        let audio_client = result
            .take()
            .ok_or_else(|| {
                RecordError::new(
                    error_codes::CAPTURE,
                    "activate system audio loopback",
                    "Windows completed process-loopback activation without a result",
                )
            })??
            .0;
        trace_windows_loopback("process-loopback audio client transferred within MTA");
        let capture: IAudioCaptureClient = audio_client
            .GetService()
            .map_err(|e| windows_error("open system audio capture client", e))?;
        let output_path = std::path::Path::new(path);
        let temp_path = std::path::PathBuf::from(format!("{path}.part"));
        remove_file_if_exists(&temp_path, "clear partial system audio")?;
        let spec = hound::WavSpec {
            channels: PROCESS_LOOPBACK_CHANNELS,
            sample_rate: PROCESS_LOOPBACK_SAMPLE_RATE,
            bits_per_sample: PROCESS_LOOPBACK_BITS_PER_SAMPLE,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = match hound::WavWriter::create(&temp_path, spec) {
            Ok(writer) => writer,
            Err(error) => {
                let _ = remove_file_if_exists(&temp_path, "remove partial system audio");
                return Err(RecordError::new(
                    error_codes::IO,
                    "create system audio",
                    error.to_string(),
                ));
            }
        };
        if let Err(error) = audio_client.Start() {
            drop(writer);
            let _ = remove_file_if_exists(&temp_path, "remove partial system audio");
            return Err(windows_error("start system audio loopback", error));
        }
        trace_windows_loopback("process-loopback audio client started");

        let started = std::time::Instant::now();
        let capture_result = (|| -> Result<(usize, Duration)> {
            let mut written_samples = 0_usize;
            let mut packets = 0_u64;
            let mut silent_packets = 0_u64;
            let mut saw_event = false;
            'capture: while !stop.load(Ordering::Relaxed) && started.elapsed() < limit {
                let wait = WaitForSingleObject(event.0, 50);
                if wait == windows::Win32::Foundation::WAIT_FAILED {
                    return Err(windows_error(
                        "wait for system audio",
                        windows::core::Error::from_thread(),
                    ));
                }
                if !saw_event {
                    trace_windows_loopback(format_args!(
                        "process-loopback first event wait returned {wait:?}"
                    ));
                    saw_event = true;
                }
                loop {
                    if stop.load(Ordering::Relaxed) || started.elapsed() >= limit {
                        break 'capture;
                    }
                    if packets == 0 {
                        trace_windows_loopback("process-loopback requesting first packet size");
                    }
                    let packet_frames = capture
                        .GetNextPacketSize()
                        .map_err(|e| windows_error("read system audio packet size", e))?;
                    if packet_frames == 0 {
                        break;
                    }
                    if packets == 0 {
                        trace_windows_loopback(format_args!(
                            "process-loopback first packet announced: {packet_frames} frames"
                        ));
                    }
                    let mut data = std::ptr::null_mut();
                    let mut frames = 0;
                    let mut flags = 0;
                    if packets == 0 {
                        trace_windows_loopback("process-loopback acquiring first packet");
                    }
                    capture
                        .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                        .map_err(|e| windows_error("read system audio packet", e))?;
                    if packets == 0 {
                        trace_windows_loopback(format_args!(
                            "process-loopback first packet acquired: {frames} frames, flags {flags:#x}"
                        ));
                    }
                    let byte_len = usize::try_from(frames)
                        .ok()
                        .and_then(|count| {
                            count.checked_mul(usize::from(PROCESS_LOOPBACK_BLOCK_ALIGN))
                        })
                        .ok_or_else(|| {
                            RecordError::new(
                                error_codes::CAPTURE,
                                "read system audio packet",
                                "WASAPI packet byte size overflowed",
                            )
                        });
                    let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
                    packets += 1;
                    if silent {
                        silent_packets += 1;
                    }
                    let decoded = byte_len.and_then(|len| {
                        let packet = if silent || data.is_null() {
                            None
                        } else {
                            Some(std::slice::from_raw_parts(data, len))
                        };
                        decode_process_loopback_packet(packet, frames, silent)
                    });
                    let release = capture.ReleaseBuffer(frames);
                    let decoded = decoded?;
                    release.map_err(|e| windows_error("release system audio packet", e))?;
                    if packets == 1 {
                        trace_windows_loopback("process-loopback first packet released");
                    }
                    written_samples =
                        written_samples.checked_add(decoded.len()).ok_or_else(|| {
                            RecordError::new(
                                error_codes::CAPTURE,
                                "write system audio",
                                "system-audio sample count overflowed",
                            )
                        })?;
                    for sample in decoded {
                        writer.write_sample(sample).map_err(|e| {
                            RecordError::new(error_codes::IO, "write system audio", e.to_string())
                        })?;
                    }
                }
            }
            let elapsed = started.elapsed().min(limit);
            trace_windows_loopback(format_args!(
                "process-loopback ended after {} ms: {packets} packets, {silent_packets} silent, {} samples",
                elapsed.as_millis(),
                written_samples
            ));
            Ok((written_samples, elapsed))
        })();
        let stopped = audio_client.Stop();
        trace_windows_loopback("process-loopback stop returned");
        let (mut written_samples, elapsed) = match capture_result {
            Ok(capture) => capture,
            Err(error) => {
                drop(writer);
                let _ = remove_file_if_exists(&temp_path, "remove partial system audio");
                return Err(error);
            }
        };
        if let Err(error) = stopped {
            drop(writer);
            let _ = remove_file_if_exists(&temp_path, "remove partial system audio");
            return Err(windows_error("stop system audio loopback", error));
        }
        let expected_samples = match process_loopback_expected_samples(
            elapsed,
            PROCESS_LOOPBACK_CHANNELS,
            PROCESS_LOOPBACK_SAMPLE_RATE,
        ) {
            Ok(samples) => samples,
            Err(error) => {
                drop(writer);
                let _ = remove_file_if_exists(&temp_path, "remove partial system audio");
                return Err(error);
            }
        };
        while written_samples < expected_samples {
            if let Err(error) = writer.write_sample(0_i16) {
                drop(writer);
                let _ = remove_file_if_exists(&temp_path, "remove partial system audio");
                return Err(RecordError::new(
                    error_codes::IO,
                    "pad system audio",
                    error.to_string(),
                ));
            }
            written_samples += 1;
        }
        if let Err(error) = writer.finalize() {
            let _ = remove_file_if_exists(&temp_path, "remove partial system audio");
            return Err(RecordError::new(
                error_codes::IO,
                "finalize system audio",
                error.to_string(),
            ));
        }
        if let Err(error) = remove_file_if_exists(output_path, "replace system audio") {
            let _ = remove_file_if_exists(&temp_path, "remove partial system audio");
            return Err(error);
        }
        if let Err(error) = std::fs::rename(&temp_path, output_path) {
            let _ = remove_file_if_exists(&temp_path, "remove partial system audio");
            return Err(RecordError::new(
                error_codes::IO,
                "publish system audio",
                error.to_string(),
            ));
        }
        trace_windows_loopback(format_args!(
            "process-loopback WAV written: {written_samples} samples"
        ));
        // The event handle must outlive every COM interface that can still signal it.
        drop(capture);
        drop(audio_client);
        drop(event);
    }
    Ok(path.to_string())
}

/// cpal capture body for the default microphone. The real-time callback only converts and
/// queues samples; this worker drains them to disk so long recordings use bounded memory.
/// `ready` flips true on the first audio callback.
fn run_stream(
    device: &cpal::Device,
    supported: cpal::SupportedStreamConfig,
    path: &str,
    max_ms: Option<u64>,
    stop: &Arc<AtomicBool>,
    ready: &Arc<AtomicBool>,
) -> Result<()> {
    let fmt = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels;
    let sample_rate = config.sample_rate.0;
    let max_pending_samples = u64::from(sample_rate)
        .checked_mul(u64::from(channels))
        .and_then(|samples| samples.checked_mul(MIC_PENDING_SECONDS))
        .and_then(|samples| usize::try_from(samples).ok())
        .ok_or_else(|| {
            RecordError::new(
                error_codes::CAPTURE,
                "microphone format",
                "microphone callback buffer size overflowed",
            )
        })?;

    let pending_samples = Arc::new(Mutex::new(PendingMicSamples::default()));
    let stream_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let err_fn = {
        let stream_error = stream_error.clone();
        move |error: cpal::StreamError| {
            eprintln!("audio stream error: {error}");
            let mut slot = stream_error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.is_none() {
                *slot = Some(error.to_string());
            }
        }
    };

    let b = pending_samples.clone();
    let rdy = ready.clone();
    let stream = match fmt {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |d: &[f32], _: &cpal::InputCallbackInfo| {
                rdy.store(true, Ordering::Relaxed);
                let mut g = b.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                g.extend(
                    d.iter().map(|&v| (v.clamp(-1.0, 1.0) * 32767.0) as i16),
                    d.len(),
                    max_pending_samples,
                );
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |d: &[i16], _: &cpal::InputCallbackInfo| {
                rdy.store(true, Ordering::Relaxed);
                b.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend(d.iter().copied(), d.len(), max_pending_samples);
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &config,
            move |d: &[u16], _: &cpal::InputCallbackInfo| {
                rdy.store(true, Ordering::Relaxed);
                let mut g = b.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                g.extend(
                    d.iter().map(|&v| (v as i32 - 32768) as i16),
                    d.len(),
                    max_pending_samples,
                );
            },
            err_fn,
            None,
        ),
        other => {
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "audio format",
                format!("unsupported sample format {other:?}"),
            ))
        }
    }
    .map_err(|e| RecordError::new(error_codes::CAPTURE, "build audio stream", e.to_string()))?;
    let sample_capacity = wav_i16_sample_capacity(channels)?;
    let output_path = std::path::Path::new(path);
    let temp_path = std::path::PathBuf::from(format!("{path}.part"));
    remove_file_if_exists(&temp_path, "clear partial microphone audio")?;
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = match hound::WavWriter::create(&temp_path, spec) {
        Ok(writer) => writer,
        Err(error) => {
            let _ = remove_file_if_exists(&temp_path, "remove partial microphone audio");
            return Err(RecordError::new(
                error_codes::IO,
                "create microphone audio",
                error.to_string(),
            ));
        }
    };
    if let Err(error) = stream.play() {
        drop(writer);
        let _ = remove_file_if_exists(&temp_path, "remove partial microphone audio");
        return Err(RecordError::new(
            error_codes::CAPTURE,
            "start microphone audio",
            error.to_string(),
        ));
    }

    let capture_error = {
        let mut written_samples = 0_u64;
        let mut drain_buffer = Vec::new();
        let mut drain_pending = || -> Result<bool> {
            let overflowed = {
                let mut pending = pending_samples
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::swap(&mut drain_buffer, &mut pending.samples);
                std::mem::take(&mut pending.overflowed)
            };
            if overflowed {
                return Err(RecordError::new(
                    error_codes::CAPTURE,
                    "capture microphone audio",
                    "microphone audio could not be written to disk fast enough",
                ));
            }
            let remaining = sample_capacity.saturating_sub(written_samples);
            let write_count = usize::try_from(remaining)
                .unwrap_or(usize::MAX)
                .min(drain_buffer.len());
            for sample in drain_buffer.drain(..write_count) {
                writer.write_sample(sample).map_err(|e| {
                    RecordError::new(error_codes::IO, "write microphone audio", e.to_string())
                })?;
            }
            let write_count = u64::try_from(write_count).map_err(|_| {
                RecordError::new(
                    error_codes::CAPTURE,
                    "write microphone audio",
                    "microphone sample count overflowed",
                )
            })?;
            written_samples += write_count;
            let reached_capacity = !drain_buffer.is_empty() || written_samples >= sample_capacity;
            drain_buffer.clear();
            Ok(reached_capacity)
        };

        let started = std::time::Instant::now();
        let mut capture_error = None;
        while !stop.load(Ordering::Relaxed) {
            if let Some(ms) = max_ms {
                if started.elapsed() >= Duration::from_millis(ms) {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(40));
            if let Some(error) = stream_error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                capture_error = Some(RecordError::new(
                    error_codes::CAPTURE,
                    "capture microphone audio",
                    error,
                ));
                break;
            }
            match drain_pending() {
                Ok(true) => {
                    // Classic WAV cannot represent another full frame. End the shared capture so
                    // video and audio stay aligned instead of silently truncating or corrupting audio.
                    stop.store(true, Ordering::Relaxed);
                    break;
                }
                Ok(false) => {}
                Err(error) => {
                    capture_error = Some(error);
                    break;
                }
            }
        }
        drop(stream); // stop capture

        if capture_error.is_none() {
            if let Some(error) = stream_error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                capture_error = Some(RecordError::new(
                    error_codes::CAPTURE,
                    "capture microphone audio",
                    error,
                ));
            } else if let Err(error) = drain_pending() {
                capture_error = Some(error);
            }
        }
        capture_error
    };
    if let Some(error) = capture_error {
        drop(writer);
        let _ = remove_file_if_exists(&temp_path, "remove partial microphone audio");
        return Err(error);
    }
    if let Err(error) = writer.finalize() {
        let _ = remove_file_if_exists(&temp_path, "remove partial microphone audio");
        return Err(RecordError::new(
            error_codes::IO,
            "finalize microphone audio",
            error.to_string(),
        ));
    }
    if let Err(error) = remove_file_if_exists(output_path, "replace microphone audio") {
        let _ = remove_file_if_exists(&temp_path, "remove partial microphone audio");
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temp_path, output_path) {
        let _ = remove_file_if_exists(&temp_path, "remove partial microphone audio");
        return Err(RecordError::new(
            error_codes::IO,
            "publish microphone audio",
            error.to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        decode_process_loopback_packet, join_bounded, process_loopback_expected_samples,
        wav_i16_sample_capacity, PendingMicSamples, WAV_HEADER_MARGIN_BYTES,
    };
    use record_core::Result;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn decodes_fixed_process_loopback_pcm() {
        let bytes = [i16::MIN, -1, 0, i16::MAX]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>();
        assert_eq!(
            decode_process_loopback_packet(Some(&bytes), 2, false).unwrap(),
            vec![i16::MIN, -1, 0, i16::MAX]
        );
    }

    #[test]
    fn rejects_truncated_process_loopback_packet() {
        assert!(decode_process_loopback_packet(Some(&[0; 7]), 2, false).is_err());
    }

    #[test]
    fn silent_packet_does_not_dereference_audio_data() {
        assert_eq!(
            decode_process_loopback_packet(None, 2, true).unwrap(),
            vec![0; 4]
        );
    }

    #[test]
    fn computes_process_loopback_wall_clock_sample_count() {
        assert_eq!(
            process_loopback_expected_samples(Duration::from_millis(20), 2, 48_000).unwrap(),
            1_920
        );
        assert_eq!(
            process_loopback_expected_samples(Duration::from_secs(6 * 60 * 60), 2, 48_000).unwrap(),
            2_073_600_000
        );
    }

    #[test]
    fn classic_wav_capacity_is_frame_aligned() {
        let stereo_capacity = wav_i16_sample_capacity(2).unwrap();
        assert_eq!(stereo_capacity % 2, 0);
        assert!(stereo_capacity * 2 + WAV_HEADER_MARGIN_BYTES <= u64::from(u32::MAX));
        assert!(wav_i16_sample_capacity(0).is_err());
    }

    #[test]
    fn microphone_callback_backlog_is_bounded() {
        let mut pending = PendingMicSamples::default();
        pending.extend([1_i16, 2, 3, 4], 4, 3);
        assert_eq!(pending.samples, [1, 2, 3]);
        assert!(pending.overflowed);
    }

    #[test]
    fn microphone_join_does_not_wait_for_a_stuck_worker() {
        // The property under test is "join gives up instead of waiting for the
        // worker", so what matters is the SEPARATION between the join timeout
        // and the worker's lifetime — not a tight wall-clock number.
        //
        // This previously slept 150 ms, timed out at 20 ms and asserted under
        // 120 ms, leaving only 30 ms between "gave up early" and "waited for the
        // worker". That is inside normal scheduler noise on shared CI hardware,
        // and it failed on a GitHub macOS runner while passing everywhere else.
        // Widening the gap keeps the proof strict — returning in under 1 s still
        // demonstrates it did not wait for a 3 s worker — while no longer
        // measuring the runner's load.
        let handle = thread::spawn(|| -> Result<String> {
            thread::sleep(Duration::from_secs(3));
            Ok("late.wav".into())
        });
        let started = std::time::Instant::now();
        assert!(join_bounded(handle, Duration::from_millis(20)).is_none());
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "join_bounded returned after {:?}; it must abandon the 3 s worker, not wait for it",
            started.elapsed()
        );
    }

    #[test]
    fn microphone_join_keeps_a_completed_result() {
        let handle = thread::spawn(|| -> Result<String> { Ok("mic.wav".into()) });
        assert_eq!(
            join_bounded(handle, Duration::from_secs(1))
                .expect("worker should join")
                .expect("worker should succeed"),
            "mic.wav"
        );
    }
}
