mod device;
#[cfg(any(windows, test))]
mod loopback_pcm;
mod stream;
mod stream_callback;
mod wav;
#[cfg(windows)]
mod windows_loopback;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use device::WarmResult;
pub(crate) use device::{join_bounded, spawn_device_mic, warm_device, CapturedMicrophone};
#[cfg(windows)]
pub use windows_loopback::capture_system_loopback;
