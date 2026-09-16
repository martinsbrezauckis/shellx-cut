use std::sync::Arc;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSScreen};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventType, CGMouseButton, CGPreflightPostEventAccess,
    CGScrollEventUnit,
};
use objc2_foundation::{
    NSData, NSDictionary, NSError, NSObject, NSObjectProtocol, NSPoint, NSString,
};
use objc2_web_kit::{
    WKContentWorld, WKFrameInfo, WKPDFConfiguration, WKSnapshotConfiguration, WKUIDelegate,
    WKWebView,
};
use serde_json::Value;
use tauri::{Manager, Runtime, WebviewWindow};
use tokio::sync::oneshot;

use crate::platform::alert_state::{AlertState, AlertStateManager, AlertType, PendingAlert};
use crate::platform::{
    wrap_script_for_frame_context, FrameId, ModifierState, PlatformExecutor, PointerEventType,
    PrintOptions,
};
use crate::server::response::WebDriverErrorResponse;
use crate::webdriver::Timeouts;

/// Key for associating the UI delegate with the webview
static DELEGATE_KEY: u8 = 0;

/// macOS `WebView` executor using `WKWebView` native APIs
#[derive(Clone)]
pub struct MacOSExecutor<R: Runtime> {
    window: WebviewWindow<R>,
    timeouts: Timeouts,
    frame_context: Vec<FrameId>,
}

impl<R: Runtime> MacOSExecutor<R> {
    pub fn new(window: WebviewWindow<R>, timeouts: Timeouts, frame_context: Vec<FrameId>) -> Self {
        Self {
            window,
            timeouts,
            frame_context,
        }
    }
}

// Native posting is intentionally process-directed. Never fall back to a global
// event tap or assume another executable's Accessibility grant applies here.
enum NativeInput {
    Pointer {
        event_type: PointerEventType,
        button: u32,
        buttons: u32,
    },
    Key {
        code: u16,
        text: Option<String>,
        down: bool,
    },
    Wheel {
        dx: i32,
        dy: i32,
    },
}

fn native_flags(m: ModifierState) -> CGEventFlags {
    let mut flags = CGEventFlags::empty();
    if m.ctrl {
        flags |= CGEventFlags::MaskControl;
    }
    if m.shift {
        flags |= CGEventFlags::MaskShift;
    }
    if m.alt {
        flags |= CGEventFlags::MaskAlternate;
    }
    if m.meta {
        flags |= CGEventFlags::MaskCommand;
    }
    flags
}

fn native_key(
    key: &str,
    modifiers: &ModifierState,
) -> Result<(u16, Option<String>), WebDriverErrorResponse> {
    // Public macOS virtual key positions; Unicode text is supplied separately.
    let special = match key {
        "\u{E003}" => Some(51),
        "\u{E004}" => Some(48),
        "\u{E006}" | "\u{E007}" => Some(36),
        "\u{E008}" => Some(56),
        "\u{E009}" => Some(59),
        "\u{E00A}" => Some(58),
        "\u{E03D}" => Some(55),
        "\u{E00C}" => Some(53),
        "\u{E00D}" => Some(49),
        "\u{E00E}" => Some(116),
        "\u{E00F}" => Some(121),
        "\u{E010}" => Some(119),
        "\u{E011}" => Some(115),
        "\u{E012}" => Some(123),
        "\u{E013}" => Some(126),
        "\u{E014}" => Some(124),
        "\u{E015}" => Some(125),
        "\u{E017}" => Some(117),
        _ => None,
    };
    if let Some(code) = special {
        return Ok((code, None));
    }
    let chars: Vec<char> = key.chars().collect();
    if chars.len() != 1 || chars[0].is_control() || ('\u{E000}'..='\u{F8FF}').contains(&chars[0]) {
        return Err(WebDriverErrorResponse::unsupported_operation(
            "unsupported native key",
        ));
    }
    let c = chars[0].to_ascii_lowercase();
    let code = match c {
        'a' => Some(0),
        's' => Some(1),
        'd' => Some(2),
        'f' => Some(3),
        'h' => Some(4),
        'g' => Some(5),
        'z' => Some(6),
        'x' => Some(7),
        'c' => Some(8),
        'v' => Some(9),
        'b' => Some(11),
        'q' => Some(12),
        'w' => Some(13),
        'e' => Some(14),
        'r' => Some(15),
        'y' => Some(16),
        't' => Some(17),
        '1' => Some(18),
        '2' => Some(19),
        '3' => Some(20),
        '4' => Some(21),
        '6' => Some(22),
        '5' => Some(23),
        '=' => Some(24),
        '9' => Some(25),
        '7' => Some(26),
        '-' => Some(27),
        '8' => Some(28),
        '0' => Some(29),
        ']' => Some(30),
        'o' => Some(31),
        'u' => Some(32),
        '[' => Some(33),
        'i' => Some(34),
        'p' => Some(35),
        'l' => Some(37),
        'j' => Some(38),
        '\'' => Some(39),
        'k' => Some(40),
        ';' => Some(41),
        '\\' => Some(42),
        ',' => Some(43),
        '/' => Some(44),
        'n' => Some(45),
        'm' => Some(46),
        '.' => Some(47),
        ' ' => Some(49),
        '`' => Some(50),
        _ => None,
    };
    if code.is_none() && (modifiers.ctrl || modifiers.alt || modifiers.meta) {
        return Err(WebDriverErrorResponse::unsupported_operation(
            "modified non-ASCII native key is unsupported",
        ));
    }
    Ok((code.unwrap_or(0), Some(key.to_string())))
}

