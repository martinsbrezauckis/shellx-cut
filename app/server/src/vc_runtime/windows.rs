//! Windows side of [`super`]: this PC's runtime state, Windows' own signature
//! check of Microsoft's installer, whether this account can approve an
//! administrator prompt, and running the installer.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cut_core::{error_codes, CutError};
use serde_json::{json, Value};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::Cryptography::{
    szOID_ORGANIZATION_NAME, CertGetNameStringW, CertVerifyCertificateChainPolicy,
    CERT_CHAIN_POLICY_MICROSOFT_ROOT, CERT_CHAIN_POLICY_PARA, CERT_CHAIN_POLICY_STATUS,
    CERT_NAME_ATTR_TYPE, MICROSOFT_ROOT_CERT_CHAIN_POLICY_CHECK_APPLICATION_ROOT_FLAG,
};
use windows_sys::Win32::Security::WinTrust::{
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust,
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_FILE_INFO, WTD_CHOICE_FILE,
    WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT, WTD_REVOKE_WHOLECHAIN, WTD_STATEACTION_CLOSE,
    WTD_STATEACTION_VERIFY, WTD_UI_NONE,
};
use windows_sys::Win32::Security::{
    AllocateAndInitializeSid, CheckTokenMembership, FreeSid, GetTokenInformation,
    TokenElevationType, TokenElevationTypeDefault, TokenElevationTypeFull,
    TokenElevationTypeLimited, PSID, SECURITY_NT_AUTHORITY, TOKEN_QUERY,
};
use windows_sys::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6432KEY,
    RRF_SUBKEY_WOW6464KEY,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::{
    classify_exit, download_error, download_url, install_error_with_log, signature_error,
    staging_dir, InstallerExit, Status, MAX_INSTALLER_BYTES, PUBLISHER, SYSTEM_FILES,
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn system_dir() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
}

/// Microsoft's registered version (`Runtimes\x64` or `\arm64`), both views.
fn registered_version() -> Option<String> {
    let key = match std::env::consts::ARCH {
        "aarch64" => r"SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\arm64",
        _ => r"SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64",
    };
    let subkey = wide(key);
    let installed = wide("Installed");
    let version = wide("Version");
    for view in [RRF_SUBKEY_WOW6464KEY, RRF_SUBKEY_WOW6432KEY] {
        let mut flag: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: valid NUL-terminated strings and an out buffer of `size` bytes.
        let read = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                subkey.as_ptr(),
                installed.as_ptr(),
                RRF_RT_REG_DWORD | view,
                std::ptr::null_mut(),
                (&mut flag as *mut u32).cast(),
                &mut size,
            )
        };
        if read != 0 || flag != 1 {
            continue;
        }
        let mut buffer = vec![0u16; 128];
        let mut bytes = (buffer.len() * 2) as u32;
        // SAFETY: as above; `bytes` is the buffer's size in bytes.
        let read = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                subkey.as_ptr(),
                version.as_ptr(),
                RRF_RT_REG_SZ | view,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if read == 0 {
            let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
            return Some(String::from_utf16_lossy(&buffer[..len]));
        }
    }
    None
}

pub(super) fn status() -> Status {
    let dir = system_dir();
    let missing: Vec<String> = SYSTEM_FILES
        .iter()
        .filter(|name| !dir.join(name).is_file())
        .map(|name| name.to_string())
        .collect();
    Status {
        applicable: true,
        installed: missing.is_empty(),
        version: registered_version(),
        missing_files: missing,
    }
}

