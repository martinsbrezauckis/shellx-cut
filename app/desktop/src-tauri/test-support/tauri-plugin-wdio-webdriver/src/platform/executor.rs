use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::webview::Cookie as TauriCookie;
use tauri::{Runtime, WebviewWindow};

#[cfg(desktop)]
use tauri::{PhysicalPosition, PhysicalSize};

use tauri::Manager;

use crate::platform::alert_state::{AlertStateManager, AlertType};
use crate::server::response::WebDriverErrorResponse;

const OPTION_SNAPSHOT_SCRIPT: &str = r#"// Read-only DOM inspection. Retained references are adapter bookkeeping only.
(function (reference, phase) {
    const key = reference + '_optionSnapshot';
    const container = reference + '_select';
    if (phase === 'clear') {
        delete window[key];
        delete window[container];
        return null;
    }
    const target = window[reference];
    if (!target || !target.isConnected) return {error: 'stale element reference'};
    if (!(target instanceof HTMLOptionElement)) {
        return phase === 'prepare' ? null : {error: 'stale element reference'};
    }
    const direct = target.parentElement;
    const select = direct instanceof HTMLOptGroupElement ? direct.parentElement : direct;
    if (!(select instanceof HTMLSelectElement) || !select.isConnected) return {error: 'stale element reference'};
    // The application may disable this SELECT or its fieldset while saving the
    // trusted change. Completion observes the committed choice, not new input.
    // Every check before Enter still requires an interactable container.
    if (phase !== 'settle' && select.matches(':disabled')) {
        return {error: 'element not interactable'};
    }
    const hidden = e => {
        const style = getComputedStyle(e);
        return e.hidden || style.display === 'none' || style.visibility !== 'visible';
    };
    const options = Array.from(select.options);
    const index = options.indexOf(target);
    // Navigate from the first enabled entry after Home. The native fixture must
    // qualify this popup behavior on each backend.
    // A disabled first option is a native popup placeholder regardless of
    // whether the app uses an empty value or a private sentinel value.
    const placeholder = options[0] instanceof HTMLOptionElement &&
        options[0].parentElement === select && options[0].disabled;
    const navigationSteps = index - (placeholder ? 1 : 0);
    const validChildren = Array.from(select.children).every(e =>
        e instanceof HTMLOptionElement || (e instanceof HTMLOptGroupElement &&
            Array.from(e.children).every(o => o instanceof HTMLOptionElement)));
    if (select.multiple || select.size > 1 || options.length < 1 || options.length > 128 || index < 0 ||
        !validChildren || hidden(select) || options.some((o, i) => (o.disabled && !(i === 0 && placeholder)) || hidden(o) ||
            (o.parentElement instanceof HTMLOptGroupElement && (o.parentElement.disabled || hidden(o.parentElement))))) {
        return {error: 'unsupported operation'};
    }
    if (navigationSteps < 0) return {error: 'element not interactable'};
    if (phase === 'prepare') {
        window[key] = {target, select, direct, options, index, value: target.value,
            navigationSteps, disabled: options.map(o => o.disabled),
            values: options.map(o => o.value), labels: options.map(o => o.label),
            parents: options.map(o => o.parentElement), selected: select.selectedIndex};
        window[container] = select;
    }
    const saved = window[key];
    if (!saved || saved.target !== target || saved.select !== select || saved.direct !== direct ||
        saved.index !== index || saved.navigationSteps !== navigationSteps ||
        saved.options.length !== options.length || saved.value !== target.value ||
        options.some((o, i) => o !== saved.options[i] || o.parentElement !== saved.parents[i] ||
            o.value !== saved.values[i] || o.label !== saved.labels[i] ||
            o.disabled !== saved.disabled[i])) return {error: 'stale element reference'};
    if (phase === 'complete' || phase === 'settle') {
        if (!target.selected || select.selectedIndex !== index || select.value !== saved.value) {
            return {error: 'unknown error', pending: phase === 'settle', message: 'native option selection did not reach the exact retained target; observation=' + JSON.stringify({
                phase, retainedIndex: saved.index, initialSelectedIndex: saved.selected,
                selectedIndex: select.selectedIndex, targetSelected: target.selected,
                selectedFlags: options.map(o => o.selected), valueMatchesRetained: select.value === saved.value
            })};
        }
    } else if (select.selectedIndex !== saved.selected) {
        return {error: 'stale element reference', message: 'option selection changed before native commit'};
    }
    return {index, navigationSteps, selected: target.selected};
})
"#;

// Native input is process-global. Refuse overlapping input requests instead of
// introducing another queue or interleaving two balanced button sequences.
static NATIVE_INPUT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

tokio::task_local! {
    pub static NATIVE_CLICK_TRACE_ID: u64;
}

pub fn native_input_guard() -> Result<tokio::sync::MutexGuard<'static, ()>, WebDriverErrorResponse>
{
    NATIVE_INPUT.try_lock().map_err(|_| {
        WebDriverErrorResponse::unsupported_operation("another native input request is in progress")
    })
}

/// Element bounding rectangle
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ElementRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Window rectangle (position and size)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WindowRect {
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Frame identifier for switching frames
#[derive(Debug, Clone)]
pub enum FrameId {
    /// Frame by index
    Index(u32),
    /// Frame by element reference (`js_var`)
    Element(String),
}

/// Pointer event type
#[derive(Debug, Clone, Copy)]
pub enum PointerEventType {
    Down,
    Up,
    Move,
    Click,
}

/// Cookie data
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default, rename = "httpOnly")]
    pub http_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "sameSite")]
    pub same_site: Option<String>,
}

/// Print options for PDF generation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrintOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "pageWidth")]
    pub page_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "pageHeight")]
    pub page_height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "marginTop")]
    pub margin_top: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "marginBottom")]
    pub margin_bottom: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "marginLeft")]
    pub margin_left: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "marginRight")]
    pub margin_right: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "shrinkToFit")]
    pub shrink_to_fit: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "pageRanges")]
    pub page_ranges: Option<Vec<String>>,
}

/// Tracks the state of modifier keys during action sequences
#[derive(Debug, Clone, Copy, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct ModifierState {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

impl ModifierState {
    /// Update modifier state when a key is pressed or released
    pub fn update(&mut self, key: &str, is_down: bool) {
        match key {
            "\u{E009}" => self.ctrl = is_down,  // Control
            "\u{E008}" => self.shift = is_down, // Shift
            "\u{E00A}" => self.alt = is_down,   // Alt
            "\u{E03D}" => self.meta = is_down,  // Meta
            _ => {}
        }
    }
}

/// Platform-agnostic trait for `WebView` operations.
/// Each platform (macOS, Windows, Linux) implements this trait.
#[async_trait]
#[allow(clippy::too_many_lines)]
pub trait PlatformExecutor<R: Runtime>: Send + Sync {
    // =========================================================================
    // Window Access
    // =========================================================================

    /// Get a reference to the underlying window
    fn window(&self) -> &WebviewWindow<R>;

