use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use tauri::Runtime;

use crate::server::response::{WebDriverErrorResponse, WebDriverResponse, WebDriverResult};
use crate::server::AppState;
use crate::webdriver::locator::LocatorStrategy;

#[derive(Debug, Deserialize)]
pub struct FindElementRequest {
    pub using: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct SendKeysRequest {
    pub text: String,
}

/// POST `/session/{session_id}/element` - Find element
pub async fn find<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(session_id): Path<String>,
    Json(request): Json<FindElementRequest>,
) -> WebDriverResult {
    let mut sessions = state.sessions.write().await;
    let session = sessions.get_mut(&session_id)?;

    let strategy = LocatorStrategy::from_string(&request.using).ok_or_else(|| {
        WebDriverErrorResponse::invalid_argument(&format!(
            "Unknown locator strategy: {}",
            request.using
        ))
    })?;

    // Store element reference and get ID
    let element_ref = session.elements.store();
    let js_var = element_ref.js_ref.clone();
    let element_id = element_ref.id.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let strategy_js = strategy.to_selector_js(&request.value);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let found = executor.find_element(&strategy_js, &js_var).await?;
    if !found {
        return Err(WebDriverErrorResponse::no_such_element());
    }

    Ok(WebDriverResponse::success(json!({
        "element-6066-11e4-a52e-4f735466cecf": element_id
    })))
}

/// POST `/session/{session_id}/elements` - Find multiple elements
pub async fn find_all<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(session_id): Path<String>,
    Json(request): Json<FindElementRequest>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let strategy = LocatorStrategy::from_string(&request.using).ok_or_else(|| {
        WebDriverErrorResponse::invalid_argument(&format!(
            "Unknown locator strategy: {}",
            request.using
        ))
    })?;

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let strategy_js = strategy.to_selector_js_multiple(&request.value);

    // Use a temporary prefix for the trait method
    let temp_prefix = "__wd_temp_";
    let count = executor.find_elements(&strategy_js, temp_prefix).await?;

    // Now store each element with proper references
    let mut elements = Vec::new();
    let mut sessions = state.sessions.write().await;
    let session = sessions.get_mut(&session_id)?;

    for i in 0..count {
        let element_ref = session.elements.store();
        let js_var = element_ref.js_ref.clone();
        let element_id = element_ref.id.clone();

        // Copy from temp storage to element's js_ref
        let copy_script = format!(
            "(function() {{ window.{js_var} = window['{temp_prefix}{i}'];  return true; }})()"
        );
        let _ = executor.evaluate_js(&copy_script).await;

        elements.push(json!({
            "element-6066-11e4-a52e-4f735466cecf": element_id
        }));
    }

    Ok(WebDriverResponse::success(elements))
}

/// POST `/session/{session_id}/element/{element_id}/click` - Click element
pub async fn click<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let _input_guard = crate::platform::native_input_guard()?;
    click_inner(&state, &session_id, &element_id).await
}

async fn click_inner<R: Runtime + 'static>(
    state: &Arc<AppState<R>>,
    session_id: &str,
    element_id: &str,
) -> WebDriverResult {
    let result = click_sequence(state, session_id, element_id).await;
    if result.is_err() {
        let mut sessions = state.sessions.write().await;
        let input = &mut sessions.get_mut(session_id)?.action_state;
        if input.has_held_input() {
            input.input_failed = true;
        }
    }
    result
}

