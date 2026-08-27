//! Exact Windows display identity resolution for future native region picking.
//!
//! `\\.\DISPLAYn` is used only to join one current `HMONITOR` to its active
//! DisplayConfig path. The public identity is derived solely from the opaque
//! physical target path, and re-resolution never falls back to display order,
//! name, primary status, or dimensions.

use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows_capture::monitor::Monitor as WcMonitor;

use crate::monitor_identity::{opaque_native_monitor_id, resolve_exact, NativeMonitorPlatform};

pub(crate) fn monitor_id(monitor: &WcMonitor) -> Option<String> {
    native_target_path(monitor)
        .and_then(|key| opaque_native_monitor_id(NativeMonitorPlatform::Windows, &key))
}

/// Resolve only a current native display that hashes to `id`. The recording
/// backend uses this for `CaptureConfig.monitor_id`; no ordinal fallback exists.
pub(crate) fn resolve_monitor(id: &str) -> Result<WcMonitor, &'static str> {
    let monitors = WcMonitor::enumerate()
        .map_err(|_| "the selected display is no longer available; reopen the source picker")?;
    resolve_exact(
        id,
        NativeMonitorPlatform::Windows,
        monitors,
        native_target_path,
    )
    .ok_or("the selected display is no longer available; reopen the source picker")
}

fn native_target_path(monitor: &WcMonitor) -> Option<String> {
    let gdi_name = monitor.device_name().ok()?;
    active_display_paths()?.into_iter().find_map(|path| {
        (source_gdi_name(&path).as_deref() == Some(gdi_name.as_str()))
            .then(|| target_device_path(&path))
            .flatten()
    })
}

fn active_display_paths() -> Option<Vec<DISPLAYCONFIG_PATH_INFO>> {
    let mut path_count = 0;
    let mut mode_count = 0;
    if unsafe {
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
    }
    .is_err()
    {
        return None;
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); usize::try_from(path_count).ok()?];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); usize::try_from(mode_count).ok()?];
    if unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
    }
    .is_err()
    {
        return None;
    }
    paths.truncate(usize::try_from(path_count).ok()?);
    Some(paths)
}

fn source_gdi_name(path: &DISPLAYCONFIG_PATH_INFO) -> Option<String> {
    let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
            size: u32::try_from(std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>()).ok()?,
            adapterId: path.sourceInfo.adapterId,
            id: path.sourceInfo.id,
        },
        ..Default::default()
    };
    (unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } == 0)
        .then(|| utf16_value(&source.viewGdiDeviceName))
        .flatten()
}

fn target_device_path(path: &DISPLAYCONFIG_PATH_INFO) -> Option<String> {
    let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            size: u32::try_from(std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>()).ok()?,
            adapterId: path.targetInfo.adapterId,
            id: path.targetInfo.id,
        },
        ..Default::default()
    };
    (unsafe { DisplayConfigGetDeviceInfo(&mut target.header) } == 0)
        .then(|| utf16_value(&target.monitorDevicePath))
        .flatten()
}

fn utf16_value(value: &[u16]) -> Option<String> {
    let end = value.iter().position(|unit| *unit == 0)?;
    let decoded = String::from_utf16(&value[..end]).ok()?;
    (!decoded.trim().is_empty()).then_some(decoded)
}
