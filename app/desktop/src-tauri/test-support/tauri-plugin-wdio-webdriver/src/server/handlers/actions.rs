use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use tauri::Runtime;

use crate::platform::PointerEventType;
use crate::server::response::{WebDriverErrorResponse, WebDriverResponse, WebDriverResult};
use crate::server::AppState;

#[derive(Debug, Deserialize)]
pub struct ActionsRequest {
    pub actions: Vec<ActionSequence>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ActionSequence {
    #[serde(rename = "key")]
    Key {
        #[serde(rename = "id")]
        _id: String,
        actions: Vec<KeyAction>,
    },
    #[serde(rename = "pointer")]
    Pointer {
        id: String,
        #[serde(default)]
        parameters: Option<serde_json::Value>,
        actions: Vec<PointerAction>,
    },
    #[serde(rename = "wheel")]
    Wheel {
        #[serde(rename = "id")]
        _id: String,
        actions: Vec<WheelAction>,
    },
    #[serde(rename = "none")]
    None {
        #[serde(rename = "id")]
        _id: String,
        actions: Vec<PauseAction>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum KeyAction {
    #[serde(rename = "keyDown")]
    KeyDown { value: String },
    #[serde(rename = "keyUp")]
    KeyUp { value: String },
    #[serde(rename = "pause")]
    Pause { duration: Option<u64> },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum PointerAction {
    #[serde(rename = "pointerDown")]
    PointerDown { button: u32 },
    #[serde(rename = "pointerUp")]
    PointerUp { button: u32 },
    #[serde(rename = "pointerMove")]
    PointerMove {
        x: i32,
        y: i32,
        duration: Option<u64>,
        #[serde(default)]
        origin: Option<Origin>,
    },
    #[serde(rename = "pause")]
    Pause { duration: Option<u64> },
}

/// W3C JSON key identifying a web element reference.
const ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

/// Coordinate origin for a `pointerMove`. Per the WebDriver Actions spec the
/// `origin` is either the string `"viewport"` (the default — x/y are absolute
/// viewport coordinates) or `"pointer"` (x/y are relative to the current pointer
/// position), or an element reference object
/// `{ "element-6066-11e4-a52e-4f735466cecf": "<id>" }` (x/y are offsets from the
/// element's in-view center point). WebdriverIO sends the element form for
/// `element.click(options)` with x/y defaulting to 0, so this must resolve to the
/// element's center rather than viewport (0,0).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Origin {
    Named(String),
    Element(HashMap<String, String>),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum WheelAction {
    #[serde(rename = "scroll")]
    Scroll {
        x: i32,
        y: i32,
        #[serde(rename = "deltaX")]
        delta_x: i32,
        #[serde(rename = "deltaY")]
        delta_y: i32,
        #[serde(default)]
        duration: Option<u64>,
    },
    #[serde(rename = "pause")]
    Pause { duration: Option<u64> },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum PauseAction {
    #[serde(rename = "pause")]
    Pause { duration: Option<u64> },
}

fn unsupported() -> WebDriverErrorResponse {
    WebDriverErrorResponse::unsupported_operation("overlay Actions supports one mouse source, instantaneous moves and balanced primary clicks only; no held state, drag, wheel, keyboard or parallel ticks")
}

fn validate(request: &ActionsRequest) -> Result<(), WebDriverErrorResponse> {
    let [ActionSequence::Pointer {
        parameters,
        actions,
        ..
    }] = request.actions.as_slice()
    else {
        return Err(unsupported());
    };
    if let Some(p) = parameters {
        if p.as_object().is_none()
            || p.get("pointerType")
                .is_some_and(|v| v.as_str() != Some("mouse"))
        {
            return Err(unsupported());
        }
    }
    if actions.len() > 1024 {
        return Err(unsupported());
    }
    let mut down = false;
    for action in actions {
        match action {
            PointerAction::PointerDown { button: 0 } if !down => down = true,
            PointerAction::PointerUp { button: 0 } if down => down = false,
            PointerAction::PointerMove {
                duration, origin, ..
            } if !down && duration.unwrap_or(0) == 0 => match origin {
                None => {}
                Some(Origin::Named(n)) if n == "viewport" || n == "pointer" => {}
                Some(Origin::Element(r)) if r.len() == 1 && r.contains_key(ELEMENT_KEY) => {}
                _ => return Err(unsupported()),
            },
            PointerAction::Pause { duration } if !down && duration.unwrap_or(0) <= 1000 => {}
            _ => return Err(unsupported()),
        }
    }
    if down {
        return Err(unsupported());
    }
    Ok(())
}

pub async fn perform<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(session_id): Path<String>,
    Json(request): Json<ActionsRequest>,
) -> WebDriverResult {
    let _input_guard = crate::platform::native_input_guard()?;
    validate(&request)?; // Validate the complete sequence before any native side effect.
    let (window, timeouts, frames, mut position) = {
        let sessions = state.sessions.read().await;
        let session = sessions.get(&session_id)?;
        if !session.action_state.pressed_keys.is_empty()
            || session
                .action_state
                .pressed_buttons
                .values()
                .any(|b| !b.is_empty())
        {
            return Err(unsupported());
        }
        (
            session.current_window.clone(),
            session.timeouts.clone(),
            session.frame_context.clone(),
            session.action_state.pointer_position,
        )
    };
    let executor = state.get_executor_for_window(&window, timeouts, frames.clone())?;
    let ActionSequence::Pointer { id, actions, .. } = &request.actions[0] else {
        unreachable!()
    };
    for action in actions {
        match action {
            PointerAction::PointerMove { x, y, origin, .. } => {
                let base = match origin {
                    None => (0, 0),
                    Some(Origin::Named(n)) if n == "viewport" => (0, 0),
                    Some(Origin::Named(_)) => position,
                    Some(Origin::Element(refs)) => {
                        let js = {
                            let sessions = state.sessions.read().await;
                            sessions
                                .get(&session_id)?
                                .elements
                                .get(&refs[ELEMENT_KEY])
                                .ok_or_else(WebDriverErrorResponse::no_such_element)?
                                .js_ref
                                .clone()
                        };
                        executor.get_element_center(&js).await?
                    }
                };
                let target = (
                    base.0.checked_add(*x).ok_or_else(unsupported)?,
                    base.1.checked_add(*y).ok_or_else(unsupported)?,
                );
                if target.0 < 0 || target.1 < 0 {
                    return Err(WebDriverErrorResponse::invalid_argument(
                        "negative viewport coordinates",
                    ));
                }
                executor
                    .dispatch_pointer_event(PointerEventType::Move, target.0, target.1, 0)
                    .await?;
                position = target;
                state
                    .sessions
                    .write()
                    .await
                    .get_mut(&session_id)?
                    .action_state
                    .pointer_position = position;
            }
            PointerAction::PointerDown { .. } => {
                state
                    .sessions
                    .write()
                    .await
                    .get_mut(&session_id)?
                    .action_state
                    .retain_primary_down(id, &window, &frames, position);
                executor
                    .dispatch_pointer_event(PointerEventType::Down, position.0, position.1, 0)
                    .await?;
            }
            PointerAction::PointerUp { .. } => {
                executor
                    .dispatch_pointer_event(PointerEventType::Up, position.0, position.1, 0)
                    .await?;
                state
                    .sessions
                    .write()
                    .await
                    .get_mut(&session_id)?
                    .action_state
                    .primary_released(id);
            }
            PointerAction::Pause { duration } => {
                tokio::time::sleep(std::time::Duration::from_millis(duration.unwrap_or(0))).await
            }
        }
    }
    Ok(WebDriverResponse::null())
}

pub async fn release<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(session_id): Path<String>,
) -> WebDriverResult {
    let _input_guard = crate::platform::native_input_guard()?;
    let (owner, timeouts, buttons) = {
        let sessions = state.sessions.read().await;
        let session = sessions.get(&session_id)?;
        if !session.action_state.pressed_keys.is_empty() {
            return Err(unsupported());
        }
        (
            session.action_state.release_owner()?,
            session.timeouts.clone(),
            session.action_state.pressed_buttons.clone(),
        )
    };
    let Some(owner) = owner else {
        return Ok(WebDriverResponse::null());
    };
    let executor = state.get_executor_for_window(&owner.window, timeouts, owner.frames)?;
    let position = owner.position;
    for (id, held) in buttons {
        for button in held {
            executor
                .dispatch_pointer_event(PointerEventType::Up, position.0, position.1, button)
                .await?;
            // Clear only after the native release succeeds; failed release remains retryable.
            state
                .sessions
                .write()
                .await
                .get_mut(&session_id)?
                .action_state
                .primary_released(&id);
        }
    }
    Ok(WebDriverResponse::null())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn admitted(value: serde_json::Value) -> bool {
        validate(&serde_json::from_value::<ActionsRequest>(value).unwrap()).is_ok()
    }
    #[test]
    fn balanced_mouse_and_unsupported_preflight() {
        use serde_json::json;
        assert!(admitted(
            json!({"actions":[{"type":"pointer","id":"mouse","parameters":{"pointerType":"mouse"},"actions":[{"type":"pointerMove","x":2,"y":3},{"type":"pointerDown","button":0},{"type":"pointerUp","button":0}]}]})
        ));
        for actions in [
            json!([{"type":"pointerDown","button":0}]),
            json!([{"type":"pointerDown","button":0},{"type":"pointerMove","x":3,"y":4},{"type":"pointerUp","button":0}]),
            json!([{"type":"pointerMove","x":1,"y":1,"duration":10}]),
        ] {
            assert!(!admitted(
                json!({"actions":[{"type":"pointer","id":"mouse","actions":actions}]})
            ));
        }
        assert!(!admitted(
            json!({"actions":[{"type":"key","id":"key","actions":[]}]})
        ));
        assert!(!admitted(
            json!({"actions":[{"type":"pointer","id":"touch","parameters":{"pointerType":"touch"},"actions":[]}]})
        ));
    }
}
