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
        )
        .with_data(session.action_state.input_state_diagnostic()));
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
    let option = executor.inspect_option(&js_var, "prepare").await?;
    let center_ref = if option.is_some() { format!("{js_var}_select") } else { js_var.clone() };
    let mut popup_started = false;
    let defer_release=option.is_some() && executor.option_defer_primary_release();
    let mut release_attempted=false;
    let mut operation = async {
    let first_key = if option.is_some() { Some(executor.option_popup_first_key()?) } else { None };
    let (x, y) = executor.get_element_center(&center_ref).await?;
    if let Some((_, selected)) = option {
        executor.inspect_option(&js_var, "verify").await?;
        option_context(state, session_id, &current_window).await?;
        if selected { return Ok(WebDriverResponse::null()); }
    }
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
    if option.is_some() {
        option_context(state, session_id, &current_window).await?;
        executor.inspect_option(&js_var, "verify").await?;
        if executor.get_element_center(&center_ref).await? != (x, y) {
            return Err(WebDriverErrorResponse::element_not_interactable("select container moved before native click"));
        }
    }
    {
        let mut sessions = state.sessions.write().await;
        let state = &mut sessions.get_mut(session_id)?.action_state;
        state.pointer_position = (x, y);
        // Retain intent even if native down delivery returns uncertain.
        state.retain_primary_down("element-click", &current_window, &frame_context, (x, y));
    }
    popup_started = option.is_some();
    for (event_type, buttons) in [(PointerEventType::Down, 1), (PointerEventType::Up, 0)] {
        if defer_release && matches!(event_type,PointerEventType::Up) {continue;}
        if option.is_some() {
            executor.dispatch_option_pointer_event(event_type, x, y, 0, buttons,
                &crate::platform::ModifierState::default()).await?;
        } else {
            executor.dispatch_pointer_event(event_type, x, y, 0, buttons,
                &crate::platform::ModifierState::default()).await?;
        }
    }
    if !defer_release {
    state
        .sessions
        .write()
        .await
        .get_mut(session_id)?
        .action_state
        .primary_released("element-click");
    }

    if let Some((index, _)) = option {
        executor.inspect_option(&js_var, "verify").await?;
        for key in option_navigation(first_key.unwrap(), index)? {
            option_context(state, session_id, &current_window).await?;
            executor.inspect_option(&js_var, "verify").await?;
            super::actions::perform_option_inner(state, session_id, option_key_pair(key)?).await?;
        }
        option_context(state, session_id, &current_window).await?;
        executor.inspect_option(&js_var, "verify").await?;
        let owner=tokio::time::timeout(std::time::Duration::from_millis(executor.script_timeout_ms().min(2000)),executor.option_completion_owner()).await
            .map_err(|_|WebDriverErrorResponse::unknown_error("option owner observation timed out"))??;
        super::actions::perform_option_inner(state, session_id, option_key_pair("\u{E007}")?).await?;
        // One Enter only. Observe the exact retained target until WebKit commits;
        // every asynchronous read shares this single bounded deadline.
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_millis(executor.script_timeout_ms().min(2000));
        let mut last_mismatch=None;
        loop {
            if tokio::time::Instant::now()>=deadline {
                return Err(last_mismatch.unwrap_or_else(||WebDriverErrorResponse::unknown_error("option completion deadline expired before observation")));
            }
            let observation=async {
                option_context(state,session_id,&current_window).await?;
                if executor.option_completion_owner().await?!=owner {return Err(WebDriverErrorResponse::unknown_error("option completion owner changed"));}
                let mut result=executor.inspect_option_completion(&js_var).await?;
                if result.is_none() {
                    result=executor.option_completion_readiness().await?;
                    // Native readiness must not hide a concurrent DOM replacement.
                    if let Some(mismatch)=executor.inspect_option_completion(&js_var).await? {result=Some(mismatch);}
                }
                option_context(state,session_id,&current_window).await?;
                if executor.option_completion_owner().await?!=owner {return Err(WebDriverErrorResponse::unknown_error("option completion owner changed"));}
                Ok::<_,WebDriverErrorResponse>(result)
            };
            match tokio::time::timeout_at(deadline,observation).await {
                Ok(Ok(None))=>break,
                Ok(Ok(Some(mismatch)))=>last_mismatch=Some(mismatch),
                Ok(Err(error))=>return Err(error),
                Err(_)=>return Err(WebDriverErrorResponse::unknown_error(&format!("option completion observation timed out; latest intact mismatch: {}",last_mismatch.as_ref().map(|e|e.message.as_str()).unwrap_or("none")))),
            }
            tokio::task::yield_now().await;
        }
        if defer_release {
            release_attempted=true;
            super::actions::release_option_inner(state,session_id,&*executor).await?;
        }
    }
    Ok(WebDriverResponse::null())
    }.await;
    if option.is_some() {
        if operation.is_err() && popup_started {
            // Never repeat a release whose posting/acknowledgement is uncertain.
            let cleanup = async {
                if defer_release {
                    if release_attempted {return Ok(WebDriverResponse::null());}
                    super::actions::release_option_keys_inner(state,session_id,&*executor).await?;
                    option_context(state,session_id,&current_window).await?;
                    super::actions::perform_option_inner(state,session_id,option_key_pair("\u{E00C}")?).await?;
                    wait_option_release_ready(state,session_id,&current_window,&*executor).await?;
                    release_attempted=true;
                    super::actions::release_option_inner(state,session_id,&*executor).await
                } else {
                    super::actions::release_option_inner(state, session_id, &*executor).await?;
                    option_context(state, session_id, &current_window).await?;
                    super::actions::perform_option_inner(state, session_id, option_key_pair("\u{E00C}")?).await
                }
            }.await;
            state.sessions.write().await.get_mut(session_id)?.action_state.input_failed = true;
            if let Err(error) = cleanup { operation = Err(error); }
        }
        // Always release adapter references, even when native cleanup fails.
        // Preserve the operation/cleanup error and its retained held-input state.
        executor.clear_option_completion();
        let cleared = executor.inspect_option(&js_var, "clear").await;
        if operation.is_ok() { cleared?; }
    }
    operation
}


