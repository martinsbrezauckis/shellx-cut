use std::sync::Arc;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSBitmapImageFileType, NSBitmapImageRep,
    NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSEventTrackingRunLoopMode, NSImage, NSRunningApplication,
    NSScreen, NSView, NSWindow, NSWorkspace,
};
use objc2_core_foundation::{CFMachPort, CFRetained, CFRunLoop, CFRunLoopSource, CFString, kCFRunLoopCommonModes};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
    CGEventType, CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy, CGMouseButton, CGPreflightPostEventAccess, CGPreflightListenEventAccess, CGScrollEventUnit,
};
use objc2_foundation::{
    NSData, NSDictionary, NSError, NSObject, NSObjectProtocol, NSOperationQueue, NSPoint,
    NSProcessInfo, NSString,
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
    option_pointer: Arc<std::sync::Mutex<Option<OptionPointerPin>>>,
    option_completion_point: Arc<std::sync::Mutex<Option<OptionCompletionPoint>>>,
}

impl<R: Runtime> MacOSExecutor<R> {
    pub fn new(window: WebviewWindow<R>, timeouts: Timeouts, frame_context: Vec<FrameId>) -> Self {
        Self {
            window,
            timeouts,
            frame_context,
            option_pointer: Arc::new(std::sync::Mutex::new(None)),
            option_completion_point: Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

// Standard HID posting is the explicit Mac input route. It requires observed
// owned foreground/key-window readiness and actual posting permission, never a fallback.
enum NativeInput {
    Pointer {
        event_type: PointerEventType,
        button: u32,
        buttons: u32,
    },
    Key {
        key: String,
        code: u16,
        text: Option<String>,
        down: bool,
    },
    Wheel {
        dx: i32,
        dy: i32,
    },
}

// Only this executor's internal OPTION Down can create the release proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OptionPointerPin { marker: i64, window: isize, view: usize }
// Readiness metadata for this gesture, never held-button/release authority.
#[derive(Clone, Copy)]
struct OptionCompletionPoint {window:isize,view:usize,viewport:(i32,i32,f64,f64),screen:NSPoint}

fn option_point_readiness(pin:OptionCompletionPoint,window:isize,view:usize,screen:NSPoint,observed:isize)->Result<Option<String>,String> {
    if view!=pin.view || window!=pin.window {return Err("option original point owner changed".into());}
    if screen!=pin.screen {return Err("option original point geometry changed".into());}
    Ok((observed!=pin.window).then(||format!("option original point remains covered; expectedWindow={} observedWindow={observed}",pin.window)))
}

unsafe fn native_screen_point(wk:&WKWebView,window:&NSWindow,viewport:(i32,i32,f64,f64))->Result<NSPoint,String> {
    let (x,y,width,height)=viewport;
    let b=wk.bounds();let i=wk.safeAreaInsets();
    let full=(b.size.width-width).abs()<=1.0 && (b.size.height-height).abs()<=1.0;
    let inset=[i.left,i.right,i.top,i.bottom].iter().all(|n|n.is_finite() && *n>=0.0)
        && (b.size.width-i.left-i.right-width).abs()<=1.0 && (b.size.height-i.top-i.bottom-height).abs()<=1.0;
    if wk.pageZoom()!=1.0 || (!full && !inset) {return Err(format!("native/DOM viewport mismatch bounds={b:?} dom={width}x{height} insets={i:?}"));}
    let left=if full{0.0}else{i.left};let top=if full{0.0}else{i.top};
    let p=NSPoint::new(b.origin.x+left+f64::from(x),b.origin.y+if wk.isFlipped(){top+f64::from(y)}else{b.size.height-top-f64::from(y)});
    Ok(window.convertPointToScreen(wk.convertPoint_toView(p,None)))
}

fn option_release_admitted(pin: Option<OptionPointerPin>, window: isize, view: usize, held: bool) -> bool {
    pin.is_some_and(|p| p.marker > 0 && p.window == window && p.view == view) && held
}

fn native_buttons_held() -> bool {
    (0..32).any(|button| {
        CGEventSource::button_state(
            CGEventSourceStateID::CombinedSessionState,
            CGMouseButton(button),
        )
    })
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
        "\u{E031}" => Some(122), // F1
        "\u{E032}" => Some(120), // F2
        "\u{E033}" => Some(99),  // F3
        "\u{E034}" => Some(118), // F4
        "\u{E035}" => Some(96),  // F5
        "\u{E036}" => Some(97),  // F6
        "\u{E037}" => Some(98),  // F7
        "\u{E038}" => Some(100), // F8
        "\u{E039}" => Some(101), // F9
        "\u{E03A}" => Some(109), // F10
        "\u{E03B}" => Some(103), // F11
        "\u{E03C}" => Some(111), // F12
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
    // Shortcut characters must be translated by AppKit from virtual keycode
    // and modifiers. Unicode override is only the plain-text insertion route.
    let text = if modifiers.ctrl || modifiers.alt || modifiers.meta {
        None
    } else {
        Some(key.to_string())
    };
    Ok((code.unwrap_or(0), text))
}

impl<R: Runtime + 'static> MacOSExecutor<R> {
    async fn remove_popup_observer(&self, marker:i64) -> Result<(),WebDriverErrorResponse> {
        let (tx,rx)=oneshot::channel();
        self.window.run_on_main_thread(move || { remove_popup_tap(marker);let _=tx.send(()); })
            .map_err(|e|WebDriverErrorResponse::unknown_error(&e.to_string()))?;
        tokio::time::timeout(std::time::Duration::from_millis(2000),rx).await
            .map_err(|_|WebDriverErrorResponse::unknown_error("popup observer cleanup timed out"))?
            .map_err(|_|WebDriverErrorResponse::unknown_error("popup observer cleanup channel closed"))
    }

    async fn complete_key_release(
        &self,
        owner: (isize, String),
    ) -> Result<(), WebDriverErrorResponse> {
        let (tx, rx) = oneshot::channel();
        self.window
            .with_webview(move |view| unsafe {
                let wk: &WKWebView = &*view.inner().cast();
                let result = if wk
                    .window()
                    .is_some_and(|window| window.windowNumber() == owner.0)
                {
                    KEY_REPRESENTATIONS.with(|keys| keys.borrow_mut().remove(&owner));
                    Ok(())
                } else {
                    Err("native release owner changed before completion")
                };
                let _ = tx.send(result);
            })
            .map_err(|error| WebDriverErrorResponse::unknown_error(&error.to_string()))?;
        match tokio::time::timeout(std::time::Duration::from_secs(2), rx).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(message))) => Err(WebDriverErrorResponse::unknown_error(message)),
            _ => Err(WebDriverErrorResponse::unknown_error(
                "native key release bookkeeping completion unknown",
            )),
        }
    }

    async fn ensure_owned_foreground(&self) -> Result<(), WebDriverErrorResponse> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut request_activation = true;
        let mut expected_window = None;
        let mut last = String::from("not observed");
        loop {
            let (tx, rx) = oneshot::channel();
            self.window.with_webview(move |view| unsafe {
                let result=(|| -> Result<(isize,bool,String),String> {
                    let mtm=MainThreadMarker::new().ok_or("focus requires main thread")?;
                    let wk: &WKWebView=&*view.inner().cast();
                    let window=wk.window().ok_or("owned window absent")?;
                    let app=NSApplication::sharedApplication(mtm);
                    if expected_window.is_some_and(|id|id!=window.windowNumber()) {return Err("owned window changed during activation".into());}
                    if !window.isVisible() || window.isMiniaturized() || wk.isHiddenOrHasHiddenAncestor() {return Err("hidden/minimized window cannot receive input".into());}
                    let mut accepted=None;
                    if request_activation && !(app.isActive() && window.isKeyWindow()) && std::time::Instant::now()<deadline {
                        accepted=Some(NSRunningApplication::currentApplication().activateWithOptions(NSApplicationActivationOptions::empty()));
                        window.makeKeyAndOrderFront(None);
                    }
                    let frontmost=NSWorkspace::sharedWorkspace().frontmostApplication().map(|a| (
                        a.processIdentifier(),a.bundleIdentifier().map(|s|s.to_string()),a.localizedName().map(|s|s.to_string())));
                    let detail=format!("pid={} window={} policy={:?} active={} key={} canBecomeKey={} currentKey={:?} activationRequestAccepted={:?} frontmostPidBundleName={:?}",std::process::id(),window.windowNumber(),app.activationPolicy(),app.isActive(),window.isKeyWindow(),window.canBecomeKeyWindow(),app.keyWindow().map(|w|w.windowNumber()),accepted,frontmost);
                    if request_activation {eprintln!("ST14A_NATIVE_FOCUS {detail}");}
                    if accepted==Some(false) {return Err(format!("owned app activation refused: {detail}"));}
                    Ok((window.windowNumber(),app.isActive() && window.isKeyWindow(),detail))
                })();let _=tx.send(result);
            }).map_err(|e|WebDriverErrorResponse::unknown_error(&e.to_string()))?;
            let result = tokio::time::timeout(
                deadline.saturating_duration_since(std::time::Instant::now()),
                rx,
            )
            .await
            .map_err(|_| {
                WebDriverErrorResponse::element_not_interactable(&format!(
                    "owned foreground deadline: {last}"
                ))
            })?
            .map_err(|_| WebDriverErrorResponse::unknown_error("owned foreground channel closed"))?
            .map_err(|e| WebDriverErrorResponse::element_not_interactable(&e))?;
            expected_window = Some(result.0);
            last = result.2;
            if result.1 {
                return Ok(());
            }
            request_activation = false;
            if std::time::Instant::now() >= deadline {
                return Err(WebDriverErrorResponse::element_not_interactable(&format!(
                    "owned foreground not ready: {last}"
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    async fn post_native(
        &self,
        input: NativeInput,
        coords: Option<(i32, i32)>,
        modifiers: ModifierState,
        option_scope: bool,
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
        // Popup Up/keys must not reactivate a lost owner. The original Down
        // retains its established foreground setup and post-AppKit barrier.
        if !option_scope || matches!(&input, NativeInput::Pointer { event_type: PointerEventType::Down, .. }) {
            self.ensure_owned_foreground().await?;
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
        let option_state = self.option_pointer.clone();
        let completion_point = self.option_completion_point.clone();
        // macOS consumes Command key equivalents at application/system level;
        // their keyUp need not reach an NSEvent local monitor. Acknowledge that
        // particular release using native session key state, never a DOM claim.
        let command_release = Arc::new(std::sync::Mutex::new(None::<(isize, String, u16)>));
        let command_posted = command_release.clone();
        let released_owner = Arc::new(std::sync::Mutex::new(None::<(isize, String)>));
        let completed_release = released_owner.clone();
        let release_was_held = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let release_proof = release_was_held.clone();
        let completion = Arc::new(std::sync::Mutex::new(Some(tx)));
        let marker = DELIVERY_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let posted_completion = completion.clone();
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pending = cancelled.clone();
        let popup_observer = option_scope && matches!(&input,NativeInput::Key{..});
        let _observer_cleanup=popup_observer.then(||PopupObserverCleanup{window:self.window.clone(),marker,cancelled:cancelled.clone()});
        self.window.with_webview(move |webview| unsafe {
            let response = (|| -> Result<(),String> {
                let mtm=MainThreadMarker::new().ok_or("native input requires main thread")?;
                let wk: &WKWebView=&*webview.inner().cast();
                let window=wk.window().ok_or("owned webview has no native window")?;
                let app=NSApplication::sharedApplication(mtm);
                if !app.isActive() || !window.isKeyWindow() || !window.isVisible() || window.isMiniaturized() || wk.isHiddenOrHasHiddenAncestor() {
                    return Err("owned view/window must be visible and not minimized".into());
                }
                let option_down = option_scope && matches!(&input, NativeInput::Pointer { event_type: PointerEventType::Down, .. });
                let location = if let Some((x,y,width,height))=viewport {
                    let screen=native_screen_point(wk,&window,(x,y,width,height))?;
                    if !releasing && NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(screen,0,mtm)!=window.windowNumber() {
                        return Err("owned window is not frontmost at native input point".into());
                    }
                    if option_down {
                        *completion_point.lock().unwrap()=Some(OptionCompletionPoint {window:window.windowNumber(),view:wk as *const WKWebView as usize,viewport:(x,y,width,height),screen});
                    }
                    let screens=NSScreen::screens(mtm);let primary=screens.firstObject().ok_or("primary screen unavailable")?;
                    let frame=primary.frame();
                    NSPoint::new(screen.x,frame.origin.y+frame.size.height-screen.y)
                } else { NSPoint::new(0.0,0.0) };
                if pending.load(std::sync::atomic::Ordering::SeqCst) || !CGPreflightPostEventAccess() { return Err("native posting request expired or permission unavailable".into()); }
                if matches!(&input,NativeInput::Wheel{..}|NativeInput::Pointer{event_type:PointerEventType::Move,buttons:0,..}) && native_buttons_held() {
                    return Err("native no-button move/wheel requires no held mouse buttons".into());
                }
                let mut input=input;
                if let NativeInput::Key{key,code,text,down}=&mut input {
                    let owner=(window.windowNumber(),key.clone());
                    let previous=KEY_REPRESENTATIONS.with(|keys|keys.borrow().get(&owner).copied());
                    if !*down && previous.is_none() { return Err("key release has no retained native representation; no event posted".into()); }
                    let plain=!(modifiers.ctrl || modifiers.shift || modifiers.alt || modifiers.meta);
                    let character_route=use_character_route(previous,*down,plain,text.is_some());
                    if *down && KEY_REPRESENTATIONS.with(|keys|keys.borrow().len()>=256) { return Err("native held key representations exceed bound".into()); }
                    if character_route {
                        let responder=window.firstResponder();
                        if !responder.as_ref().and_then(|r|r.downcast_ref::<NSView>()).is_some_and(|view|view.isDescendantOf(wk)) {
                            return Err("native character key requires owned webview responder".into());
                        }
                        let characters=NSString::from_str(key);
                        let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                            if *down { NSEventType::KeyDown } else { NSEventType::KeyUp }, NSPoint::new(0.0,0.0),NSEventModifierFlags(native_flags(modifiers).0 as usize),NSProcessInfo::processInfo().systemUptime(),window.windowNumber(),None,&characters,&characters,false,*code
                        ).ok_or("native character key event unavailable")?;
                        // Pin the representation before dispatch; common owns the
                        // original window and key lifecycle across requests.
                        if *down { KEY_REPRESENTATIONS.with(|keys|keys.borrow_mut().insert(owner.clone(),KeyRepresentation::Character)); }
                        if *down { wk.keyDown(&event); } else { wk.keyUp(&event); *completed_release.lock().unwrap()=Some(owner); }
                        eprintln!("ST14A_NATIVE_DELIVERY route=character-responder down={down} ownerWindow={}",window.windowNumber());
                        if let Some(tx)=posted_completion.lock().unwrap().take() { let _=tx.send(Ok(())); }
                        return Ok(());
                    }
                    if let Some(KeyRepresentation::Hardware{code:original,..})=previous { *code=original; *text=None; }
                    if *down && previous.is_none() {
                        let mapped=native_key(key,&modifiers).map_err(|error|error.message)?;
                        *code=mapped.0;*text=mapped.1;
                        KEY_REPRESENTATIONS.with(|keys|keys.borrow_mut().insert(owner,KeyRepresentation::Hardware{code:*code,held:false}));
                    }
                }
                let option_up = option_scope && matches!(&input, NativeInput::Pointer { event_type: PointerEventType::Up, .. });
                let expected_view = wk as *const WKWebView as usize;
                let keyboard=matches!(&input,NativeInput::Key{..});
                let keyboard_transition=match &input { NativeInput::Key{key,code,down,..}=>Some((key.clone(),*code,*down)), _=>None };
                let no_button_move=matches!(&input,NativeInput::Pointer{event_type:PointerEventType::Move,buttons:0,..});
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
                    NativeInput::Key{code,text,down,..} => {
                        let responder=window.firstResponder();
                        let already_owned=responder.as_ref().and_then(|r|r.downcast_ref::<NSView>()).is_some_and(|view|view.isDescendantOf(wk));
                        eprintln!("ST14A_NATIVE_RESPONDER ownedDescendant={already_owned} class={:?}",responder.as_ref().map(|r|r.class().name()));
                        // Reassigning the WK wrapper while its content/editor
                        // view already owns focus can disturb a live selection.
                        if !popup_observer && !already_owned && !window.makeFirstResponder(Some(wk)) { return Err("owned webview refuses first responder".into()); }
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
                // This is the selected standard native route, not a fallback. The
                // caller holds the Runner desktop lease and common original-owner pin.
                eprintln!("ST14A_NATIVE_POST route=hid pid={} type={:?} point={:?} window={} active=true key=true postAccess=true",
                    std::process::id(),CGEvent::r#type(Some(&event)),location,window.windowNumber());
                CGEvent::set_integer_value_field(Some(&event),CGEventField::EventSourceUserData,marker);
                if popup_observer {
                    let expected_window=window.windowNumber();
                    let retained_view=Retained::retain(wk as *const WKWebView as *mut WKWebView).ok_or("owned view retention failed")?;
                    let expected_kind=CGEvent::r#type(Some(&event));
                    let field=if keyboard{CGEventField::KeyboardEventKeycode}else{CGEventField::MouseEventButtonNumber};
                    let detail=CGEvent::integer_value_field(Some(&event),field);
                    let flags=native_flags(modifiers);
                    let delivered=posted_completion.clone();let native_release=completed_release.clone();
                    install_popup_tap(marker,expected_kind,Box::new(move |kind,incoming| {
                        let result=(||->Result<(),String>{
                            if kind==CGEventType::TapDisabledByTimeout || kind==CGEventType::TapDisabledByUserInput {
                                return Err("popup process event tap disabled; delivery unknown".into());
                            }
                            let mtm=MainThreadMarker::new().ok_or("popup callback not on main thread")?;
                            let owner=retained_view.window().ok_or("popup owner window missing")?;
                            let app=NSApplication::sharedApplication(mtm);
                            let owner_valid=owner.windowNumber()==expected_window && app.isActive() && owner.isKeyWindow()
                                && owner.isVisible() && !owner.isMiniaturized() && !retained_view.isHiddenOrHasHiddenAncestor();
                            if !popup_event_matches(kind,expected_kind,
                                CGEvent::integer_value_field(Some(incoming),field),detail,
                                CGEvent::flags(Some(incoming)),flags,
                                CGEvent::integer_value_field(Some(incoming),CGEventField::EventTargetUnixProcessID),std::process::id(),owner_valid) {
                                return Err("marked popup event identity or owner changed".into());
                            }
                            if let Some((key,code,down))=&keyboard_transition {
                                // Diagnostic only: the enclosing tap already matched this exact
                                // marked control key and owned window. Conversion is optional and
                                // does not establish AppKit/menu consumption or change admission.
                                let representation=NSEvent::eventWithCGEvent(incoming).map(|event| {
                                    let units=|text:Option<Retained<NSString>>|text.map(|s|s.to_string().encode_utf16().take(8).collect::<Vec<_>>());
                                    (event.keyCode(),event.modifierFlags().bits(),units(event.characters()),units(event.charactersIgnoringModifiers()))
                                });
                                eprintln!("ST43_OPTION_KEY_REPRESENTATION marker={marker} expectedCode={code} observedCode={} cgFlags={} nsEvent={representation:?} afterAppKitClaimed=false",
                                    CGEvent::integer_value_field(Some(incoming),CGEventField::KeyboardEventKeycode),CGEvent::flags(Some(incoming)).bits());
                                let held=CGEventSource::key_state(CGEventSourceStateID::CombinedSessionState,*code);
                                if held!=*down{return Err("popup key state differs from observed transition".into());}
                                if *down {KEY_REPRESENTATIONS.with(|keys|keys.borrow_mut().insert((expected_window,key.clone()),KeyRepresentation::Hardware{code:*code,held}));}
                                else {*native_release.lock().unwrap()=Some((expected_window,key.clone()));}
                            }
                            eprintln!("ST43_OPTION_DELIVERY basis=own-process-cgevent-tap marker={marker} eventType={kind:?} ownerPid={} ownerWindow={expected_window} afterAppKitClaimed=false",std::process::id());
                            Ok(())
                        })();
                        if let Some(tx)=delivered.lock().unwrap().take(){let _=tx.send(result);}
                    }))?;
                    CGEvent::post(CGEventTapLocation::HIDEventTap,Some(&event));
                    return Ok(());
                }
                if let Some((key,code,false))=keyboard_transition.as_ref().filter(|_|modifiers.meta) {
                    let code=*code;
                    let held_pin=KEY_REPRESENTATIONS.with(|keys|matches!(keys.borrow().get(&(window.windowNumber(),key.clone())),Some(KeyRepresentation::Hardware{held:true,..})));
                    *command_posted.lock().unwrap()=Some((window.windowNumber(),key.clone(),code));
                    // Always allow one owner-bound release attempt, even if
                    // Down acknowledgement failed. Missing evidence must keep
                    // common held intent unknown, not prevent cleanup posting.
                    release_proof.store(held_pin && CGEventSource::key_state(CGEventSourceStateID::CombinedSessionState,code),std::sync::atomic::Ordering::SeqCst);
                    eprintln!("ST14A_NATIVE_RELEASE beforeHeld={} ownerWindow={expected_window}",CGEventSource::key_state(CGEventSourceStateID::CombinedSessionState,code),expected_window=window.windowNumber());
                    CGEvent::post(CGEventTapLocation::HIDEventTap,Some(&event));
                    if let Some(tx)=posted_completion.lock().unwrap().take() { let _=tx.send(Ok(())); }
                    return Ok(());
                }
                let expected_window=window.windowNumber();
                let delivered=posted_completion.clone();
                let native_release=completed_release.clone();
                let seen=Arc::new(std::sync::atomic::AtomicBool::new(false));
                let release_pin=option_state.clone();
                let diagnostic_owner=window.clone();
                let delivery_view=Retained::retain(wk as *const WKWebView as *mut WKWebView).ok_or("owned delivery view retention failed")?;
                let handler=RcBlock::new(move |pointer: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
                    let incoming=pointer.as_ref();
                    let marked=incoming.CGEvent().is_some_and(|cg|
                        CGEvent::integer_value_field(Some(&cg),CGEventField::EventSourceUserData)==marker);
                    let actual_window=incoming.windowNumber();
                    if marked { eprintln!("ST14A_NATIVE_DELIVERY marker={marker} eventType={:?} window={actual_window} expectedWindow={expected_window}",incoming.r#type()); }
                    if marked && !keyboard && actual_window!=expected_window {
                        // Diagnose only this exact owned marked input. The narrow
                        // missing-object mouse-move binding below is separately checked.
                        let event_window=MainThreadMarker::new().and_then(|mtm|incoming.window(mtm)).map(|w|
                            (format!("{:p}",&*w),w.class().name().to_string_lossy().chars().take(128).collect::<String>(),w.windowNumber(),w.level(),w.isVisible(),std::ptr::eq(&*w,&*diagnostic_owner)));
                        let fields=incoming.CGEvent().map(|cg|(
                            CGEvent::integer_value_field(Some(&cg),CGEventField::MouseEventWindowUnderMousePointer),
                            CGEvent::integer_value_field(Some(&cg),CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent)));
                        let mode=CFRunLoop::current().and_then(|r|r.current_mode()).map(|m|m.to_string().chars().take(128).collect::<String>());
                        eprintln!("ST43_MARKED_WINDOW_MISMATCH marker={marker} eventWindowNumber={actual_window} eventWindow={event_window:?} retainedOwner={:p} retainedOwnerNumber={expected_window} cgWindows={fields:?} runLoopMode={mode:?}",&*diagnostic_owner);
                    }
                    // Key equivalents are application events and may have no
                    // event window. Bind those to the still-active original key
                    // window; mouse events must identify that window directly.
                    let owner_key=MainThreadMarker::new().is_some_and(|mtm| {
                        let app=NSApplication::sharedApplication(mtm);
                        app.isActive() && app.keyWindow().is_some_and(|w|w.windowNumber()==expected_window)
                    });
                    let cg_bound=if marked && no_button_move && actual_window!=expected_window {
                        MainThreadMarker::new().is_some_and(|mtm| {
                            let app=NSApplication::sharedApplication(mtm);
                            let owner_valid=app.isActive() && diagnostic_owner.windowNumber()==expected_window && diagnostic_owner.isKeyWindow() && diagnostic_owner.isVisible()
                                && !diagnostic_owner.isMiniaturized() && !delivery_view.isHiddenOrHasHiddenAncestor()
                                && delivery_view.window().is_some_and(|w|std::ptr::eq(&*w,&*diagnostic_owner))
                                && app.keyWindow().is_some_and(|w|std::ptr::eq(&*w,&*diagnostic_owner));
                            incoming.CGEvent().is_some_and(|cg|cg_mouse_move_binding(
                                marked,no_button_move,incoming.r#type(),CGEvent::r#type(Some(&cg)),
                                CGEvent::location(Some(&cg)),location,incoming.window(mtm).is_some(),
                                (CGEvent::integer_value_field(Some(&cg),CGEventField::MouseEventWindowUnderMousePointer),
                                 CGEvent::integer_value_field(Some(&cg),CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent)),
                                expected_window,owner_valid,native_buttons_held()))
                        })
                    } else {false};
                    if cg_bound {eprintln!("ST43_NATIVE_DELIVERY_BINDING marker={marker} basis=own-cg-window-ids ownerWindow={expected_window} legacyEventWindow={actual_window} eventWindowObject=none");}
                    let matched=marked && (actual_window==expected_window || (keyboard && actual_window==0 && owner_key) || cg_bound);
                    if matched && !seen.swap(true,std::sync::atomic::Ordering::SeqCst) {
                        let delivered=delivered.clone();
                        let keyboard_transition=keyboard_transition.clone();
                        let native_release=native_release.clone();
                        let option_state=option_state.clone();
                        let after_dispatch=RcBlock::new(move || {
                            remove_delivery_monitor(marker);
                            let result=if let Some((key,code,down))=&keyboard_transition {
                                let held=CGEventSource::key_state(CGEventSourceStateID::CombinedSessionState,*code);
                                if *down && !held { Err("acknowledged key Down lacks native session held state".into()) }
                                else { KEY_REPRESENTATIONS.with(|keys| { if *down { keys.borrow_mut().insert((expected_window,key.clone()),KeyRepresentation::Hardware{code:*code,held}); } else { *native_release.lock().unwrap()=Some((expected_window,key.clone())); } }); Ok(()) }
                            } else if option_down {
                                if !CGEventSource::button_state(CGEventSourceStateID::CombinedSessionState, CGMouseButton::Left) {
                                    Err("acknowledged OPTION Down lacks native held button state".into())
                                } else {
                                    *option_state.lock().unwrap() = Some(OptionPointerPin { marker, window: expected_window, view: expected_view });
                                    Ok(())
                                }
                            } else if option_up {
                                if CGEventSource::button_state(CGEventSourceStateID::CombinedSessionState,CGMouseButton::Left) {Err("acknowledged OPTION Up still has held primary button".into())}
                                else {*option_state.lock().unwrap()=None;Ok(())}
                            } else { Ok(()) };
                            if let Some(tx)=delivered.lock().unwrap().take() { let _=tx.send(result); }
                        });
                        NSOperationQueue::mainQueue().addOperationWithBlock(&after_dispatch);
                    }
                    pointer.as_ptr()
                });
                let monitor=NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::Any,&handler)
                    .ok_or("owned event delivery monitor unavailable")?;
                DELIVERY_MONITORS.with(|items|items.borrow_mut().insert(marker,monitor));
                if option_up {
                    let mut pin=release_pin.lock().unwrap();
                    if !option_release_admitted(*pin,window.windowNumber(),expected_view,
                        CGEventSource::button_state(CGEventSourceStateID::CombinedSessionState,CGMouseButton::Left)) {
                        return Err("OPTION Up requires exact acknowledged Down and held primary button; no Up posted".into());
                    }
                    // Consume one release attempt before posting. Unknown delivery
                    // retains common held intent but cannot post this Up again.
                    pin.as_mut().unwrap().marker=0;
                }
                CGEvent::post(CGEventTapLocation::HIDEventTap,Some(&event));
                Ok(())
            })();
            if let Err(error)=response {
                remove_delivery_monitor(marker);
                if let Some(tx)=posted_completion.lock().unwrap().take() { let _=tx.send(Err(error)); }
            }
        }).map_err(|e|WebDriverErrorResponse::unknown_error(&e.to_string()))?;
        let outcome=tokio::time::timeout(
            std::time::Duration::from_millis(self.timeouts.script_ms.min(2000)),rx).await;
        if popup_observer { cancelled.store(true,std::sync::atomic::Ordering::SeqCst);self.remove_popup_observer(marker).await?; }
        match outcome {
            Ok(Ok(Ok(()))) => {
                let release_record = command_release.lock().unwrap().clone();
                if let Some((owner_window, key, code)) = release_record {
                    if !release_was_held.load(std::sync::atomic::Ordering::SeqCst) {
                        return Err(WebDriverErrorResponse::unknown_error("owned Command key release posted, but prior held state is unproven; release remains unknown"));
                    }
                    let deadline =
                        tokio::time::Instant::now() + std::time::Duration::from_millis(500);
                    loop {
                        if !CGEventSource::key_state(
                            CGEventSourceStateID::CombinedSessionState,
                            code,
                        ) {
                            eprintln!("ST14A_NATIVE_DELIVERY commandKeyRelease=session-state-transition-down-to-up");
                            *released_owner.lock().unwrap() = Some((owner_window, key));
                            break;
                        }
                        if tokio::time::Instant::now() >= deadline {
                            return Err(WebDriverErrorResponse::unknown_error(
                                "native Command key release remained held",
                            ));
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                    }
                }
                let release_owner = { released_owner.lock().unwrap().take() };
                if let Some(owner) = release_owner {
                    self.complete_key_release(owner).await?;
                }
                Ok(())
            }
            Ok(Ok(Err(e))) => Err(WebDriverErrorResponse::element_not_interactable(&e)),
            Ok(Err(_)) => Err(WebDriverErrorResponse::unknown_error(
                "native posting channel closed",
            )),
            Err(_) => {
                cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                // Main-thread cleanup also handles a queued post that timed out
                // before execution; the cancellation flag prevents that post.
                let _ = self
                    .window
                    .with_webview(move |_| remove_delivery_monitor(marker));
                Err(WebDriverErrorResponse::unknown_error(
                    "native event dispatch acknowledgement timed out; delivery may be unknown",
                ))
            }
        }
    }
}

// A no-button move may carry no AppKit window object after native menu use.
// Normalize only that observed representation; never apply to captured drags,
// buttons, wheel or keyboard, and keep the same local after-dispatch barrier.
fn cg_mouse_move_binding(marked:bool,no_button_move:bool,ns_type:NSEventType,cg_type:CGEventType,
    point:NSPoint,expected_point:NSPoint,has_window_object:bool,windows:(i64,i64),owner:isize,owner_valid:bool,buttons_held:bool)->bool {
    marked && no_button_move && ns_type==NSEventType::MouseMoved && cg_type==CGEventType::MouseMoved
        && point==expected_point && !has_window_object && owner>0 && windows==(owner as i64,owner as i64)
        && owner_valid && !buttons_held
}

// This observer is scoped to one marked event addressed to this process. It
// does not claim AppKit consumption; the original OPTION Down keeps its local
// monitor ordering barrier and the oracle still verifies actual trusted effects.
fn popup_event_matches(kind:CGEventType,expected:CGEventType,detail:i64,expected_detail:i64,flags:CGEventFlags,expected_flags:CGEventFlags,target:i64,pid:u32,owner:bool)->bool {
    let relevant=CGEventFlags::MaskShift|CGEventFlags::MaskControl|CGEventFlags::MaskAlternate|CGEventFlags::MaskCommand;
    kind==expected && detail==expected_detail && flags&relevant==expected_flags&relevant
        // A zero metadata field has no contrary PID claim; the tap itself is
        // created for the exact own PID. Any nonzero contradiction refuses.
        && (target==0 || target==i64::from(pid)) && owner
}
fn popup_first_receipt(expected:i64,marker:Option<i64>,seen:&std::cell::Cell<bool>)->bool {
    // None is used only for a native tap-disabled notification, never an input.
    (marker.is_none() || marker==Some(expected)) && !seen.replace(true)
}
struct PopupTapContext {marker:i64,seen:std::cell::Cell<bool>,callback:Box<dyn Fn(CGEventType,&CGEvent)>}
struct PopupTap {port:CFRetained<CFMachPort>,source:CFRetained<CFRunLoopSource>,run_loop:CFRetained<CFRunLoop>,tracking:CFRetained<CFString>,_context:Box<PopupTapContext>}
impl Drop for PopupTap {
    fn drop(&mut self) {unsafe {
        CGEvent::tap_enable(&self.port,false);self.port.invalidate();
        self.run_loop.remove_source(Some(&self.source),kCFRunLoopCommonModes);
        self.run_loop.remove_source(Some(&self.source),Some(&self.tracking));
    }}
}
thread_local! {static POPUP_TAPS:std::cell::RefCell<std::collections::HashMap<i64,PopupTap>>=Default::default();}
unsafe extern "C-unwind" fn popup_tap_callback(_proxy:CGEventTapProxy,kind:CGEventType,event:std::ptr::NonNull<CGEvent>,data:*mut std::ffi::c_void)->*mut CGEvent {
    let context=&*(data as *const PopupTapContext);
    let disabled=kind==CGEventType::TapDisabledByTimeout || kind==CGEventType::TapDisabledByUserInput;
    // Read no key data and retain nothing for unrelated events.
    let marker=if disabled{None}else{Some(CGEvent::integer_value_field(Some(event.as_ref()),CGEventField::EventSourceUserData))};
    if popup_first_receipt(context.marker,marker,&context.seen) {
        (context.callback)(kind,event.as_ref());
    }
    event.as_ptr()
}
fn install_popup_tap(marker:i64,kind:CGEventType,callback:Box<dyn Fn(CGEventType,&CGEvent)>)->Result<(),String> {unsafe {
    if POPUP_TAPS.with(|items|!items.borrow().is_empty()){return Err("popup observer already active".into());}
    let mut context=Box::new(PopupTapContext{marker,seen:std::cell::Cell::new(false),callback});
    let port=CGEvent::tap_create_for_pid(std::process::id() as i32,CGEventTapPlacement::TailAppendEventTap,CGEventTapOptions::ListenOnly,1u64<<kind.0,Some(popup_tap_callback),(&mut *context as *mut PopupTapContext).cast())
        .ok_or_else(||format!("own-process popup event tap unavailable; listenPreflight={}",CGPreflightListenEventAccess()))?;
    let source=CFMachPort::new_run_loop_source(None,Some(&port),0).ok_or("popup run-loop source unavailable")?;
    let run_loop=CFRunLoop::main().ok_or("main run loop unavailable")?;
    let tracking=CFString::from_str(&NSEventTrackingRunLoopMode.to_string());
    let holder=PopupTap{port,source,run_loop,tracking,_context:context};
    holder.run_loop.add_source(Some(&holder.source),kCFRunLoopCommonModes);
    holder.run_loop.add_source(Some(&holder.source),Some(&holder.tracking));
    CGEvent::tap_enable(&holder.port,true);
    if !holder.port.is_valid() || !CGEvent::tap_is_enabled(&holder.port){return Err("popup event tap not enabled".into());}
    POPUP_TAPS.with(|items|items.borrow_mut().insert(marker,holder));Ok(())
}}
fn remove_popup_tap(marker:i64) {POPUP_TAPS.with(|items|{items.borrow_mut().remove(&marker);});}
struct PopupObserverCleanup<R:Runtime> {window:WebviewWindow<R>,marker:i64,cancelled:Arc<std::sync::atomic::AtomicBool>}
impl<R:Runtime> Drop for PopupObserverCleanup<R> {
    fn drop(&mut self) {self.cancelled.store(true,std::sync::atomic::Ordering::SeqCst);let marker=self.marker;let _=self.window.run_on_main_thread(move ||remove_popup_tap(marker));}
}

// A monitor is only an acknowledgement of our marked event entering AppKit.
// Queue completion after that dispatch so a following Up cannot change the HID
// button state before WebKit constructs the preceding Down event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyRepresentation {
    Character,
    Hardware { code: u16, held: bool },
}
fn use_character_route(
    previous: Option<KeyRepresentation>,
    down: bool,
    plain: bool,
    printable: bool,
) -> bool {
    match previous {
        Some(KeyRepresentation::Character) => true,
        Some(KeyRepresentation::Hardware { .. }) => false,
        None => down && plain && printable,
    }
}
thread_local! {
    static KEY_REPRESENTATIONS: std::cell::RefCell<std::collections::HashMap<(isize,String),KeyRepresentation>> = Default::default();
    static DELIVERY_MONITORS: std::cell::RefCell<std::collections::HashMap<i64, Retained<AnyObject>>> = Default::default();
}
static DELIVERY_SEQUENCE: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
fn remove_delivery_monitor(marker: i64) {
    DELIVERY_MONITORS.with(|items| {
        if let Some(monitor) = items.borrow_mut().remove(&marker) {
            unsafe { NSEvent::removeMonitor(&monitor) };
        }
    });
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
    fn preflight_key_event(&self, key: &str) -> Result<(), WebDriverErrorResponse> {
        native_key(key, &ModifierState::default()).map(|_| ())
    }

    fn option_defer_primary_release(&self) -> bool { true }

    fn clear_option_completion(&self) { *self.option_completion_point.lock().unwrap()=None; }

    async fn option_completion_readiness(&self) -> Result<Option<WebDriverErrorResponse>,WebDriverErrorResponse> {
        let pin=(*self.option_completion_point.lock().unwrap()).ok_or_else(||WebDriverErrorResponse::unknown_error("option original point missing"))?;
        let value=self.evaluate_js("({width:innerWidth,height:innerHeight,scale:visualViewport?visualViewport.scale:1,x:visualViewport?visualViewport.offsetLeft:0,y:visualViewport?visualViewport.offsetTop:0})").await?;
        let v=&value["value"];
        if v["width"].as_f64()!=Some(pin.viewport.2) || v["height"].as_f64()!=Some(pin.viewport.3) || v["scale"].as_f64()!=Some(1.0) || v["x"].as_f64()!=Some(0.0) || v["y"].as_f64()!=Some(0.0) {
            return Err(WebDriverErrorResponse::unknown_error("option viewport changed during completion"));
        }
        let (tx,rx)=oneshot::channel();
        self.window.with_webview(move |view| unsafe {
            let result=(||->Result<Option<String>,String>{
                let mtm=MainThreadMarker::new().ok_or("option readiness not on main thread")?;
                let wk:&WKWebView=&*view.inner().cast();
                let window=wk.window().ok_or("option readiness window missing")?;
                if wk as *const WKWebView as usize!=pin.view || window.windowNumber()!=pin.window {return Err("option original point owner changed".into());}
                let screen=native_screen_point(wk,&window,pin.viewport)?;
                let observed=NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(screen,0,mtm);
                option_point_readiness(pin,window.windowNumber(),wk as *const WKWebView as usize,screen,observed)
            })();
            let _=tx.send(result);
        }).map_err(|e|WebDriverErrorResponse::unknown_error(&e.to_string()))?;
        rx.await.map_err(|_|WebDriverErrorResponse::unknown_error("option readiness channel closed"))?
            .map(|pending|pending.map(|m|WebDriverErrorResponse::unknown_error(&m)))
            .map_err(|e|WebDriverErrorResponse::unknown_error(&e))
    }

    async fn option_completion_owner(&self) -> Result<String,WebDriverErrorResponse> {
        let (tx,rx)=oneshot::channel();
        self.window.with_webview(move |view| unsafe {
            let result=(||->Result<String,String>{
                let mtm=MainThreadMarker::new().ok_or("option owner check not on main thread")?;
                let wk:&WKWebView=&*view.inner().cast();
                let window=wk.window().ok_or("option owner window missing")?;
                if !NSApplication::sharedApplication(mtm).isActive() || !window.isKeyWindow() || !window.isVisible() || window.isMiniaturized() || wk.isHiddenOrHasHiddenAncestor() {
                    return Err("option completion owner is no longer active and visible".into());
                }
                Ok(format!("{}:{:p}",window.windowNumber(),wk))
            })();
            let _=tx.send(result);
        }).map_err(|e|WebDriverErrorResponse::unknown_error(&e.to_string()))?;
        rx.await.map_err(|_|WebDriverErrorResponse::unknown_error("option owner observation channel closed"))?
            .map_err(|e|WebDriverErrorResponse::unknown_error(&e))
    }

    async fn dispatch_option_pointer_event(
        &self, event_type: PointerEventType, x: i32, y: i32, button: u32,
        buttons: u32, modifiers: &ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        if button != 0 || !matches!((event_type, buttons), (PointerEventType::Down, 1) | (PointerEventType::Up, 0)) {
            return Err(WebDriverErrorResponse::unsupported_operation("OPTION scope requires balanced primary pointer input"));
        }
        self.post_native(NativeInput::Pointer { event_type, button, buttons }, Some((x,y)), *modifiers, true).await
    }

    async fn dispatch_option_key_event(&self,key:&str,is_down:bool,modifiers:&ModifierState)->Result<(),WebDriverErrorResponse> {
        if !matches!(key,"\u{E011}"|"\u{E015}"|"\u{E007}"|"\u{E00C}") || (modifiers.ctrl || modifiers.shift || modifiers.alt || modifiers.meta) {
            return Err(WebDriverErrorResponse::unsupported_operation("OPTION scope requires unmodified bounded popup keys"));
        }
        let (code,text)=native_key(key,modifiers)?;
        self.post_native(NativeInput::Key{key:key.to_owned(),code,text,down:is_down},None,*modifiers,true).await
    }

    fn option_popup_first_key(&self) -> Result<&'static str, WebDriverErrorResponse> {
        // Existing native Home mapping; popup delivery is separately qualified.
        Ok("\u{E011}")
    }

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
            false,
        )
        .await
    }

    async fn dispatch_key_event(
        &self,
        key: &str,
        is_down: bool,
        modifiers: &ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        // Up resolves its original native representation on the owned main
        // thread; modifiers may have changed since Down.
        let (code, text) = native_key(key, &ModifierState::default())?;
        self.post_native(
            NativeInput::Key {
                key: key.to_owned(),
                code,
                text,
                down: is_down,
            },
            None,
            *modifiers,
            false,
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
        // Quartz wheel delivery follows the native pointer location. Establish
        // that location through the same acknowledged owned move before scroll.
        // The scroll API has no held-button mask; refuse rather than turn this
        // positioning move into an unrequested drag or release.
        if native_buttons_held() {
            return Err(WebDriverErrorResponse::unsupported_operation(
                "native wheel while mouse buttons are held is unsupported",
            ));
        }
        self.post_native(
            NativeInput::Pointer {
                event_type: PointerEventType::Move,
                button: 0,
                buttons: 0,
            },
            Some((x, y)),
            *modifiers,
            false,
        )
        .await?;
        self.post_native(NativeInput::Wheel { dx, dy }, Some((x, y)), *modifiers, false)
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
    #[test]
    fn popup_receipt_ignores_unrelated_markers_and_completes_once() {
        let seen=std::cell::Cell::new(false);
        assert!(!super::popup_first_receipt(12,Some(13),&seen));assert!(!seen.get());
        assert!(super::popup_first_receipt(12,Some(12),&seen));
        assert!(!super::popup_first_receipt(12,Some(12),&seen));
        let disabled=std::cell::Cell::new(false);assert!(super::popup_first_receipt(12,None,&disabled));
        assert!(!super::popup_first_receipt(12,Some(12),&disabled));
    }

    #[test]
    fn popup_receipt_requires_exact_transition_and_retained_owner() {
        let kind=CGEventType::KeyDown;let flags=CGEventFlags::empty();
        assert!(super::popup_event_matches(kind,kind,115,115,flags,flags,42,42,true));
        assert!(super::popup_event_matches(kind,kind,115,115,flags,flags,0,42,true));
        assert!(!super::popup_event_matches(CGEventType::KeyUp,kind,115,115,flags,flags,42,42,true));
        assert!(!super::popup_event_matches(kind,kind,116,115,flags,flags,42,42,true));
        assert!(!super::popup_event_matches(kind,kind,115,115,CGEventFlags::MaskShift,flags,42,42,true));
        assert!(!super::popup_event_matches(kind,kind,115,115,flags,flags,43,42,true));
        assert!(!super::popup_event_matches(kind,kind,115,115,flags,flags,42,42,false));
        assert!(!super::popup_event_matches(CGEventType::TapDisabledByTimeout,kind,115,115,flags,flags,42,42,true));
    }

    #[test]
    fn cg_move_binding_requires_exact_missing_object_representation() {
        let point=NSPoint::new(10.0,20.0);
        let valid=|marked,no_buttons,ns,cg,p,object,ids,owner,live,held|super::cg_mouse_move_binding(marked,no_buttons,ns,cg,p,point,object,ids,owner,live,held);
        assert!(valid(true,true,NSEventType::MouseMoved,CGEventType::MouseMoved,point,false,(12,12),12,true,false));
        for (marked,no_buttons,object,ids,owner,live,held) in [
            (false,true,false,(12,12),12,true,false),(true,false,false,(12,12),12,true,false),
            (true,true,true,(12,12),12,true,false),(true,true,false,(0,12),12,true,false),
            (true,true,false,(12,13),12,true,false),(true,true,false,(13,12),12,true,false),
            (true,true,false,(0,0),0,true,false),(true,true,false,(12,12),13,true,false),
            (true,true,false,(12,12),12,false,false),(true,true,false,(12,12),12,true,true)] {
            assert!(!valid(marked,no_buttons,NSEventType::MouseMoved,CGEventType::MouseMoved,point,object,ids,owner,live,held));
        }
        assert!(!valid(true,true,NSEventType::LeftMouseDragged,CGEventType::MouseMoved,point,false,(12,12),12,true,false));
        assert!(!valid(true,true,NSEventType::MouseMoved,CGEventType::LeftMouseDown,point,false,(12,12),12,true,false));
        assert!(!valid(true,true,NSEventType::MouseMoved,CGEventType::MouseMoved,NSPoint::new(11.0,20.0),false,(12,12),12,true,false));
    }

    #[test]
    fn original_option_point_readiness_only_waits_for_numeric_coverage() {
        let point=NSPoint::new(10.0,20.0);
        let pin=super::OptionCompletionPoint {window:12,view:34,viewport:(1,2,100.0,200.0),screen:point};
        assert_eq!(super::option_point_readiness(pin,12,34,point,12).unwrap(),None);
        let mismatch=super::option_point_readiness(pin,12,34,point,99).unwrap().unwrap();
        assert!(mismatch.contains("expectedWindow=12 observedWindow=99"));
        assert!(super::option_point_readiness(pin,12,35,point,12).is_err());
        assert!(super::option_point_readiness(pin,13,34,point,12).is_err());
        assert!(super::option_point_readiness(pin,12,34,NSPoint::new(11.0,20.0),12).is_err());
    }

    #[test]
    fn option_release_requires_acknowledged_exact_owner_and_held_button() {
        let pin = super::OptionPointerPin { marker: 7, window: 12, view: 34 };
        assert!(super::option_release_admitted(Some(pin), 12, 34, true));
        assert!(!super::option_release_admitted(None, 12, 34, true));
        assert!(!super::option_release_admitted(Some(pin), 13, 34, true));
        assert!(!super::option_release_admitted(Some(pin), 12, 35, true));
        assert!(!super::option_release_admitted(Some(pin), 12, 34, false));
        assert!(!super::option_release_admitted(Some(super::OptionPointerPin { marker: 0, ..pin }), 12, 34, true));
    }
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
        let codes = [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111];
        for (index, code) in codes.into_iter().enumerate() {
            let webdriver_key = char::from_u32(0xE031 + index as u32).unwrap().to_string();
            assert_eq!(native_key(&webdriver_key, &ModifierState::default()).unwrap(), (code, None));
        }
        assert!(native_key("\u{E030}", &ModifierState::default()).is_err());
    }

    #[test]
    fn modified_unknown_position_is_refused() {
        let m = ModifierState {
            ctrl: true,
            ..Default::default()
        };
        assert!(native_key("é", &m).is_err());
        assert_eq!(native_key("k", &m).unwrap(), (40, None));
        assert_eq!(
            native_key(
                "a",
                &ModifierState {
                    meta: true,
                    ..Default::default()
                }
            )
            .unwrap(),
            (0, None)
        );
    }

    #[test]
    fn original_key_representation_survives_modifier_changes_and_repeat() {
        for down in [true, false] {
            assert!(use_character_route(
                Some(KeyRepresentation::Character),
                down,
                false,
                true
            ));
            assert!(!use_character_route(
                Some(KeyRepresentation::Hardware {
                    code: 0,
                    held: true
                }),
                down,
                true,
                true
            ));
        }
    }

    #[test]
    fn new_character_pair_requires_plain_printable_down() {
        assert!(use_character_route(None, true, true, true));
        assert!(!use_character_route(None, false, true, true));
        assert!(!use_character_route(None, true, false, true));
        assert!(!use_character_route(None, true, true, false));
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