    /// Get the script timeout in milliseconds
    fn script_timeout_ms(&self) -> u64;

    // =========================================================================
    // Core JavaScript Execution
    // =========================================================================

    /// Execute JavaScript and return the result as JSON
    async fn evaluate_js(&self, script: &str) -> Result<Value, WebDriverErrorResponse>;

    // =========================================================================
    // Navigation
    // =========================================================================

    /// Navigate to a URL
    async fn navigate(&self, url: &str) -> Result<(), WebDriverErrorResponse> {
        let script = format!(
            r"window.location.href = '{}'; null;",
            url.replace('\\', "\\\\").replace('\'', "\\'")
        );
        self.evaluate_js(&script).await?;
        Ok(())
    }

    /// Get current URL
    async fn get_url(&self) -> Result<String, WebDriverErrorResponse> {
        let result = self.evaluate_js("window.location.href").await?;
        extract_string_value(&result)
    }

    /// Get page title
    async fn get_title(&self) -> Result<String, WebDriverErrorResponse> {
        let result = self.evaluate_js("document.title").await?;
        extract_string_value(&result)
    }

    /// Navigate back in history
    async fn go_back(&self) -> Result<(), WebDriverErrorResponse> {
        self.evaluate_js("window.history.back(); null;").await?;
        Ok(())
    }

    /// Navigate forward in history
    async fn go_forward(&self) -> Result<(), WebDriverErrorResponse> {
        self.evaluate_js("window.history.forward(); null;").await?;
        Ok(())
    }

    /// Refresh the current page
    async fn refresh(&self) -> Result<(), WebDriverErrorResponse> {
        self.evaluate_js("window.location.reload(); null;").await?;
        Ok(())
    }

    // =========================================================================
    // Document
    // =========================================================================

    /// Get page source HTML
    async fn get_source(&self) -> Result<String, WebDriverErrorResponse> {
        let result = self
            .evaluate_js("document.documentElement.outerHTML")
            .await?;
        extract_string_value(&result)
    }

    // =========================================================================
    // Element Operations
    // =========================================================================