impl<R: Runtime + 'static> MacOSExecutor<R> {
    async fn post_native(
        &self,
        input: NativeInput,
        coords: Option<(i32, i32)>,
        modifiers: ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        if !self.frame_context.is_empty() {
            return Err(WebDriverErrorResponse::unsupported_operation(
                "nested frame native input is unsupported",
            ));
        }
        if !CGPreflightPostEventAccess() {
            return Err(WebDriverErrorResponse::unsupported_operation(&format!(
                "native input posting unavailable: CGPreflightPostEventAccess=false pid={} executable={}",
                std::process::id(), std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default())));
        }
        // An Up may release the original owned press after the view shrank.
        // Common state supplies the last successfully admitted point, never a failed move.
        let releasing = matches!(
            &input,
            NativeInput::Pointer {
                event_type: PointerEventType::Up,
                ..
            }
        );
        let viewport = if let Some((x, y)) = coords {
            let value = self.evaluate_js("({width:innerWidth,height:innerHeight,scale:visualViewport?visualViewport.scale:1,x:visualViewport?visualViewport.offsetLeft:0,y:visualViewport?visualViewport.offsetTop:0})").await?;
            let v = &value["value"];
            let w = v["width"].as_f64().unwrap_or(0.0);
            let h = v["height"].as_f64().unwrap_or(0.0);
            if w <= 0.0
                || h <= 0.0
                || (!releasing && (x < 0 || y < 0 || f64::from(x) >= w || f64::from(y) >= h))
                || v["scale"].as_f64() != Some(1.0)
                || v["x"].as_f64() != Some(0.0)
                || v["y"].as_f64() != Some(0.0)
            {
                return Err(WebDriverErrorResponse::element_not_interactable(
                    "native input requires unzoomed in-bounds viewport",
                ));
            }
            Some((x, y, w, h))
        } else {
            None
        };
        let (tx, rx) = oneshot::channel();
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pending = cancelled.clone();
        self.window.with_webview(move |webview| unsafe {
            let response = (|| -> Result<(),String> {
                let mtm=MainThreadMarker::new().ok_or("native input requires main thread")?;
                let wk: &WKWebView=&*webview.inner().cast();
                let window=wk.window().ok_or("owned webview has no native window")?;
                if !window.isVisible() || window.isMiniaturized() || wk.isHiddenOrHasHiddenAncestor() {
                    return Err("owned view/window must be visible and not minimized".into());
                }
                let location = if let Some((x,y,width,height))=viewport {
                    let b=wk.bounds();let i=wk.safeAreaInsets();
                    let full=(b.size.width-width).abs()<=1.0 && (b.size.height-height).abs()<=1.0;
                    let inset=[i.left,i.right,i.top,i.bottom].iter().all(|n|n.is_finite() && *n>=0.0)
                        && (b.size.width-i.left-i.right-width).abs()<=1.0 && (b.size.height-i.top-i.bottom-height).abs()<=1.0;
                    if wk.pageZoom()!=1.0 || (!full && !inset) { return Err(format!("native/DOM viewport mismatch bounds={b:?} dom={width}x{height} insets={i:?}")); }
                    let left=if full{0.0}else{i.left};let top=if full{0.0}else{i.top};
                    let p=NSPoint::new(b.origin.x+left+f64::from(x),b.origin.y+if wk.isFlipped(){top+f64::from(y)}else{b.size.height-top-f64::from(y)});
                    let screen=window.convertPointToScreen(wk.convertPoint_toView(p,None));
                    let screens=NSScreen::screens(mtm);let primary=screens.firstObject().ok_or("primary screen unavailable")?;
                    let frame=primary.frame();
                    NSPoint::new(screen.x,frame.origin.y+frame.size.height-screen.y)
                } else { NSPoint::new(0.0,0.0) };
                if pending.load(std::sync::atomic::Ordering::SeqCst) || !CGPreflightPostEventAccess() { return Err("native posting request expired or permission unavailable".into()); }
                let event=match input {
                    NativeInput::Pointer{event_type,button,buttons} => {
                        let active=if matches!(event_type,PointerEventType::Move) {
                            if buttons&1!=0{0}else if buttons&2!=0{2}else if buttons&4!=0{1}else{button}
                        } else {button};
                        let native_button=match active{0=>CGMouseButton::Left,1=>CGMouseButton::Center,_=>CGMouseButton::Right};
                        let kind=match event_type {
                            PointerEventType::Down=>match active{0=>CGEventType::LeftMouseDown,1=>CGEventType::OtherMouseDown,_=>CGEventType::RightMouseDown},
                            PointerEventType::Up=>match active{0=>CGEventType::LeftMouseUp,1=>CGEventType::OtherMouseUp,_=>CGEventType::RightMouseUp},
                            PointerEventType::Move if buttons!=0=>match active{0=>CGEventType::LeftMouseDragged,1=>CGEventType::OtherMouseDragged,_=>CGEventType::RightMouseDragged},
                            PointerEventType::Move=>CGEventType::MouseMoved,
                            PointerEventType::Click=>return Err("synthetic click forbidden".into()),
                        };
                        let e=CGEvent::new_mouse_event(None,kind,location,native_button).ok_or("native mouse event unavailable")?;
                        CGEvent::set_integer_value_field(Some(&e),CGEventField::MouseEventClickState,if matches!(event_type,PointerEventType::Move){0}else{1});
                        e
                    },
                    NativeInput::Key{code,text,down} => {
                        if !window.makeFirstResponder(Some(wk)) { return Err("owned webview refuses first responder".into()); }
                        let e=CGEvent::new_keyboard_event(None,code,down).ok_or("native keyboard event unavailable")?;
                        if let Some(text)=text { let units:Vec<u16>=text.encode_utf16().collect();CGEvent::keyboard_set_unicode_string(Some(&e),units.len() as _,units.as_ptr()); }
                        e
                    },
                    NativeInput::Wheel{dx,dy} => {
                        let e=CGEvent::new_scroll_wheel_event2(None,CGScrollEventUnit::Pixel,2,dy,dx,0).ok_or("native wheel event unavailable")?;
                        CGEvent::set_location(Some(&e),location);e
                    },
                };
                CGEvent::set_flags(Some(&event),native_flags(modifiers));
                CGEvent::post_to_pid(std::process::id() as i32,Some(&event));
                Ok(())
            })();let _=tx.send(response);
        }).map_err(|e|WebDriverErrorResponse::unknown_error(&e.to_string()))?;
        match tokio::time::timeout(
            std::time::Duration::from_millis(self.timeouts.script_ms),
            rx,
        )
        .await
        {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(e))) => Err(WebDriverErrorResponse::element_not_interactable(&e)),
            Ok(Err(_)) => Err(WebDriverErrorResponse::unknown_error(
                "native posting channel closed",
            )),
            Err(_) => {
                cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                Err(WebDriverErrorResponse::unknown_error(
                    "native posting timed out; delivery may be unknown",
                ))
            }
        }
    }
}

