//! Native macOS source selection for one SCK recording stream.

use crate::macos::{cap_err, recording_stream_config};
use crate::{
    macos_region_capture::prepare_region_capture_from_config, surface_coordinates, CaptureConfig,
};
use record_core::Result;
use screencapturekit::prelude::*;
use screencapturekit::shareable_content::SCShareableContentInfo;

const ENV_CONTROLLER_OWNER: &str = "SHELLX_CUT_MACOS_CONTROLLER_OWNER";
const ENV_CONTROLLER_OWNER_PID: &str = "SHELLX_CUT_MACOS_CONTROLLER_OWNER_PID";
const CONTROLLER_OWNER: &str = "tauri-shell-v1";

pub(super) type PreparedMacCaptureTarget = (
    SCContentFilter,
    u32,
    u32,
    Option<surface_coordinates::CaptureSurface>,
    SCStreamConfiguration,
    bool,
);

/// Build the exact selected SCK target. The final boolean states whether this
/// is a region path. Full-display capture includes every visible window,
/// including Cut, so its preview can reflect the same pixels as its recording.
pub(super) fn prepare_capture_target(
    cfg: &CaptureConfig,
    fps: u32,
) -> Result<PreparedMacCaptureTarget> {
    let region_backend = cfg
        .region
        .is_some()
        .then(|| prepare_region_capture_from_config(cfg, fps))
        .transpose()?;
    if let Some(region) = region_backend {
        if let Some(placement) = cfg.controller_placement.as_ref() {
            placement.unavailable(
                "This private Region capture path has no admitted controller-exclusion projection.",
            );
        }
        let (width, height) = region.output_size;
        return Ok((
            region.filter,
            width,
            height,
            Some(region.input_surface),
            region.stream_config,
            true,
        ));
    }

    let content = SCShareableContent::get()
        .map_err(|error| cap_err("SCShareableContent::get", format!("{error:?}")))?;
    // Do not use the crate's batched snapshot: v8.0.0 can panic after real
    // Screen Recording consent. The individual SCK accessors remain current.
    let windows = content.windows();
    let displays = content.displays();
    let (filter, fallback_width, fallback_height, surface) = if let Some(want) =
        cfg.window.as_deref()
    {
        if let Some(placement) = cfg.controller_placement.as_ref() {
            placement.unavailable(
                    "Selected-window capture preserves its exact source semantics; no controller exclusion or auto-hide is applied.",
                );
        }
        let want_id = crate::window_target::parse_macos_window_id(want).ok_or_else(|| {
            cap_err(
                "find the window to capture",
                "the selected window id is malformed; reopen the source picker",
            )
        })?;
        let window = windows
            .iter()
            .find(|window| window.window_id() == want_id)
            .ok_or_else(|| {
                cap_err(
                    "find the window to capture",
                    "the selected window is no longer available; reopen the source picker",
                )
            })?;
        let frame = window.frame();
        (
            SCContentFilter::create().with_window(window).build(),
            frame.size.width as u32,
            frame.size.height as u32,
            // RecordingOutput has no timestamped window geometry stream.
            None,
        )
    } else {
        let display = select_display(cfg, &displays)?;
        let frame = display.frame();
        if let Some(placement) = cfg.controller_placement.as_ref() {
            placement.not_excluded(
                "Full-display recording can include Cut when unobscured; no controller exclusion or auto-hide is applied.",
            );
        }
        (
            SCContentFilter::create()
                .with_display(display)
                .with_excluding_windows(&[])
                .build(),
            display.width(),
            display.height(),
            surface_coordinates::CaptureSurface::new(
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            ),
        )
    };

    let (pixel_width, pixel_height) = SCShareableContentInfo::for_filter(&filter)
        .map(|info| info.pixel_size())
        .filter(|(width, height)| *width > 0 && *height > 0)
        .unwrap_or((fallback_width.max(2), fallback_height.max(2)));
    let width = pixel_width & !1;
    let height = pixel_height & !1;
    Ok((
        filter,
        width,
        height,
        surface,
        recording_stream_config(width, height, fps, cfg.capture_cursor),
        false,
    ))
}

fn select_display<'a>(cfg: &CaptureConfig, displays: &'a [SCDisplay]) -> Result<&'a SCDisplay> {
    if let Some(id) = cfg.monitor_id.as_deref() {
        return displays
            .iter()
            .find(|display| crate::macos_monitor_target::monitor_id(display).as_deref() == Some(id))
            .ok_or_else(|| {
                cap_err(
                    "resolve the selected monitor identity",
                    "the selected display is no longer available; reopen the source picker",
                )
            });
    }
    let index = cfg
        .monitor
        .and_then(|monitor| usize::try_from(monitor).ok())
        .map(|monitor| monitor.saturating_sub(1))
        .unwrap_or(0);
    displays
        .get(index)
        .or_else(|| displays.first())
        .ok_or_else(|| cap_err("select a display", "no displays available"))
}

/// The selected controller must be the immediate Tauri parent which installed
/// the private marker for this child. An adopted engine has no authority to
/// hide its parent from the window picker. Full-display recording does not
/// exclude either process, so visible Cut pixels remain in the source.
pub(super) fn admitted_controller_owner_pid() -> Option<i32> {
    extern "C" {
        fn getppid() -> i32;
    }
    // SAFETY: getppid takes no arguments and has no memory-safety preconditions.
    let parent_pid = unsafe { getppid() };
    controller_owner_pid(
        std::env::var(ENV_CONTROLLER_OWNER).ok().as_deref(),
        std::env::var(ENV_CONTROLLER_OWNER_PID).ok().as_deref(),
        parent_pid,
    )
}

/// Accept only a fresh marker written by the immediate Tauri parent. A marker
/// copied from another shell, a malformed PID, or a normal/adopted engine has
/// no exclusion authority. The PID remains private transport and is never
/// projected through `screen_record.status`.
fn controller_owner_pid(
    owner: Option<&str>,
    owner_pid: Option<&str>,
    parent_pid: i32,
) -> Option<i32> {
    if owner != Some(CONTROLLER_OWNER) || parent_pid <= 0 {
        return None;
    }
    owner_pid
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|pid| *pid > 0 && *pid == parent_pid)
}

#[cfg(test)]
mod controller_owner_tests {
    use super::{controller_owner_pid, CONTROLLER_OWNER};

    #[test]
    fn missing_owner_marker_fails_closed_for_adopted_engine() {
        assert_eq!(controller_owner_pid(None, None, 41), None);
    }

    #[test]
    fn spoofed_owner_marker_fails_closed() {
        assert_eq!(
            controller_owner_pid(Some("terminal-v1"), Some("41"), 41),
            None
        );
    }

    #[test]
    fn mismatched_or_malformed_owner_pid_fails_closed() {
        assert_eq!(
            controller_owner_pid(Some(CONTROLLER_OWNER), Some("42"), 41),
            None
        );
        assert_eq!(
            controller_owner_pid(Some(CONTROLLER_OWNER), Some("not-a-pid"), 41),
            None
        );
    }

    #[test]
    fn exact_immediate_tauri_parent_is_the_only_admitted_owner() {
        assert_eq!(
            controller_owner_pid(Some(CONTROLLER_OWNER), Some("41"), 41),
            Some(41)
        );
    }
}
