mod audio_level;
mod device;
mod device_run;
mod device_warm;
#[cfg(any(windows, test))]
mod loopback_pcm;
mod stream;
mod stream_callback;
mod stream_callbacks;
mod wav;
mod wav_layout;
#[cfg(windows)]
mod windows_loopback;

#[cfg(test)]
mod audio_level_tests;
#[cfg(test)]
mod tests;

pub use audio_level::{AudioLevelLifecycle, AudioLevelSnapshot, RollingAudioLevel};
pub(crate) use device::{
    join_bounded, reserve_microphone_capture, spawn_device_mic, spawn_device_mic_reserved,
    spawn_device_mic_reserved_unpadded, spawn_device_mic_with_level, CapturedMicrophone,
    MicRecordingGate, MicrophoneCaptureReservation,
};
#[allow(unused_imports)]
pub(crate) use device_warm::{warm_device, WarmResult};
#[cfg(windows)]
pub use windows_loopback::{capture_system_loopback, capture_system_loopback_with_level};