/// Windows' own Authenticode check (WinVerifyTrust, revocation checked), a
/// chain that ends at a Microsoft root (the Microsoft-root chain policy), and a
/// signer whose organization is Microsoft Corporation. Ok carries the signer.
pub(super) fn verify_microsoft_signature(path: &Path) -> Result<String, String> {
    let path_w: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: path_w.as_ptr(),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_WHOLECHAIN,
        dwUnionChoice: WTD_CHOICE_FILE,
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT,
        ..Default::default()
    };
    data.Anonymous.pFile = &mut file;
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: `data` and `file` outlive both calls; the state is closed below.
    let trust = unsafe {
        WinVerifyTrust(
            std::ptr::null_mut(),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast(),
        )
    };
    let result = (|| {
        if trust != 0 {
            return Err(format!(
                "Windows did not accept the file's signature (WinVerifyTrust 0x{:08X})",
                trust as u32
            ));
        }
        // SAFETY: a successful verify keeps provider data in hWVTStateData.
        let provider = unsafe { WTHelperProvDataFromStateData(data.hWVTStateData) };
        if provider.is_null() {
            return Err("Windows returned no signer data".to_string());
        }
        let signer = unsafe { WTHelperGetProvSignerFromChain(provider, 0, 0, 0) };
        if signer.is_null() {
            return Err("the file has no signer".to_string());
        }
        let signer = unsafe { &*signer };
        let policy = CERT_CHAIN_POLICY_PARA {
            cbSize: std::mem::size_of::<CERT_CHAIN_POLICY_PARA>() as u32,
            dwFlags: MICROSOFT_ROOT_CERT_CHAIN_POLICY_CHECK_APPLICATION_ROOT_FLAG,
            pvExtraPolicyPara: std::ptr::null_mut(),
        };
        let mut outcome = CERT_CHAIN_POLICY_STATUS {
            cbSize: std::mem::size_of::<CERT_CHAIN_POLICY_STATUS>() as u32,
            ..Default::default()
        };
        // SAFETY: the chain context belongs to the open WinVerifyTrust state.
        let checked = unsafe {
            CertVerifyCertificateChainPolicy(
                CERT_CHAIN_POLICY_MICROSOFT_ROOT,
                signer.pChainContext,
                &policy,
                &mut outcome,
            )
        };
        if checked == 0 || outcome.dwError != 0 {
            return Err(format!(
                "the signature's chain does not end at a Microsoft root (0x{:08X})",
                outcome.dwError
            ));
        }
        if signer.csCertChain == 0 || signer.pasCertChain.is_null() {
            return Err("the signer certificate is missing".to_string());
        }
        // SAFETY: element 0 of the provider chain is the signing certificate.
        let cert = unsafe { (*signer.pasCertChain).pCert };
        let mut name = vec![0u16; 256];
        let len = unsafe {
            CertGetNameStringW(
                cert,
                CERT_NAME_ATTR_TYPE,
                0,
                szOID_ORGANIZATION_NAME.cast(),
                name.as_mut_ptr(),
                name.len() as u32,
            )
        };
        let organization = String::from_utf16_lossy(&name[..(len.saturating_sub(1)) as usize]);
        if organization != PUBLISHER {
            return Err(format!("the signer is '{organization}', not {PUBLISHER}"));
        }
        Ok(organization)
    })();
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: closes the state opened above.
    unsafe {
        WinVerifyTrust(
            std::ptr::null_mut(),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast(),
        );
    }
    result
}

/// Can this account approve Windows' administrator prompt itself? True for an
/// administrator (split UAC token, already elevated, or UAC off); a standard
/// user gets the prompt asking for an administrator's password instead.
pub(super) fn can_approve_admin_prompt() -> bool {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo-handle of this process; token closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let mut kind: i32 = 0;
    let mut size = 0u32;
    let read = unsafe {
        GetTokenInformation(
            token,
            TokenElevationType,
            (&mut kind as *mut i32).cast(),
            std::mem::size_of::<i32>() as u32,
            &mut size,
        )
    };
    unsafe { CloseHandle(token) };
    if read == 0 {
        return false;
    }
    if kind == TokenElevationTypeLimited || kind == TokenElevationTypeFull {
        return true;
    }
    if kind != TokenElevationTypeDefault {
        return false;
    }
    // No split token: an administrator only if BUILTIN\Administrators is enabled.
    let mut admins: PSID = std::ptr::null_mut();
    // 32 = SECURITY_BUILTIN_DOMAIN_RID, 544 = DOMAIN_ALIAS_RID_ADMINS.
    let made = unsafe {
        AllocateAndInitializeSid(
            &SECURITY_NT_AUTHORITY,
            2,
            32,
            544,
            0,
            0,
            0,
            0,
            0,
            0,
            &mut admins,
        )
    };
    if made == 0 {
        return false;
    }
    let mut member = 0;
    let checked = unsafe { CheckTokenMembership(std::ptr::null_mut(), admins, &mut member) };
    unsafe { FreeSid(admins) };
    checked != 0 && member != 0
}

