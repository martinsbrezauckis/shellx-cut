//! Reads the Windows capability consent store for the microphone. The read
//! opens nothing visible and never changes a value.

use super::privacy::{ConsentScope, ConsentSource};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
};

const USER_KEY: PCWSTR = w!(
    r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone"
);
const DESKTOP_APPS_KEY: PCWSTR = w!(
    r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone\NonPackaged"
);
const MACHINE_KEY: PCWSTR = w!(
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone"
);

/// The real consent store.
pub(super) struct Registry;

impl ConsentSource for Registry {
    fn value(&self, scope: ConsentScope) -> Option<String> {
        match scope {
            ConsentScope::Device => read(HKEY_LOCAL_MACHINE, MACHINE_KEY),
            ConsentScope::Apps => read(HKEY_CURRENT_USER, USER_KEY),
            ConsentScope::DesktopApps => read(HKEY_CURRENT_USER, DESKTOP_APPS_KEY),
        }
    }
}

fn read(root: HKEY, key: PCWSTR) -> Option<String> {
    let mut buffer = [0_u16; 32];
    let mut bytes = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: `buffer` and `bytes` outlive the call and `bytes` is its byte size.
    let status = unsafe {
        RegGetValueW(
            root,
            key,
            w!("Value"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let units = (bytes as usize / 2).min(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..units]);
    Some(text.trim_end_matches('\0').to_string())
}
