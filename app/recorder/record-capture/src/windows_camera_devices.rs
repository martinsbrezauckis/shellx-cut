//! Permission-neutral Windows camera enumeration and runtime leases.

use std::sync::OnceLock;

use record_core::Result;
use sha2::{Digest, Sha256};
use windows::core::PWSTR;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, MFCreateAttributes, MFEnumDeviceSources, MFShutdown, MFStartup, MFSTARTUP_FULL,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_VERSION,
};
use windows::Win32::System::Com::{
    CoIncrementMTAUsage, CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED,
};

use super::{camera_error, windows_error};
use crate::CameraDevice;

/// Private selected-device activation. The raw symbolic link never crosses this
/// module boundary; only the Capture Engine activation and an opaque hash do.
pub(super) struct DeviceSource {
    pub(super) device: CameraDevice,
    pub(super) activation: IMFActivate,
}

/// `MFEnumDeviceSources` is enumeration only; it does not hand an explicit-use
/// intent to Capture Engine and must not cause the desktop privacy prompt.
pub(super) fn enumerate_devices() -> Result<Vec<DeviceSource>> {
    let _com = CameraComApartment::initialize()?;
    let _mf = MediaFoundationLease::start()?;
    // SAFETY: Media Foundation is started on this thread; each activation is
    // moved out of the CoTaskMem array before that array is released.
    unsafe {
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 1)
            .map_err(|error| windows_error("create camera enumeration attributes", error))?;
        let attributes = attributes.ok_or_else(|| {
            camera_error(
                "create camera enumeration attributes",
                "Media Foundation returned no camera enumeration attributes",
            )
        })?;
        attributes
            .SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )
            .map_err(|error| windows_error("select video capture devices", error))?;
        let mut raw = std::ptr::null_mut::<Option<IMFActivate>>();
        let mut count = 0_u32;
        MFEnumDeviceSources(&attributes, &mut raw, &mut count)
            .map_err(|error| windows_error("enumerate Windows camera devices", error))?;
        if raw.is_null() || count == 0 {
            return Ok(Vec::new());
        }
        let entries = std::slice::from_raw_parts_mut(raw, count as usize);
        let mut devices = Vec::with_capacity(entries.len());
        let mut failure = None;
        for entry in entries {
            let Some(activation) = entry.take() else {
                continue;
            };
            match source_from_activation(activation) {
                Ok(source) => devices.push(source),
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        CoTaskMemFree(Some(raw.cast()));
        failure.map_or(Ok(devices), Err)
    }
}

pub(super) fn find_device(device_id: &str) -> Result<Option<DeviceSource>> {
    Ok(enumerate_devices()?
        .into_iter()
        .find(|source| source.device.id == device_id))
}

fn source_from_activation(activation: IMFActivate) -> Result<DeviceSource> {
    let symbolic_link = allocated_string(
        &activation,
        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
    )?;
    Ok(DeviceSource {
        device: CameraDevice {
            id: opaque_device_id(&symbolic_link),
            // A model/friendly name is also host-specific device identity.
            // This private enumeration keeps only an opaque selection token.
            label: "Windows camera".into(),
        },
        activation,
    })
}

fn opaque_device_id(symbolic_link: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"shellx-cut/windows-camera-device/v1\0");
    hasher.update(symbolic_link.as_bytes());
    format!("win_camera_{:x}", hasher.finalize())
}

fn allocated_string(activation: &IMFActivate, key: &windows::core::GUID) -> Result<String> {
    // SAFETY: Capture Engine allocates the returned UTF-16 buffer with the COM
    // task allocator; we copy its declared length, then release that buffer.
    unsafe {
        let mut value = PWSTR::null();
        let mut length = 0_u32;
        activation
            .GetAllocatedString(key, &mut value, &mut length)
            .map_err(|error| windows_error("read Windows camera device attribute", error))?;
        if value.is_null() {
            return Err(camera_error(
                "read Windows camera device attribute",
                "Media Foundation returned an empty attribute buffer",
            ));
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(value.0, length as usize));
        CoTaskMemFree(Some(value.0.cast()));
        Ok(text)
    }
}

/// Keep the MTA alive process-wide while Capture Engine and generated WinRT
/// factories may coexist. Per-run `CoInitializeEx` still binds the calling
/// thread to COM and is paired below.
static PROCESS_MTA_PIN: OnceLock<std::result::Result<usize, String>> = OnceLock::new();

pub(super) struct CameraComApartment(bool);

impl CameraComApartment {
    pub(super) fn initialize() -> Result<Self> {
        match PROCESS_MTA_PIN.get_or_init(|| {
            unsafe { CoIncrementMTAUsage() }
                .map(|cookie| cookie.0 as usize)
                .map_err(|error| format!("CoIncrementMTAUsage: {error}"))
        }) {
            Ok(_) => {}
            Err(error) => return Err(camera_error("pin Windows camera MTA", error.clone())),
        }
        // SAFETY: this private adapter owns the thread's successful COM init
        // for its scope. A changed-mode result means the caller owns pairing.
        let status = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if status.is_ok() {
            Ok(Self(true))
        } else if status == RPC_E_CHANGED_MODE {
            Ok(Self(false))
        } else {
            Err(camera_error(
                "initialize Windows camera COM apartment",
                format!("CoInitializeEx failed: {status:?}"),
            ))
        }
    }
}

impl Drop for CameraComApartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: paired with this thread's successful CoInitializeEx.
            unsafe { CoUninitialize() };
        }
    }
}

pub(super) struct MediaFoundationLease;

impl MediaFoundationLease {
    pub(super) fn start() -> Result<Self> {
        // SAFETY: the matching lease outlives every Capture Engine/source-reader
        // use on this adapter thread.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
            .map_err(|error| windows_error("start Media Foundation for camera capture", error))?;
        Ok(Self)
    }
}

impl Drop for MediaFoundationLease {
    fn drop(&mut self) {
        // SAFETY: one matching best-effort shutdown for this successful lease.
        let _ = unsafe { MFShutdown() };
    }
}