/// Register `WKWebView` handlers at webview creation time.
/// This is called from the plugin's `on_webview_ready` hook to ensure
/// the UI delegate is registered before any navigation completes.
pub fn register_webview_handlers<R: Runtime>(webview: &tauri::Webview<R>) {
    use objc2::ffi::{objc_setAssociatedObject, OBJC_ASSOCIATION_RETAIN_NONATOMIC};

    // Get per-window alert state from the manager
    let manager = webview.app_handle().state::<AlertStateManager>();
    let alert_state = manager.get_or_create(webview.label());

    let _ = webview.with_webview(move |webview| unsafe {
        let wk_webview: &WKWebView = &*webview.inner().cast();

        let delegate = WebDriverUIDelegate::new(alert_state);
        let delegate_protocol: Retained<ProtocolObject<dyn WKUIDelegate>> =
            ProtocolObject::from_retained(delegate);

        let _: () = msg_send![wk_webview, setUIDelegate: &*delegate_protocol];

        // Associate delegate with webview - released when webview is deallocated
        objc_setAssociatedObject(
            std::ptr::from_ref::<WKWebView>(wk_webview)
                .cast_mut()
                .cast(),
            std::ptr::addr_of!(DELEGATE_KEY).cast(),
            Retained::into_raw(delegate_protocol).cast(),
            OBJC_ASSOCIATION_RETAIN_NONATOMIC,
        );

        tracing::debug!("Registered UI delegate for webview");
    });
}

