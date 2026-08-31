mod device;
mod device_run;
#[cfg(any(windows, test))]
mod loopback_pcm;
mod stream;
mod stream_callback;
mod wav;
mod wav_layout;
#[cfg(windows)]
mod windows_loopback;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use device::WarmResult;
pub(crate) use device::{
    join_bounded, reserve_microphone_capture, spawn_device_mic, spawn_device_mic_reserved,
    spawn_device_mic_reserved_unpadded, warm_device, CapturedMicrophone, MicRecordingGate,
    MicrophoneCaptureReservation,
};
#[cfg(windows)]
pub use windows_loopback::capture_system_loopback;
