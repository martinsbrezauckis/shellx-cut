use std::collections::{HashMap, HashSet};

use serde::Serialize;
use uuid::Uuid;

use super::element::ElementStore;
use crate::platform::FrameId;
use crate::server::response::WebDriverErrorResponse;

/// Tracks currently pressed keys and pointer buttons for action state
#[derive(Debug, Default, Clone)]
pub struct ActionState {
    /// Currently pressed keyboard keys (`WebDriver` key codes)
    pub pressed_keys: HashSet<String>,
    /// Currently pressed pointer buttons by source ID
    pub pressed_buttons: HashMap<String, HashSet<u32>>,
    /// Last pointer position in viewport coordinates. Persisted across
    /// `performActions` calls so an `origin: "pointer"` move resolves relative to
    /// where the pointer actually is, not (0, 0) at the start of every call.
    pub pointer_position: (i32, i32),
    /// Exact dispatch owner retained before potentially uncertain native down.
    pub native_pointer_owner: Option<NativePointerOwner>,
}

#[derive(Debug, Clone)]
pub struct NativePointerOwner {
    pub window: String,
    pub frames: Vec<FrameId>,
    pub position: (i32, i32),
}

impl ActionState {
    pub fn retain_primary_down(
        &mut self,
        source: &str,
        window: &str,
        frames: &[FrameId],
        position: (i32, i32),
    ) {
        self.native_pointer_owner = Some(NativePointerOwner {
            window: window.into(),
            frames: frames.to_vec(),
            position,
        });
        self.pressed_buttons
            .entry(source.into())
            .or_default()
            .insert(0);
    }

    pub fn release_owner(&self) -> Result<Option<NativePointerOwner>, WebDriverErrorResponse> {
        if self.pressed_buttons.values().all(HashSet::is_empty) {
            return Ok(None);
        }
        self.native_pointer_owner.clone().map(Some).ok_or_else(|| {
            WebDriverErrorResponse::unsupported_operation("held input has no retained native owner")
        })
    }

    /// Called only after successful native Up on the retained owner.
    pub fn primary_released(&mut self, source: &str) {
        self.pressed_buttons.remove(source);
        if self.pressed_buttons.values().all(HashSet::is_empty) {
            self.native_pointer_owner = None;
        }
    }
}

/// Session timeouts configuration
#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_field_names)]
pub struct Timeouts {
    /// Implicit wait timeout in milliseconds
    pub implicit_ms: u64,
    /// Page load timeout in milliseconds
    pub page_load_ms: u64,
    /// Script execution timeout in milliseconds
    pub script_ms: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            implicit_ms: 0,
            page_load_ms: 300_000,
            script_ms: 30_000,
        }
    }
}

/// Represents a `WebDriver` session
#[derive(Debug)]
pub struct Session {
    /// Unique session identifier
    pub id: String,
    /// Session timeouts
    pub timeouts: Timeouts,
    /// Element reference storage
    pub elements: ElementStore,
    /// Current window handle
    pub current_window: String,
    /// Current frame context (stack of frame selectors)
    pub frame_context: Vec<FrameId>,
    /// Action state tracking for pressed keys/buttons
    pub action_state: ActionState,
}

impl Session {
    pub fn new(initial_window: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timeouts: Timeouts::default(),
            elements: ElementStore::new(),
            current_window: initial_window,
            frame_context: Vec::new(),
            action_state: ActionState::default(),
        }
    }
}

/// Manages `WebDriver` sessions
#[derive(Debug, Default)]
pub struct SessionManager {
    sessions: HashMap<String, Session>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Create a new session
    pub fn create(&mut self, initial_window: String) -> &Session {
        let session = Session::new(initial_window);
        let id = session.id.clone();
        self.sessions.insert(id.clone(), session);
        self.sessions.get(&id).expect("session was just inserted")
    }

    /// Get a session by ID
    pub fn get(&self, id: &str) -> Result<&Session, WebDriverErrorResponse> {
        self.sessions
            .get(id)
            .ok_or_else(|| WebDriverErrorResponse::invalid_session_id(id))
    }

    /// Get a mutable session by ID
    pub fn get_mut(&mut self, id: &str) -> Result<&mut Session, WebDriverErrorResponse> {
        self.sessions
            .get_mut(id)
            .ok_or_else(|| WebDriverErrorResponse::invalid_session_id(id))
    }

    /// Delete a session
    pub fn delete(&mut self, id: &str) -> bool {
        self.sessions.remove(id).is_some()
    }
}

#[cfg(test)]
mod native_owner_tests {
    use super::*;
    #[test]
    fn release_uses_original_owner_after_current_window_and_frame_change() {
        let mut session = Session::new("original".into());
        session
            .action_state
            .retain_primary_down("mouse", &session.current_window, &[], (12, 34));
        session.current_window = "different".into();
        session.frame_context = vec![FrameId::Index(2)];
        session.action_state.pointer_position = (90, 91);
        let owner = session.action_state.release_owner().unwrap().unwrap();
        assert_eq!(owner.window, "original");
        assert!(owner.frames.is_empty());
        assert_eq!(owner.position, (12, 34));
        // Failed native release does not call primary_released: intent survives.
        assert!(session.action_state.release_owner().unwrap().is_some());
        session.action_state.primary_released("mouse");
        assert!(session.action_state.release_owner().unwrap().is_none());
        assert!(session.action_state.native_pointer_owner.is_none());
    }
    #[test]
    fn missing_owner_is_not_replaced_by_current_session_window() {
        let mut state = ActionState::default();
        state
            .pressed_buttons
            .entry("mouse".into())
            .or_default()
            .insert(0);
        assert!(state.release_owner().is_err());
    }
}