#[async_trait]
impl<R: Runtime + 'static> PlatformExecutor<R> for MacOSExecutor<R> {
    // =========================================================================
    // Window Access
    // =========================================================================

    fn window(&self) -> &WebviewWindow<R> {
        &self.window
    }

    fn script_timeout_ms(&self) -> u64 {
        self.timeouts.script_ms
    }

    async fn dispatch_pointer_event(
        &self,
        event_type: PointerEventType,
        x: i32,
        y: i32,
        button: u32,
        buttons: u32,
        modifiers: &ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        if button > 2 || buttons & !7 != 0 || matches!(event_type, PointerEventType::Click) {
            return Err(WebDriverErrorResponse::unsupported_operation(
                "unsupported native mouse button or synthetic click",
            ));
        }
        self.post_native(
            NativeInput::Pointer {
                event_type,
                button,
                buttons,
            },
            Some((x, y)),
            *modifiers,
        )
        .await
    }

    async fn dispatch_key_event(
        &self,
        key: &str,
        is_down: bool,
        modifiers: &ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        let (code, text) = native_key(key, modifiers)?;
        self.post_native(
            NativeInput::Key {
                code,
                text,
                down: is_down,
            },
            None,
            *modifiers,
        )
        .await
    }

    async fn dispatch_scroll_event(
        &self,
        x: i32,
        y: i32,
        delta_x: i32,
        delta_y: i32,
        modifiers: &ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        let dx = delta_x
            .checked_neg()
            .ok_or_else(|| WebDriverErrorResponse::invalid_argument("wheel delta out of range"))?;
        let dy = delta_y
            .checked_neg()
            .ok_or_else(|| WebDriverErrorResponse::invalid_argument("wheel delta out of range"))?;
        self.post_native(NativeInput::Wheel { dx, dy }, Some((x, y)), *modifiers)
            .await
    }

    // =========================================================================
    // Core JavaScript Execution
    // =========================================================================

    async fn evaluate_js(&self, script: &str) -> Result<Value, WebDriverErrorResponse> {
        let (tx, rx) = oneshot::channel();
        let script_owned = wrap_script_for_frame_context(script, &self.frame_context);

        let result = self.window.with_webview(move |webview| unsafe {
            let wk_webview: &WKWebView = &*webview.inner().cast();
            let ns_script = NSString::from_str(&script_owned);

            let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
            let block = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
                let response = if !error.is_null() {
                    let error_ref = &*error;
                    let description = error_ref.localizedDescription();
                    Err(description.to_string())
                } else if result.is_null() {
                    Ok(Value::Null)
                } else {
                    let obj = &*result;
                    Ok(ns_object_to_json(obj))
                };

                if let Ok(mut guard) = tx.lock() {
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(response);
                    }
                }
            });

            // Use evaluateJavaScript for script execution
            wk_webview.evaluateJavaScript_completionHandler(&ns_script, Some(&block));
        });

        if let Err(e) = result {
            return Err(WebDriverErrorResponse::javascript_error(
                &e.to_string(),
                None,
            ));
        }

        let timeout = std::time::Duration::from_millis(self.timeouts.script_ms);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(value))) => Ok(serde_json::json!({
                "success": true,
                "value": value
            })),
            Ok(Ok(Err(error))) => Err(WebDriverErrorResponse::javascript_error(&error, None)),
            Ok(Err(_)) => Err(WebDriverErrorResponse::unknown_error("Channel closed")),
            Err(_) => Err(WebDriverErrorResponse::script_timeout()),
        }
    }

    // =========================================================================
    // Async Script Execution (using callAsyncJavaScript)
    // =========================================================================

    async fn execute_async_script(
        &self,
        script: &str,
        args: &[Value],
    ) -> Result<Value, WebDriverErrorResponse> {
        let args_json = serde_json::to_string(args)
            .map_err(|e| WebDriverErrorResponse::invalid_argument(&e.to_string()))?;

        // Build wrapper that includes argument deserialization.
        // callAsyncJavaScript treats the body as function statements, so `return` is required —
        // without it the function returns undefined immediately and the Promise is discarded.
        let wrapper = format!(
            r"return new Promise((resolve, reject) => {{
                var ELEMENT_KEY = 'element-6066-11e4-a52e-4f735466cecf';
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
                var __done = function(result, error) {{
                    if (error) {{
                        reject(new Error(typeof error === 'string' ? error : String(error)));
                    }} else {{
                        resolve(result);
                    }}
                }};
                var __args = {args_json}.map(deserializeArg);
                __args.push(__done);
                try {{
                    (function() {{ {script} }}).apply(null, __args);
                }} catch (e) {{
                    reject(e);
                }}
            }})"
        );

        let (tx, rx) = oneshot::channel();

        let result = self.window.with_webview(move |webview| unsafe {
            let wk_webview: &WKWebView = &*webview.inner().cast();
            let ns_script = NSString::from_str(&wrapper);
            let mtm = MainThreadMarker::new_unchecked();

            // Empty dictionary for arguments (we pass args via JSON in the script)
            let empty_dict: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::new();

            // Get the page content world
            let content_world = WKContentWorld::pageWorld(mtm);

            let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
            let block = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
                let response = if !error.is_null() {
                    let error_ref = &*error;
                    let description = error_ref.localizedDescription();
                    Err(description.to_string())
                } else if result.is_null() {
                    Ok(Value::Null)
                } else {
                    let obj = &*result;
                    Ok(ns_object_to_json(obj))
                };

                if let Ok(mut guard) = tx.lock() {
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(response);
                    }
                }
            });

            wk_webview.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
                &ns_script,
                Some(&empty_dict),
                None,
                &content_world,
                Some(&block),
            );
        });

        if let Err(e) = result {
            return Err(WebDriverErrorResponse::javascript_error(
                &e.to_string(),
                None,
            ));
        }

        let timeout = std::time::Duration::from_millis(self.timeouts.script_ms);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(error))) => Err(WebDriverErrorResponse::javascript_error(&error, None)),
            Ok(Err(_)) => Err(WebDriverErrorResponse::unknown_error("Channel closed")),
            Err(_) => Err(WebDriverErrorResponse::script_timeout()),
        }
    }

    // =========================================================================
    // Screenshots
    // =========================================================================

    async fn take_screenshot(&self) -> Result<String, WebDriverErrorResponse> {
        let (tx, rx) = oneshot::channel();

        let result = self.window.with_webview(move |webview| unsafe {
            let wk_webview: &WKWebView = &*webview.inner().cast();
            let mtm = MainThreadMarker::new_unchecked();
            let config = WKSnapshotConfiguration::new(mtm);

            let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
            let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                let response = if !error.is_null() {
                    let error_ref = &*error;
                    let description = error_ref.localizedDescription();
                    Err(description.to_string())
                } else if image.is_null() {
                    Err("No image returned".to_string())
                } else {
                    let image_ref = &*image;
                    image_to_png_base64(image_ref)
                };

                if let Ok(mut guard) = tx.lock() {
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(response);
                    }
                }
            });

            wk_webview.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block);
        });

        if let Err(e) = result {
            return Err(WebDriverErrorResponse::unknown_error(&e.to_string()));
        }

        let timeout = std::time::Duration::from_millis(self.timeouts.script_ms);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(base64))) => Ok(base64),
            Ok(Ok(Err(error))) => Err(WebDriverErrorResponse::unknown_error(&error)),
            Ok(Err(_)) => Err(WebDriverErrorResponse::unknown_error("Channel closed")),
            Err(_) => Err(WebDriverErrorResponse::script_timeout()),
        }
    }

    async fn take_element_screenshot(
        &self,
        js_var: &str,
    ) -> Result<String, WebDriverErrorResponse> {
        // For element screenshots, we use JavaScript canvas approach
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}

                // Use html2canvas-like approach if available, otherwise scroll into view
                el.scrollIntoView({{ block: 'center', inline: 'center' }});

                // Return element bounds for clipping
                var rect = el.getBoundingClientRect();
                return {{
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height
                }};
            }})()"
        );
        self.evaluate_js(&script).await?;

        self.take_screenshot().await
    }

    // =========================================================================
    // Print
    // =========================================================================

    async fn print_page(&self, options: PrintOptions) -> Result<String, WebDriverErrorResponse> {
        // First, inject CSS @page rules for print settings
        let page_width = options.page_width.unwrap_or(21.0);
        let page_height = options.page_height.unwrap_or(29.7);
        let margin_top = options.margin_top.unwrap_or(1.0);
        let margin_bottom = options.margin_bottom.unwrap_or(1.0);
        let margin_left = options.margin_left.unwrap_or(1.0);
        let margin_right = options.margin_right.unwrap_or(1.0);
        let orientation = options.orientation.as_deref().unwrap_or("portrait");

        // Inject @page CSS rules
        let css_script = format!(
            r"(function() {{
                var style = document.createElement('style');
                style.id = '__webdriver_print_style';
                style.textContent = `
                    @page {{
                        size: {page_width}cm {page_height}cm {orientation};
                        margin: {margin_top}cm {margin_right}cm {margin_bottom}cm {margin_left}cm;
                    }}
                    @media print {{
                        body {{
                            -webkit-print-color-adjust: exact;
                            print-color-adjust: exact;
                        }}
                    }}
                `;
                document.head.appendChild(style);
                return true;
            }})()"
        );
        self.evaluate_js(&css_script).await?;

        // Now create PDF using WKWebView's native API
        let (tx, rx) = oneshot::channel();

        let result = self.window.with_webview(move |webview| unsafe {
            let wk_webview: &WKWebView = &*webview.inner().cast();
            let mtm = MainThreadMarker::new_unchecked();

            // Create PDF configuration
            let config = WKPDFConfiguration::new(mtm);
            // Note: WKPDFConfiguration only has rect and allowTransparentBackground
            // Page size/margins are handled via CSS @page rules above

            let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
            let block = RcBlock::new(move |data: *mut NSData, error: *mut NSError| {
                let response = if !error.is_null() {
                    let error_ref = &*error;
                    let description = error_ref.localizedDescription();
                    Err(description.to_string())
                } else if data.is_null() {
                    Err("No PDF data returned".to_string())
                } else {
                    let data_ref = &*data;
                    let bytes = data_ref.to_vec();
                    Ok(bytes)
                };

                if let Ok(mut guard) = tx.lock() {
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(response);
                    }
                }
            });

            wk_webview.createPDFWithConfiguration_completionHandler(Some(&config), &block);
        });

        if let Err(e) = result {
            return Err(WebDriverErrorResponse::unknown_error(&e.to_string()));
        }

        // Wait for result
        let timeout = std::time::Duration::from_millis(self.timeouts.script_ms);
        let pdf_result = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(bytes))) => Ok(bytes),
            Ok(Ok(Err(error))) => Err(WebDriverErrorResponse::unknown_error(&error)),
            Ok(Err(_)) => Err(WebDriverErrorResponse::unknown_error("Channel closed")),
            Err(_) => Err(WebDriverErrorResponse::script_timeout()),
        };

        // Clean up injected style
        let _ = self
            .evaluate_js(
                r"(function() {
                var style = document.getElementById('__webdriver_print_style');
                if (style) style.remove();
                return true;
            })()",
            )
            .await;

        // Return base64 encoded PDF
        pdf_result.map(|bytes| BASE64_STANDARD.encode(&bytes))
    }
}

