// windows_region_picker.cpp -- private foreground-owned Windows Region overlay.
//
// WGC picks a display/window, not a rectangle. This foreground Tauri overlay
// owns a single-display crop plus its exact DisplayConfig snapshot; it neither
// starts capture nor exposes a browser/API result to cutd or the UI.

#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <windowsx.h>
#include "windows_region_topology.h"
#include <algorithm>
#include <cstdint>
#include <limits>

extern "C" {
struct SXCWindowsNativePickerSelection {
    // The picker takes the physical target path and a SHA-256 fingerprint of
    // the active DisplayConfig topology in the same validated snapshot as the
    // crop. Neither raw value is a public or durable wire field.
    SXCWindowsTopologySnapshot topology;
    uint32_t left;
    uint32_t top;
    uint32_t width;
    uint32_t height;
    uint32_t parent_width;
    uint32_t parent_height;
};
enum SXCWindowsNativePickerResult : int32_t {
    SXCWindowsNativePickerPicked = 0,
    SXCWindowsNativePickerCancelled = 1,
    SXCWindowsNativePickerRefused = 2,
    SXCWindowsNativePickerFailure = 3,
};

int32_t sxc_windows_region_picker_present(
    HWND main_window,
    SXCWindowsNativePickerSelection* selection);
int32_t sxc_windows_region_picker_selection_is_current(
    const SXCWindowsNativePickerSelection* selection);
}

namespace {
constexpr wchar_t kPickerClass[] = L"ShellXCutPrivateWindowsRegionPicker";
constexpr int32_t kPickerPicked = SXCWindowsNativePickerPicked;
constexpr int32_t kPickerCancelled = SXCWindowsNativePickerCancelled;
constexpr int32_t kPickerRefused = SXCWindowsNativePickerRefused;
constexpr int32_t kPickerFailure = SXCWindowsNativePickerFailure;

struct PickerState {
    HWND hwnd = nullptr;
    HWND main_window = nullptr;
    RECT desktop{};
    RECT monitor{};
    SXCWindowsTopologySnapshot topology{};
    POINT anchor{};
    POINT current{};
    bool selecting = false;
    bool has_monitor = false;
    bool has_topology = false;
    bool invalid_selection = false;
    bool finished = false;
    int32_t result = kPickerFailure;
    SXCWindowsNativePickerSelection* output = nullptr;
};

bool dimensions(const RECT& rect, uint32_t* width, uint32_t* height) {
    const int64_t wide = static_cast<int64_t>(rect.right) - rect.left;
    const int64_t high = static_cast<int64_t>(rect.bottom) - rect.top;
    if (!width || !height || wide <= 0 || high <= 0
        || wide > std::numeric_limits<uint32_t>::max()
        || high > std::numeric_limits<uint32_t>::max()) {
        return false;
    }
    *width = static_cast<uint32_t>(wide);
    *height = static_cast<uint32_t>(high);
    return true;
}

POINT clamp_to_monitor(POINT point, const RECT& monitor) {
    // Mouse coordinates name pixel boundaries. Keeping the final edge legal
    // lets a drag cover the complete native monitor when that crop is encoder
    // representable, without ever crossing into a differently scaled display.
    point.x = std::min(std::max(point.x, monitor.left), monitor.right);
    point.y = std::min(std::max(point.y, monitor.top), monitor.bottom);
    return point;
}

bool choose_monitor(PickerState* state, POINT point) {
    if (!state) return false;
    const HMONITOR monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONULL);
    if (!monitor) return false;
    SXCWindowsTopologySnapshot topology{};
    if (!sxc_windows_topology_snapshot_for_monitor(monitor, &state->monitor, &topology)) return false;
    state->topology = topology;
    state->has_monitor = true;
    state->has_topology = true;
    return true;
}

POINT client_to_desktop(HWND hwnd, LPARAM lparam) {
    POINT point{GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};
    ClientToScreen(hwnd, &point);
    return point;
}

void finish(PickerState* state, int32_t result) {
    if (!state || state->finished) return;
    state->finished = true;
    state->result = result;
    if (state->hwnd) DestroyWindow(state->hwnd);
}