/// Start Microsoft's installer with its documented switches and wait. It is an
/// "asInvoker" program that asks Windows to elevate itself: Windows shows its
/// administrator prompt to the person; nothing here answers it.
fn run_installer(installer: &Path, log: &Path) -> Result<u32, CutError> {
    let mut command = std::process::Command::new(installer);
    command
        .args(["/install", "/passive", "/norestart", "/log"])
        .arg(log)
        // Its own private staging folder, never the engine's working folder
        // (which may be a received project), and no current-folder search.
        .env("NoDefaultCurrentDirectoryInExePath", "1");
    if let Some(staging) = installer.parent() {
        command.current_dir(staging);
    }
    match cut_media::toolpath::child_path_value() {
        Some(path) => command.env("PATH", path),
        None => command.env_remove("PATH"),
    };
    let mut child = command.spawn().map_err(|e| {
        CutError::new(
            error_codes::JOB_FAILED,
            "Microsoft's Visual C++ Redistributable installer could not be started",
            e.to_string(),
        )
        .with_suggested_action(format!("Install it from {}", super::OFFICIAL_PAGE))
    })?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.code().unwrap_or(-1) as u32),
            Ok(None) => std::thread::sleep(Duration::from_millis(500)),
            Err(e) => {
                return Err(CutError::new(
                    error_codes::JOB_FAILED,
                    "lost track of Microsoft's installer",
                    e.to_string(),
                ))
            }
        }
    }
}

pub(super) fn install(
    progress: &dyn Fn(f32, &str),
    cancelled: &dyn Fn() -> bool,
) -> Result<Value, CutError> {
    let before = status();
    if before.installed {
        return Ok(
            json!({"installed": true, "already_installed": true, "version": before.version}),
        );
    }
    let url = download_url(std::env::consts::ARCH).ok_or_else(|| {
        CutError::new(
            error_codes::UNSUPPORTED_PLATFORM,
            "Microsoft publishes no Visual C++ Redistributable for this processor",
            std::env::consts::ARCH.to_string(),
        )
    })?;
    let staging = staging_dir()?;
    let installer = staging.path().join("vc_redist.exe");
    progress(0.02, "downloading Microsoft's installer (about 20 MB)");
    let (bytes, sha256) =
        crate::fetch::download_file(url, &installer, MAX_INSTALLER_BYTES, &|f, _| {
            progress(
                0.02 + f * 0.48,
                "downloading Microsoft's installer (about 20 MB)",
            )
        })
        .map_err(download_error)?;
    if cancelled() {
        return Err(CutError::new(
            error_codes::JOB_FAILED,
            "cancelled before Microsoft's installer started",
            "nothing was installed; the download was discarded",
        ));
    }
    progress(0.5, "checking Microsoft's signature");
    let signer =
        verify_microsoft_signature(&installer).map_err(|reason| signature_error(&reason))?;
    let can_approve = can_approve_admin_prompt();
    progress(
        0.55,
        if can_approve {
            "Microsoft's installer is running; approve the Windows administrator prompt if it appears"
        } else {
            "Microsoft's installer is running; Windows asks for an administrator's permission if needed"
        },
    );
    let log = staging.path().join("vc_redist.log");
    let code = run_installer(&installer, &log)?;
    let exit = classify_exit(code);
    let after = status();
    if !after.installed {
        let log_text = std::fs::read_to_string(&log).unwrap_or_default();
        return Err(install_error_with_log(exit, can_approve, &log_text));
    }
    let restart_needed = matches!(
        exit,
        InstallerExit::Installed {
            restart_needed: true
        }
    );
    progress(
        1.0,
        if restart_needed {
            "installed; Windows asks for a restart to finish"
        } else {
            "installed"
        },
    );
    Ok(json!({
        "installed": true,
        "version": after.version,
        "restart_needed": restart_needed,
        "installer_exit_code": code,
        "signer": signer,
        "source_url": url,
        "bytes": bytes,
        "sha256": sha256,
    }))
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
