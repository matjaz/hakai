//! AppKit / CoreGraphics escape hatches — everything winit doesn't expose.
//!
//! The overlay recipe is the Swift original's `OverlayWindow`: a borderless, non-opaque
//! window at `CGShieldingWindowLevel()` (the screensaver level — the only one reliably
//! above the menu bar and the Dock), joining every Space and staying put in Mission
//! Control, framed to the whole `NSScreen` rather than its visible area.

use std::ffi::c_void;

use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSRect;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::monitor::MonitorHandle;
use winit::platform::macos::MonitorHandleExtMacOS;
use winit::window::Window;

// ── CoreGraphics FFI ─────────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

pub type CGImageRef = *mut c_void;
pub type CGContextRef = *mut c_void;
pub type CGColorSpaceRef = *mut c_void;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub fn CGShieldingWindowLevel() -> i32;
    pub fn CGMainDisplayID() -> u32;
    pub fn CGDisplayBounds(display: u32) -> CGRect;
    pub fn CGPreflightScreenCaptureAccess() -> bool;
    pub fn CGRequestScreenCaptureAccess() -> bool;
    pub fn CGWindowListCreateImage(bounds: CGRect, list_option: u32, window_id: u32, image_option: u32) -> CGImageRef;
    pub fn CGImageGetWidth(image: CGImageRef) -> usize;
    pub fn CGImageGetHeight(image: CGImageRef) -> usize;
    pub fn CGImageRelease(image: CGImageRef);
    pub fn CGColorSpaceCreateDeviceRGB() -> CGColorSpaceRef;
    pub fn CGColorSpaceRelease(space: CGColorSpaceRef);
    pub fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: CGColorSpaceRef,
        bitmap_info: u32,
    ) -> CGContextRef;
    pub fn CGContextDrawImage(ctx: CGContextRef, rect: CGRect, image: CGImageRef);
    pub fn CGContextRelease(ctx: CGContextRef);
}

// NSWindowCollectionBehavior
const CAN_JOIN_ALL_SPACES: usize = 1 << 0;
const STATIONARY: usize = 1 << 4;
const IGNORES_CYCLE: usize = 1 << 6;
// NSApplicationPresentationOptions
const HIDE_DOCK: usize = 1 << 1;
const HIDE_MENU_BAR: usize = 1 << 3;

/// The `NSWindow*` behind a winit window.
fn ns_window(window: &Window) -> Option<*mut AnyObject> {
    let RawWindowHandle::AppKit(h) = window.window_handle().ok()?.as_raw() else { return None };
    let view = h.ns_view.as_ptr() as *mut AnyObject;
    let win: *mut AnyObject = unsafe { msg_send![view, window] };
    (!win.is_null()).then_some(win)
}

/// The window's `CGWindowID` — what `CGWindowListCreateImage` excludes everything at and
/// above, so a capture never contains the overlay itself.
pub fn window_number(window: &Window) -> Option<u32> {
    let win = ns_window(window)?;
    let n: isize = unsafe { msg_send![win, windowNumber] };
    (n > 0).then_some(n as u32)
}

/// Turns a plain winit window into the overlay: screensaver level, every Space, no shadow,
/// framed to the monitor's full `NSScreen` frame (menu bar and Dock included).
pub fn make_overlay(window: &Window, monitor: Option<&MonitorHandle>, windowed: bool) {
    let Some(win) = ns_window(window) else {
        log::warn!("no NSWindow behind the winit window — overlay styling skipped");
        return;
    };
    unsafe {
        let _: () = msg_send![win, setOpaque: false];
        let _: () = msg_send![win, setHasShadow: false];
        if windowed {
            return;
        }
        let level = CGShieldingWindowLevel() as isize;
        let _: () = msg_send![win, setLevel: level];
        let behavior = CAN_JOIN_ALL_SPACES | STATIONARY | IGNORES_CYCLE;
        let _: () = msg_send![win, setCollectionBehavior: behavior];
        let _: () = msg_send![win, setMovable: false];
        if let Some(screen) = monitor.and_then(|m| m.ns_screen()) {
            let screen = screen as *mut AnyObject;
            let frame: NSRect = msg_send![screen, frame];
            let _: () = msg_send![win, setFrame: frame, display: true];
        }
        let _: () = msg_send![win, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
    }
}

/// Brings the app to the front and hides the Dock and the menu bar while it runs — the
/// window already sits above both, this just stops them reacting underneath it.
pub fn take_over_screen() {
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: true];
        let options = HIDE_DOCK | HIDE_MENU_BAR;
        let _: () = msg_send![app, setPresentationOptions: options];
    }
}