bool snap_even_crop(PickerState* state) {
    if (!state || !state->output || !state->has_monitor || !state->has_topology) return false;
    const int64_t start_x = std::min(state->anchor.x, state->current.x);
    const int64_t start_y = std::min(state->anchor.y, state->current.y);
    const int64_t end_x = std::max(state->anchor.x, state->current.x);
    const int64_t end_y = std::max(state->anchor.y, state->current.y);
    const int64_t relative_left = start_x - state->monitor.left;
    const int64_t relative_top = start_y - state->monitor.top;
    const int64_t relative_right = end_x - state->monitor.left;
    const int64_t relative_bottom = end_y - state->monitor.top;
    uint32_t parent_width = 0;
    uint32_t parent_height = 0;
    if (relative_left < 0 || relative_top < 0 || relative_right <= relative_left
        || relative_bottom <= relative_top || !dimensions(state->monitor, &parent_width, &parent_height)) {
        return false;
    }

    const uint64_t left = static_cast<uint64_t>(relative_left) & ~uint64_t{1};
    const uint64_t top = static_cast<uint64_t>(relative_top) & ~uint64_t{1};
    uint64_t right = static_cast<uint64_t>(relative_right);
    uint64_t bottom = static_cast<uint64_t>(relative_bottom);
    if (right & 1U) ++right;
    if (bottom & 1U) ++bottom;
    if (right <= left || bottom <= top || right > parent_width || bottom > parent_height
        || right - left < 2 || bottom - top < 2) {
        return false;
    }

    auto* output = state->output;
    *output = {};
    if (!sxc_windows_topology_snapshot_is_current(
            &state->topology, parent_width, parent_height)) {
        return false;
    }
    output->topology = state->topology;
    output->left = static_cast<uint32_t>(left);
    output->top = static_cast<uint32_t>(top);
    output->width = static_cast<uint32_t>(right - left);
    output->height = static_cast<uint32_t>(bottom - top);
    output->parent_width = parent_width;
    output->parent_height = parent_height;
    return true;
}

void paint_overlay(PickerState* state, HDC dc) {
    RECT client{};
    GetClientRect(state->hwnd, &client);
    FillRect(dc, &client, static_cast<HBRUSH>(GetStockObject(BLACK_BRUSH)));
    if (!state->has_monitor) return;

    const POINT first = state->anchor;
    const POINT second = state->current;
    if (first.x == second.x || first.y == second.y) return;
    RECT selection{
        std::min(first.x, second.x) - state->desktop.left,
        std::min(first.y, second.y) - state->desktop.top,
        std::max(first.x, second.x) - state->desktop.left,
        std::max(first.y, second.y) - state->desktop.top,
    };
    HBRUSH brush = CreateSolidBrush(state->invalid_selection ? RGB(220, 38, 38) : RGB(59, 130, 246));
    FrameRect(dc, &selection, brush);
    InflateRect(&selection, -2, -2);
    FrameRect(dc, &selection, brush);
    DeleteObject(brush);
}

LRESULT CALLBACK picker_window_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam) {
    auto* state = reinterpret_cast<PickerState*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
    if (message == WM_NCCREATE) {
        state = reinterpret_cast<PickerState*>(reinterpret_cast<CREATESTRUCTW*>(lparam)->lpCreateParams);
        if (!state) return FALSE;
        state->hwnd = hwnd;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(state));
    }
    switch (message) {
        case WM_ACTIVATE:
            if (state && (LOWORD(wparam) == WA_INACTIVE
                || GetWindow(hwnd, GW_OWNER) != state->main_window)) {
                finish(state, kPickerRefused);
            }
            return 0;
        case WM_ACTIVATEAPP:
            if (state && !wparam) finish(state, kPickerRefused);
            return 0;
        case WM_DISPLAYCHANGE:
        case WM_DPICHANGED:
            if (state) finish(state, kPickerRefused);
            return 0;
        case WM_CANCELMODE:
            if (state) finish(state, kPickerCancelled);
            return 0;
        case WM_CLOSE:
            if (state) finish(state, kPickerCancelled);
            return 0;
        case WM_SETCURSOR:
            SetCursor(LoadCursorW(nullptr, MAKEINTRESOURCEW(32515)));
            return TRUE;
        case WM_LBUTTONDOWN: {
            if (!state) return 0;
            POINT point = client_to_desktop(hwnd, lparam);
            if (!choose_monitor(state, point)) {
                state->invalid_selection = true;
                InvalidateRect(hwnd, nullptr, FALSE);
                return 0;
            }
            state->anchor = clamp_to_monitor(point, state->monitor);
            state->current = state->anchor;
            state->selecting = true;
            state->invalid_selection = false;
            SetCapture(hwnd);
            InvalidateRect(hwnd, nullptr, FALSE);
            return 0;
        }
        case WM_MOUSEMOVE:
            if (state && state->selecting && state->has_monitor) {
                state->current = clamp_to_monitor(client_to_desktop(hwnd, lparam), state->monitor);
                state->invalid_selection = false;
                InvalidateRect(hwnd, nullptr, FALSE);
            }
            return 0;
        case WM_LBUTTONUP:
            if (state && state->selecting) {
                state->current = clamp_to_monitor(client_to_desktop(hwnd, lparam), state->monitor);
                state->selecting = false;
                if (GetCapture() == hwnd) ReleaseCapture();
                InvalidateRect(hwnd, nullptr, FALSE);
            }
            return 0;
        case WM_KEYDOWN:
            if (!state) return 0;
            if (wparam == VK_ESCAPE) {
                finish(state, kPickerCancelled);
                return 0;
            }
            if (wparam == VK_RETURN) {
                if (GetForegroundWindow() != hwnd) {
                    finish(state, kPickerRefused);
                } else if (snap_even_crop(state)) {
                    finish(state, kPickerPicked);
                } else {
                    state->invalid_selection = true;
                    MessageBeep(MB_ICONWARNING);
                    InvalidateRect(hwnd, nullptr, FALSE);
                }
                return 0;
            }
            break;
        case WM_PAINT:
            if (state) {
                PAINTSTRUCT paint{};
                HDC dc = BeginPaint(hwnd, &paint);
                paint_overlay(state, dc);
                EndPaint(hwnd, &paint);
                return 0;
            }
            break;
    }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}

