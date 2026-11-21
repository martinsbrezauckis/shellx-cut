//! Windows page keys use the ordinary guarded window close.

#[link(name = "user32")]
extern "system" {
    fn GetForegroundWindow() -> *mut std::ffi::c_void;
    fn GetAncestor(window: *mut std::ffi::c_void, flags: u32) -> *mut std::ffi::c_void;
}

/// A page key requests the same guarded close as the menu and title-bar button.
#[tauri::command]
pub(crate) fn request_window_close(window: tauri::WebviewWindow) -> Result<(), String> {
    // The native focus API excludes a focused child webview; foreground does not.
    let active = unsafe { GetAncestor(GetForegroundWindow(), 2) };
    if window.label() != "main" || active != window.hwnd().map_err(|error| error.to_string())?.0 {
        return Err("Quit requires the active ShellX Cut window".into());
    }
    window.close().map_err(|error| error.to_string())
}

pub(crate) fn on_page_load(
    webview: &tauri::Webview,
    payload: &tauri::webview::PageLoadPayload<'_>,
) {
    if webview.label() == "main" && payload.event() == tauri::webview::PageLoadEvent::Finished {
        if let Err(error) = webview.eval(include_str!("windows_quit_key.js")) {
            eprintln!("[shellx-cut] could not bind Quit: {error}");
        }
    }
}
