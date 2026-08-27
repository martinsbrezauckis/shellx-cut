#[cfg(windows)]
mod activation;
#[cfg(windows)]
mod capture;

#[cfg(windows)]
pub use capture::capture_system_loopback;
