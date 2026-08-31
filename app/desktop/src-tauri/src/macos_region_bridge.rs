//! macOS foreground Region-picker bridge owned by the Tauri shell.
//!
//! AppKit interaction stays in the foreground window owner. A selected native
//! crop crosses once to this shell's spawned cutd child over a secret,
//! desktop/cutd epoch, PID, and request-correlation bound loopback lane; cutd
//! then mints and burns its own ticket before ordinary reservation. Neither
//! the browser nor an adopted external engine receives a reusable capability.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub(crate) const ENV_BRIDGE_SECRET: &str = "SHELLX_CUT_REGION_BRIDGE_SECRET";
pub(crate) const ENV_DESKTOP_EPOCH: &str = "SHELLX_CUT_REGION_DESKTOP_EPOCH";
pub(crate) const ENV_CUTD_EPOCH: &str = "SHELLX_CUT_REGION_CUTD_EPOCH";
pub(crate) const ENV_DESKTOP_PID: &str = "SHELLX_CUT_REGION_DESKTOP_PID";

const BRIDGE_PATH: &str = "/api/private/foreground-region-start";
const BRIDGE_HEADER: &str = "X-ShellX-Cut-Region-Bridge";
const DESKTOP_EPOCH_HEADER: &str = "X-ShellX-Cut-Desktop-Epoch";
const CUTD_EPOCH_HEADER: &str = "X-ShellX-Cut-Cutd-Epoch";
const MAX_RESPONSE_BYTES: u64 = 1_048_576;
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Process-private material known only to this shell and the child it spawned.
/// It is deliberately absent from `engine_status` and every browser payload.
#[derive(Clone)]
pub(crate) struct ForegroundRegionBridge {
    addr: String,
    secret: String,
    desktop_epoch: String,
    cutd_epoch: String,
    child_pid: Option<u32>,
}

impl ForegroundRegionBridge {
    pub(crate) fn for_spawned_child(addr: String) -> Result<Self, String> {
        Ok(Self {
            addr,
            secret: random_hex("region_bridge_", 32)?,
            desktop_epoch: random_hex("desktop_epoch_", 32)?,
            cutd_epoch: random_hex("cutd_epoch_", 32)?,
            child_pid: None,
        })
    }

    pub(crate) fn bind_spawned_child(&mut self, child_pid: u32) -> Result<(), String> {
        if child_pid == 0 || self.child_pid.replace(child_pid).is_some() {
            return Err("could not bind the foreground Region bridge to cutd".to_string());
        }
        Ok(())
    }

    pub(crate) fn apply_to_child(&self, command: &mut std::process::Command) {
        command
            .env(ENV_BRIDGE_SECRET, &self.secret)
            .env(ENV_DESKTOP_EPOCH, &self.desktop_epoch)
            .env(ENV_CUTD_EPOCH, &self.cutd_epoch)
            .env(ENV_DESKTOP_PID, std::process::id().to_string());
    }

    /// Submit a platform-private selection without making it browser-visible.
    pub(crate) fn submit<S: Serialize>(
        &self,
        selection: S,
        start: RegionStartOptions,
    ) -> Result<Value, String> {
        let child_pid = self.child_pid.ok_or_else(|| {
            "the foreground Region bridge has no spawned cutd identity".to_string()
        })?;
        let request_id = random_hex("region_request_", 32)?;
        let body = serde_json::to_vec(&json!({
            "request_id": request_id,
            "desktop_epoch": self.desktop_epoch,
            "cutd_epoch": self.cutd_epoch,
            "selection": selection,
            "start": start,
        }))
        .map_err(|_| "could not encode the foreground Region admission".to_string())?;

        let mut stream = TcpStream::connect(&self.addr)
            .map_err(|_| "the foreground Region engine is no longer available".to_string())?;
        let _ = stream.set_read_timeout(Some(BRIDGE_TIMEOUT));
        let _ = stream.set_write_timeout(Some(BRIDGE_TIMEOUT));
        let request = format!(
            "POST {BRIDGE_PATH} HTTP/1.1\r\n\
             Host: {}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             {BRIDGE_HEADER}: {}\r\n\
             {DESKTOP_EPOCH_HEADER}: {}\r\n\
             {CUTD_EPOCH_HEADER}: {}\r\n\
             X-ShellX-Cut-Cutd-Pid: {child_pid}\r\n\
             Connection: close\r\n\r\n",
            self.addr,
            body.len(),
            self.secret,
            self.desktop_epoch,
            self.cutd_epoch,
        );
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.write_all(&body))
            .map_err(|_| "the foreground Region admission did not reach cutd".to_string())?;

