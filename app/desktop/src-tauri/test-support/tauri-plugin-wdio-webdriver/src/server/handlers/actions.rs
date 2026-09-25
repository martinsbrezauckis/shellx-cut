use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use tauri::Runtime;

use crate::platform::{PointerEventType,PlatformExecutor};
use crate::server::response::{WebDriverErrorResponse, WebDriverResponse, WebDriverResult};
use crate::server::AppState;
use crate::webdriver::session::ActionState;

async fn prepare_key_down<F: Future<Output = Result<(), WebDriverErrorResponse>>>(
    input: &mut ActionState, window: &str, frames: &[crate::platform::FrameId],
    source: &str, value: &str, preflight: F,
) -> Result<(), WebDriverErrorResponse> {
    preflight.await?;
    input.bind_owner(window, frames)?;
    input.keyboard_source = Some(source.into());
    input.retain_key(value);
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct ActionsRequest {
    pub actions: Vec<ActionSequence>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ActionSequence {
    #[serde(rename = "key")]
    Key { id: String, actions: Vec<KeyAction> },
    #[serde(rename = "pointer")]
    Pointer {
        id: String,
        #[serde(default)]
        parameters: Option<serde_json::Value>,
        actions: Vec<PointerAction>,
    },
    #[serde(rename = "wheel")]
    Wheel {
        id: String,
        actions: Vec<WheelAction>,
    },
    #[serde(rename = "none")]
    None {
        id: String,
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
        #[serde(default)]
        origin: Option<Origin>,
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
    WebDriverErrorResponse::unsupported_operation("native input supports bounded top-level mouse, keyboard and wheel actions; touch/pen and multiple sources of one type are unsupported")
}

fn key_supported(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(c) = chars.next() else {
        return false;
    };
    chars.next().is_none()
        && ((!c.is_control() && !(('\u{E000}'..='\u{F8FF}').contains(&c)))
            || matches!(
                c,
                '\u{E003}'
                    | '\u{E004}'
                    | '\u{E006}'
                    | '\u{E007}'
                    | '\u{E008}'
                    | '\u{E009}'
                    | '\u{E00A}'
                    | '\u{E00C}'
                    | '\u{E00D}'
                    | '\u{E00E}'
                    | '\u{E00F}'
                    | '\u{E010}'
                    | '\u{E011}'
                    | '\u{E012}'
                    | '\u{E013}'
                    | '\u{E014}'
                    | '\u{E015}'
                    | '\u{E016}'
                    | '\u{E017}'
                    | '\u{E03D}'
            )
            || ('\u{E031}'..='\u{E03C}').contains(&c))
}
impl ActionSequence {
    fn identity(&self) -> (&str, &'static str) {
        match self {
            Self::Key { id, .. } => (id, "key"),
            Self::Pointer { id, .. } => (id, "pointer"),
            Self::Wheel { id, .. } => (id, "wheel"),
            Self::None { id, .. } => (id, "none"),
        }
    }
    fn len(&self) -> usize {
        match self {
            Self::Key { actions, .. } => actions.len(),
            Self::Pointer { actions, .. } => actions.len(),
            Self::Wheel { actions, .. } => actions.len(),
            Self::None { actions, .. } => actions.len(),
        }
    }
    fn duration(&self, tick: usize) -> u64 {
        match self {
            Self::Key { actions, .. } => match actions.get(tick) {
                Some(KeyAction::Pause { duration }) => duration.unwrap_or(0),
                _ => 0,
            },
            Self::Pointer { actions, .. } => match actions.get(tick) {
                Some(
                    PointerAction::Pause { duration } | PointerAction::PointerMove { duration, .. },
                ) => duration.unwrap_or(0),
                _ => 0,
            },
            Self::Wheel { actions, .. } => match actions.get(tick) {
                Some(WheelAction::Pause { duration } | WheelAction::Scroll { duration, .. }) => {
                    duration.unwrap_or(0)
                }
                _ => 0,
            },
            Self::None { actions, .. } => match actions.get(tick) {
                Some(PauseAction::Pause { duration }) => duration.unwrap_or(0),
                _ => 0,
            },
        }
    }
}
fn valid_origin(origin: &Option<Origin>, pointer: bool) -> bool {
    match origin {
        None => true,
        Some(Origin::Named(n)) => n == "viewport" || (pointer && n == "pointer"),
        Some(Origin::Element(r)) => r.len() == 1 && r.contains_key(ELEMENT_KEY),
    }
}
pub(crate) fn validate(request: &ActionsRequest) -> Result<(), WebDriverErrorResponse> {
    if request.actions.is_empty() || request.actions.len() > 4 {
        return Err(unsupported());
    }
    let mut ids = std::collections::HashSet::new();
    let mut kinds = std::collections::HashSet::new();
    for source in &request.actions {
        let (id, kind) = source.identity();
        if id.is_empty()
            || id.len() > 128
            || !ids.insert(id)
            || !kinds.insert(kind)
            || source.len() > 8192
        {
            return Err(unsupported());
        }
        match source {
            ActionSequence::Key { actions, .. } => {
                for a in actions {
                    if let KeyAction::KeyDown { value } | KeyAction::KeyUp { value } = a {
                        if !key_supported(value) {
                            return Err(unsupported());
                        }
                    }
                }
            }
            ActionSequence::Pointer {
                parameters,
                actions,
                ..
            } => {
                if parameters.as_ref().is_some_and(|p| {
                    p.as_object().is_none()
                        || p.get("pointerType")
                            .is_some_and(|v| v.as_str() != Some("mouse"))
                }) {
                    return Err(unsupported());
                }
                for a in actions {
                    match a {
                        PointerAction::PointerDown { button }
                        | PointerAction::PointerUp { button }
                            if *button > 2 =>
                        {
                            return Err(unsupported())
                        }
                        PointerAction::PointerMove { origin, .. }
                            if !valid_origin(origin, true) =>
                        {
                            return Err(unsupported())
                        }
                        _ => {}
                    }
                }
            }
            ActionSequence::Wheel { actions, .. } => {
                for a in actions {
                    if let WheelAction::Scroll { origin, .. } = a {
                        if !valid_origin(origin, false) {
                            return Err(unsupported());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let ticks = request
        .actions
        .iter()
        .map(ActionSequence::len)
        .max()
        .unwrap_or(0);
    let mut total = 0u64;
    for tick in 0..ticks {
        let duration = request
            .actions
            .iter()
            .map(|s| s.duration(tick))
            .max()
            .unwrap_or(0);
        if duration > 10_000 {
            return Err(unsupported());
        }
        total = total.checked_add(duration).ok_or_else(unsupported)?;
    }
    if total > 30_000 {
        return Err(unsupported());
    }
    Ok(())
}

pub async fn perform<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(session_id): Path<String>,
    Json(request): Json<ActionsRequest>,
) -> WebDriverResult {
    let _guard = crate::platform::native_input_guard()?;
    perform_inner(&state, &session_id, request).await
}

pub(crate) async fn perform_inner<R: Runtime + 'static>(
    state: &Arc<AppState<R>>,
    session_id: &str,
    request: ActionsRequest,
) -> WebDriverResult {
    perform_scoped(state, session_id, request, false).await
}

pub(crate) async fn perform_option_inner<R: Runtime + 'static>(state: &Arc<AppState<R>>, session_id: &str, request: ActionsRequest) -> WebDriverResult {
    perform_scoped(state, session_id, request, true).await
}

fn preflight_keyboard_actions(
    request: &ActionsRequest,
    mut preflight: impl FnMut(&str) -> Result<(), WebDriverErrorResponse>,
) -> Result<(), WebDriverErrorResponse> {
    for source in &request.actions {
        if let ActionSequence::Key { actions, .. } = source {
            for action in actions {
                if let KeyAction::KeyDown { value } | KeyAction::KeyUp { value } = action {
                    preflight(value)?;
                }
            }
        }
    }
    Ok(())
}

async fn perform_scoped<R: Runtime + 'static>(state: &Arc<AppState<R>>, session_id: &str, request: ActionsRequest, option: bool) -> WebDriverResult {
    validate(&request)?;
    if state
        .sessions
        .read()
        .await
        .get(session_id)?
        .action_state
        .input_failed
    {
        return Err(WebDriverErrorResponse::unsupported_operation(
            "release prior failed native input before continuing",
        ));
    }
    if request.actions.iter().any(|source| matches!(source, ActionSequence::Key { .. })) {
        let (window, timeouts, frames) = {
            let sessions = state.sessions.read().await;
            let session = sessions.get(session_id)?;
            (
                session.current_window.clone(),
                session.timeouts.clone(),
                session.frame_context.clone(),
            )
        };
        let executor = state.get_executor_for_window(&window, timeouts, frames)?;
        // This pure representability check runs before perform_sequence's
        // conservative error latch and before any key is retained. A rejected
        // key has no native effect; a dispatch failure still needs release.
        preflight_keyboard_actions(&request, |key| executor.preflight_key_event(key))?;
    }
    // Target lookup and action admission can fail before native input begins.
    // Only a dispatch attempt has uncertain native state that needs release.
    let mut native_dispatch_attempted = false;
    let result = perform_sequence(state, session_id, request, option, &mut native_dispatch_attempted).await;
    if result.is_err() && native_dispatch_attempted {
        state
            .sessions
            .write()
            .await
            .get_mut(session_id)?
            .action_state
            .input_failed = true;
    }
    result
}

async fn perform_sequence<R: Runtime + 'static>(
    state: &Arc<AppState<R>>,
    session_id: &str,
    request: ActionsRequest,
    option: bool,
    native_dispatch_attempted: &mut bool,
) -> WebDriverResult {
    validate(&request)?;
    let (window, timeouts, frames, mut input) = {
        let sessions = state.sessions.read().await;
        let s = sessions.get(session_id)?;
        (
            s.current_window.clone(),
            s.timeouts.clone(),
            s.frame_context.clone(),
            s.action_state.clone(),
        )
    };
    input.bind_owner(&window, &frames)?;
    for source in &request.actions {
        let (id, kind) = source.identity();
        if input.source_types.get(id).is_some_and(|old| old != kind)
            || input.source_types.len() >= 32 && !input.source_types.contains_key(id)
        {
            return Err(unsupported());
        }
        if kind == "pointer"
            && input
                .pressed_buttons
                .iter()
                .any(|(old, b)| old != id && !b.is_empty())
        {
            return Err(unsupported());
        }
        if kind == "key" && !input.accepts_keyboard_source(id) {
            return Err(unsupported());
        }
        input.source_types.insert(id.into(), kind.into());
    }
    let executor = state.get_executor_for_window(&window, timeouts, frames.clone())?;
    let ticks = request
        .actions
        .iter()
        .map(ActionSequence::len)
        .max()
        .unwrap_or(0);
    for tick in 0..ticks {
        let start = tokio::time::Instant::now();
        let duration = request
            .actions
            .iter()
            .map(|s| s.duration(tick))
            .max()
            .unwrap_or(0);
        // Resolve all targets before this tick changes input state.
        let mut pointer_target = None;
        let mut wheel_target = None;
        for source in &request.actions {
            match source {
                ActionSequence::Pointer { actions, .. } => {
                    if let Some(PointerAction::PointerMove { x, y, origin, .. }) = actions.get(tick)
                    {
                        let base = origin_base(
                            state,
                            session_id,
                            &*executor,
                            origin,
                            input.pointer_position,
                        )
                        .await?;
                        pointer_target = Some((
                            base.0.checked_add(*x).ok_or_else(unsupported)?,
                            base.1.checked_add(*y).ok_or_else(unsupported)?,
                        ));
                    }
                }
                ActionSequence::Wheel { actions, .. } => {
                    if let Some(WheelAction::Scroll { x, y, origin, .. }) = actions.get(tick) {
                        let base =
                            origin_base(state, session_id, &*executor, origin, (0, 0)).await?;
                        wheel_target = Some((
                            base.0.checked_add(*x).ok_or_else(unsupported)?,
                            base.1.checked_add(*y).ok_or_else(unsupported)?,
                        ));
                    }
                }
                _ => {}
            }
        }
        for source in &request.actions {
            if let ActionSequence::Key { id, actions, .. } = source {
                match actions.get(tick) {
                    Some(KeyAction::KeyDown { value }) => {
                        prepare_key_down(&mut input, &window, &frames, id, value, async {
                            if option { Ok(()) } else { executor.preflight_native_key_down().await }
                        }).await?;
                        save_input(state, session_id, &input).await?;
                        *native_dispatch_attempted = true;
                        if option { executor.dispatch_option_key_event(value, true, &input.modifiers()).await?; }
                        else { executor.dispatch_key_event(value, true, &input.modifiers()).await?; }
                    }
                    Some(KeyAction::KeyUp { value }) if input.pressed_keys.contains(value) => {
                        let mut next = input.clone();
                        next.released(&crate::webdriver::session::NativeRelease::Key(
                            value.clone(),
                        ));
                        *native_dispatch_attempted = true;
                        if option { executor.dispatch_option_key_event(value, false, &next.modifiers()).await?; }
                        else { executor.dispatch_key_event(value, false, &next.modifiers()).await?; }
                        input = next;
                        save_input(state, session_id, &input).await?;
                    }
                    _ => {}
                }
            }
        }
        for source in &request.actions {
            if let ActionSequence::Pointer { id, actions, .. } = source {
                match actions.get(tick) {
                    Some(PointerAction::PointerDown { button }) => {
                        if input
                            .pressed_buttons
                            .get(id)
                            .is_some_and(|b| b.contains(button))
                        {
                            continue;
                        }
                        input.bind_owner(&window, &frames)?;
                        input.retain_button(id, *button);
                        save_input(state, session_id, &input).await?;
                        *native_dispatch_attempted = true;
                        executor
                            .dispatch_pointer_event(
                                PointerEventType::Down,
                                input.pointer_position.0,
                                input.pointer_position.1,
                                *button,
                                input.buttons(),
                                &input.modifiers(),
                            )
                            .await?;
                    }
                    Some(PointerAction::PointerUp { button }) => {
                        if !input
                            .pressed_buttons
                            .get(id)
                            .is_some_and(|b| b.contains(button))
                        {
                            continue;
                        }
                        let mut next = input.clone();
                        next.released(&crate::webdriver::session::NativeRelease::Button(
                            id.clone(),
                            *button,
                        ));
                        *native_dispatch_attempted = true;
                        executor
                            .dispatch_pointer_event(
                                PointerEventType::Up,
                                input.pointer_position.0,
                                input.pointer_position.1,
                                *button,
                                next.buttons(),
                                &next.modifiers(),
                            )
                            .await?;
                        input = next;
                        save_input(state, session_id, &input).await?;
                    }
                    _ => {}
                }
            }
        }
        let from = input.pointer_position;
        let steps = if pointer_target.is_some() || wheel_target.is_some() {
            (duration / 16).max(1)
        } else {
            0
        };
        let mut wheel_sent = (0i32, 0i32);
        for step in 1..=steps {
            if duration > 0 {
                tokio::time::sleep_until(
                    start + std::time::Duration::from_millis(duration * step / steps),
                )
                .await;
            }
            if let Some(target) = pointer_target {
                let point = (
                    interpolate(from.0, target.0, step, steps),
                    interpolate(from.1, target.1, step, steps),
                );
                // Failed or rejected movement must not replace the last admitted release coordinates.
                *native_dispatch_attempted = true;
                executor
                    .dispatch_pointer_event(
                        PointerEventType::Move,
                        point.0,
                        point.1,
                        0,
                        input.buttons(),
                        &input.modifiers(),
                    )
                    .await?;
                input.moved(point);
                save_input(state, session_id, &input).await?;
            }
            if let Some(point) = wheel_target {
                for source in &request.actions {
                    if let ActionSequence::Wheel { actions, .. } = source {
                        if let Some(WheelAction::Scroll {
                            delta_x, delta_y, ..
                        }) = actions.get(tick)
                        {
                            let total = (
                                interpolate(0, *delta_x, step, steps),
                                interpolate(0, *delta_y, step, steps),
                            );
                            *native_dispatch_attempted = true;
                            executor
                                .dispatch_scroll_event(
                                    point.0,
                                    point.1,
                                    total.0 - wheel_sent.0,
                                    total.1 - wheel_sent.1,
                                    &input.modifiers(),
                                )
                                .await?;
                            wheel_sent = total;
                        }
                    }
                }
            }
        }
        if duration > 0 {
            tokio::time::sleep_until(start + std::time::Duration::from_millis(duration)).await;
        }
    }
    save_input(state, session_id, &input).await?;
    Ok(WebDriverResponse::null())
}
fn interpolate(from: i32, to: i32, step: u64, steps: u64) -> i32 {
    (i64::from(from) + (i64::from(to) - i64::from(from)) * step as i64 / steps as i64) as i32
}
async fn save_input<R: Runtime + 'static>(
    state: &Arc<AppState<R>>,
    id: &str,
    input: &crate::webdriver::session::ActionState,
) -> Result<(), WebDriverErrorResponse> {
    state.sessions.write().await.get_mut(id)?.action_state = input.clone();
    Ok(())
}
async fn origin_base<R: Runtime + 'static>(
    state: &Arc<AppState<R>>,
    id: &str,
    executor: &dyn crate::platform::PlatformExecutor<R>,
    origin: &Option<Origin>,
    position: (i32, i32),
) -> Result<(i32, i32), WebDriverErrorResponse> {
    match origin {
        None => Ok((0, 0)),
        Some(Origin::Named(n)) if n == "pointer" => Ok(position),
        Some(Origin::Named(_)) => Ok((0, 0)),
        Some(Origin::Element(r)) => {
            let js = state
                .sessions
                .read()
                .await
                .get(id)?
                .elements
                .get(&r[ELEMENT_KEY])
                .ok_or_else(WebDriverErrorResponse::no_such_element)?
                .js_ref
                .clone();
            executor.get_element_center(&js).await
        }
    }
}

pub async fn release<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<String>,
) -> WebDriverResult {
    let _guard = crate::platform::native_input_guard()?;
    release_inner(&state, &id).await
}

pub(crate) async fn release_inner<R: Runtime + 'static>(
    state: &Arc<AppState<R>>,
    id: &str,
) -> WebDriverResult {
    release_scoped(state, id, None, false).await
}

