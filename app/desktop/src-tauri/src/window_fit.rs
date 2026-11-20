//! The first window always fits the screen it opens on.
//!
//! The default window is 1440x900 (minimum 1100x680). On a smaller screen,
//! for example 1280x800 with a taskbar, it was larger than the work area, so
//! the status bar (where the interrupted-export notice and the export folder
//! live) was hidden behind the taskbar until the window was maximized. Large
//! apps never open larger than the work area: at start the window is shrunk to
//! the work area of its monitor (the screen minus taskbar, Dock and menu bar)
//! and centred in it; the minimum size is lowered when even it does not fit.
//! The shell does not remember a window size between launches, so every
//! launch starts from the default and is fitted the same way.

use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

/// A rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Where the window goes: its inner (content) size and its outer position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Fit {
    pub inner_width: u32,
    pub inner_height: u32,
    pub x: i32,
    pub y: i32,
}

/// `None` when the window already fits inside `work`. Otherwise the inner
/// size that makes the OUTER window (with its frame and title bar) fit, and
/// the position that centres it in the work area.
pub(crate) fn fit(work: Rect, outer: Rect, inner_width: u32, inner_height: u32) -> Option<Fit> {
    let inside = outer.x >= work.x
        && outer.y >= work.y
        && i64::from(outer.x) + i64::from(outer.width) <= i64::from(work.x) + i64::from(work.width)
        && i64::from(outer.y) + i64::from(outer.height)
            <= i64::from(work.y) + i64::from(work.height);
    if inside || work.width == 0 || work.height == 0 {
        return None;
    }
    let frame_w = outer.width.saturating_sub(inner_width);
    let frame_h = outer.height.saturating_sub(inner_height);
    let outer_w = outer.width.min(work.width);
    let outer_h = outer.height.min(work.height);
    let center = |start: i32, span: u32, size: u32| {
        let offset = i64::from(span.saturating_sub(size)) / 2;
        i32::try_from(i64::from(start) + offset).unwrap_or(start)
    };
    Some(Fit {
        inner_width: outer_w.saturating_sub(frame_w).max(1),
        inner_height: outer_h.saturating_sub(frame_h).max(1),
        x: center(work.x, work.width, outer_w),
        y: center(work.y, work.height, outer_h),
    })
}

/// Fit `window` to the work area of the monitor it is on. Best effort: a
/// platform that cannot report or move windows (Wayland positions) keeps
/// what it has.
pub(crate) fn fit_to_work_area(window: &WebviewWindow) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let (Ok(outer_size), Ok(outer_pos), Ok(inner)) = (
        window.outer_size(),
        window.outer_position(),
        window.inner_size(),
    ) else {
        return;
    };
    let area = monitor.work_area();
    let work = Rect {
        x: area.position.x,
        y: area.position.y,
        width: area.size.width,
        height: area.size.height,
    };
    let outer = Rect {
        x: outer_pos.x,
        y: outer_pos.y,
        width: outer_size.width,
        height: outer_size.height,
    };
    let Some(target) = fit(work, outer, inner.width, inner.height) else {
        return;
    };
    // The configured minimum may itself be larger than a small work area.
    let _ = window.set_min_size(Some(PhysicalSize::new(
        target.inner_width.min(inner.width),
        target.inner_height.min(inner.height),
    )));
    let _ = window.set_size(PhysicalSize::new(target.inner_width, target.inner_height));
    let _ = window.set_position(PhysicalPosition::new(target.x, target.y));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_window_larger_than_a_small_screen_is_shrunk_and_centred() {
        // 1280x800 screen with a 48 px taskbar; default window 1440x900 with
        // an 8 px frame and a 31 px title bar, centred off both edges.
        let work = rect(0, 0, 1280, 752);
        let outer = rect(-88, -64, 1456, 939);
        let fitted = fit(work, outer, 1440, 900).unwrap();
        assert_eq!(fitted.inner_width + 16, 1280, "the frame fits exactly");
        assert_eq!(
            fitted.inner_height + 39,
            752,
            "the status bar is above the taskbar"
        );
        assert_eq!((fitted.x, fitted.y), (0, 0));
    }

    #[test]
    fn a_window_that_fits_is_left_alone() {
        let work = rect(0, 25, 2560, 1415);
        assert_eq!(fit(work, rect(560, 270, 1440, 900), 1440, 900), None);
    }

    #[test]
    fn a_secondary_monitor_with_an_offset_and_a_partly_fitting_window() {
        // A monitor left of the primary; the window is narrow enough but too tall.
        let work = rect(-1920, 0, 1920, 1040);
        let fitted = fit(work, rect(-1700, 100, 1200, 1100), 1200, 1060).unwrap();
        assert_eq!(fitted.inner_width, 1200);
        assert_eq!(fitted.inner_height, 1000);
        assert_eq!((fitted.x, fitted.y), (-1920 + 360, 0));
    }

    #[test]
    fn an_unreported_work_area_changes_nothing() {
        assert_eq!(
            fit(rect(0, 0, 0, 0), rect(0, 0, 1440, 900), 1440, 900),
            None
        );
    }
}
