//! Cross-platform passive global listening for keyboard and mouse input.
//!
//! This maintained in-tree fork retains event/key conversion and a passive
//! listener surface for Cut's recorder. Its owned listener API has explicit
//! stop and join semantics; it does not expose interception or simulation.

mod rdevin;
pub use crate::rdevin::{
    Button, Event, EventType, Key, KeyCode, KeyboardState, RawKey, UnicodeInfo,
};

/// Different OSes use different numerical representations for keys. Cut needs
/// those conversions internally to decode passive events; they are not a
/// public input-control surface.
#[allow(dead_code)]
mod keycodes;

#[cfg(target_os = "linux")]
#[allow(dead_code)]
mod linux;
#[cfg(target_os = "macos")]
#[allow(dead_code)]
mod macos;
#[cfg(target_os = "windows")]
#[allow(dead_code)]
mod windows;

// Cut intentionally exposes only its owned passive route. The upstream
// fire-and-forget `listen` API is kept file-private during the approved cleanup
// so it cannot install a competing callback or hook.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::sync::{Arc, Mutex};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::time::Duration;

#[cfg(target_os = "linux")]
use crate::linux::OwnedListener as PlatformOwnedListener;
#[cfg(target_os = "macos")]
use crate::macos::OwnedListener as PlatformOwnedListener;
#[cfg(target_os = "windows")]
use crate::windows::OwnedListener as PlatformOwnedListener;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static OWNED_LISTENER_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Starting a second owned listener would replace a platform callback global,
/// so it is explicitly refused instead.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Debug, thiserror::Error)]
pub enum OwnedListenerStartError {
    #[error("a process-global rdevin listener is already active")]
    AlreadyActive,
    #[error("failed to start the owned native listener: {0}")]
    Listen(#[source] ListenError),
}

/// A native listener either exits with its platform listener error or its
/// dedicated thread panics. Both outcomes are observable by the recorder.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Debug, thiserror::Error)]
pub enum OwnedListenerJoinError {
    #[error("owned native listener failed: {0}")]
    Listen(#[from] ListenError),
    #[error("owned native listener thread panicked")]
    ThreadPanicked,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
struct OwnedListenerActive;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl OwnedListenerActive {
    fn acquire() -> Result<Arc<Self>, OwnedListenerStartError> {
        OWNED_LISTENER_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Arc::new(Self))
            .map_err(|_| OwnedListenerStartError::AlreadyActive)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl Drop for OwnedListenerActive {
    fn drop(&mut self) {
        OWNED_LISTENER_ACTIVE.store(false, Ordering::Release);
    }
}

/// One process-global, passive native listener whose owner can request stop and
/// wait for native teardown. It deliberately exposes no interception or input
/// simulation control.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub struct OwnedListener {
    inner: Option<PlatformOwnedListener>,
    // A retained reaper keeps this guard alive until the native callback has
    // actually exited. Dropping the public handle therefore cannot accidentally
    // admit a second listener while a hook/tap/RECORD context survives.
    active: Arc<OwnedListenerActive>,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl OwnedListener {
    /// Ask the native listener to stop. Then use [`Self::wait_for_exit`].
    pub fn request_stop(&self) -> Result<(), ListenError> {
        self.inner
            .as_ref()
            .map_or(Ok(()), PlatformOwnedListener::request_stop)
    }

    /// Observe native teardown without allowing the caller to block forever.
    /// `Ok(false)` retains the listener's singleton admission until a later
    /// observation or the non-blocking destructor reaper sees its exit.
    pub fn wait_for_exit(&mut self, timeout: Duration) -> Result<bool, OwnedListenerJoinError> {
        let outcome = self
            .inner
            .as_mut()
            .map_or(Ok(true), |inner| inner.wait_for_exit(timeout));
        if !matches!(&outcome, Ok(false)) {
            let _ = self.inner.take();
        }
        outcome
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl Drop for OwnedListener {
    fn drop(&mut self) {
        let Some(inner) = self.inner.take() else {
            return;
        };
        retain_platform_listener(inner, self.active.clone());
    }
}

/// Start a passive owned listener. Only one such listener may be live in the
/// process; a second start fails rather than replacing a callback global.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub fn listen_owned<T>(callback: T) -> Result<OwnedListener, OwnedListenerStartError>
where
    T: FnMut(Event) + Send + 'static,
{
    let active = OwnedListenerActive::acquire()?;
    let inner = PlatformOwnedListener::start(callback).map_err(OwnedListenerStartError::Listen)?;
    Ok(OwnedListener {
        inner: Some(inner),
        active,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn retain_platform_listener(inner: PlatformOwnedListener, active: Arc<OwnedListenerActive>) {
    // Never block Drop on a native listener. If it cannot be stopped promptly,
    // this reaper retains the admission guard until the platform reports exit.
    let retained = Arc::new(Mutex::new(Some((inner, active))));
    let worker = retained.clone();
    let spawn = std::thread::Builder::new()
        .name("cut-rdevin-reaper".into())
        .spawn(move || {
            let (mut inner, active) = worker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .expect("rdevin reaper owns one listener");
            if let Err(error) = inner.request_stop() {
                eprintln!("rdevin retained listener stop failed: {error}");
            }
            loop {
                match inner.wait_for_exit(Duration::from_millis(100)) {
                    Ok(true) => return drop(active),
                    Ok(false) => continue,
                    Err(error) => {
                        eprintln!("rdevin retained listener ended: {error}");
                        return drop(active);
                    }
                }
            }
        });
    if let Err(error) = spawn {
        // Preserve the guard if the recovery worker cannot start. This is an
        // intentional fail-closed leak rather than permission to register a
        // replacement callback over an unknown live native listener.
        let retained = retained
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .expect("failed rdevin reaper keeps its listener");
        Box::leak(Box::new(retained));
        eprintln!("rdevin reaper could not start; ownership retained: {error}");
    }
}

#[cfg(all(
    test,
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
mod owned_listener_tests {
    use super::*;

    #[test]
    fn process_global_guard_refuses_a_second_owned_listener() {
        let first = OwnedListenerActive::acquire().unwrap();
        assert!(matches!(
            OwnedListenerActive::acquire(),
            Err(OwnedListenerStartError::AlreadyActive)
        ));
        drop(first);
        assert!(OwnedListenerActive::acquire().is_ok());
    }
}

#[cfg(target_os = "macos")]
pub use crate::macos::ListenError;

#[cfg(target_os = "linux")]
pub use crate::linux::ListenError;

#[cfg(target_os = "windows")]
pub use crate::windows::ListenError;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn keyboard_only() -> bool {
    !std::env::var("KEYBOARD_ONLY")
        .unwrap_or_default()
        .is_empty()
}
