//! Owns window identity and same-stream SCK frame/button correlation.
use crate::macos_window_coordinates::{map_click, Frame, Rect, MAX_AGE_MS};
use crate::surface_coordinates::AbsoluteInputOutput;
use record_core::{
    ClickPositionQuality, ClickSample, CursorCoordinateSource, CursorCoordinateState,
    CursorCorrelation,
};
use screencapturekit::{
    cm::{CMSampleBuffer, CMSampleBufferSCExt, SCFrameStatus},
    prelude::*,
};
use std::sync::{Arc, Mutex};
use std::time::Instant;

struct State {
    clock: Option<Instant>,
    previous: Option<Frame>,
    clicks: Vec<(ClickSample, u32, &'static str)>,
    closed: bool,
    metadata: Option<String>,
}

#[derive(Clone)]
pub(crate) struct WindowClickCapture {
    id: u32,
    owner: i32,
    output: (u32, u32),
    state: Arc<Mutex<State>>,
}

impl WindowClickCapture {
    pub(crate) fn new(window: &SCWindow, output: (u32, u32)) -> Option<Self> {
        let owner = window.owning_application()?.process_id();
        (owner > 0).then(|| Self {
            id: window.window_id(),
            owner,
            output,
            state: Arc::new(Mutex::new(State {
                clock: None,
                previous: None,
                clicks: vec![],
                closed: false,
                metadata: None,
            })),
        })
    }
    pub(crate) fn start(&self, start: Instant) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clock = Some(start);
    }
    pub(crate) fn source_closed(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        state.previous = None;
    }
    /// Targeted live-window lookup runs on the screen-output callback, never
    /// on the event tap. Missing metadata inserts a barrier for queued clicks.
    pub(crate) fn observe_frame(&self, sample: &CMSampleBuffer) {
        let lookup_started = Instant::now();
        let (clock, held) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed {
                return;
            }
            (state.clock, state.previous)
        };
        let Some(clock) = clock else {
            return;
        };
        let sample_info = sample.frame_info();
        let sample_status = crate::macos_frame_status::read_status(sample);
        let mut native_bounds = None;
        let frame_result: Result<Frame, &'static str> = (|| {
            let info = sample_info
                .as_ref()
                .ok_or("missing SCK frame attachments")?;
            let status = SCFrameStatus::from_raw(sample_status?)
                .ok_or("SCK status integer outside SCFrameStatus range")?;
            let idle = status == SCFrameStatus::Idle;
            if !status.has_content() && !idle {
                return Err("blank or suspended SCK content");
            }
            // SCK displayTime is Mach ticks; reject delayed compositor samples.
            let age = mach_age_ms(
                info.display_time
                    .ok_or("missing SCK displayTime attachment")?,
            )
            .ok_or("invalid SCK Mach display timestamp")?;
            let elapsed =
                u64::try_from(clock.elapsed().as_millis()).map_err(|_| "capture clock overflow")?;
            if age > MAX_AGE_MS || age > elapsed {
                return Err("stale or pre-capture SCK frame timestamp");
            }
            let t_ms = elapsed - age;
            let output = match sample.image_buffer() {
                Some(buffer) => (
                    u32::try_from(buffer.width()).map_err(|_| "invalid SCK pixel width")?,
                    u32::try_from(buffer.height()).map_err(|_| "invalid SCK pixel height")?,
                ),
                // Idle means unchanged content. It can renew fresh metadata
                // only for an already pixel-attested identical transform.
                None if idle => {
                    held.ok_or("Idle without prior pixel-attested content")?
                        .output
                }
                None => return Err("missing SCK pixel buffer"),
            };
            if output != self.output {
                return Err("SCK pixel dimensions disagree with encoded output");
            }
            let (owner, live) =
                live_window(self.id).ok_or("selected native window bounds unavailable")?;
            native_bounds = Some(live);
            if owner != self.owner {
                self.source_closed();
                return Err("selected native window owner changed");
            }
            if u64::try_from(lookup_started.elapsed().as_millis())
                .map_err(|_| "native bounds timing overflow")?
                > MAX_AGE_MS
            {
                return Err("native window bounds lookup stale");
            }
            let elapsed_now =
                u64::try_from(clock.elapsed().as_millis()).map_err(|_| "capture clock overflow")?;
            if elapsed_now.saturating_sub(t_ms) > MAX_AGE_MS {
                return Err("native bounds no longer fresh for SCK frame");
            }
            let frame = Frame::verified(
                t_ms,
                live,
                rect(
                    info.screen_rect
                        .ok_or("missing SCK screen_rect attachment")?,
                ),
                rect(
                    info.bounding_rect
                        .ok_or("missing SCK bounding_rect attachment")?,
                ),
                rect(
                    info.content_rect
                        .ok_or("missing SCK content_rect attachment")?,
                ),
                info.scale_factor
                    .ok_or("missing SCK scale_factor attachment")?,
                info.content_scale
                    .ok_or("missing SCK content_scale attachment")?,
                output,
            )
            .ok_or("SCK coordinate domains, content scale or crop disagree with native bounds")?;
            if idle && !held.is_some_and(|held| held.same_mapping(frame)) {
                return Err("Idle transform changed without a new pixel-attested frame");
            }
            Ok(frame)
        })();
        let frame = frame_result.as_ref().ok().copied();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return;
        }
        // One bounded same-stream diagnostic explains native calibration
        // without a second capture stream or a per-frame logging framework.
        let pixels = sample
            .image_buffer()
            .map(|buffer| (buffer.width(), buffer.height()));
        state.metadata = Some(format!(
            "SCK window={} owner={} native={native_bounds:?} pixels={pixels:?} status_attachment={sample_status:?} attachments={sample_info:?}",
            self.id, self.owner
        ));
        let previous = state.previous;
        for (click, age, reason) in &mut state.clicks {
            if click.position_quality == ClickPositionQuality::Unavailable {
                *reason = map_click(click, *age, &[previous, frame], self.output);
                if click.position_quality == ClickPositionQuality::Unavailable {
                    if let Err(frame_reason) = frame_result {
                        *reason = frame_reason;
                    }
                }
            }
        }
        state.previous = frame;
    }
    pub(crate) fn observe_event(&self, event: &rdevin::Event, callback_ms: u64) {
        let (button, down) = match event.event_type {
            rdevin::EventType::ButtonPress(b) => (crate::input::map_button(b), true),
            rdevin::EventType::ButtonRelease(b) => (crate::input::map_button(b), false),
            _ => return,
        };
        let point = event.native_pointer.as_ref();
        let age_ms = point.map_or(u32::MAX, |p| p.age_ms);
        if point.is_some() && u64::from(age_ms) > callback_ms {
            return;
        }
        let click = ClickSample {
            t_ms: if point.is_some() {
                callback_ms - u64::from(age_ms)
            } else {
                callback_ms
            },
            x: point.map_or(0.0, |p| p.x),
            y: point.map_or(0.0, |p| p.y),
            button,
            down,
            position_quality: ClickPositionQuality::Unavailable,
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let reason = if state.closed {
            "selected SCK source closed"
        } else if point.is_none() {
            "native button point missing"
        } else {
            "no following frame or Idle observation"
        };
        state.clicks.push((click, age_ms, reason));
    }
    pub(crate) fn finish(
        &self,
        duration_ms: u64,
        output: (u32, u32),
        clicks: &mut Vec<ClickSample>,
    ) -> AbsoluteInputOutput {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        let mut exact = 0u32;
        let mut unavailable = 0u32;
        let mut reasons = std::collections::BTreeMap::<&str, u32>::new();
        *clicks = std::mem::take(&mut state.clicks)
            .into_iter()
            .filter(|(c, _, _)| c.t_ms < duration_ms)
            .map(|(mut c, _, mut reason)| {
                if output != self.output {
                    c.position_quality = ClickPositionQuality::Unavailable;
                    reason = "final encoded dimensions disagree with observed SCK output";
                }
                if c.position_quality == ClickPositionQuality::Exact {
                    exact = exact.saturating_add(1);
                } else {
                    unavailable = unavailable.saturating_add(1);
                    *reasons.entry(reason).or_default() += 1;
                }
                c
            })
            .collect();
        AbsoluteInputOutput {
            cursor: vec![],
            scrolls: vec![],
            correlation: CursorCorrelation {
                source: CursorCoordinateSource::RdevinAbsolute,
                state: if exact == 0 {
                    CursorCoordinateState::Unavailable
                } else if unavailable > 0 {
                    CursorCoordinateState::Approximate
                } else {
                    CursorCoordinateState::Exact
                },
                exact_clicks: exact,
                approximate_clicks: 0,
                unavailable_clicks: unavailable,
                // An admitted click is bounded by both surrounding frames as
                // well as native event age; report the conservative bound.
                max_metadata_age_ms: (exact > 0).then_some(MAX_AGE_MS),
                detail: Some({
                    let reasons = reasons
                        .into_iter()
                        .map(|(reason, count)| format!("{reason}: {count}"))
                        .collect::<Vec<_>>()
                        .join("; ");
                    format!(
                        "{exact} exact, {unavailable} unavailable; {reasons}; {}",
                        state
                            .metadata
                            .as_deref()
                            .unwrap_or("no SCK frame metadata observed")
                    )
                }),
            },
        }
    }
}