async fn click_sequence<R: Runtime + 'static>(
    state: &Arc<AppState<R>>,
    session_id: &str,
    element_id: &str,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(session_id)?;
    if session.action_state.input_failed
        || !session.action_state.pressed_keys.is_empty()
        || session
            .action_state
            .pressed_buttons
            .values()
            .any(|b| !b.is_empty())
    {
        return Err(WebDriverErrorResponse::unsupported_operation(
            "release prior held input before another click",
        ));
    }

    let element = session
        .elements
        .get(element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor =
        state.get_executor_for_window(&current_window, timeouts, frame_context.clone())?;
    use crate::platform::PointerEventType;
    let (x, y) = executor.get_element_center(&js_var).await?;
    executor
        .dispatch_pointer_event(
            PointerEventType::Move,
            x,
            y,
            0,
            0,
            &crate::platform::ModifierState::default(),
        )
        .await?;
    {
        let mut sessions = state.sessions.write().await;
        let state = &mut sessions.get_mut(session_id)?.action_state;
        state.pointer_position = (x, y);
        // Retain intent even if native down delivery returns uncertain.
        state.retain_primary_down("element-click", &current_window, &frame_context, (x, y));
    }
    executor
        .dispatch_pointer_event(
            PointerEventType::Down,
            x,
            y,
            0,
            1,
            &crate::platform::ModifierState::default(),
        )
        .await?;
    executor
        .dispatch_pointer_event(
            PointerEventType::Up,
            x,
            y,
            0,
            0,
            &crate::platform::ModifierState::default(),
        )
        .await?;
    state
        .sessions
        .write()
        .await
        .get_mut(session_id)?
        .action_state
        .primary_released("element-click");

    Ok(WebDriverResponse::null())
}

/// POST `/session/{session_id}/element/{element_id}/clear` - Clear element
pub async fn clear<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    edit_input(&state, &session_id, &element_id, None).await
}

/// POST element value: native keyboard input only, no DOM value setter.
pub async fn send_keys<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
    Json(request): Json<SendKeysRequest>,
) -> WebDriverResult {
    edit_input(&state, &session_id, &element_id, Some(&request.text)).await
}

async fn edit_input<R: Runtime + 'static>(
    state: &Arc<AppState<R>>,
    session_id: &str,
    element_id: &str,
    text: Option<&str>,
) -> WebDriverResult {
    let _guard = crate::platform::native_input_guard()?;
    if text.is_some_and(|t| t.chars().count() > 4096) {
        return Err(WebDriverErrorResponse::invalid_argument(
            "text input exceeds 4096 characters",
        ));
    }
    let (window, timeouts, frames, js) = {
        let sessions = state.sessions.read().await;
        let session = sessions.get(session_id)?;
        (
            session.current_window.clone(),
            session.timeouts.clone(),
            session.frame_context.clone(),
            session
                .elements
                .get(element_id)
                .ok_or_else(WebDriverErrorResponse::no_such_element)?
                .js_ref
                .clone(),
        )
    };
    let executor = state.get_executor_for_window(&window, timeouts, frames)?;
    let editable=executor.evaluate_js(&format!("(function(){{var e=window.{js};return !!(e&&e.isConnected&&!e.disabled&&!e.readOnly&&(e.isContentEditable||e.tagName==='TEXTAREA'||(e.tagName==='INPUT'&&!['file','checkbox','radio','button','submit','reset','range','color','hidden'].includes(e.type))));}})()")).await?;
    if editable["value"].as_bool() != Some(true) {
        return Err(WebDriverErrorResponse::element_not_interactable(
            "element is not an editable native text target",
        ));
    }
    let mut actions = Vec::new();
    let mut tap = |key: &str| {
        actions.push(serde_json::json!({"type":"keyDown","value":key}));
        actions.push(serde_json::json!({"type":"keyUp","value":key}));
    };
    if let Some(text) = text {
        for ch in text.chars() {
            let key = match ch {
                '\n' | '\r' => "\u{E007}".into(),
                '\t' => "\u{E004}".into(),
                _ => ch.to_string(),
            };
            tap(&key);
        }
    } else {
        let select = if cfg!(target_os = "macos") {
            "\u{E03D}"
        } else {
            "\u{E009}"
        };
        actions.push(serde_json::json!({"type":"keyDown","value":select}));
        actions.push(serde_json::json!({"type":"keyDown","value":"a"}));
        actions.push(serde_json::json!({"type":"keyUp","value":"a"}));
        actions.push(serde_json::json!({"type":"keyUp","value":select}));
        actions.push(serde_json::json!({"type":"keyDown","value":"\u{E003}"}));
        actions.push(serde_json::json!({"type":"keyUp","value":"\u{E003}"}));
        // Native focus traversal commits change; no synthetic blur/change event.
        actions.push(serde_json::json!({"type":"keyDown","value":"\u{E004}"}));
        actions.push(serde_json::json!({"type":"keyUp","value":"\u{E004}"}));
    }
    let request: super::actions::ActionsRequest = serde_json::from_value(
        serde_json::json!({"actions":[{"type":"key","id":"element-text","actions":actions}]}),
    )
    .map_err(|_| WebDriverErrorResponse::invalid_argument("invalid native text sequence"))?;
    super::actions::validate(&request)?;
    click_inner(state, session_id, element_id).await?;
    super::actions::perform_inner(state, session_id, request).await?;
    if text.is_none() {
        click_inner(state, session_id, element_id).await?;
    }
    Ok(WebDriverResponse::null())
}