    /// Find element and store reference in a JavaScript variable
    /// Returns true if element was found
    async fn find_element(
        &self,
        strategy_js: &str,
        js_var: &str,
    ) -> Result<bool, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = {strategy_js};
                if (el) {{
                    window.{js_var} = el;
                    return true;
                }}
                return false;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_bool_value(&result)
    }

    /// Find multiple elements and store count
    /// Returns the number of elements found
    async fn find_elements(
        &self,
        strategy_js: &str,
        js_var_prefix: &str,
    ) -> Result<usize, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var elements = {strategy_js};
                var count = elements.length;
                for (var i = 0; i < count; i++) {{
                    window['{js_var_prefix}' + i] = elements[i];
                }}
                return count;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_usize_value(&result)
    }

    /// Find element from a parent element and store reference
    /// Returns true if element was found
    async fn find_element_from_element(
        &self,
        parent_js_var: &str,
        strategy_js: &str,
        js_var: &str,
    ) -> Result<bool, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var parent = window.{parent_js_var};
                if (!parent || !parent.isConnected) {{
                    throw new Error('stale element reference');
                }}
                var el = {strategy_js};
                if (el) {{
                    window.{js_var} = el;
                    return true;
                }}
                return false;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_bool_value(&result)
    }

    /// Find multiple elements from a parent element
    /// Returns count of elements found, stores as {prefix}0, {prefix}1, etc.
    async fn find_elements_from_element(
        &self,
        parent_js_var: &str,
        strategy_js: &str,
        js_var_prefix: &str,
    ) -> Result<usize, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var parent = window.{parent_js_var};
                if (!parent || !parent.isConnected) {{
                    throw new Error('stale element reference');
                }}
                var elements = {strategy_js};
                var count = elements.length;
                for (var i = 0; i < count; i++) {{
                    window['{js_var_prefix}' + i] = elements[i];
                }}
                return count;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_usize_value(&result)
    }

    /// Get element text content
    async fn get_element_text(&self, js_var: &str) -> Result<String, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                return el.textContent || '';
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_string_value(&result)
    }

    /// Get element tag name (lowercase)
    async fn get_element_tag_name(&self, js_var: &str) -> Result<String, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                return el.tagName.toLowerCase();
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_string_value(&result)
    }

    /// Get element attribute value
    /// Per W3C `WebDriver` spec, certain attributes should return current property values:
    /// - "value" on input/textarea returns current value property
    /// - "checked" on checkbox/radio returns current checked state
    /// - "selected" on option returns current selected state
    async fn get_element_attribute(
        &self,
        js_var: &str,
        name: &str,
    ) -> Result<Option<String>, WebDriverErrorResponse> {
        let escaped_name = name.replace('\\', "\\\\").replace('\'', "\\'");
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                var attrName = '{escaped_name}'.toLowerCase();
                var tagName = el.tagName.toLowerCase();

                // Per W3C WebDriver spec, return property values for certain attributes
                if (attrName === 'value') {{
                    if (tagName === 'input' || tagName === 'textarea') {{
                        return el.value;
                    }}
                }}
                if (attrName === 'checked') {{
                    if (tagName === 'input' && (el.type === 'checkbox' || el.type === 'radio')) {{
                        return el.checked ? 'true' : null;
                    }}
                }}
                if (attrName === 'selected') {{
                    if (tagName === 'option') {{
                        return el.selected ? 'true' : null;
                    }}
                }}

                return el.getAttribute('{escaped_name}');
            }})()"
        );
        let result = self.evaluate_js(&script).await?;

        if let Some(value) = result.get("value") {
            if value.is_null() {
                return Ok(None);
            }
            if let Some(s) = value.as_str() {
                return Ok(Some(s.to_string()));
            }
        }
        Ok(None)
    }

    /// Get element property value
    async fn get_element_property(
        &self,
        js_var: &str,
        name: &str,
    ) -> Result<Value, WebDriverErrorResponse> {
        let escaped_name = name.replace('\\', "\\\\").replace('\'', "\\'");
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                return el['{escaped_name}'];
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_value(&result)
    }

    /// Get element CSS property value
    async fn get_element_css_value(
        &self,
        js_var: &str,
        property: &str,
    ) -> Result<String, WebDriverErrorResponse> {
        let escaped_prop = property.replace('\\', "\\\\").replace('\'', "\\'");
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                return window.getComputedStyle(el).getPropertyValue('{escaped_prop}');
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_string_value(&result)
    }

    /// Get element bounding rectangle
    async fn get_element_rect(&self, js_var: &str) -> Result<ElementRect, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                var rect = el.getBoundingClientRect();
                return {{
                    x: rect.x + window.scrollX,
                    y: rect.y + window.scrollY,
                    width: rect.width,
                    height: rect.height
                }};
            }})()"
        );
        let result = self.evaluate_js(&script).await?;

        if let Some(value) = result.get("value") {
            return Ok(ElementRect {
                x: value.get("x").and_then(Value::as_f64).unwrap_or(0.0),
                y: value.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                width: value.get("width").and_then(Value::as_f64).unwrap_or(0.0),
                height: value.get("height").and_then(Value::as_f64).unwrap_or(0.0),
            });
        }
        Ok(ElementRect::default())
    }

    /// Get an element's in-view center point in **client (viewport)** coordinates,
    /// scrolling it into view first. Unlike [`Executor::get_element_rect`], this
    /// does not add scroll offsets: pointer events dispatch against viewport
    /// coordinates (`clientX`/`clientY`), so the center must be viewport-relative.
    async fn get_element_center(&self, js_var: &str) -> Result<(i32, i32), WebDriverErrorResponse> {
        self.get_element_center_checked(js_var, true).await
    }

    /// Recheck a previously admitted click point without changing scroll state.
    async fn get_element_center_without_scroll(
        &self,
        js_var: &str,
    ) -> Result<(i32, i32), WebDriverErrorResponse> {
        self.get_element_center_checked(js_var, false).await
    }

    async fn get_element_center_checked(
        &self,
        js_var: &str,
        scroll: bool,
    ) -> Result<(i32, i32), WebDriverErrorResponse> {
        let scroll_script = if scroll {
            "el.scrollIntoView({ behavior: 'instant', block: 'center', inline: 'center' });"
        } else {
            ""
        };
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    return {{ error: 'stale element reference' }};
                }}
                {scroll_script}
                var r = el.getBoundingClientRect();
                var left = Math.max(0, r.left), right = Math.min(innerWidth, r.right);
                var top = Math.max(0, r.top), bottom = Math.min(innerHeight, r.bottom);
                if (right <= left || bottom <= top) return {{ error: 'element not interactable', reason: 'no in-view rectangle' }};
                if (el.disabled) return {{ error: 'element not interactable', reason: 'disabled at click preflight' }};
                var x = Math.floor((left + right) / 2), y = Math.floor((top + bottom) / 2);
                var hit = document.elementFromPoint(x, y);
                if (!hit || (hit !== el && !el.contains(hit))) return {{ error: 'element click intercepted' }};
                return {{ x: x, y: y }};
            }})()"
        );
        let result = self.evaluate_js(&script).await?;

        let value = result.get("value").cloned().ok_or_else(|| {
            WebDriverErrorResponse::unknown_error("element center script returned no value")
        })?;

        if let Some(error) = value.get("error").and_then(Value::as_str) {
            if error == "stale element reference" {
                return Err(WebDriverErrorResponse::stale_element_reference());
            }
            let detail = value.get("reason").and_then(Value::as_str).unwrap_or(error);
            return Err(WebDriverErrorResponse::new(
                axum::http::StatusCode::BAD_REQUEST,
                error,
                detail,
                None,
            ));
        }
        #[derive(serde::Deserialize)]
        struct Center {
            x: i32,
            y: i32,
        }
        let center: Center = serde_json::from_value(value).map_err(|err| {
            WebDriverErrorResponse::unknown_error(&format!("could not read element center: {err}"))
        })?;
        Ok((center.x, center.y))
    }

    fn option_defer_primary_release(&self) -> bool { false }

    /// Additional native readiness at the original gesture point; no input replay.
    async fn option_completion_readiness(&self) -> Result<Option<WebDriverErrorResponse>,WebDriverErrorResponse> { Ok(None) }
    fn clear_option_completion(&self) {}

    /// Read-only owner identity for completion; never activates or changes focus.
    async fn option_completion_owner(&self) -> Result<String, WebDriverErrorResponse> {
        Ok(self.window().label().to_owned())
    }

    /// None means exact completion; Some retains the latest intact mismatch.
    async fn inspect_option_completion(&self, js_var: &str) -> Result<Option<WebDriverErrorResponse>, WebDriverErrorResponse> {
        let script=format!("({})({}, \"settle\")",OPTION_SNAPSHOT_SCRIPT,serde_json::to_string(js_var).unwrap());
        let result=self.evaluate_js(&script).await?;
        let value=result.get("value").ok_or_else(||WebDriverErrorResponse::unknown_error("option inspection returned no value"))?;
        if let Some(error)=value.get("error").and_then(Value::as_str) {
            let response=WebDriverErrorResponse::new(axum::http::StatusCode::BAD_REQUEST,error,value.get("message").and_then(Value::as_str).unwrap_or(error),None);
            if error=="unknown error" && value.get("pending")==Some(&Value::Bool(true)) {return Ok(Some(response));}
            return Err(response);
        }
        if value.get("index").and_then(Value::as_u64).is_some_and(|i|i<128) && value.get("selected")==Some(&Value::Bool(true)) {return Ok(None);}
        Err(WebDriverErrorResponse::unknown_error("invalid option completion result"))
    }

    /// Snapshot/verify an OPTION without changing DOM selection or emitting events.
    async fn inspect_option(&self, js_var: &str, phase: &str) -> Result<Option<(usize, bool)>, WebDriverErrorResponse> {
        let script = format!("({})({}, {})", OPTION_SNAPSHOT_SCRIPT,
            serde_json::to_string(js_var).unwrap(), serde_json::to_string(phase).unwrap());
        let result = self.evaluate_js(&script).await?;
        let value = result.get("value").ok_or_else(|| WebDriverErrorResponse::unknown_error("option inspection returned no value"))?;
        if value.is_null() { return Ok(None); }
        if let Some(error) = value.get("error").and_then(Value::as_str) {
            return Err(WebDriverErrorResponse::new(axum::http::StatusCode::BAD_REQUEST, error,
                value.get("message").and_then(Value::as_str).unwrap_or(error), None));
        }
        let index = value.get("navigationSteps").and_then(Value::as_u64).filter(|v| *v < 128)
            .ok_or_else(|| WebDriverErrorResponse::unknown_error("invalid option navigation steps"))?;
        let selected = value.get("selected").and_then(Value::as_bool)
            .ok_or_else(|| WebDriverErrorResponse::unknown_error("invalid option selectedness"))?;
        Ok(Some((index as usize, selected)))
    }

    /// Native popup first-item key. Backends must opt in explicitly.
    /// Internal OPTION keyboard scope; ordinary Actions retain their normal route.
    async fn dispatch_option_key_event(&self, key: &str, is_down: bool, modifiers: &ModifierState) -> Result<(), WebDriverErrorResponse> {
        self.dispatch_key_event(key, is_down, modifiers).await
    }

    fn option_popup_first_key(&self) -> Result<&'static str, WebDriverErrorResponse> {
        Err(WebDriverErrorResponse::unsupported_operation("native option popup navigation is unavailable on this platform"))
    }

    /// Check if element is displayed
    async fn is_element_displayed(&self, js_var: &str) -> Result<bool, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                var style = window.getComputedStyle(el);
                return style.display !== 'none' && style.visibility !== 'hidden' && el.offsetParent !== null;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_bool_value(&result)
    }

    /// Check if element is enabled
    async fn is_element_enabled(&self, js_var: &str) -> Result<bool, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                return !el.disabled;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_bool_value(&result)
    }

    /// Check if element is selected (for checkboxes, radio buttons, options)
    async fn is_element_selected(&self, js_var: &str) -> Result<bool, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    return {{ error: 'stale element reference' }};
                }}
                if (el.tagName === 'INPUT' && (el.type === 'checkbox' || el.type === 'radio')) {{
                    return el.checked;
                }}
                if (el.tagName === 'OPTION') {{
                    return el.selected;
                }}
                return false;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_selected_value(&result)
    }

    /// Click on element
    async fn click_element(&self, js_var: &str) -> Result<(), WebDriverErrorResponse> {
        let (x, y) = self.get_element_center(js_var).await?;
        self.dispatch_pointer_event(
            PointerEventType::Move,
            x,
            y,
            0,
            0,
            &ModifierState::default(),
        )
        .await?;
        self.dispatch_pointer_event(
            PointerEventType::Down,
            x,
            y,
            0,
            1,
            &ModifierState::default(),
        )
        .await?;
        self.dispatch_pointer_event(PointerEventType::Up, x, y, 0, 0, &ModifierState::default())
            .await
    }

    /// Clear element content (for inputs/textareas)
    async fn clear_element(&self, _js_var: &str) -> Result<(), WebDriverErrorResponse> {
        Err(WebDriverErrorResponse::unsupported_operation(
            "native clear is not supported by this overlay",
        ))
    }

    /// Send keys to element
    async fn send_keys_to_element(
        &self,
        _js_var: &str,
        _text: &str,
    ) -> Result<(), WebDriverErrorResponse> {
        Err(WebDriverErrorResponse::unsupported_operation(
            "native text input is not supported by this overlay",
        ))
    }

    /// Get the active (focused) element and store in `js_var`
    /// Returns true if an active element was found
    async fn get_active_element(&self, js_var: &str) -> Result<bool, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = document.activeElement;
                if (el && el !== document.body) {{
                    window.{js_var} = el;
                    return true;
                }}
                return false;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_bool_value(&result)
    }

    /// Get element's computed accessibility role
    async fn get_element_computed_role(
        &self,
        js_var: &str,
    ) -> Result<String, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}

                // Check for explicit role attribute first
                var explicitRole = el.getAttribute('role');
                if (explicitRole) return explicitRole;

                // Try computedRole if available (Chrome/Edge)
                if (el.computedRole) return el.computedRole;

                // Compute implicit role based on element type
                var tag = el.tagName.toLowerCase();
                var type = el.type ? el.type.toLowerCase() : '';

                // Map elements to their implicit ARIA roles
                var roleMap = {{
                    'a': el.hasAttribute('href') ? 'link' : 'generic',
                    'article': 'article',
                    'aside': 'complementary',
                    'button': 'button',
                    'datalist': 'listbox',
                    'details': 'group',
                    'dialog': 'dialog',
                    'fieldset': 'group',
                    'figure': 'figure',
                    'footer': 'contentinfo',
                    'form': 'form',
                    'h1': 'heading',
                    'h2': 'heading',
                    'h3': 'heading',
                    'h4': 'heading',
                    'h5': 'heading',
                    'h6': 'heading',
                    'header': 'banner',
                    'hr': 'separator',
                    'img': el.getAttribute('alt') === '' ? 'presentation' : 'img',
                    'li': 'listitem',
                    'main': 'main',
                    'menu': 'list',
                    'meter': 'meter',
                    'nav': 'navigation',
                    'ol': 'list',
                    'optgroup': 'group',
                    'option': 'option',
                    'output': 'status',
                    'progress': 'progressbar',
                    'section': 'region',
                    'select': el.multiple ? 'listbox' : 'combobox',
                    'summary': 'button',
                    'table': 'table',
                    'tbody': 'rowgroup',
                    'td': 'cell',
                    'textarea': 'textbox',
                    'tfoot': 'rowgroup',
                    'th': 'columnheader',
                    'thead': 'rowgroup',
                    'tr': 'row',
                    'ul': 'list'
                }};

                // Handle input types
                if (tag === 'input') {{
                    var inputRoles = {{
                        'button': 'button',
                        'checkbox': 'checkbox',
                        'email': 'textbox',
                        'image': 'button',
                        'number': 'spinbutton',
                        'radio': 'radio',
                        'range': 'slider',
                        'reset': 'button',
                        'search': 'searchbox',
                        'submit': 'button',
                        'tel': 'textbox',
                        'text': 'textbox',
                        'url': 'textbox'
                    }};
                    return inputRoles[type] || 'textbox';
                }}

                return roleMap[tag] || '';
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_string_value(&result)
    }

    /// Get element's computed accessibility label
    async fn get_element_computed_label(
        &self,
        js_var: &str,
    ) -> Result<String, WebDriverErrorResponse> {
        let script = format!(
            r#"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}

                // Try computedName if available (Chrome/Edge)
                if (el.computedName) return el.computedName;

                // Check aria-labelledby first (highest priority)
                var labelledBy = el.getAttribute('aria-labelledby');
                if (labelledBy) {{
                    var labels = labelledBy.split(/\s+/).map(function(id) {{
                        var labelEl = document.getElementById(id);
                        return labelEl ? labelEl.textContent : '';
                    }});
                    var combined = labels.join(' ').trim();
                    if (combined) return combined;
                }}

                // Check aria-label
                var ariaLabel = el.getAttribute('aria-label');
                if (ariaLabel) return ariaLabel;

                // For inputs, check associated label
                var tag = el.tagName.toLowerCase();
                if (tag === 'input' || tag === 'textarea' || tag === 'select') {{
                    // Check for label with 'for' attribute
                    if (el.id) {{
                        var label = document.querySelector("label[for='" + el.id + "']");
                        if (label) return label.textContent.trim();
                    }}
                    // Check for wrapping label
                    var parentLabel = el.closest('label');
                    if (parentLabel) {{
                        // Get label text excluding the input's value
                        var clone = parentLabel.cloneNode(true);
                        var inputs = clone.querySelectorAll('input, textarea, select');
                        inputs.forEach(function(input) {{ input.remove(); }});
                        var labelText = clone.textContent.trim();
                        if (labelText) return labelText;
                    }}
                    // Check placeholder
                    if (el.placeholder) return el.placeholder;
                }}

                // For buttons and links, use text content
                if (tag === 'button' || tag === 'a') {{
                    return el.textContent.trim();
                }}

                // For images, use alt text
                if (tag === 'img') {{
                    return el.getAttribute('alt') || '';
                }}

                // Check title attribute as last resort
                var title = el.getAttribute('title');
                if (title) return title;

                // Fall back to text content for other elements
                return el.textContent ? el.textContent.trim() : '';
            }})()"#
        );
        let result = self.evaluate_js(&script).await?;
        extract_string_value(&result)
    }

    // =========================================================================
    // Shadow DOM
    // =========================================================================

    /// Get element's shadow root and store in `shadow_var`
    /// Returns true if shadow root exists
    async fn get_element_shadow_root(
        &self,
        js_var: &str,
        shadow_var: &str,
    ) -> Result<bool, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                var shadow = el.shadowRoot;
                if (shadow) {{
                    window.{shadow_var} = shadow;
                    return true;
                }}
                return false;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_bool_value(&result)
    }

    /// Find element within a shadow root
    async fn find_element_from_shadow(
        &self,
        shadow_var: &str,
        strategy_js: &str,
        js_var: &str,
    ) -> Result<bool, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var shadow = window.{shadow_var};
                if (!shadow) {{
                    throw new Error('no such shadow root');
                }}
                var el = {strategy_js};
                if (el) {{
                    window.{js_var} = el;
                    return true;
                }}
                return false;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_bool_value(&result)
    }

    /// Find multiple elements within a shadow root
    async fn find_elements_from_shadow(
        &self,
        shadow_var: &str,
        strategy_js: &str,
        js_var_prefix: &str,
    ) -> Result<usize, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var shadow = window.{shadow_var};
                if (!shadow) {{
                    throw new Error('no such shadow root');
                }}
                var elements = {strategy_js};
                var count = elements.length;
                for (var i = 0; i < count; i++) {{
                    window['{js_var_prefix}' + i] = elements[i];
                }}
                return count;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_usize_value(&result)
    }

    // =========================================================================
    // Script Execution
    // =========================================================================

    /// Execute synchronous JavaScript with arguments
    async fn execute_script(
        &self,
        script: &str,
        args: &[Value],
    ) -> Result<Value, WebDriverErrorResponse> {
        let args_json = serde_json::to_string(args)
            .map_err(|e| WebDriverErrorResponse::invalid_argument(&e.to_string()))?;

        // Generate unique result variable name
        let result_var = format!("__wdio_exec_{}", uuid::Uuid::new_v4());

        // Wrapper script that:
        // 1. Executes the user's script as a function body (per W3C WebDriver spec §13.2.2)
        // 2. Stores result in a global variable for polling
        // Note: We use an IIFE that returns `undefined` to avoid Promise serialization issues
        //
        // The script is treated as a function body. Clients that want to return a value must
        // include an explicit `return` statement — this matches WebdriverIO's function-object
        // wrapping (`return (fn).apply(null, arguments)`) and raw string scripts like
        // `"return document.title"`.
        let wrapper = format!(
            r"(function() {{
                var ELEMENT_KEY = 'element-6066-11e4-a52e-4f735466cecf';

                function serializeValue(value) {{
                    if (value === null || value === undefined) return null;
                    if (typeof value === 'boolean') return value;
                    if (typeof value === 'number') {{
                        if (!isFinite(value)) return null;
                        return value;
                    }}
                    if (typeof value === 'string') return value;
                    if (typeof value === 'function') return null;
                    if (typeof value === 'symbol') return null;
                    if (typeof value === 'bigint') return Number(value);
                    if (Array.isArray(value)) {{
                        return value.map(serializeValue);
                    }}
                    if (typeof value === 'object') {{
                        if (value[ELEMENT_KEY]) return value;
                        if (value && value.nodeType && value.nodeType === 1) return null;
                        var result = {{}};
                        for (var key in value) {{
                            if (value.hasOwnProperty(key)) {{
                                result[key] = serializeValue(value[key]);
                            }}
                        }}
                        return result;
                    }}
                    return null;
                }}

                function deserializeArg(arg) {{
                    if (arg === null || arg === undefined) return arg;
                    if (Array.isArray(arg)) return arg.map(deserializeArg);
                    if (typeof arg === 'object') {{
                        if (arg[ELEMENT_KEY]) {{
                            var el = window['__wd_el_' + arg[ELEMENT_KEY].replace(/-/g, '')];
                            if (!el) throw new Error('stale element reference');
                            return el;
                        }}
                        var result = {{}};
                        for (var key in arg) {{
                            if (arg.hasOwnProperty(key)) result[key] = deserializeArg(arg[key]);
                        }}
                        return result;
                    }}
                    return arg;
                }}

                // Start async execution (fire and forget)
                (async function() {{
                    try {{
                        var args = {args_json}.map(deserializeArg);
                        // W3C-compliant: wrap as function body, apply with args
                        var raw_result = await (async function() {{ {script} }}).apply(null, args);
                        var serialized = serializeValue(raw_result);
                        window['{result_var}'] = {{ __wd_success: true, __wd_value: serialized }};
                    }} catch (e) {{
                        window['{result_var}'] = {{ __wd_success: false, __wd_error: e.message || String(e) }};
                    }}
                }})();

                // Return undefined to avoid Promise serialization issues
                return undefined;
            }})()",
        );

        // Execute the wrapper script (fire and forget, returns undefined)
        self.evaluate_js(&wrapper).await?;

        // Poll for the result with timeout
        let poll_script = format!("window['{}']", result_var);
        let timeout = std::time::Duration::from_millis(self.script_timeout_ms());
        let start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_millis(50);

        loop {
            let poll_result = self.evaluate_js(&poll_script).await?;
            let inner = poll_result.get("value").cloned().unwrap_or(Value::Null);

            // Check if we have a result
            if !inner.is_null() && inner.get("__wd_success").is_some() {
                // Clean up the global variable
                let cleanup_script = format!("delete window['{}']", result_var);
                let _ = self.evaluate_js(&cleanup_script).await;

                return extract_script_result_from_inner(&inner);
            }

            if start.elapsed() > timeout {
                // Clean up on timeout
                let cleanup_script = format!("delete window['{}']", result_var);
                let _ = self.evaluate_js(&cleanup_script).await;

                return Err(WebDriverErrorResponse::script_timeout());
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Execute asynchronous JavaScript with callback.
    ///
    /// Each platform must implement this using native message handlers.
    async fn execute_async_script(
        &self,
        script: &str,
        args: &[Value],
    ) -> Result<Value, WebDriverErrorResponse>;

    // =========================================================================
    // Screenshots
    // =========================================================================

    /// Take screenshot of the page, returns base64-encoded PNG
    async fn take_screenshot(&self) -> Result<String, WebDriverErrorResponse>;

    /// Take screenshot of a specific element, returns base64-encoded PNG
    async fn take_element_screenshot(&self, js_var: &str)
        -> Result<String, WebDriverErrorResponse>;

    // =========================================================================
    // Actions (Keyboard/Pointer)
    // =========================================================================

    /// Reject keys that this backend cannot represent before Actions retains
    /// any input or posts the first native event. Dispatch failures remain
    /// uncertain and still require an explicit release.
    fn preflight_key_event(&self, _key: &str) -> Result<(), WebDriverErrorResponse> {
        Err(WebDriverErrorResponse::unsupported_operation(
            "native keyboard input is not supported by this overlay",
        ))
    }

    /// Check native foreground admission before a new ordinary key-down is
    /// retained. The dispatch path must still recheck because focus can change.
    async fn preflight_native_key_down(&self) -> Result<(), WebDriverErrorResponse> {
        Ok(())
    }

    /// Dispatch a keyboard event with modifier state
    async fn dispatch_key_event(
        &self,
        _key: &str,
        _is_down: bool,
        _modifiers: &ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        Err(WebDriverErrorResponse::unsupported_operation(
            "native keyboard input is not supported by this overlay",
        ))
    }

    /// Internal OPTION popup scope; ordinary pointer dispatch is unchanged.
    async fn dispatch_option_pointer_event(
        &self, event_type: PointerEventType, x: i32, y: i32, button: u32,
        buttons: u32, modifiers: &ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        self.dispatch_pointer_event(event_type, x, y, button, buttons, modifiers).await
    }

    async fn dispatch_pointer_event(
        &self,
        _event_type: PointerEventType,
        _x: i32,
        _y: i32,
        _button: u32,
        _buttons: u32,
        _modifiers: &ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        Err(WebDriverErrorResponse::unsupported_operation(
            "native pointer input is unavailable on this platform",
        ))
    }

    async fn dispatch_scroll_event(
        &self,
        _x: i32,
        _y: i32,
        _delta_x: i32,
        _delta_y: i32,
        _modifiers: &ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        Err(WebDriverErrorResponse::unsupported_operation(
            "native wheel input is not supported by this overlay",
        ))
    }

    // =========================================================================
    // Window Management
    // =========================================================================

    /// Get window rectangle (position and size)
    #[cfg(desktop)]
    async fn get_window_rect(&self) -> Result<WindowRect, WebDriverErrorResponse> {
        if let Ok(position) = self.window().outer_position() {
            if let Ok(size) = self.window().outer_size() {
                return Ok(WindowRect {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                });
            }
        }
        Ok(WindowRect::default())
    }

    #[cfg(mobile)]
    async fn get_window_rect(&self) -> Result<WindowRect, WebDriverErrorResponse>;

    /// Set window rectangle (position and size)
    #[cfg(desktop)]
    async fn set_window_rect(
        &self,
        rect: WindowRect,
    ) -> Result<WindowRect, WebDriverErrorResponse> {
        // Exit fullscreen/maximized state before setting rect
        // Otherwise the window manager may ignore our size/position request
        if self.window().is_fullscreen().unwrap_or(false) {
            let _ = self.window().set_fullscreen(false);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        if self.window().is_maximized().unwrap_or(false) {
            let _ = self.window().unmaximize();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        let _ = self
            .window()
            .set_position(PhysicalPosition::new(rect.x, rect.y));

        // Calculate chrome/decoration size to set outer size correctly
        // On Windows/Linux, set_size sets inner size, but we want to set outer size
        let (chrome_width, chrome_height) = if let (Ok(outer), Ok(inner)) =
            (self.window().outer_size(), self.window().inner_size())
        {
            (
                outer.width.saturating_sub(inner.width),
                outer.height.saturating_sub(inner.height),
            )
        } else {
            (0, 0)
        };

        // Set inner size = requested outer size - chrome
        let inner_width = rect.width.saturating_sub(chrome_width);
        let inner_height = rect.height.saturating_sub(chrome_height);
        let _ = self
            .window()
            .set_size(PhysicalSize::new(inner_width, inner_height));

        self.get_window_rect().await
    }

    /// Maximize window
    #[cfg(desktop)]
    async fn maximize_window(&self) -> Result<WindowRect, WebDriverErrorResponse> {
        let _ = self.window().maximize();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        self.get_window_rect().await
    }

    /// Minimize window
    #[cfg(desktop)]
    async fn minimize_window(&self) -> Result<(), WebDriverErrorResponse> {
        let _ = self.window().minimize();
        Ok(())
    }

    /// Set window to fullscreen
    #[cfg(desktop)]
    async fn fullscreen_window(&self) -> Result<WindowRect, WebDriverErrorResponse> {
        let _ = self.window().set_fullscreen(true);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        self.get_window_rect().await
    }

    /// Set window rectangle (mobile unsupported)
    #[cfg(mobile)]
    async fn set_window_rect(
        &self,
        _rect: WindowRect,
    ) -> Result<WindowRect, WebDriverErrorResponse> {
        Err(WebDriverErrorResponse::unsupported_operation(
            "Setting window rect is not supported on mobile platforms",
        ))
    }

    /// Maximize window (mobile unsupported)
    #[cfg(mobile)]
    async fn maximize_window(&self) -> Result<WindowRect, WebDriverErrorResponse> {
        Err(WebDriverErrorResponse::unsupported_operation(
            "Maximizing window is not supported on mobile platforms",
        ))
    }

    /// Minimize window (mobile unsupported)
    #[cfg(mobile)]
    async fn minimize_window(&self) -> Result<(), WebDriverErrorResponse> {
        Err(WebDriverErrorResponse::unsupported_operation(
            "Minimizing window is not supported on mobile platforms",
        ))
    }

    /// Set window to fullscreen (mobile unsupported)
    #[cfg(mobile)]
    async fn fullscreen_window(&self) -> Result<WindowRect, WebDriverErrorResponse> {
        Err(WebDriverErrorResponse::unsupported_operation(
            "Fullscreen window is not supported on mobile platforms",
        ))
    }

    // =========================================================================
    // Frames
    // =========================================================================

    /// Switch to a frame by ID (index or element reference)
    async fn switch_to_frame(&self, id: FrameId) -> Result<(), WebDriverErrorResponse> {
        match id {
            FrameId::Index(index) => {
                let script = format!(
                    r"(function() {{
                        var frames = document.querySelectorAll('iframe, frame');
                        if ({index} >= frames.length) {{
                            return false;
                        }}
                        return true;
                    }})()"
                );
                let result = self.evaluate_js(&script).await?;
                if result.get("value") == Some(&Value::Bool(false)) {
                    return Err(WebDriverErrorResponse::no_such_frame());
                }
                Ok(())
            }
            FrameId::Element(js_var) => {
                let script = format!(
                    r"(function() {{
                        var el = window.{js_var};
                        if (!el || !el.isConnected) {{
                            throw new Error('stale element reference');
                        }}
                        if (el.tagName !== 'IFRAME' && el.tagName !== 'FRAME') {{
                            throw new Error('element is not a frame');
                        }}
                        return true;
                    }})()"
                );
                self.evaluate_js(&script).await?;
                Ok(())
            }
        }
    }

    /// Switch to parent frame
    async fn switch_to_parent_frame(&self) -> Result<(), WebDriverErrorResponse> {
        // No-op - frame context is managed by the session, not the executor
        Ok(())
    }

    // =========================================================================
    // Cookies (using Tauri's native cookie APIs)
    // =========================================================================

    /// Get all cookies
    async fn get_all_cookies(&self) -> Result<Vec<Cookie>, WebDriverErrorResponse> {
        self.window()
            .cookies()
            .map(|cookies| cookies.iter().map(tauri_cookie_to_webdriver).collect())
            .map_err(|e| WebDriverErrorResponse::unknown_error(&e.to_string()))
    }

    /// Get a specific cookie by name
    async fn get_cookie(&self, name: &str) -> Result<Option<Cookie>, WebDriverErrorResponse> {
        let cookies = self.get_all_cookies().await?;
        Ok(cookies.into_iter().find(|c| c.name == name))
    }

    /// Add a cookie
    async fn add_cookie(&self, mut cookie: Cookie) -> Result<(), WebDriverErrorResponse> {
        // Per WebDriver spec: if no domain is specified, use the current page's domain
        if cookie.domain.is_none() {
            if let Ok(url) = self.window().url() {
                cookie.domain = url.host_str().map(String::from);
            }
        }

        // Default path to "/" if not specified
        if cookie.path.is_none() {
            cookie.path = Some("/".to_string());
        }

        let tauri_cookie = webdriver_cookie_to_tauri(&cookie);
        self.window()
            .set_cookie(tauri_cookie)
            .map_err(|e| WebDriverErrorResponse::unknown_error(&e.to_string()))
    }

    /// Delete a cookie by name
    async fn delete_cookie(&self, name: &str) -> Result<(), WebDriverErrorResponse> {
        // Find the cookie first to get its exact domain/path for deletion
        let cookies = self
            .window()
            .cookies()
            .map_err(|e| WebDriverErrorResponse::unknown_error(&e.to_string()))?;

        for cookie in cookies {
            if cookie.name() == name {
                self.window()
                    .delete_cookie(cookie)
                    .map_err(|e| WebDriverErrorResponse::unknown_error(&e.to_string()))?;
                return Ok(());
            }
        }
        Ok(())
    }

    /// Delete all cookies
    async fn delete_all_cookies(&self) -> Result<(), WebDriverErrorResponse> {
        let cookies = self
            .window()
            .cookies()
            .map_err(|e| WebDriverErrorResponse::unknown_error(&e.to_string()))?;

        for cookie in cookies {
            self.window()
                .delete_cookie(cookie)
                .map_err(|e| WebDriverErrorResponse::unknown_error(&e.to_string()))?;
        }
        Ok(())
    }

    // =========================================================================
    // Alerts (using per-window alert state)
    // =========================================================================

    /// Dismiss the current alert (cancel)
    async fn dismiss_alert(&self) -> Result<(), WebDriverErrorResponse> {
        let manager = self.window().app_handle().state::<AlertStateManager>();
        let alert_state = manager.get_or_create(self.window().label());
        if alert_state.respond(false, None) {
            Ok(())
        } else {
            Err(WebDriverErrorResponse::no_such_alert())
        }
    }

    /// Accept the current alert (OK)
    async fn accept_alert(&self) -> Result<(), WebDriverErrorResponse> {
        let manager = self.window().app_handle().state::<AlertStateManager>();
        let alert_state = manager.get_or_create(self.window().label());
        // For prompts, use input text if set, otherwise default text
        let prompt_text = alert_state
            .get_prompt_input()
            .or_else(|| alert_state.get_default_text());
        if alert_state.respond(true, prompt_text) {
            Ok(())
        } else {
            Err(WebDriverErrorResponse::no_such_alert())
        }
    }

    /// Get the text of the current alert
    async fn get_alert_text(&self) -> Result<String, WebDriverErrorResponse> {
        let manager = self.window().app_handle().state::<AlertStateManager>();
        let alert_state = manager.get_or_create(self.window().label());
        match alert_state.get_message() {
            Some(msg) => Ok(msg),
            None => Err(WebDriverErrorResponse::no_such_alert()),
        }
    }

    /// Send text to the current alert (for prompts)
    async fn send_alert_text(&self, text: &str) -> Result<(), WebDriverErrorResponse> {
        let manager = self.window().app_handle().state::<AlertStateManager>();
        let alert_state = manager.get_or_create(self.window().label());
        match alert_state.get_alert_type() {
            None => Err(WebDriverErrorResponse::no_such_alert()),
            Some(AlertType::Prompt) => {
                // Store the text for when acceptAlert is called
                if alert_state.set_prompt_input(text.to_string()) {
                    Ok(())
                } else {
                    Err(WebDriverErrorResponse::no_such_alert())
                }
            }
            Some(_) => Err(WebDriverErrorResponse::element_not_interactable(
                "User prompt is not a prompt dialog",
            )),
        }
    }

    // =========================================================================
    // Print
    // =========================================================================

    /// Print page to PDF, returns base64-encoded PDF
    async fn print_page(&self, options: PrintOptions) -> Result<String, WebDriverErrorResponse>;
}