bool register_picker_class() {
    WNDCLASSEXW klass{};
    klass.cbSize = sizeof(klass);
    klass.hCursor = LoadCursorW(nullptr, MAKEINTRESOURCEW(32515));
    klass.hInstance = GetModuleHandleW(nullptr);
    klass.lpfnWndProc = picker_window_proc;
    klass.lpszClassName = kPickerClass;
    const ATOM registered = RegisterClassExW(&klass);
    return registered != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS;
}

bool exact_foreground_main_window_on_its_owner_thread(HWND main_window) {
    if (!main_window || !IsWindow(main_window) || GetForegroundWindow() != main_window) {
        return false;
    }
    DWORD process_id = 0;
    const DWORD owner_thread = GetWindowThreadProcessId(main_window, &process_id);
    return owner_thread != 0 && process_id == GetCurrentProcessId()
        && owner_thread == GetCurrentThreadId();
}

}  // namespace

extern "C" int32_t sxc_windows_region_picker_selection_is_current(
    const SXCWindowsNativePickerSelection* selection) {
    return selection && sxc_windows_topology_snapshot_is_current(
                            &selection->topology, selection->parent_width, selection->parent_height);
}

extern "C" int32_t sxc_windows_region_picker_present(
    HWND main_window,
    SXCWindowsNativePickerSelection* selection) {
    // The shell passes its exact Tauri `main` HWND from `run_on_main_thread`.
    // Process identity alone is insufficient: another window in the process
    // must not acquire a visual-capture admission, and a cross-thread call
    // must not create a second Win32 message loop for the Tauri window.
    if (!selection || !exact_foreground_main_window_on_its_owner_thread(main_window)) {
        return kPickerRefused;
    }
    *selection = {};

    // GetSystemMetrics and cursor/window coordinates are native physical pixels
    // only under a per-monitor-aware context. Restore the caller's context on
    // every exit; the Tauri shell itself remains responsible for its own DPI UI.
    const auto previous_dpi = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    if (!previous_dpi) return kPickerRefused;
    struct DpiContextRestore {
        DPI_AWARENESS_CONTEXT previous;
        ~DpiContextRestore() { SetThreadDpiAwarenessContext(previous); }
    } restore{previous_dpi};

    PickerState state{};
    state.main_window = main_window;
    state.output = selection;
    state.desktop.left = GetSystemMetrics(SM_XVIRTUALSCREEN);
    state.desktop.top = GetSystemMetrics(SM_YVIRTUALSCREEN);
    const int width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    const int height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
    if (width <= 0 || height <= 0 || !register_picker_class()) return kPickerRefused;

    HWND window = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
        kPickerClass,
        L"ShellX Cut private Region selection",
        WS_POPUP,
        state.desktop.left,
        state.desktop.top,
        width,
        height,
        main_window,
        nullptr,
        GetModuleHandleW(nullptr),
        &state);
    if (!window) return kPickerFailure;
    if (GetWindow(window, GW_OWNER) != main_window
        || GetWindowThreadProcessId(window, nullptr) != GetCurrentThreadId()) {
        DestroyWindow(window);
        return kPickerRefused;
    }
    SetLayeredWindowAttributes(window, 0, 118, LWA_ALPHA);
    ShowWindow(window, SW_SHOWNORMAL);
    SetWindowPos(window, HWND_TOPMOST, state.desktop.left, state.desktop.top, width, height, SWP_SHOWWINDOW);
    SetForegroundWindow(window);
    SetFocus(window);
    if (GetForegroundWindow() != window) {
        finish(&state, kPickerRefused);
        return kPickerRefused;
    }

    MSG message{};
    while (!state.finished) {
        const BOOL received = GetMessageW(&message, nullptr, 0, 0);
        if (received == 0) {
            // This nested picker runs on Tauri's main event-loop thread. A
            // WM_QUIT belongs to that outer loop, so return it after ending
            // the modal picker rather than accidentally consuming shutdown.
            PostQuitMessage(static_cast<int>(message.wParam));
            finish(&state, kPickerFailure);
            break;
        }
        if (received == -1) {
            finish(&state, kPickerFailure);
            break;
        }
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    return state.result;
}
