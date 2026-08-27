//! Windows MMDevice input enumeration. Endpoint ids remain private references.

use super::{MicrophoneEndpoint, MicrophoneEndpointRef};
use windows::core::PWSTR;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Media::Audio::{
    eCapture, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};

struct ComApartment(bool);

impl ComApartment {
    unsafe fn enter() -> windows::core::Result<Self> {
        let result = CoInitializeEx(None, COINIT_MULTITHREADED);
        if result.is_ok() {
            Ok(Self(true))
        } else if result == RPC_E_CHANGED_MODE {
            Ok(Self(false))
        } else {
            Err(result.into())
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// Enumerate active capture endpoints through MMDevice. The implementation is
/// isolated here because the returned endpoint id is sensitive device identity;
/// callers receive it only inside the private endpoint struct.
pub(super) fn list() -> Vec<MicrophoneEndpoint> {
    enumerate_mmdevice_capture_endpoints().unwrap_or_default()
}

fn enumerate_mmdevice_capture_endpoints() -> windows::core::Result<Vec<MicrophoneEndpoint>> {
    unsafe {
        let _apartment = ComApartment::enter()?;
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let devices = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
        let count = devices.GetCount()?;
        let mut endpoints = Vec::with_capacity(count as usize);
        for index in 0..count {
            let device = devices.Item(index)?;
            let id = take_pwstr(device.GetId()?)?;
            let store = device.OpenPropertyStore(STGM_READ)?;
            let mut value = store.GetValue(&PKEY_Device_FriendlyName)?;
            let label = match PropVariantToStringAlloc(&value) {
                Ok(value) => take_pwstr(value).unwrap_or_default(),
                Err(_) => String::new(),
            };
            let _ = PropVariantClear(&mut value);
            endpoints.push(MicrophoneEndpoint {
                reference: MicrophoneEndpointRef::Windows { endpoint_id: id },
                label,
            });
        }
        Ok(endpoints)
    }
}

unsafe fn take_pwstr(value: PWSTR) -> windows::core::Result<String> {
    let text = value.to_string();
    CoTaskMemFree(Some(value.0.cast()));
    Ok(text?)
}