        let mut raw = Vec::new();
        Read::by_ref(&mut stream)
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut raw)
            .map_err(|_| "cutd did not answer the foreground Region admission".to_string())?;
        if raw.len() as u64 > MAX_RESPONSE_BYTES {
            return Err("cutd returned an oversized foreground Region response".to_string());
        }
        let raw = String::from_utf8(raw)
            .map_err(|_| "cutd returned a malformed foreground Region response".to_string())?;
        if http_status(&raw) != Some(200) {
            return Err("the foreground Region admission was refused".to_string());
        }
        let (_, body) = raw
            .split_once("\r\n\r\n")
            .ok_or_else(|| "cutd returned a malformed foreground Region response".to_string())?;
        serde_json::from_str(body)
            .map_err(|_| "cutd returned an invalid foreground Region response".to_string())
    }
}

/// Start choices intentionally mirror only the ordinary capture reservation's
/// safe non-target fields. There is no monitor, window, region, ticket, path,
/// or rationale field for browser content to smuggle into the private bridge.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegionStartOptions {
    pub(crate) duration_ms: Option<u64>,
    pub(crate) fps: Option<f64>,
    pub(crate) quality: Option<RegionQuality>,
    #[serde(default)]
    pub(crate) audio: bool,
    #[serde(default)]
    pub(crate) system_audio: bool,
    pub(crate) studio: Option<Value>,
    #[serde(default)]
    pub(crate) keys: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegionQuality {
    output_size: String,
    profile: String,
}