// =============================================================================
// Utility Functions
// =============================================================================

/// Convert `NSImage` to PNG and encode as base64
unsafe fn image_to_png_base64(image: &NSImage) -> Result<String, String> {
    let tiff_data: Option<objc2::rc::Retained<NSData>> = image.TIFFRepresentation();
    let tiff_data = tiff_data.ok_or("Failed to get TIFF representation")?;

    let bitmap_rep = NSBitmapImageRep::imageRepWithData(&tiff_data)
        .ok_or("Failed to create bitmap image rep")?;

    let empty_dict: objc2::rc::Retained<NSDictionary<NSString>> = NSDictionary::new();
    let png_data: Option<objc2::rc::Retained<NSData>> =
        bitmap_rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &empty_dict);
    let png_data = png_data.ok_or("Failed to convert to PNG")?;

    let bytes = png_data.to_vec();
    Ok(BASE64_STANDARD.encode(&bytes))
}

/// Convert an `NSObject` to a JSON value
pub(super) unsafe fn ns_object_to_json(obj: &AnyObject) -> Value {
    use objc2_foundation::NSString as NSStr;

    let class = obj.class();
    let class_name = class.name().to_str().unwrap_or("");

    if class_name.contains("String") {
        let ns_str: &NSStr = &*std::ptr::from_ref::<AnyObject>(obj).cast::<NSStr>();
        return Value::String(ns_str.to_string());
    }

    if class_name.contains("Number") || class_name.contains("Boolean") {
        use objc2::msg_send;
        use objc2::runtime::Bool;

        if class_name.contains("Boolean") {
            let bool_val: Bool = msg_send![obj, boolValue];
            return Value::Bool(bool_val.as_bool());
        }

        let double_val: f64 = msg_send![obj, doubleValue];
        let int_val: i64 = msg_send![obj, longLongValue];

        #[allow(clippy::cast_precision_loss)]
        if (int_val as f64 - double_val).abs() < f64::EPSILON {
            return Value::Number(serde_json::Number::from(int_val));
        } else if let Some(n) = serde_json::Number::from_f64(double_val) {
            return Value::Number(n);
        }
        return Value::Null;
    }

    if class_name.contains("Array") {
        use objc2::msg_send;

        let count: usize = msg_send![obj, count];
        let mut arr = Vec::new();
        for i in 0..count {
            let item: *mut AnyObject = msg_send![obj, objectAtIndex: i];
            if !item.is_null() {
                arr.push(ns_object_to_json(&*item));
            }
        }
        return Value::Array(arr);
    }

    if class_name.contains("Dictionary") {
        use objc2::msg_send;

        let keys: *mut AnyObject = msg_send![obj, allKeys];
        if keys.is_null() {
            return Value::Object(serde_json::Map::new());
        }

        let count: usize = msg_send![keys, count];
        let mut map = serde_json::Map::new();

        for i in 0..count {
            let key: *mut AnyObject = msg_send![keys, objectAtIndex: i];
            if key.is_null() {
                continue;
            }

            let key_class = (&*key).class().name().to_str().unwrap_or("");
            if !key_class.contains("String") {
                continue;
            }

            let ns_key: &NSStr = &*key.cast_const().cast::<NSStr>();
            let key_str = ns_key.to_string();

            let val: *mut AnyObject = msg_send![obj, objectForKey: key];
            if !val.is_null() {
                map.insert(key_str, ns_object_to_json(&*val));
            }
        }
        return Value::Object(map);
    }

    if class_name.contains("Null") {
        return Value::Null;
    }

    Value::Null
}

