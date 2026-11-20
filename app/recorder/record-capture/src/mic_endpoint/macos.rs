//! macOS Core Audio input enumeration. Device UIDs remain private references.

use super::{MicrophoneEndpoint, MicrophoneEndpointRef};
use serde::Deserialize;
use std::ffi::{c_char, CStr};

#[derive(Deserialize)]
struct NativeEndpoint {
    uid: String,
    name: String,
}

unsafe extern "C" {
    fn shellx_cut_mic_endpoints_json() -> *mut c_char;
    fn shellx_cut_free_mic_endpoints_json(value: *mut c_char);
    fn shellx_cut_microphone_authorization() -> i32;
    fn shellx_cut_request_microphone_access();
}

/// Microphone authorization read without showing a prompt.
#[cfg(feature = "capture-macos")]
pub(super) fn access() -> super::MicrophoneAccess {
    // SAFETY: the shim only reads AVFoundation's authorization state.
    match unsafe { shellx_cut_microphone_authorization() } {
        2 => super::MicrophoneAccess::Granted,
        0 => super::MicrophoneAccess::NotDetermined,
        1 => super::MicrophoneAccess::Denied,
        _ => super::MicrophoneAccess::Restricted,
    }
}

/// Show the macOS microphone prompt if the person has not answered yet.
#[cfg(feature = "capture-macos")]
pub(super) fn request_access() {
    // SAFETY: the shim starts an asynchronous AVFoundation request and returns.
    unsafe { shellx_cut_request_microphone_access() }
}

/// Enumerate Core Audio devices that publish at least one input stream. The UID
/// remains in private state; the server turns the `name` into a sanitized public
/// label and issues the UI's short-lived capability token.
pub(super) fn list() -> Vec<MicrophoneEndpoint> {
    unsafe {
        let raw = shellx_cut_mic_endpoints_json();
        if raw.is_null() {
            return Vec::new();
        }
        let parsed = serde_json::from_slice::<Vec<NativeEndpoint>>(CStr::from_ptr(raw).to_bytes())
            .unwrap_or_default();
        shellx_cut_free_mic_endpoints_json(raw);
        parsed
            .into_iter()
            .filter(|endpoint| !endpoint.uid.is_empty())
            .map(|endpoint| MicrophoneEndpoint {
                reference: MicrophoneEndpointRef::Macos {
                    device_uid: endpoint.uid,
                },
                label: endpoint.name,
            })
            .collect()
    }
}