// =============================================================================
// Helper Functions for Default Implementations
// =============================================================================

/// Extract string value from JavaScript result
fn extract_string_value(result: &Value) -> Result<String, WebDriverErrorResponse> {
    if let Some(success) = result.get("success").and_then(Value::as_bool) {
        if success {
            if let Some(value) = result.get("value") {
                if let Some(s) = value.as_str() {
                    return Ok(s.to_string());
                }
                return Ok(value.to_string());
            }
        } else if let Some(error) = result.get("error").and_then(Value::as_str) {
            return Err(WebDriverErrorResponse::javascript_error(error, None));
        }
    }
    Ok(String::new())
}

/// Extract boolean value from JavaScript result
fn extract_bool_value(result: &Value) -> Result<bool, WebDriverErrorResponse> {
    if let Some(success) = result.get("success").and_then(Value::as_bool) {
        if success {
            if let Some(value) = result.get("value").and_then(Value::as_bool) {
                return Ok(value);
            }
        } else if let Some(error) = result.get("error").and_then(Value::as_str) {
            return Err(WebDriverErrorResponse::javascript_error(error, None));
        }
    }
    Ok(false)
}

/// Selectedness has an explicit stale result: WebView2 may serialize an
/// uncaught JavaScript exception as null, which must never become false.
fn extract_selected_value(result: &Value) -> Result<bool, WebDriverErrorResponse> {
    let value = extract_value(result)?;
    if value.get("error").and_then(Value::as_str) == Some("stale element reference") {
        return Err(WebDriverErrorResponse::stale_element_reference());
    }
    value.as_bool().ok_or_else(|| {
        WebDriverErrorResponse::unknown_error("selectedness script returned no boolean value")
    })
}