// =============================================================================
// Native UI Delegate for JavaScript Alerts
// =============================================================================

/// Instance variables for UI delegate - stores per-window alert state
struct WebDriverUIDelegateIvars {
    alert_state: Arc<AlertState>,
}

// SAFETY: Arc<AlertState> is Send + Sync
unsafe impl Send for WebDriverUIDelegateIvars {}
unsafe impl Sync for WebDriverUIDelegateIvars {}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WebDriverUIDelegate"]
    #[ivars = WebDriverUIDelegateIvars]
    struct WebDriverUIDelegate;

    unsafe impl NSObjectProtocol for WebDriverUIDelegate {}

    #[allow(non_snake_case)]
    unsafe impl WKUIDelegate for WebDriverUIDelegate {
        /// Handle JavaScript `alert()` calls
        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn webView_runJavaScriptAlertPanelWithMessage_initiatedByFrame_completionHandler(
            &self,
            _webview: &WKWebView,
            message: &NSString,
            _frame: &WKFrameInfo,
            completion_handler: &DynBlock<dyn Fn()>,
        ) {
            let message_str = message.to_string();
            tracing::debug!("Intercepted alert: {message_str}");

            // Create channel for WebDriver response
            let (tx, rx) = std::sync::mpsc::channel();

            // Store alert state with responder (using per-window state from ivars)
            self.ivars().alert_state.set_pending(PendingAlert {
                message: message_str,
                default_text: None,
                alert_type: AlertType::Alert,
                responder: tx,
            });

            // Wait for accept/dismiss (with timeout)
            let timeout = std::time::Duration::from_secs(30);
            let _ = rx.recv_timeout(timeout);

            completion_handler.call(());
        }

        /// Handle JavaScript `confirm()` calls
        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn webView_runJavaScriptConfirmPanelWithMessage_initiatedByFrame_completionHandler(
            &self,
            _webview: &WKWebView,
            message: &NSString,
            _frame: &WKFrameInfo,
            completion_handler: &DynBlock<dyn Fn(objc2::runtime::Bool)>,
        ) {
            let message_str = message.to_string();
            tracing::debug!("Intercepted confirm: {message_str}");

            // Create channel for WebDriver response
            let (tx, rx) = std::sync::mpsc::channel();

            // Store confirm state with responder (using per-window state from ivars)
            self.ivars().alert_state.set_pending(PendingAlert {
                message: message_str,
                default_text: None,
                alert_type: AlertType::Confirm,
                responder: tx,
            });

            // Wait for accept/dismiss (with timeout)
            let timeout = std::time::Duration::from_secs(30);
            let response = rx.recv_timeout(timeout);

            // Return true if accepted, false if dismissed or timeout
            let accepted = response.map(|r| r.accepted).unwrap_or(true);

            completion_handler.call((objc2::runtime::Bool::from(accepted),));
        }

        /// Handle JavaScript `prompt()` calls
        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn webView_runJavaScriptTextInputPanelWithPrompt_defaultText_initiatedByFrame_completionHandler(
            &self,
            _webview: &WKWebView,
            prompt: &NSString,
            default_text: Option<&NSString>,
            _frame: &WKFrameInfo,
            completion_handler: &DynBlock<dyn Fn(*mut NSString)>,
        ) {
            let prompt_str = prompt.to_string();
            let default = default_text.map(std::string::ToString::to_string);
            tracing::debug!("Intercepted prompt: {prompt_str}");

            // Create channel for WebDriver response
            let (tx, rx) = std::sync::mpsc::channel();

            // Store prompt state with responder (using per-window state from ivars)
            self.ivars().alert_state.set_pending(PendingAlert {
                message: prompt_str,
                default_text: default.clone(),
                alert_type: AlertType::Prompt,
                responder: tx,
            });

            // Wait for accept/dismiss (with timeout)
            let timeout = std::time::Duration::from_secs(30);
            let response = rx.recv_timeout(timeout);

            // Return the prompt text if accepted, null if dismissed
            let result: *mut NSString = match response {
                Ok(r) if r.accepted => {
                    let text = r.prompt_text.or(default).unwrap_or_default();
                    let ns_str = NSString::from_str(&text);
                    Retained::into_raw(ns_str)
                }
                _ => std::ptr::null_mut(), // Dismissed or timeout = null (cancel)
            };

            completion_handler.call((result,));
        }
    }
);