/// GET `/session/{session_id}/element/{element_id}/text` - Get element text
pub async fn get_text<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let text = executor.get_element_text(&js_var).await?;
    Ok(WebDriverResponse::success(text))
}

/// GET `/session/{session_id}/element/{element_id}/name` - Get element tag name
pub async fn get_tag_name<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let tag_name = executor.get_element_tag_name(&js_var).await?;
    Ok(WebDriverResponse::success(tag_name))
}

/// GET `/session/{session_id}/element/{element_id}/attribute/{name}` - Get element attribute
pub async fn get_attribute<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id, name)): Path<(String, String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let attr = executor.get_element_attribute(&js_var, &name).await?;
    Ok(WebDriverResponse::success(attr))
}

/// GET `/session/{session_id}/element/{element_id}/property/{name}` - Get element property
pub async fn get_property<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id, name)): Path<(String, String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let prop = executor.get_element_property(&js_var, &name).await?;
    Ok(WebDriverResponse::success(prop))
}

/// GET `/session/{session_id}/element/{element_id}/displayed` - Is element displayed
pub async fn is_displayed<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let displayed = executor.is_element_displayed(&js_var).await?;
    Ok(WebDriverResponse::success(displayed))
}

/// GET `/session/{session_id}/element/{element_id}/enabled` - Is element enabled
pub async fn is_enabled<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let enabled = executor.is_element_enabled(&js_var).await?;
    Ok(WebDriverResponse::success(enabled))
}

/// GET `/session/{session_id}/element/active` - Get active element
pub async fn get_active<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(session_id): Path<String>,
) -> WebDriverResult {
    let mut sessions = state.sessions.write().await;
    let session = sessions.get_mut(&session_id)?;

    // Store element reference for the active element
    let element_ref = session.elements.store();
    let js_var = element_ref.js_ref.clone();
    let element_id = element_ref.id.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let found = executor.get_active_element(&js_var).await?;
    if !found {
        return Err(WebDriverErrorResponse::no_such_element());
    }

    Ok(WebDriverResponse::success(json!({
        "element-6066-11e4-a52e-4f735466cecf": element_id
    })))
}