impl RegionStartOptions {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.duration_ms == Some(0) {
            return Err("duration_ms must be at least 1 when provided".to_string());
        }
        if let Some(fps) = self.fps {
            if !fps.is_finite() || !(1.0..=240.0).contains(&fps) {
                return Err("fps must be finite and between 1 and 240".to_string());
            }
        }
        if let Some(quality) = &self.quality {
            if !matches!(quality.output_size.as_str(), "source" | "1080p" | "720p")
                || !matches!(quality.profile.as_str(), "standard" | "high")
            {
                return Err("quality must use a supported output_size and profile".to_string());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Serialize)]
struct WireRegionSelection {
    display_id: u32,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    parent_width: u32,
    parent_height: u32,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct NativePickerSelection {
    display_id: u32,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    parent_width: u32,
    parent_height: u32,
}

#[cfg(target_os = "macos")]
extern "C" {
    fn sxc_macos_region_picker_is_main_thread() -> i32;
    fn sxc_macos_region_picker_present(selection: *mut NativePickerSelection) -> i32;
}

#[cfg(target_os = "macos")]
const PICKED: i32 = 0;
#[cfg(target_os = "macos")]
const CANCELLED: i32 = 1;
#[cfg(target_os = "macos")]
const REFUSED: i32 = 2;

#[cfg(target_os = "macos")]
struct AppKitMainThreadOwner(std::marker::PhantomData<std::rc::Rc<()>>);

#[cfg(target_os = "macos")]
impl AppKitMainThreadOwner {
    fn current() -> Result<Self, String> {
        // SAFETY: this queries only whether the current callback is on AppKit's
        // main thread. The non-Send proof remains inside that callback.
        (unsafe { sxc_macos_region_picker_is_main_thread() } != 0)
            .then_some(Self(std::marker::PhantomData))
            .ok_or_else(|| "the Region picker requires the AppKit main thread".to_string())
    }
}

#[cfg(target_os = "macos")]
fn choose_region_on_foreground_main_thread(
    app: &tauri::AppHandle,
) -> Result<Option<WireRegionSelection>, String> {
    use tauri::Manager;

    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "the foreground ShellX Cut window is unavailable".to_string())?;
    if !window.is_focused().unwrap_or(false) {
        return Err(
            "Region selection is allowed only from the foreground ShellX Cut window".to_string(),
        );
    }
    let _owner = AppKitMainThreadOwner::current()?;
    let mut selection = NativePickerSelection {
        display_id: 0,
        left: 0,
        top: 0,
        width: 0,
        height: 0,
        parent_width: 0,
        parent_height: 0,
    };
    // SAFETY: the AppKit-main-thread proof above and the native bridge's own
    // check guarantee that the synchronous modal panel stays foreground-owned.
    match unsafe { sxc_macos_region_picker_present(&mut selection) } {
        PICKED => valid_native_selection(selection).map(Some),
        CANCELLED => Ok(None),
        REFUSED => Err("the Region selection lost foreground focus and was refused".to_string()),
        _ => Err("the native Region picker did not finish successfully".to_string()),
    }
}

#[cfg(target_os = "macos")]
fn valid_native_selection(selection: NativePickerSelection) -> Result<WireRegionSelection, String> {
    let right = selection.left.checked_add(selection.width);
    let bottom = selection.top.checked_add(selection.height);
    let valid = selection.display_id != 0
        && selection.parent_width > 0
        && selection.parent_height > 0
        && selection.left.is_multiple_of(2)
        && selection.top.is_multiple_of(2)
        && selection.width >= 2
        && selection.height >= 2
        && selection.width.is_multiple_of(2)
        && selection.height.is_multiple_of(2)
        && right.is_some_and(|right| right <= selection.parent_width)
        && bottom.is_some_and(|bottom| bottom <= selection.parent_height);
    valid
        .then_some(WireRegionSelection {
            display_id: selection.display_id,
            left: selection.left,
            top: selection.top,
            width: selection.width,
            height: selection.height,
            parent_width: selection.parent_width,
            parent_height: selection.parent_height,
        })
        .ok_or_else(|| "the native Region picker returned an invalid physical crop".to_string())
}

/// The only real foreground call site. It is intentionally absent from the UI
/// until the compiled/native path is qualified, but its exact-origin Tauri
/// permission lets a reviewed future UI owner use the same verified bridge.
#[cfg(target_os = "macos")]
#[tauri::command]
pub(crate) async fn start_macos_region_capture(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::ForegroundRegionBridgeState>,
    start: RegionStartOptions,
) -> Result<Value, String> {
    start.validate()?;
    let bridge = state
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .ok_or_else(|| {
            "Region capture requires this desktop's spawned foreground engine".to_string()
        })?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let picker_app = app.clone();
    app.run_on_main_thread(move || {
        let _ = sender.send(choose_region_on_foreground_main_thread(&picker_app));
    })
    .map_err(|_| "could not schedule the foreground Region picker".to_string())?;
    let Some(selection) = receiver
        .await
        .map_err(|_| "the foreground Region picker did not return".to_string())??
    else {
        return Ok(json!({"ok": false, "cancelled": true}));
    };
    tauri::async_runtime::spawn_blocking(move || bridge.submit(selection, start))
        .await
        .map_err(|_| "the foreground Region admission worker stopped unexpectedly".to_string())?
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(crate) async fn start_macos_region_capture(
    _app: tauri::AppHandle,
    _state: tauri::State<'_, crate::ForegroundRegionBridgeState>,
    _start: RegionStartOptions,
) -> Result<Value, String> {
    Err("macOS Region capture is unavailable on this platform".to_string())
}

fn random_hex(prefix: &str, bytes: usize) -> Result<String, String> {
    let mut random = vec![0_u8; bytes];
    getrandom::fill(&mut random)
        .map_err(|_| "could not create the private foreground Region session".to_string())?;
    let mut value = String::with_capacity(prefix.len() + bytes * 2);
    value.push_str(prefix);
    for byte in random {
        use std::fmt::Write as _;
        write!(value, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(value)
}

fn http_status(raw: &str) -> Option<u16> {
    raw.lines().next()?.split_whitespace().nth(1)?.parse().ok()
}