impl WebDriverUIDelegate {
    /// # Safety
    /// Must be called from the main thread.
    unsafe fn new(alert_state: Arc<AlertState>) -> Retained<Self> {
        let mtm = MainThreadMarker::new_unchecked();
        let this = Self::alloc(mtm);
        let this = this.set_ivars(WebDriverUIDelegateIvars { alert_state });
        msg_send![super(this), init]
    }
}

#[cfg(test)]
mod native_input_mapping_tests {
    use super::*;

    #[test]
    fn keyboard_specials_and_unicode_are_distinct() {
        assert_eq!(
            native_key("\u{E003}", &ModifierState::default()).unwrap(),
            (51, None)
        );
        assert_eq!(
            native_key("é", &ModifierState::default()).unwrap(),
            (0, Some("é".into()))
        );
        assert!(native_key("\u{E099}", &ModifierState::default()).is_err());
        assert!(native_key("ab", &ModifierState::default()).is_err());
    }

    #[test]
    fn modified_unknown_position_is_refused() {
        let m = ModifierState {
            ctrl: true,
            ..Default::default()
        };
        assert!(native_key("é", &m).is_err());
        assert_eq!(native_key("k", &m).unwrap().0, 40);
    }

    #[test]
    fn native_flags_preserve_only_declared_modifiers() {
        let m = ModifierState {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            native_flags(m),
            CGEventFlags::MaskControl | CGEventFlags::MaskShift
        );
        assert!(native_flags(ModifierState::default()).is_empty());
    }
}
