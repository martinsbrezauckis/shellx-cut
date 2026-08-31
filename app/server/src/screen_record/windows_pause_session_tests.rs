//! Bounded Windows pause-session test routes.

use super::*;

#[path = "windows_pause_session_tests/lifecycle.rs"]
mod lifecycle;
#[path = "windows_pause_session_tests/support.rs"]
mod support;