fn rect(rect: screencapturekit::cg::CGRect) -> Rect {
    Rect {
        x: rect.origin.x,
        y: rect.origin.y,
        w: rect.size.width,
        h: rect.size.height,
    }
}

/// Query only the exact window's native description. CoreGraphics reports
/// window bounds in desktop points with the same axes as CGEvent.location.
/// The copied CF array is released even when any required field is absent.
fn live_window(id: u32) -> Option<(i32, Rect)> {
    use std::ffi::c_void;
    type Ref = *const c_void;
    #[repr(C)]
    struct NativeRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(options: u32, window: u32) -> Ref;
        fn CGRectMakeWithDictionaryRepresentation(dictionary: Ref, rect: *mut NativeRect) -> bool;
        static kCGWindowOwnerPID: Ref;
        static kCGWindowBounds: Ref;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFArrayGetCount(array: Ref) -> isize;
        fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
        fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
        fn CFNumberGetValue(number: Ref, kind: isize, value: *mut c_void) -> u8;
        fn CFRelease(value: Ref);
    }
    struct OwnedArray(Ref);
    impl Drop for OwnedArray {
        fn drop(&mut self) {
            // SAFETY: this owns the +1 reference returned by CopyWindowInfo.
            unsafe { CFRelease(self.0) }
        }
    }
    // SAFETY: CoreGraphics returns an array of dictionaries with typed OwnerPID
    // and Bounds fields. All borrowed values live until the owned array drops.
    unsafe {
        let array = CGWindowListCopyWindowInfo(1 << 3, id);
        if array.is_null() {
            return None;
        }
        let array = OwnedArray(array);
        if CFArrayGetCount(array.0) != 1 {
            return None;
        }
        let dictionary = CFArrayGetValueAtIndex(array.0, 0);
        let owner = CFDictionaryGetValue(dictionary, kCGWindowOwnerPID);
        let bounds = CFDictionaryGetValue(dictionary, kCGWindowBounds);
        if owner.is_null() || bounds.is_null() {
            return None;
        }
        let mut pid = 0i32;
        let mut rect = NativeRect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
        // kCFNumberSInt32Type = 3; the destination is exactly i32.
        if CFNumberGetValue(owner, 3, (&mut pid as *mut i32).cast()) == 0
            || !CGRectMakeWithDictionaryRepresentation(bounds, &mut rect)
        {
            return None;
        }
        Some((
            pid,
            Rect {
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: rect.h,
            },
        ))
    }
}

fn mach_age_ms(timestamp: u64) -> Option<u64> {
    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }
    extern "C" {
        fn mach_absolute_time() -> u64;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
    }
    let mut info = Timebase { numer: 0, denom: 0 };
    // SAFETY: timebase pointer is a valid local output; neither API retains it.
    let now = unsafe {
        if mach_timebase_info(&mut info) != 0 || info.denom == 0 {
            return None;
        }
        mach_absolute_time()
    };
    if timestamp == 0 {
        return None;
    }
    let ticks = now.checked_sub(timestamp)?;
    u64::try_from(u128::from(ticks) * u128::from(info.numer) / u128::from(info.denom) / 1_000_000)
        .ok()
}
