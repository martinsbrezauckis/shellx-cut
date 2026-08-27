//! Opaque native display identities for exact-target capture admission.
//!
//! A display picker returns its current opaque id unchanged to
//! `screen_record.start{monitor_id}`. This module deliberately has no ordinal,
//! title, primary-display, or geometry input, so native capture can resolve only
//! the exact display it originally enumerated.

use sha2::{Digest, Sha256};

const PREFIX: &str = "shellx-monitor-v1";
const MAX_NATIVE_KEY_BYTES: usize = 4_096;

/// Native display namespaces intentionally stay distinct even if two platform
/// APIs happen to return the same implementation value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeMonitorPlatform {
    #[cfg(any(windows, test))]
    Windows,
    #[cfg(any(target_os = "macos", test))]
    Macos,
}

impl NativeMonitorPlatform {
    const fn label(self) -> &'static str {
        match self {
            #[cfg(any(windows, test))]
            Self::Windows => "windows",
            #[cfg(any(target_os = "macos", test))]
            Self::Macos => "macos",
        }
    }
}

/// Return a versioned opaque identity for one exact native display key.
///
/// The native value is never returned. A SHA-256 digest preserves a stable
/// comparison key for a freshly enumerated live display without exposing a
/// Windows device path or a CoreGraphics display handle to callers.
pub(crate) fn opaque_native_monitor_id(
    platform: NativeMonitorPlatform,
    native_key: &str,
) -> Option<String> {
    if native_key.trim().is_empty() || native_key.len() > MAX_NATIVE_KEY_BYTES {
        return None;
    }
    let mut digest = Sha256::new();
    digest.update(PREFIX.as_bytes());
    digest.update([0]);
    digest.update(platform.label().as_bytes());
    digest.update([0]);
    digest.update(native_key.as_bytes());
    Some(format!(
        "{PREFIX}:{}:{}",
        platform.label(),
        hex(&digest.finalize())
    ))
}

/// Re-resolve an opaque selection against a fresh native enumeration.
///
/// This is deliberately an exact equality scan with no fallback candidate:
/// reordered lists, changed display labels, a new primary display, and matching
/// dimensions all leave a stale selection unresolved.
#[allow(dead_code)] // Used by the next exact-target consumer; pure tests lock its refusal semantics.
pub(crate) fn resolve_exact<T, I, F>(
    requested_id: &str,
    platform: NativeMonitorPlatform,
    candidates: I,
    mut native_key: F,
) -> Option<T>
where
    I: IntoIterator<Item = T>,
    F: FnMut(&T) -> Option<String>,
{
    candidates.into_iter().find(|candidate| {
        native_key(candidate)
            .and_then(|key| opaque_native_monitor_id(platform, &key))
            .as_deref()
            == Some(requested_id)
    })
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Candidate {
        native_key: &'static str,
        title: &'static str,
        width: u32,
        height: u32,
        primary: bool,
    }

    fn display(native_key: &'static str, title: &'static str, primary: bool) -> Candidate {
        Candidate {
            native_key,
            title,
            width: 1_920,
            height: 1_080,
            primary,
        }
    }

    #[test]
    fn opaque_identity_has_a_versioned_shape_without_the_native_value() {
        let native = r"\\?\DISPLAY#ACM1234#5&1a2b3c4d&0&UID1234#{abc}";
        let id = opaque_native_monitor_id(NativeMonitorPlatform::Windows, native).unwrap();
        assert!(id.starts_with("shellx-monitor-v1:windows:"));
        assert_eq!(id.len(), "shellx-monitor-v1:windows:".len() + 64);
        assert!(id.rsplit(':').next().unwrap().bytes().all(|byte| {
            byte.is_ascii_digit() || (byte.is_ascii_lowercase() && byte.is_ascii_hexdigit())
        }));
        assert!(!id.contains(native));
        assert_ne!(
            id,
            opaque_native_monitor_id(NativeMonitorPlatform::Macos, native).unwrap()
        );
        assert!(opaque_native_monitor_id(NativeMonitorPlatform::Macos, "  ").is_none());
    }

    #[test]
    fn exact_resolution_survives_reordering_without_using_presentation_fields() {
        let requested = opaque_native_monitor_id(NativeMonitorPlatform::Macos, "cg:42").unwrap();
        let selected = resolve_exact(
            &requested,
            NativeMonitorPlatform::Macos,
            [
                display("cg:99", "Studio", true),
                display("cg:42", "Renamed display", false),
            ],
            |candidate| Some(candidate.native_key.to_owned()),
        )
        .unwrap();
        assert_eq!(selected.native_key, "cg:42");
        assert_eq!(selected.title, "Renamed display");
        assert!(!selected.primary);
    }

    #[test]
    fn stale_or_replaced_identity_never_falls_back_to_matching_geometry_or_primary() {
        let requested =
            opaque_native_monitor_id(NativeMonitorPlatform::Windows, "target:retired").unwrap();
        let candidates = [
            display("target:replacement", "Same display title", true),
            display("target:other", "Same display title", false),
        ];
        assert_eq!(
            resolve_exact(
                &requested,
                NativeMonitorPlatform::Windows,
                candidates,
                |candidate| Some(candidate.native_key.to_owned()),
            ),
            None
        );
        assert_eq!(
            resolve_exact(
                "shellx-monitor-v1:windows:not-a-native-id",
                NativeMonitorPlatform::Windows,
                [display("target:replacement", "Same display title", true)],
                |candidate| Some(candidate.native_key.to_owned()),
            ),
            None
        );
    }
}