/// Extract usize value from JavaScript result
fn extract_usize_value(result: &Value) -> Result<usize, WebDriverErrorResponse> {
    if let Some(success) = result.get("success").and_then(Value::as_bool) {
        if success {
            if let Some(count) = result.get("value").and_then(Value::as_u64) {
                return Ok(usize::try_from(count).unwrap_or(0));
            }
        } else if let Some(error) = result.get("error").and_then(Value::as_str) {
            return Err(WebDriverErrorResponse::javascript_error(error, None));
        }
    }
    Ok(0)
}

/// Extract raw Value from JavaScript result
fn extract_value(result: &Value) -> Result<Value, WebDriverErrorResponse> {
    if let Some(success) = result.get("success").and_then(Value::as_bool) {
        if success {
            return Ok(result.get("value").cloned().unwrap_or(Value::Null));
        } else if let Some(error) = result.get("error").and_then(Value::as_str) {
            return Err(WebDriverErrorResponse::javascript_error(error, None));
        }
    }
    Ok(Value::Null)
}

/// Extract result from inner value (the __wd_success/__wd_value object)
fn extract_script_result_from_inner(inner: &Value) -> Result<Value, WebDriverErrorResponse> {
    // Check the script execution result
    if let Some(success) = inner.get("__wd_success").and_then(Value::as_bool) {
        if success {
            return Ok(inner.get("__wd_value").cloned().unwrap_or(Value::Null));
        } else if let Some(error) = inner.get("__wd_error").and_then(Value::as_str) {
            return Err(WebDriverErrorResponse::javascript_error(error, None));
        }
    }

    // If we got null or no wrapper structure, it's likely a syntax error
    if inner.is_null() || inner.get("__wd_success").is_none() {
        return Err(WebDriverErrorResponse::javascript_error(
            "Script execution failed (possible syntax error)",
            None,
        ));
    }

    Ok(Value::Null)
}