pub(crate) async fn release_option_inner<R: Runtime + 'static>(state: &Arc<AppState<R>>, id: &str, executor: &dyn PlatformExecutor<R>) -> WebDriverResult {
    release_scoped(state, id, Some(executor), false).await
}

pub(crate) async fn release_option_keys_inner<R: Runtime + 'static>(state:&Arc<AppState<R>>,id:&str,executor:&dyn PlatformExecutor<R>)->WebDriverResult {
    release_scoped(state,id,Some(executor),true).await
}

fn release_plan(input:&crate::webdriver::session::ActionState,keys_only:bool)->Vec<crate::webdriver::session::NativeRelease> {
    input.release_order.iter().rev().filter(|item|!keys_only || matches!(item,crate::webdriver::session::NativeRelease::Key(_))).cloned().collect()
}

async fn release_scoped<R: Runtime + 'static>(state: &Arc<AppState<R>>, id: &str, option_executor: Option<&dyn PlatformExecutor<R>>, keys_only:bool) -> WebDriverResult {
    let option=option_executor.is_some();
    let (mut input, timeouts) = {
        let sessions = state.sessions.read().await;
        let s = sessions.get(&id)?;
        (s.action_state.clone(), s.timeouts.clone())
    };
    let Some(owner) = input.release_owner()? else {
        input.input_failed = false;
        save_input(&state, &id, &input).await?;
        return Ok(WebDriverResponse::null());
    };
    let original_executor = state.get_executor_for_window(&owner.window, timeouts, owner.frames)?;
    let executor=option_executor.unwrap_or(&*original_executor);
    for item in release_plan(&input,keys_only) {
        let mut next = input.clone();
        next.released(&item);
        match &item {
            crate::webdriver::session::NativeRelease::Key(key) => {
                if option { executor.dispatch_option_key_event(key, false, &next.modifiers()).await?; }
                else { executor.dispatch_key_event(key, false, &next.modifiers()).await?; }
            }
            crate::webdriver::session::NativeRelease::Button(_, button) => {
                if option && *button==0 {
                    executor.dispatch_option_pointer_event(PointerEventType::Up,owner.position.0,owner.position.1,*button,next.buttons(),&next.modifiers()).await?;
                } else {
                executor
                    .dispatch_pointer_event(
                        PointerEventType::Up,
                        owner.position.0,
                        owner.position.1,
                        *button,
                        next.buttons(),
                        &next.modifiers(),
                    )
                    .await?
                }
            }
        }
        input = next;
        save_input(&state, &id, &input).await?;
    }
    input.input_failed = false;
    save_input(&state, &id, &input).await?;
    Ok(WebDriverResponse::null())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refused_foreground_preflight_leaves_delete_actions_without_native_key_up() {
        let mut input = ActionState::default();
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let error = runtime.block_on(prepare_key_down(
            &mut input, "owned-window", &[], "keyboard", "\u{E00C}",
            async { Err(WebDriverErrorResponse::element_not_interactable("owned foreground not ready")) },
        )).unwrap_err();
        assert_eq!(error.message, "owned foreground not ready");
        assert!(!input.has_held_input());
        assert!(!input.input_failed);
        assert!(input.keyboard_source.is_none());
        assert!(input.release_owner().unwrap().is_none());
        assert!(release_plan(&input, false).is_empty(), "DELETE /actions has no key-up to dispatch");
    }
    #[test]
    fn unsupported_native_key_refuses_before_any_input_is_retained() {
        use crate::webdriver::session::ActionState;

        // U+E016 is a syntactically valid WebDriver key, but a backend may not
        // have a native representation for it. The same preflight traverses
        // the request before the first pointer or keyboard dispatch.
        let request: ActionsRequest = serde_json::from_value(serde_json::json!({
            "actions": [
                {"type": "pointer", "id": "mouse", "actions": [{"type": "pointerDown", "button": 0}]},
                {"type": "key", "id": "keys", "actions": [{"type": "keyDown", "value": "\u{E016}"}]}
            ]
        })).unwrap();
        validate(&request).unwrap();
        let input = ActionState::default();
        let mut inspected = Vec::new();
        let error = preflight_keyboard_actions(&request, |key| {
            inspected.push(key.to_owned());
            Err(WebDriverErrorResponse::unsupported_operation("unsupported native key"))
        }).unwrap_err();
        assert_eq!(inspected, ["\u{E016}"]);
        assert_eq!(error.message, "unsupported native key");
        assert!(!input.has_held_input());
        assert!(!input.input_failed);
        assert!(input.release_order.is_empty());
    }

    #[test]
    fn deferred_popup_cleanup_keeps_original_primary_after_keys_only() {
        use crate::webdriver::session::{ActionState,NativeRelease};
        let mut input=ActionState::default();
        input.retain_primary_down("element-click","original",&[],(10,20));
        input.retain_key("Home");input.retain_key("Escape");
        assert_eq!(release_plan(&input,false),vec![NativeRelease::Key("Escape".into()),NativeRelease::Key("Home".into()),NativeRelease::Button("element-click".into(),0)]);
        for item in release_plan(&input,true) {input.released(&item);}
        assert!(input.pressed_keys.is_empty());assert_eq!(input.buttons(),1);
        let owner=input.release_owner().unwrap().unwrap();assert_eq!(owner.window,"original");assert_eq!(owner.position,(10,20));
        let final_release=release_plan(&input,false);assert_eq!(final_release,vec![NativeRelease::Button("element-click".into(),0)]);
        // Simulated unknown delivery does not apply released() or erase authority.
        assert_eq!(input.buttons(),1);assert!(input.release_owner().unwrap().is_some());
        input.released(&final_release[0]);assert!(!input.has_held_input());assert!(input.release_owner().unwrap().is_none());
    }

    fn admitted(v: serde_json::Value) -> bool {
        validate(&serde_json::from_value(v).unwrap()).is_ok()
    }
    #[test]
    fn accepts_required_ticks_and_rejects_unknown_sources() {
        assert!(admitted(
            serde_json::json!({"actions":[{"type":"key","id":"keys","actions":[{"type":"keyDown","value":"\\uE009".replace("\\uE009","\u{E009}")}]},{"type":"pointer","id":"mouse","actions":[{"type":"pointerDown","button":2}]}]})
        ));
        assert!(admitted(
            serde_json::json!({"actions":[{"type":"pointer","id":"mouse","actions":[{"type":"pointerMove","x":40,"y":50,"duration":120},{"type":"pointerDown","button":0}]}]})
        ));
        assert!(!admitted(
            serde_json::json!({"actions":[{"type":"pointer","id":"mouse","parameters":{"pointerType":"pen"},"actions":[]}]})
        ));
        assert!(!admitted(
            serde_json::json!({"actions":[{"type":"pointer","id":"mouse","actions":[{"type":"pointerDown","button":3}]}]})
        ));
    }
    #[test]
    fn interpolation_exact_endpoint_without_overflow() {
        assert_eq!(interpolate(i32::MIN, i32::MAX, 10, 10), i32::MAX);
        assert_eq!(interpolate(20, -20, 1, 2), 0);
    }
    #[test]
    fn keys_and_durations_are_bounded() {
        assert!(key_supported("é"));
        assert!(!key_supported("ab"));
        assert!(!key_supported("\u{E050}"));
        assert!(!admitted(
            serde_json::json!({"actions":[{"type":"none","id":"pause","actions":[{"type":"pause","duration":10001}]}]})
        ));
    }
}
