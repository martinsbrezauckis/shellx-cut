use std::collections::{HashMap, HashSet};

use serde::Serialize;
use uuid::Uuid;

use super::element::ElementStore;
use crate::platform::{FrameId, ModifierState};
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
    pub release_order: Vec<NativeRelease>,
    pub input_failed: bool,
    pub source_types: HashMap<String, String>,
    pub keyboard_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NativeRelease {
    Key(String),
    Button(String, u32),
}

#[derive(Debug, Clone)]
pub struct NativePointerOwner {
    pub window: String,
    pub frames: Vec<FrameId>,
    pub position: (i32, i32),
}

impl ActionState {
    pub fn accepts_keyboard_source(&self, source: &str) -> bool {
        self.pressed_keys.is_empty() || self.keyboard_source.as_deref() == Some(source)
    }
    pub fn has_held_input(&self) -> bool {
        !self.pressed_keys.is_empty() || self.pressed_buttons.values().any(|v| !v.is_empty())
    }
    pub fn modifiers(&self) -> ModifierState {
        let mut value = ModifierState::default();
        for key in &self.pressed_keys {
            value.update(key, true);
        }
        value
    }
    pub fn buttons(&self) -> u32 {
        self.pressed_buttons
            .values()
            .flatten()
            .fold(0, |mask, button| {
                mask | match button {
                    0 => 1,
                    1 => 4,
                    2 => 2,
                    _ => 0,
                }
            })
    }
    pub fn bind_owner(
        &mut self,
        window: &str,
        frames: &[FrameId],
    ) -> Result<(), WebDriverErrorResponse> {
        if !frames.is_empty() {
            return Err(WebDriverErrorResponse::unsupported_operation(
                "native input supports top-level frames only",
            ));
        }
        if self.has_held_input() {
            let owner = self.release_owner()?.ok_or_else(|| {
                WebDriverErrorResponse::unsupported_operation("held input owner missing")
            })?;
            if owner.window != window || !owner.frames.is_empty() {
                return Err(WebDriverErrorResponse::unsupported_operation(
                    "release held input on its original owner before switching context",
                ));
            }
        } else {
            self.native_pointer_owner = Some(NativePointerOwner {
                window: window.into(),
                frames: frames.to_vec(),
                position: self.pointer_position,
            });
        }
        Ok(())
    }
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
        self.retain_button(source, 0);
    }
    pub fn retain_button(&mut self, source: &str, button: u32) {
        if self
            .pressed_buttons
            .entry(source.into())
            .or_default()
            .insert(button)
        {
            self.release_order
                .push(NativeRelease::Button(source.into(), button));
        }
    }
    pub fn retain_key(&mut self, key: &str) {
        if self.pressed_keys.insert(key.into()) {
            self.release_order.push(NativeRelease::Key(key.into()));
        }
    }
    pub fn released(&mut self, item: &NativeRelease) {
        match item {
            NativeRelease::Key(key) => {
                self.pressed_keys.remove(key);
                if self.pressed_keys.is_empty() {
                    self.keyboard_source = None;
                }
            }
            NativeRelease::Button(source, button) => {
                if let Some(held) = self.pressed_buttons.get_mut(source) {
                    held.remove(button);
                }
            }
        }
        self.release_order.retain(|x| x != item);
        if !self.has_held_input() {
            self.native_pointer_owner = None;
        }
    }
    pub fn moved(&mut self, position: (i32, i32)) {
        self.pointer_position = position;
        if let Some(owner) = &mut self.native_pointer_owner {
            owner.position = position;
        }
    }
    pub fn release_owner(&self) -> Result<Option<NativePointerOwner>, WebDriverErrorResponse> {
        if !self.has_held_input() {
            return Ok(None);
        }
        self.native_pointer_owner.clone().map(Some).ok_or_else(|| {
            WebDriverErrorResponse::unsupported_operation("held input has no retained native owner")
        })
    }
    pub fn primary_released(&mut self, source: &str) {
        self.released(&NativeRelease::Button(source.into(), 0));
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

    pub fn has_session(&self) -> bool {
        !self.sessions.is_empty()
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

    /// Only a successfully released session may be discarded.
    pub fn delete_released(&mut self, id: &str) -> Result<(), WebDriverErrorResponse> {
        let session = self.get(id)?;
        if session.action_state.has_held_input() || session.action_state.input_failed {
            return Err(WebDriverErrorResponse::unsupported_operation(
                "native input release must finish before session deletion",
            ));
        }
        self.sessions.remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod native_owner_tests {
    use super::*;
    #[test]
    fn session_delete_preserves_held_owner_until_release_succeeds() {
        let mut manager = SessionManager::new();
        let id = manager.create("owned-window".into()).id.clone();
        let input = &mut manager.get_mut(&id).unwrap().action_state;
        input.bind_owner("owned-window", &[]).unwrap();
        input.keyboard_source = Some("keys".into());
        input.retain_key("\u{E008}");
        assert!(manager.delete_released(&id).is_err());
        assert_eq!(
            manager
                .get(&id)
                .unwrap()
                .action_state
                .release_owner()
                .unwrap()
                .unwrap()
                .window,
            "owned-window"
        );
        let input = &mut manager.get_mut(&id).unwrap().action_state;
        input.released(&NativeRelease::Key("\u{E008}".into()));
        input.input_failed = true;
        assert!(manager.delete_released(&id).is_err());
        manager.get_mut(&id).unwrap().action_state.input_failed = false;
        manager.delete_released(&id).unwrap();
        assert!(!manager.has_session());
    }
    #[test]
    fn keyboard_identity_is_pinned_until_last_native_release() {
        let mut state = ActionState::default();
        state.bind_owner("window", &[]).unwrap();
        state.keyboard_source = Some("original-keyboard".into());
        state.retain_key("\u{E008}");
        assert!(state.accepts_keyboard_source("original-keyboard"));
        assert!(!state.accepts_keyboard_source("different-keyboard"));
        state.released(&NativeRelease::Key("\u{E008}".into()));
        assert!(state.accepts_keyboard_source("different-keyboard"));
        assert!(state.keyboard_source.is_none());
    }
    #[test]
    fn held_modifiers_drag_and_reverse_release_keep_owner() {
        let mut state = ActionState::default();
        state.bind_owner("original", &[]).unwrap();
        state.retain_key("\u{E008}");
        state.retain_button("mouse", 2);
        state.moved((60, 70));
        assert_eq!(state.buttons(), 2);
        assert!(state.modifiers().shift);
        assert!(state.bind_owner("different", &[]).is_err());
        let order = state.release_order.clone();
        assert_eq!(
            order.last(),
            Some(&NativeRelease::Button("mouse".into(), 2))
        );
        state.released(order.last().unwrap());
        assert_eq!(state.buttons(), 0);
        assert_eq!(state.release_owner().unwrap().unwrap().position, (60, 70));
        assert!(state.modifiers().shift);
        state.released(&NativeRelease::Key("\u{E008}".into()));
        assert!(!state.has_held_input());
        assert!(state.native_pointer_owner.is_none());
    }
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