/// Wrap a JavaScript script to execute within a specific frame context.
/// If `frame_context` is empty (top-level), returns the script unchanged.
/// Otherwise, wraps the script to navigate to the correct frame before execution.
pub fn wrap_script_for_frame_context(script: &str, frame_context: &[FrameId]) -> String {
    use std::fmt::Write;

    if frame_context.is_empty() {
        return script.to_string();
    }

    // Build JavaScript to navigate to the target frame
    let mut frame_nav = String::new();
    frame_nav.push_str("(function() {\n");
    frame_nav.push_str("  var ctx = window;\n");
    frame_nav.push_str("  var doc = document;\n");

    for (i, frame_id) in frame_context.iter().enumerate() {
        match frame_id {
            FrameId::Index(index) => {
                let _ = writeln!(
                    frame_nav,
                    "  var frames{i} = doc.querySelectorAll('iframe, frame');"
                );
                let _ = writeln!(
                    frame_nav,
                    "  if ({index} >= frames{i}.length) throw new Error('no such frame');"
                );
                let _ = writeln!(frame_nav, "  var frame{i} = frames{i}[{index}];");
                let _ = writeln!(
                    frame_nav,
                    "  if (!frame{i}.contentWindow) throw new Error('no such frame');"
                );
                let _ = writeln!(frame_nav, "  ctx = frame{i}.contentWindow;");
                let _ = writeln!(frame_nav, "  doc = frame{i}.contentDocument;");
            }
            FrameId::Element(js_var) => {
                let _ = writeln!(frame_nav, "  var frame{i} = window.{js_var};");
                let _ = writeln!(
                    frame_nav,
                    "  if (!frame{i} || !doc.contains(frame{i})) throw new Error('stale element reference');"
                );
                let _ = writeln!(
                    frame_nav,
                    "  if (frame{i}.tagName !== 'IFRAME' && frame{i}.tagName !== 'FRAME') throw new Error('element is not a frame');"
                );
                let _ = writeln!(
                    frame_nav,
                    "  if (!frame{i}.contentWindow) throw new Error('no such frame');"
                );
                let _ = writeln!(frame_nav, "  ctx = frame{i}.contentWindow;");
                let _ = writeln!(frame_nav, "  doc = frame{i}.contentDocument;");
            }
        }
    }

    // Execute the original script in the frame context
    // We use Function constructor to evaluate in the frame's context
    let escaped_script = script
        .replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace("${", "\\${");

    let _ = writeln!(frame_nav, "  return ctx.eval(`{escaped_script}`);");
    frame_nav.push_str("})()");

    frame_nav
}

