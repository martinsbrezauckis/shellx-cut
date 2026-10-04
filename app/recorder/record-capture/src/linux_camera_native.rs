//! One-owner Linux GStreamer native run and C ABI.

use std::ffi::CStr;
use std::fs::File;
use std::ptr::NonNull;
use std::time::Instant;

use record_core::Result;

use super::{error, DRAIN_MS};

#[repr(C)]
pub(super) struct NativeCameraRunOpaque {
    _private: [u8; 0],
}

unsafe extern "C" {
    pub(super) fn sxc_linux_camera_missing_plugin() -> *const libc::c_char;
    pub(super) fn sxc_linux_camera_start(
        device: *const libc::c_char,
        rdev: u64,
        inode: u64,
        output: *const libc::c_char,
        reason: *mut libc::c_char,
        reason_size: u32,
    ) -> *mut NativeCameraRunOpaque;
    pub(super) fn sxc_linux_camera_first(
        run: *mut NativeCameraRunOpaque,
        timeout_ms: u32,
    ) -> libc::c_int;
    pub(super) fn sxc_linux_camera_stop(
        run: *mut NativeCameraRunOpaque,
        timeout_ms: u32,
    ) -> libc::c_int;
    pub(super) fn sxc_linux_camera_retired(run: *mut NativeCameraRunOpaque) -> libc::c_int;
    pub(super) fn sxc_linux_camera_diagnosis(
        run: *mut NativeCameraRunOpaque,
    ) -> *const libc::c_char;
    #[cfg(test)]
    pub(super) fn sxc_linux_camera_validate_fd(
        fd: libc::c_int,
        rdev: u64,
        inode: u64,
    ) -> libc::c_int;
    #[cfg(test)]
    pub(super) fn sxc_linux_camera_test_empty() -> *mut NativeCameraRunOpaque;
    pub(super) fn sxc_linux_camera_free(run: *mut NativeCameraRunOpaque);
    pub(super) fn sxc_linux_camera_count(run: *mut NativeCameraRunOpaque) -> u32;
    pub(super) fn sxc_linux_camera_interval(
        run: *mut NativeCameraRunOpaque,
        index: u32,
        start: *mut u64,
        end: *mut u64,
    ) -> libc::c_int;
    pub(super) fn sxc_linux_camera_shape(
        run: *mut NativeCameraRunOpaque,
        w: *mut u32,
        h: *mut u32,
        n: *mut u32,
        d: *mut u32,
    ) -> libc::c_int;
    pub(super) fn sxc_linux_camera_clock_pair(
        run: *mut NativeCameraRunOpaque,
        gst: *mut u64,
        mono: *mut u64,
    ) -> libc::c_int;
}

pub(super) struct NativeRun {
    pub(super) pointer: NonNull<NativeCameraRunOpaque>,
    pub(super) writer: Option<File>,
    pub(super) gst_clock_ns: u64,
    pub(super) clock_anchor: Option<Instant>,
    pub(super) screen_origin: Instant,
}

// The pointer has one Rust owner; GStreamer callbacks synchronize through the
// native mutex, and stop retires those callbacks before Rust reads their data.
unsafe impl Send for NativeRun {}

impl NativeRun {
    pub(super) fn ptr(&self) -> *mut NativeCameraRunOpaque {
        self.pointer.as_ptr()
    }

    pub(super) fn retire(&mut self) -> Result<bool> {
        let status = unsafe { sxc_linux_camera_stop(self.ptr(), DRAIN_MS) };
        let retired = unsafe { sxc_linux_camera_retired(self.ptr()) != 0 };
        if !retired {
            return Err(error("stop Linux camera", "native writers did not retire"));
        }
        Ok(status == 0)
    }

    pub(super) fn diagnosis(&self) -> String {
        let pointer = unsafe { sxc_linux_camera_diagnosis(self.ptr()) };
        if pointer.is_null() {
            return String::new();
        }
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for NativeRun {
    fn drop(&mut self) {
        if unsafe { sxc_linux_camera_retired(self.ptr()) } == 0 {
            let _ = self.retire();
        }
        if unsafe { sxc_linux_camera_retired(self.ptr()) } != 0 {
            unsafe { sxc_linux_camera_free(self.ptr()) };
        } else if let Some(writer) = self.writer.take() {
            // A failed NULL transition cannot justify closing or sealing the
            // writable stage while the native sink might still use its alias.
            std::mem::forget(writer);
        }
    }
}