fn option_navigation(first: &'static str, index: usize) -> Result<Vec<&'static str>, WebDriverErrorResponse> {
    if index >= 128 { return Err(WebDriverErrorResponse::unsupported_operation("option count exceeds native navigation bound")); }
    let mut keys = vec![first];
    keys.extend(std::iter::repeat("\u{E015}").take(index));
    Ok(keys)
}

fn option_key_pair(key: &str) -> Result<super::actions::ActionsRequest, WebDriverErrorResponse> {
    serde_json::from_value(json!({"actions":[{"type":"key","id":"element-option","actions":[
        {"type":"keyDown","value":key},{"type":"keyUp","value":key}]}]}))
        .map_err(|_| WebDriverErrorResponse::invalid_argument("invalid native option key pair"))
}

// Cancelled/stale DOM does not erase the original native release obligation.
async fn wait_option_release_ready<R:Runtime+'static>(state:&Arc<AppState<R>>,id:&str,window:&str,executor:&dyn crate::platform::PlatformExecutor<R>)->Result<(),WebDriverErrorResponse> {
    let deadline=tokio::time::Instant::now()+std::time::Duration::from_millis(executor.script_timeout_ms().min(2000));
    let mut last=None;
    loop {
        if tokio::time::Instant::now()>=deadline {return Err(last.unwrap_or_else(||WebDriverErrorResponse::unknown_error("option release readiness deadline expired")));}
        let observation=async {
            option_context(state,id,window).await?;
            let owner=executor.option_completion_owner().await?;
            let result=executor.option_completion_readiness().await?;
            if executor.option_completion_owner().await?!=owner {return Err(WebDriverErrorResponse::unknown_error("option release owner changed"));}
            option_context(state,id,window).await?;
            Ok::<_,WebDriverErrorResponse>(result)
        };
        match tokio::time::timeout_at(deadline,observation).await {
            Ok(Ok(None))=>return Ok(()),Ok(Ok(Some(error)))=>last=Some(error),Ok(Err(error))=>return Err(error),
            Err(_)=>return Err(WebDriverErrorResponse::unknown_error("option release readiness observation timed out")),
        }
        tokio::task::yield_now().await;
    }
}

async fn option_context<R: Runtime + 'static>(state: &Arc<AppState<R>>, session_id: &str, window: &str) -> Result<(), WebDriverErrorResponse> {
    let sessions = state.sessions.read().await;
    let session = sessions.get(session_id)?;
    if session.current_window != window || !session.frame_context.is_empty() {
        return Err(WebDriverErrorResponse::unsupported_operation("option click context changed before native delivery"));
    }
    Ok(())
}

#[cfg(test)]
mod option_tests {
    use super::*;
    #[test]
    fn popup_navigation_is_bounded_and_commit_is_separate() {
        assert_eq!(option_navigation("\u{E011}", 0).unwrap(), vec!["\u{E011}"]);
        assert_eq!(option_navigation("\u{E011}", 2).unwrap(), vec!["\u{E011}", "\u{E015}", "\u{E015}"]);
        assert_eq!(option_navigation("\u{E011}", 127).unwrap().len(), 128);
        assert!(option_navigation("\u{E011}", 128).is_err());
        for key in ["\u{E011}", "\u{E015}", "\u{E007}", "\u{E00C}"] {
            super::super::actions::validate(&option_key_pair(key).unwrap()).unwrap();
        }
    }
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