// =============================================================================
// Cookie Conversion Functions
// =============================================================================

/// Convert Tauri cookie to `WebDriver` cookie
fn tauri_cookie_to_webdriver(cookie: &TauriCookie<'static>) -> Cookie {
    use tauri::webview::cookie::{Expiration, SameSite};

    Cookie {
        name: cookie.name().to_string(),
        value: cookie.value().to_string(),
        path: cookie.path().map(String::from),
        domain: cookie.domain().map(String::from),
        secure: cookie.secure().unwrap_or(false),
        http_only: cookie.http_only().unwrap_or(false),
        expiry: cookie.expires().and_then(|exp| match exp {
            Expiration::DateTime(dt) => Some(dt.unix_timestamp().cast_unsigned()),
            Expiration::Session => None,
        }),
        same_site: cookie.same_site().map(|ss| match ss {
            SameSite::Strict => "Strict".to_string(),
            SameSite::Lax => "Lax".to_string(),
            SameSite::None => "None".to_string(),
        }),
    }
}

/// Convert `WebDriver` cookie to Tauri cookie
fn webdriver_cookie_to_tauri(cookie: &Cookie) -> TauriCookie<'static> {
    use tauri::webview::cookie::{time::OffsetDateTime, Expiration, SameSite};

    let mut builder = TauriCookie::build((cookie.name.clone(), cookie.value.clone()));

    if let Some(ref path) = cookie.path {
        builder = builder.path(path.clone());
    }

    if let Some(ref domain) = cookie.domain {
        builder = builder.domain(domain.clone());
    }

    // SECURITY: Always set Secure attribute for all cookies.
    // This WebDriver implementation supports cookie testing per the WebDriver specification.
    builder = builder.secure(true);

    if cookie.http_only {
        builder = builder.http_only(true);
    }

    if let Some(expiry) = cookie.expiry {
        if let Ok(dt) = OffsetDateTime::from_unix_timestamp(expiry.cast_signed()) {
            builder = builder.expires(Expiration::DateTime(dt));
        }
    }

    if let Some(ref same_site) = cookie.same_site {
        let ss = match same_site.to_lowercase().as_str() {
            "strict" => SameSite::Strict,
            "lax" => SameSite::Lax,
            _ => SameSite::None,
        };
        builder = builder.same_site(ss);
    }

    builder.build()
}

#[cfg(test)]
mod native_input_tests {
    #[test]
    fn selectedness_preserves_booleans_and_refuses_stale_or_missing_values() {
        use serde_json::json;
        for selected in [false, true] {
            assert_eq!(super::extract_selected_value(&json!({"success":true,"value":selected})).unwrap(), selected);
        }
        let stale = super::extract_selected_value(&json!({"success":true,"value":{"error":"stale element reference"}})).unwrap_err();
        assert_eq!(stale.error, "stale element reference");
        assert_eq!(stale.status, axum::http::StatusCode::NOT_FOUND);
        for result in [json!({"success":true,"value":null}), json!({"success":true}), json!({}), json!({"success":true,"value":"false"})] {
            assert_eq!(super::extract_selected_value(&result).unwrap_err().error, "unknown error");
        }
        assert_eq!(super::extract_selected_value(&json!({"success":false,"error":"evaluation failed"})).unwrap_err().error, "javascript error");
    }

    #[test]
    fn overlapping_input_is_refused_and_guard_release_restores_admission() {
        let guard = super::native_input_guard().unwrap();
        assert!(super::native_input_guard().is_err());
        drop(guard);
        assert!(super::native_input_guard().is_ok());
    }
}
