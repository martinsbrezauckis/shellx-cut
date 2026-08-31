//! Cutd-side revalidation for a private Windows Region selection.
//!
//! The foreground Tauri process owns the overlay. Its child must not trust an
//! earlier screen enumeration, a same-sized replacement display, or a GDI
//! source-name lookup: this module recreates the exact DisplayConfig snapshot
//! ABI and checks it immediately while burning the child-local ticket.

use crate::{region_geometry::NativePixelCrop, windows_monitor_target, CaptureRegion};

const GDI_DEVICE_UNITS: usize = 32;
const TARGET_PATH_UNITS: usize = 128;
const TOPOLOGY_DIGEST_BYTES: usize = 32;

#[repr(C)]
struct NativeTopologySnapshot {
    source_gdi: [u16; GDI_DEVICE_UNITS],
    target_path: [u16; TARGET_PATH_UNITS],
    digest: [u8; TOPOLOGY_DIGEST_BYTES],
}

extern "C" {
    fn sxc_windows_topology_snapshot_is_current_ffi(
        snapshot: *const NativeTopologySnapshot,
        parent_width: u32,
        parent_height: u32,
    ) -> i32;
}

/// Revalidate the opaque display identity and the original full DisplayConfig
/// snapshot before a private ticket may reach any capture reservation. The
/// ordinary Windows capture owner then binds the exact GPU crop to this same
/// parent frame and maps input against its matching subsurface.
pub(crate) fn selection_is_current(
    source_gdi: &str,
    target_path: &str,
    topology_digest: [u8; TOPOLOGY_DIGEST_BYTES],
    monitor_id: &str,
    region: CaptureRegion,
) -> bool {
    let Some(crop) = NativePixelCrop::from_capture_region(region) else {
        return false;
    };
    let Some(expected_identity) = windows_monitor_target::monitor_id_from_target_path(target_path)
    else {
        return false;
    };
    if expected_identity != monitor_id
        || windows_monitor_target::resolve_monitor(monitor_id).is_err()
    {
        return false;
    }
    let Some(source_gdi) = utf16_snapshot_array::<GDI_DEVICE_UNITS>(source_gdi) else {
        return false;
    };
    let Some(target_path) = utf16_snapshot_array::<TARGET_PATH_UNITS>(target_path) else {
        return false;
    };
    let snapshot = NativeTopologySnapshot {
        source_gdi,
        target_path,
        digest: topology_digest,
    };
    let (parent_width, parent_height) = crop.parent_size();
    // SAFETY: `snapshot` exactly matches the shared repr(C) topology ABI and
    // remains borrowed only for this synchronous DisplayConfig re-enumeration.
    unsafe {
        sxc_windows_topology_snapshot_is_current_ffi(&snapshot, parent_width, parent_height) != 0
    }
}

fn utf16_snapshot_array<const N: usize>(value: &str) -> Option<[u16; N]> {
    let units = value.encode_utf16().collect::<Vec<_>>();
    (!value.is_empty()
        && !value.contains('\0')
        && units.len() < N
        && units.iter().all(|unit| *unit != 0))
    .then(|| {
        let mut output = [0_u16; N];
        output[..units.len()].copy_from_slice(&units);
        output
    })
}
