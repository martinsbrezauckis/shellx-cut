//! Private foreground-owned Windows Region-picker seam.
//!
//! Windows Graphics Capture exposes a system picker for whole displays or
//! windows, not an arbitrary desktop rectangle. The owned Win32 overlay behind
//! this module owns a private one-display crop plus the exact physical target
//! path and active DisplayConfig topology digest from one native snapshot. The
//! authenticated bridge revalidates that immutable snapshot and replaces its
//! private target path with the existing opaque identity before issuing a
//! one-use server ticket.
//! No public verb, capability, browser API, or capture-start caller exposes
//! this module.

use tauri::Manager;

const GDI_DEVICE_UNITS: usize = 32;
const TARGET_PATH_UNITS: usize = 128;
const TOPOLOGY_DIGEST_BYTES: usize = 32;
const PICKED: i32 = 0;
const CANCELLED: i32 = 1;
const REFUSED: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct NativePickerSelection {
    topology: NativeTopologySnapshot,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    parent_width: u32,
    parent_height: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeTopologySnapshot {
    source_gdi: [u16; GDI_DEVICE_UNITS],
    target_path: [u16; TARGET_PATH_UNITS],
    digest: [u8; TOPOLOGY_DIGEST_BYTES],
}

extern "C" {
    fn sxc_windows_region_picker_present(
        main_hwnd: *mut core::ffi::c_void,
        selection: *mut NativePickerSelection,
    ) -> i32;
    fn sxc_windows_region_picker_selection_is_current(
        selection: *const NativePickerSelection,
    ) -> i32;
}

/// One validated native result. Its fields stay private so a future bridge can
/// perform the mandatory immediate identity conversion without publishing raw
/// desktop geometry to a caller, receipt, response, or UI state.
pub(crate) struct WindowsRegionSelection {
    topology: NativeTopologySnapshot,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    parent_width: u32,
    parent_height: u32,
}

impl WindowsRegionSelection {
    /// Private physical target key. The consuming foreground bridge must turn
    /// it into record-capture's opaque DisplayConfig identity immediately.
    pub(crate) fn target_device_path(&self) -> String {
        utf16_key(&self.topology.target_path, "physical display key")
            .expect("validated Windows Region selection retains target path")
    }

    /// Private exact DisplayConfig fields. They cross only the authenticated
    /// shell-to-child loopback lane, then the child replaces `target_path`
    /// with the opaque WGC identity before it stores its one-use ticket.
    pub(crate) fn topology_parts(
        &self,
    ) -> Result<(String, String, [u8; TOPOLOGY_DIGEST_BYTES]), String> {
        Ok((
            utf16_key(&self.topology.source_gdi, "source display key")?,
            self.target_device_path(),
            self.topology.digest,
        ))
    }

    pub(crate) fn native_parts(&self) -> (u32, u32, u32, u32, u32, u32) {
        (
            self.left,
            self.top,
            self.width,
            self.height,
            self.parent_width,
            self.parent_height,
        )
    }

    /// Recheck the original source-to-physical-target association, complete
    /// active DisplayConfig digest, and selected native parent dimensions.
    /// A bridge must consume only `require_current` immediately before it mints
    /// the server-private ticket; equal-sized replacement displays therefore
    /// cannot inherit the original crop.
    pub(crate) fn require_current(self) -> Result<Self, String> {
        let native = NativePickerSelection {
            topology: self.topology,
            left: self.left,
            top: self.top,
            width: self.width,
            height: self.height,
            parent_width: self.parent_width,
            parent_height: self.parent_height,
        };
        // SAFETY: `native` is a repr(C) copy of this already validated,
        // immutable selection. The native side only re-enumerates Windows
        // topology and does not retain the pointer.
        (unsafe { sxc_windows_region_picker_selection_is_current(&native) } != 0)
            .then_some(self)
            .ok_or_else(|| {
                "the selected display topology changed; choose the Region again".to_string()
            })
    }
}

/// Present the Win32 overlay only from the shell's foreground UI thread.
///
/// This must run only inside Tauri's `run_on_main_thread` callback. The native
/// ABI receives the exact `main` HWND and rejects any other process window,
/// main-window thread, or foreground source before it creates the overlay.
pub(crate) fn choose_region_on_foreground_main_thread(
    app: &tauri::AppHandle,
) -> Result<Option<WindowsRegionSelection>, String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "the foreground ShellX Cut window is unavailable".to_string())?;
    if !window.is_focused().unwrap_or(false) {
        return Err(
            "Region selection is allowed only from the foreground ShellX Cut window".to_string(),
        );
    }
    let main_hwnd = window
        .hwnd()
        .map_err(|_| "the foreground ShellX Cut window has no native HWND".to_string())?;
    if main_hwnd.0.is_null() {
        return Err("the foreground ShellX Cut window has an invalid native HWND".to_string());
    }
    let mut selection = NativePickerSelection {
        topology: NativeTopologySnapshot {
            source_gdi: [0; GDI_DEVICE_UNITS],
            target_path: [0; TARGET_PATH_UNITS],
            digest: [0; TOPOLOGY_DIGEST_BYTES],
        },
        left: 0,
        top: 0,
        width: 0,
        height: 0,
        parent_width: 0,
        parent_height: 0,
    };
    // SAFETY: this function is invoked only by Tauri's main-thread scheduler;
    // the native bridge verifies that this exact HWND is foreground-owned by
    // this process and that the caller is the HWND's owning UI thread. It
    // writes only the repr(C) storage above for a confirmed selection.
    match unsafe { sxc_windows_region_picker_present(main_hwnd.0, &mut selection) } {
        PICKED => valid_native_selection(selection)
            .and_then(WindowsRegionSelection::require_current)
            .map(Some),
        CANCELLED => Ok(None),
        REFUSED => Err("the Region selection lost foreground focus and was refused".to_string()),
        _ => Err("the native Region picker did not finish successfully".to_string()),
    }
}