/// POST `/session/{session_id}/element/{element_id}/element` - Find element from element
pub async fn find_from_element<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, parent_element_id)): Path<(String, String)>,
    Json(request): Json<FindElementRequest>,
) -> WebDriverResult {
    let mut sessions = state.sessions.write().await;
    let session = sessions.get_mut(&session_id)?;

    let parent_element = session
        .elements
        .get(&parent_element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;
    let parent_js_var = parent_element.js_ref.clone();

    let strategy = LocatorStrategy::from_string(&request.using).ok_or_else(|| {
        WebDriverErrorResponse::invalid_argument(&format!(
            "Unknown locator strategy: {}",
            request.using
        ))
    })?;

    // Store element reference and get ID
    let element_ref = session.elements.store();
    let js_var = element_ref.js_ref.clone();
    let element_id = element_ref.id.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    // Use the locator method that generates expressions expecting `parent` to be defined
    let strategy_js = strategy.to_selector_js_single_from_element(&request.value);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let found = executor
        .find_element_from_element(&parent_js_var, &strategy_js, &js_var)
        .await?;
    if !found {
        return Err(WebDriverErrorResponse::no_such_element());
    }

    Ok(WebDriverResponse::success(json!({
        "element-6066-11e4-a52e-4f735466cecf": element_id
    })))
}

/// POST `/session/{session_id}/element/{element_id}/elements` - Find elements from element
pub async fn find_all_from_element<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, parent_element_id)): Path<(String, String)>,
    Json(request): Json<FindElementRequest>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let parent_element = session
        .elements
        .get(&parent_element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;
    let parent_js_var = parent_element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let strategy = LocatorStrategy::from_string(&request.using).ok_or_else(|| {
        WebDriverErrorResponse::invalid_argument(&format!(
            "Unknown locator strategy: {}",
            request.using
        ))
    })?;

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let strategy_js = strategy.to_selector_js_from_element(&request.value);

    // Use a temporary prefix for the trait method
    let temp_prefix = "__wd_temp_";
    let count = executor
        .find_elements_from_element(&parent_js_var, &strategy_js, temp_prefix)
        .await?;

    // Now store each element with proper references
    let mut elements = Vec::new();
    let mut sessions = state.sessions.write().await;
    let session = sessions.get_mut(&session_id)?;

    for i in 0..count {
        let element_ref = session.elements.store();
        let js_var = element_ref.js_ref.clone();
        let element_id = element_ref.id.clone();

        // Copy from temp storage to element's js_ref
        let copy_script = format!(
            "(function() {{ window.{js_var} = window['{temp_prefix}{i}'];  return true; }})()"
        );
        let _ = executor.evaluate_js(&copy_script).await;

        elements.push(json!({
            "element-6066-11e4-a52e-4f735466cecf": element_id
        }));
    }

    Ok(WebDriverResponse::success(elements))
}

/// GET `/session/{session_id}/element/{element_id}/selected` - Is element selected
pub async fn is_selected<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let selected = executor.is_element_selected(&js_var).await?;
    Ok(WebDriverResponse::success(selected))
}

/// GET `/session/{session_id}/element/{element_id}/css/{property_name}` - Get CSS value
pub async fn get_css_value<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id, property_name)): Path<(String, String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let value = executor
        .get_element_css_value(&js_var, &property_name)
        .await?;
    Ok(WebDriverResponse::success(value))
}

/// GET `/session/{session_id}/element/{element_id}/rect` - Get element rect
pub async fn get_rect<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let rect = executor.get_element_rect(&js_var).await?;
    Ok(WebDriverResponse::success(json!({
        "x": rect.x,
        "y": rect.y,
        "width": rect.width,
        "height": rect.height
    })))
}

/// GET `/session/{session_id}/element/{element_id}/computedrole` - Get computed ARIA role
pub async fn get_computed_role<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let role = executor.get_element_computed_role(&js_var).await?;
    Ok(WebDriverResponse::success(role))
}

/// GET `/session/{session_id}/element/{element_id}/computedlabel` - Get computed accessible name
pub async fn get_computed_label<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let label = executor.get_element_computed_label(&js_var).await?;
    Ok(WebDriverResponse::success(label))
}

/// GET `/session/{session_id}/element/{element_id}/screenshot` - Take element screenshot
pub async fn take_screenshot<R: Runtime + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path((session_id, element_id)): Path<(String, String)>,
) -> WebDriverResult {
    let sessions = state.sessions.read().await;
    let session = sessions.get(&session_id)?;

    let element = session
        .elements
        .get(&element_id)
        .ok_or_else(WebDriverErrorResponse::no_such_element)?;

    let js_var = element.js_ref.clone();
    let current_window = session.current_window.clone();
    let timeouts = session.timeouts.clone();
    let frame_context = session.frame_context.clone();
    drop(sessions);

    let executor = state.get_executor_for_window(&current_window, timeouts, frame_context)?;
    let screenshot = executor.take_element_screenshot(&js_var).await?;
    Ok(WebDriverResponse::success(screenshot))
}
