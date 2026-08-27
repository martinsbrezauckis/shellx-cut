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
