//! Windows DACL hardening and verification for Cut's private export staging
//! directories (`%LOCALAPPDATA%/ShellX Cut/export-staging` and children).
//!
//! Uses handle-based `SetKernelObjectSecurity` / `GetKernelObjectSecurity` with a
//! protected DACL granting `FILE_ALL_ACCESS` only to the current process token's
//! user SID, eliminating parent inheritance and avoiding path-based ACL races.

use cut_core::{error_codes, CutError};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_INSUFFICIENT_BUFFER, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::{
    AclSizeInformation, AddAccessAllowedAceEx, EqualSid, GetAce, GetAclInformation,
    GetKernelObjectSecurity, GetLengthSid, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
    GetSecurityDescriptorOwner, GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor,
    IsValidSid, SetKernelObjectSecurity, SetSecurityDescriptorControl, SetSecurityDescriptorDacl,
    SetSecurityDescriptorOwner, TokenOwner, TokenUser, ACCESS_ALLOWED_ACE, ACL, ACL_REVISION,
    ACL_SIZE_INFORMATION, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, OBJECT_INHERIT_ACE,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSID, SECURITY_DESCRIPTOR,
    SE_DACL_PROTECTED, TOKEN_INFORMATION_CLASS, TOKEN_OWNER, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ALL_ACCESS, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL, SYNCHRONIZE, WRITE_DAC, WRITE_OWNER,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

// [MS-DTYP] constants not re-exported without Win32_System_SystemServices.
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

struct TokenSid {
    buf: Vec<u64>,
    extract: unsafe fn(*const u8) -> PSID,
}

impl TokenSid {
    fn user() -> Result<Self, CutError> {
        unsafe fn sid_from_user(ptr: *const u8) -> PSID {
            (*(ptr.cast::<TOKEN_USER>())).User.Sid
        }
        Self::query(
            TokenUser,
            std::mem::size_of::<TOKEN_USER>(),
            sid_from_user,
            "current Windows user SID is invalid",
        )
    }

    fn default_owner() -> Result<Self, CutError> {
        unsafe fn sid_from_owner(ptr: *const u8) -> PSID {
            (*(ptr.cast::<TOKEN_OWNER>())).Owner
        }
        Self::query(
            TokenOwner,
            std::mem::size_of::<TOKEN_OWNER>(),
            sid_from_owner,
            "current Windows token default owner SID is invalid",
        )
    }

    fn query(
        class: TOKEN_INFORMATION_CLASS,
        min_bytes: usize,
        extract: unsafe fn(*const u8) -> PSID,
        invalid_msg: &'static str,
    ) -> Result<Self, CutError> {
        let mut token: HANDLE = null_mut();
        // SAFETY: `GetCurrentProcess()` returns a pseudo-handle requiring no close.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: `token` is a newly opened token handle owned by this guard.
        let token_guard = unsafe { OwnedHandle::from_raw_handle(token.cast()) };
        let mut needed = 0u32;
        // SAFETY: probing required buffer size for token SID information.
        unsafe {
            GetTokenInformation(
                token_guard.as_raw_handle().cast(),
                class,
                null_mut(),
                0,
                &mut needed,
            );
        }
        let err = unsafe { GetLastError() };
        if err != ERROR_INSUFFICIENT_BUFFER && err != 0 {
            return Err(std::io::Error::from_raw_os_error(err as i32).into());
        }
        let words = (needed as usize).max(min_bytes).div_ceil(8);
        let mut buf = vec![0u64; words];
        if unsafe {
            GetTokenInformation(
                token_guard.as_raw_handle().cast(),
                class,
                buf.as_mut_ptr().cast(),
                (buf.len() * 8) as u32,
                &mut needed,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let sid = unsafe { extract(buf.as_ptr().cast()) };
        if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
            return Err(CutError::new(
                error_codes::IO,
                invalid_msg,
                "cannot verify Windows file ownership SID",
            ));
        }
        Ok(Self { buf, extract })
    }

    fn as_psid(&self) -> PSID {
        // SAFETY: `self.buf` holds initialized token SID bytes validated in `query`.
        unsafe { (self.extract)(self.buf.as_ptr().cast()) }
    }
}

fn open_dir_handle_no_follow(path: &Path, desired_access: u32) -> Result<OwnedHandle, CutError> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` is NUL-terminated and remains live across the Win32 call.
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            desired_access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: `raw` is a valid directory handle just created by `CreateFileW`.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
    let file = std::fs::File::from(handle);
    let tag = cut_core::reparse::handle_reparse_tag(&file)?;
    if tag.is_some_and(cut_core::reparse::is_name_surrogate_tag) {
        return Err(CutError::new(
            error_codes::IO,
            format!(
                "private staging directory is a link or junction: {}",
                path.display()
            ),
            "staging directory must be a plain local directory",
        ));
    }
    let meta = file.metadata()?;
    if !meta.is_dir() {
        return Err(CutError::new(
            error_codes::IO,
            format!(
                "private staging path is not a directory: {}",
                path.display()
            ),
            "expected a directory",
        ));
    }
    Ok(OwnedHandle::from(file))
}

/// Harden `path` with a protected DACL granting full access ONLY to the current
/// user SID, and verify the live descriptor via `GetKernelObjectSecurity`.
pub(super) fn harden_and_verify_private_dir(path: &Path) -> Result<(), CutError> {
    let user = TokenSid::user()?;
    let user_sid = user.as_psid();
    let handle = open_dir_handle_no_follow(
        path,
        FILE_LIST_DIRECTORY
            | FILE_READ_ATTRIBUTES
            | READ_CONTROL
            | WRITE_DAC
            | WRITE_OWNER
            | SYNCHRONIZE,
    )?;

    let sid_len = unsafe { GetLengthSid(user_sid) } as usize;
    let acl_bytes = std::mem::size_of::<ACL>() + std::mem::size_of::<ACCESS_ALLOWED_ACE>()
        - std::mem::size_of::<u32>()
        + sid_len;
    let acl_words = acl_bytes.div_ceil(4);
    let mut acl_buf = vec![0u32; acl_words];
    let acl_ptr: *mut ACL = acl_buf.as_mut_ptr().cast();

    if unsafe { InitializeAcl(acl_ptr, (acl_words * 4) as u32, ACL_REVISION) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let flags = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
    if unsafe { AddAccessAllowedAceEx(acl_ptr, ACL_REVISION, flags, FILE_ALL_ACCESS, user_sid) }
        == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }

    let mut sd: SECURITY_DESCRIPTOR = unsafe { std::mem::zeroed() };
    let sd_ptr: *mut core::ffi::c_void = (&mut sd as *mut SECURITY_DESCRIPTOR).cast();
    if unsafe { InitializeSecurityDescriptor(sd_ptr, SECURITY_DESCRIPTOR_REVISION) } == 0
        || unsafe { SetSecurityDescriptorOwner(sd_ptr, user_sid, 0) } == 0
        || unsafe { SetSecurityDescriptorDacl(sd_ptr, 1, acl_ptr, 0) } == 0
        // `SetKernelObjectSecurity` ignores PROTECTED_DACL_SECURITY_INFORMATION:
        // the protection must be carried by the descriptor's own control bits.
        || unsafe { SetSecurityDescriptorControl(sd_ptr, SE_DACL_PROTECTED, SE_DACL_PROTECTED) }
            == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }

    let info_flags = OWNER_SECURITY_INFORMATION
        | DACL_SECURITY_INFORMATION
        | PROTECTED_DACL_SECURITY_INFORMATION;
    if unsafe { SetKernelObjectSecurity(handle.as_raw_handle().cast(), info_flags, sd_ptr) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    verify_handle_private_user_acl(path, handle.as_raw_handle().cast(), user_sid)
}

#[allow(dead_code)]
pub(super) fn verify_existing_private_dir(path: &Path) -> Result<(), CutError> {
    let user = TokenSid::user()?;
    let handle = open_dir_handle_no_follow(
        path,
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL,
    )?;
    verify_handle_private_user_acl(path, handle.as_raw_handle().cast(), user.as_psid())
}

/// True only when `path` is a file that is not a link and whose owner is the
/// current user SID or the token's default owner SID (e.g. BUILTIN\Administrators
/// for an elevated process). Any read failure answers false (the file is then kept).
pub(super) fn file_is_owned_by_current_user(path: &Path) -> bool {
    owned_by_current_user(path).unwrap_or(false)
}

fn owned_by_current_user(path: &Path) -> Result<bool, CutError> {
    let user = TokenSid::user()?;
    let default_owner = TokenSid::default_owner().ok();
    let handle = {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: `wide` is NUL-terminated and remains live across the Win32 call.
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_READ_ATTRIBUTES | READ_CONTROL,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: `raw` is a valid handle just created by `CreateFileW`.
        std::fs::File::from(unsafe { OwnedHandle::from_raw_handle(raw.cast()) })
    };
    if cut_core::reparse::handle_reparse_tag(&handle)?.is_some() || !handle.metadata()?.is_file() {
        return Ok(false);
    }
    let raw: HANDLE = handle.as_raw_handle().cast();
    let mut needed = 0u32;
    unsafe {
        GetKernelObjectSecurity(raw, OWNER_SECURITY_INFORMATION, null_mut(), 0, &mut needed);
    }
    let err = unsafe { GetLastError() };
    if err != ERROR_INSUFFICIENT_BUFFER && err != 0 {
        return Err(std::io::Error::from_raw_os_error(err as i32).into());
    }
    let words = (needed as usize)
        .max(std::mem::size_of::<SECURITY_DESCRIPTOR>())
        .div_ceil(8);
    let mut sd_buf = vec![0u64; words];
    let sd_ptr: *mut core::ffi::c_void = sd_buf.as_mut_ptr().cast();
    if unsafe {
        GetKernelObjectSecurity(
            raw,
            OWNER_SECURITY_INFORMATION,
            sd_ptr,
            (sd_buf.len() * 8) as u32,
            &mut needed,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut owner: PSID = null_mut();
    let mut owner_defaulted = 0;
    if unsafe { GetSecurityDescriptorOwner(sd_ptr, &mut owner, &mut owner_defaulted) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(!owner.is_null()
        && unsafe {
            EqualSid(owner, user.as_psid()) != 0
                || default_owner
                    .as_ref()
                    .is_some_and(|d| EqualSid(owner, d.as_psid()) != 0)
        })
}

fn verify_handle_private_user_acl(
    path: &Path,
    handle: HANDLE,
    user_sid: PSID,
) -> Result<(), CutError> {
    let requested = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let mut needed = 0u32;
    unsafe {
        GetKernelObjectSecurity(handle, requested, null_mut(), 0, &mut needed);
    }
    let err = unsafe { GetLastError() };
    if err != ERROR_INSUFFICIENT_BUFFER && err != 0 {
        return Err(std::io::Error::from_raw_os_error(err as i32).into());
    }
    let words = (needed as usize)
        .max(std::mem::size_of::<SECURITY_DESCRIPTOR>())
        .div_ceil(8);
    let mut sd_buf = vec![0u64; words];
    let sd_ptr: *mut core::ffi::c_void = sd_buf.as_mut_ptr().cast();
    if unsafe {
        GetKernelObjectSecurity(
            handle,
            requested,
            sd_ptr,
            (sd_buf.len() * 8) as u32,
            &mut needed,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }

    let mut control = 0u16;
    let mut rev = 0u32;
    if unsafe { GetSecurityDescriptorControl(sd_ptr, &mut control, &mut rev) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if (control & SE_DACL_PROTECTED) == 0 {
        return Err(acl_error(
            path,
            "private staging DACL is not protected from inherited permissions",
        ));
    }

    let mut owner: PSID = null_mut();
    let mut owner_defaulted = 0;
    if unsafe { GetSecurityDescriptorOwner(sd_ptr, &mut owner, &mut owner_defaulted) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if owner.is_null() || unsafe { EqualSid(owner, user_sid) } == 0 {
        return Err(acl_error(
            path,
            "private staging directory owner is not the current user",
        ));
    }

    let mut dacl_present = 0;
    let mut dacl_defaulted = 0;
    let mut dacl: *mut ACL = null_mut();
    if unsafe {
        GetSecurityDescriptorDacl(sd_ptr, &mut dacl_present, &mut dacl, &mut dacl_defaulted)
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    if dacl_present == 0 || dacl.is_null() {
        return Err(acl_error(path, "private staging directory lacks a DACL"));
    }

    let mut size_info = ACL_SIZE_INFORMATION::default();
    if unsafe {
        GetAclInformation(
            dacl,
            (&mut size_info as *mut ACL_SIZE_INFORMATION).cast(),
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    if size_info.AceCount != 1 {
        return Err(acl_error(
            path,
            &format!(
                "expected exactly 1 current-user ACE in staging ACL, found {}",
                size_info.AceCount
            ),
        ));
    }

    let mut ace_raw: *mut core::ffi::c_void = null_mut();
    if unsafe { GetAce(dacl, 0, &mut ace_raw) } == 0 || ace_raw.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    let ace = unsafe { &*(ace_raw.cast::<ACCESS_ALLOWED_ACE>()) };
    if ace.Header.AceType != ACCESS_ALLOWED_ACE_TYPE {
        return Err(acl_error(
            path,
            "private staging ACL sole ACE is not an allow entry",
        ));
    }
    let ace_sid: PSID = std::ptr::addr_of!(ace.SidStart).cast_mut().cast();
    if unsafe { EqualSid(ace_sid, user_sid) } == 0 {
        return Err(acl_error(
            path,
            "private staging ACL grants access to another principal",
        ));
    }
    Ok(())
}

fn acl_error(path: &Path, cause: &str) -> CutError {
    CutError::new(
        error_codes::IO,
        format!(
            "private export staging directory is not private: {}",
            path.display()
        ),
        cause,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_staging_dir_receives_user_only_protected_dacl() {
        let dir = tempfile::tempdir().unwrap();
        harden_and_verify_private_dir(dir.path()).unwrap();
        verify_existing_private_dir(dir.path()).unwrap();
        let child = dir.path().join("child.mp4");
        std::fs::write(&child, b"ok").unwrap();
        assert!(child.is_file());
    }

    /// `icacls /setowner`: true when the owner was changed.
    fn set_owner(path: &Path, account: &str) -> bool {
        std::process::Command::new("icacls")
            .arg(path)
            .args(["/setowner", account])
            .output()
            .is_ok_and(|out| out.status.success())
    }

    #[test]
    fn only_a_plain_file_owned_by_this_user_counts_as_owned() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("mine.tmp");
        std::fs::write(&file, b"ok").unwrap();
        // An elevated shell owns new files as Administrators: hand it to this user
        // (already the owner in an ordinary session, where this may fail).
        let user = std::env::var("USERNAME").unwrap_or_default();
        let domain = std::env::var("USERDOMAIN").unwrap_or_default();
        let _ = set_owner(&file, &format!("{domain}\\{user}"));
        let _ = set_owner(&file, &user);
        assert!(file_is_owned_by_current_user(&file));
        assert!(!file_is_owned_by_current_user(&dir.path().join("missing")));
        assert!(!file_is_owned_by_current_user(dir.path()));
        // Another account's file is not owned; skipped where an owner cannot be handed over.
        if set_owner(&file, "NT AUTHORITY\\SYSTEM") {
            assert!(!file_is_owned_by_current_user(&file));
        } else {
            eprintln!("NOT RUN: cannot hand a file to another account here");
        }
    }

    #[test]
    fn file_created_with_process_default_owner_counts_as_owned() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("default_owner.tmp");
        std::fs::write(&file, b"ok").unwrap();
        assert!(
            file_is_owned_by_current_user(&file),
            "file created in this session (TokenUser or TokenOwner) must count as owned without icacls handover"
        );
    }
}
