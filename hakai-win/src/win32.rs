//! Small Win32 tweaks to the winit windows that have no winit API.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;
use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW,
    ShowWindow, GWL_EXSTYLE, LWA_ALPHA, SW_MINIMIZE, WS_EX_LAYERED,
};

fn hwnd_of(window: &Window) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}

/// Marks the overlay as non-occluding so full-screen-aware apps behind it keep painting.
///
/// Chromium (Brave/Chrome/Edge) tracks whether its window is covered and throttles the
/// tab — including video playback — when it thinks it's fully occluded. A plain
/// always-on-top window over the browser trips that. Adding `WS_EX_LAYERED` with a
/// per-window alpha below 255 makes Chromium's occlusion tracker treat this window as
/// *not* occluding (it can't know what a translucent window actually covers), so a
/// YouTube video stays visible and playing underneath. Alpha 254 is a 0.4% dim — not
/// perceptible — and, unlike `WS_EX_TRANSPARENT`, `WS_EX_LAYERED` doesn't stop the window
/// receiving the mouse clicks the tools need.
pub fn mark_non_occluding(window: &Window) {
    if std::env::var("HAKAI_NO_LAYERED").is_ok() {
        return;
    }
    let Some(hwnd) = hwnd_of(window) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
        // Fails harmlessly on some compositor states; Chromium's fallback for a layered
        // window whose attributes can't be read is also "does not occlude", so either
        // outcome is fine.
        match SetLayeredWindowAttributes(hwnd, COLORREF(0), 254, LWA_ALPHA) {
            Ok(()) => log::debug!("overlay marked non-occluding (WS_EX_LAYERED, alpha 254)"),
            Err(e) => log::debug!("SetLayeredWindowAttributes: {e} (WS_EX_LAYERED still set)"),
        }
    }
}

/// Minimises the launching terminal at startup. Only fires when hakai was actually
/// started from a terminal (`GetConsoleWindow` is non-null — a double-click from Explorer
/// has no console, and nothing should be minimised then). The target is the current
/// foreground window, which at this point — before any overlay window exists — is that
/// terminal, whether it's the classic console host or Windows Terminal (whose own console
/// host is an invisible helper). Set `HAKAI_KEEP_TERMINAL` to skip.
pub fn minimize_launcher() {
    if std::env::var("HAKAI_KEEP_TERMINAL").is_ok() {
        return;
    }
    unsafe {
        if GetConsoleWindow().0.is_null() {
            log::debug!("no console — not launched from a terminal, nothing to minimize");
            return;
        }
        let fg = GetForegroundWindow();
        if !fg.0.is_null() {
            let _ = ShowWindow(fg, SW_MINIMIZE);
            log::debug!("minimized the launching terminal");
        }
    }
}
