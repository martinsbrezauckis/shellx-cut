//! Owned macOS process-tap lifetime and measured packet-clock facts.

use std::ptr::NonNull;

extern "C" {
    fn sxc_sysaudio_start() -> *mut std::ffi::c_void;
    fn sxc_sysaudio_stop(
        ctx: *mut std::ffi::c_void,
        out_samples: *mut *mut f32,
        out_count: *mut u64,
        out_channels: *mut u32,
        out_rate: *mut f64,
        out_first_packet_ms: *mut u64,
    ) -> i32;
    fn sxc_sysaudio_free(p: *mut f32);
}

pub(crate) struct SystemAudioTap {
    ctx: Option<NonNull<std::ffi::c_void>>,
    clock_offset_ms: u64,
}

impl SystemAudioTap {
    pub(crate) fn start(clock_offset_ms: u64) -> Option<Self> {
        let ctx = NonNull::new(unsafe { sxc_sysaudio_start() })?;
        Some(Self {
            ctx: Some(ctx),
            clock_offset_ms,
        })
    }

    pub(crate) fn finish(mut self) -> SystemAudioResult {
        let ctx = self.ctx.take().expect("system audio context is present");
        let mut out_ptr = std::ptr::null_mut();
        let mut count = 0;
        let mut channels = 0;
        let mut rate = 0.0;
        let mut first_packet_after_tap_ms = u64::MAX;
        let rc = unsafe {
            sxc_sysaudio_stop(
                ctx.as_ptr(),
                &mut out_ptr,
                &mut count,
                &mut channels,
                &mut rate,
                &mut first_packet_after_tap_ms,
            )
        };
        let samples = if rc == 0 {
            unsafe { SystemAudioBuffer::from_ffi(out_ptr, count) }
        } else {
            if !out_ptr.is_null() {
                unsafe { sxc_sysaudio_free(out_ptr) };
            }
            None
        };
        SystemAudioResult {
            rc,
            samples,
            count,
            channels,
            rate,
            first_packet_offset_ms: (first_packet_after_tap_ms != u64::MAX).then(|| {
                self.clock_offset_ms
                    .saturating_add(first_packet_after_tap_ms)
            }),
        }
    }
}

impl Drop for SystemAudioTap {
    fn drop(&mut self) {
        let Some(ctx) = self.ctx.take() else { return };
        unsafe {
            sxc_sysaudio_stop(
                ctx.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
        }
    }
}

pub(crate) struct SystemAudioBuffer {
    ptr: NonNull<f32>,
    len: usize,
}

impl SystemAudioBuffer {
    unsafe fn from_ffi(ptr: *mut f32, count: u64) -> Option<Self> {
        let ptr = NonNull::new(ptr)?;
        let len = match usize::try_from(count) {
            Ok(len) if len > 0 && len <= (isize::MAX as usize) / std::mem::size_of::<f32>() => len,
            _ => {
                unsafe { sxc_sysaudio_free(ptr.as_ptr()) };
                return None;
            }
        };
        Some(Self { ptr, len })
    }

    pub(crate) fn as_slice(&self) -> &[f32] {
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for SystemAudioBuffer {
    fn drop(&mut self) {
        unsafe { sxc_sysaudio_free(self.ptr.as_ptr()) };
    }
}

pub(crate) struct SystemAudioResult {
    pub(crate) rc: i32,
    pub(crate) samples: Option<SystemAudioBuffer>,
    pub(crate) count: u64,
    pub(crate) channels: u32,
    pub(crate) rate: f64,
    pub(crate) first_packet_offset_ms: Option<u64>,
}