fn valid_native_selection(
    selection: NativePickerSelection,
) -> Result<WindowsRegionSelection, String> {
    let source_gdi = utf16_key(&selection.topology.source_gdi, "source display key")?;
    let target_path = utf16_key(&selection.topology.target_path, "physical display key")?;
    let right = selection.left.checked_add(selection.width);
    let bottom = selection.top.checked_add(selection.height);
    let valid = source_gdi.starts_with(r"\\.\DISPLAY")
        && target_path.starts_with(r"\\?\")
        && selection.topology.digest.iter().any(|byte| *byte != 0)
        && selection.parent_width > 0
        && selection.parent_height > 0
        && selection.left.is_multiple_of(2)
        && selection.top.is_multiple_of(2)
        && selection.width >= 2
        && selection.height >= 2
        && selection.width.is_multiple_of(2)
        && selection.height.is_multiple_of(2)
        && right.is_some_and(|right| right <= selection.parent_width)
        && bottom.is_some_and(|bottom| bottom <= selection.parent_height);
    valid
        .then_some(WindowsRegionSelection {
            topology: selection.topology,
            left: selection.left,
            top: selection.top,
            width: selection.width,
            height: selection.height,
            parent_width: selection.parent_width,
            parent_height: selection.parent_height,
        })
        .ok_or_else(|| "the native Region picker returned an invalid physical crop".to_string())
}

fn utf16_key(value: &[u16], name: &str) -> Result<String, String> {
    let end = value
        .iter()
        .position(|unit| *unit == 0)
        .ok_or_else(|| format!("the native Region picker returned an unterminated {name}"))?;
    let key = String::from_utf16(&value[..end])
        .map_err(|_| format!("the native Region picker returned an invalid {name}"))?;
    (!key.trim().is_empty())
        .then_some(key)
        .ok_or_else(|| format!("the native Region picker returned an empty {name}"))
}

#[cfg(test)]
mod tests {
    use super::{
        valid_native_selection, NativePickerSelection, NativeTopologySnapshot, GDI_DEVICE_UNITS,
        TARGET_PATH_UNITS, TOPOLOGY_DIGEST_BYTES,
    };

    fn selected() -> NativePickerSelection {
        let mut source_gdi = [0_u16; GDI_DEVICE_UNITS];
        for (slot, unit) in r"\\.\DISPLAY7".encode_utf16().enumerate() {
            source_gdi[slot] = unit;
        }
        let mut target_path = [0_u16; TARGET_PATH_UNITS];
        for (slot, unit) in r"\\?\DISPLAY#example".encode_utf16().enumerate() {
            target_path[slot] = unit;
        }
        NativePickerSelection {
            topology: NativeTopologySnapshot {
                source_gdi,
                target_path,
                digest: [9; TOPOLOGY_DIGEST_BYTES],
            },
            left: 2,
            top: 4,
            width: 800,
            height: 600,
            parent_width: 1920,
            parent_height: 1080,
        }
    }

    #[test]
    fn native_selection_requires_an_even_contained_crop_and_bound_topology_snapshot() {
        assert!(valid_native_selection(selected()).is_ok());
        let mut invalid = selected();
        invalid.width = 801;
        assert!(valid_native_selection(invalid).is_err());
        let mut invalid = selected();
        invalid.topology.digest = [0; TOPOLOGY_DIGEST_BYTES];
        assert!(valid_native_selection(invalid).is_err());
    }
}
