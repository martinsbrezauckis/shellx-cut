//! input.rs — shared rdevin global input hook (Windows + macOS live capture).
//!
//! Both platform backends collect the SAME input event stream (cursor/click/
//! scroll/key) via rdevin, so it lives here. `ButtonPress` carries no coordinates,
//! so we track the last `MouseMove` position and stamp clicks/scrolls with it.
//! Timestamps are our own monotonic clock (Instant) relative to capture start.
//!
//! `Input` is deliberately only the collected event data. Native listener
//! ownership, bounded teardown, and half-open sealing live in
//! `input_listener.rs` so this shared data model stays small and auditable.

use record_core::{ClickSample, CursorSample, KeySample, MouseButton, ScrollSample};

pub(crate) use crate::input_listener::InputListener;

/// Accumulated input, filled by the listener thread.
#[derive(Default)]
pub struct Input {
    pub last: (f64, f64),
    /// A button event carries no coordinates. Until rdevin delivered a real global
    /// move, `last` is only its default and cannot identify a captured-frame point.
    pub has_absolute_position: bool,
    pub cursor: Vec<CursorSample>,
    pub clicks: Vec<ClickSample>,
    pub scrolls: Vec<ScrollSample>,
    pub keys: Vec<KeySample>,
}

impl Input {
    /// Clone out the collected events.
    pub fn snapshot(
        &self,
    ) -> (
        Vec<CursorSample>,
        Vec<ClickSample>,
        Vec<ScrollSample>,
        Vec<KeySample>,
    ) {
        (
            self.cursor.clone(),
            self.clicks.clone(),
            self.scrolls.clone(),
            self.keys.clone(),
        )
    }
}

pub(crate) type InputSnapshot = (
    Vec<CursorSample>,
    Vec<ClickSample>,
    Vec<ScrollSample>,
    Vec<KeySample>,
);

pub(crate) fn map_button(b: rdevin::Button) -> MouseButton {
    match b {
        rdevin::Button::Left => MouseButton::Left,
        rdevin::Button::Right => MouseButton::Right,
        rdevin::Button::Middle => MouseButton::Middle,
        _ => MouseButton::Left,
    }
}
