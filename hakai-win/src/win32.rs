//! Small Win32 tweaks to the winit windows that have no winit API.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, COLORREF, HWND, LPARAM};
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, MOD_NOREPEAT, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetMessageW, GetWindow, GetWindowLongPtrW,
    GetWindowThreadProcessId, IsWindowVisible, SetLayeredWindowAttributes, SetWindowLongPtrW,
    ShowWindowAsync, GWL_EXSTYLE, GW_OWNER, LWA_ALPHA, MSG, SW_MINIMIZE, WM_HOTKEY, WS_EX_LAYERED,
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
        match SetLayeredWindowAttributes(hwnd, COLORREF(0), 254, LWA_ALPHA) {
            Ok(()) => log::debug!("overlay marked non-occluding (WS_EX_LAYERED, alpha 254)"),
            Err(e) => log::debug!("SetLayeredWindowAttributes: {e} (WS_EX_LAYERED still set)"),
        }
    }
}

// ── Minimising the launcher ─────────────────────────────────────────────────────────────

fn minimize(hwnd: HWND) -> bool {
    if hwnd.0.is_null() {
        return false;
    }
    // ShowWindowAsync, not ShowWindow: posts the message rather than sending it, which is
    // the reliable form for a window owned by another process's thread.
    unsafe { ShowWindowAsync(hwnd, SW_MINIMIZE).as_bool() }
}

/// `(parent pid, lowercased exe name)` for `pid`.
fn process_info(pid: u32) -> Option<(u32, String)> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut out = None;
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                if entry.th32ProcessID == pid {
                    let end = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..end]).to_ascii_lowercase();
                    out = Some((entry.th32ParentProcessID, name));
                    break;
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        out
    }
}

struct FindCtx {
    pid: u32,
    hwnd: HWND,
}

unsafe extern "system" fn find_window_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut FindCtx);
    let mut wpid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut wpid));
    let owned = GetWindow(hwnd, GW_OWNER).map(|h| !h.0.is_null()).unwrap_or(false);
    if wpid == ctx.pid && IsWindowVisible(hwnd).as_bool() && !owned {
        ctx.hwnd = hwnd;
        return BOOL(0); // stop
    }
    BOOL(1)
}

fn main_window_of_pid(pid: u32) -> Option<HWND> {
    let mut ctx = FindCtx { pid, hwnd: HWND(std::ptr::null_mut()) };
    unsafe {
        let _ = EnumWindows(Some(find_window_cb), LPARAM(&mut ctx as *mut _ as isize));
    }
    (!ctx.hwnd.0.is_null()).then_some(ctx.hwnd)
}

/// Minimises the terminal hakai was launched from, at startup — before any overlay window
/// exists, so the foreground window is still the launcher.
///
/// Every path is tried, because "the terminal" is a different window in each host and
/// process tree:
///
/// - **`GetConsoleWindow()`** — the visible window for a classic console host.
/// - **Ancestor walk** — Windows Terminal (`WindowsTerminal.exe` above us via ConPTY),
///   mintty (git-bash), alacritty, ...: the first ancestor that owns a visible top-level
///   window. Stops at `explorer.exe` so the desktop is never minimised.
/// - **Foreground window** — the catch-all. Right now that's whatever launched us.
///
/// Run with `RUST_LOG=hakai_win::win32=debug` to see the process chain and which windows
/// were minimised. `HAKAI_KEEP_TERMINAL` skips the whole thing.
pub fn minimize_launcher() {
    if std::env::var("HAKAI_KEEP_TERMINAL").is_ok() {
        return;
    }

    unsafe {
        let console = GetConsoleWindow();
        if minimize(console) {
            log::debug!("minimize_launcher: minimized console window");
        }

        // Ancestor chain.
        let mut pid = GetCurrentProcessId();
        for depth in 0..16 {
            let Some((ppid, name)) = process_info(pid) else { break };
            let parent_name = process_info(ppid).map(|(_, n)| n).unwrap_or_default();
            log::debug!("minimize_launcher: [{depth}] {name} (pid {pid}) <- {parent_name} (pid {ppid})");
            if ppid == 0 || ppid == pid || parent_name == "explorer.exe" || parent_name.is_empty() {
                break;
            }
            if let Some(hwnd) = main_window_of_pid(ppid) {
                if minimize(hwnd) {
                    log::debug!("minimize_launcher: minimized ancestor {parent_name} (pid {ppid})");
                }
                break;
            }
            pid = ppid;
        }

        // Catch-all: the window the user was looking at when they launched us.
        let fg = GetForegroundWindow();
        if fg != console && minimize(fg) {
            log::debug!("minimize_launcher: minimized foreground window");
        }
    }
}

// ── Global Esc-to-quit ──────────────────────────────────────────────────────────────────

/// Registers a system-wide **Esc** hotkey so hakai always closes on Esc, even after you
/// Alt+Tab away to another app (a video, a browser). Runs its own tiny message loop on a
/// dedicated thread — `RegisterHotKey` delivers `WM_HOTKEY` to the thread that called it,
/// which winit's event loop doesn't surface. On the hotkey, `on_quit` runs once and the
/// thread ends (which also unregisters the hotkey).
///
/// Trade-off: while hakai runs, Esc is reserved — it won't reach other apps (so it can't,
/// e.g., leave a full-screen YouTube video). That's the "Esc must always close hakai"
/// behaviour that was asked for. `HAKAI_NO_GLOBAL_ESC` keeps Esc app-local (it then only
/// quits hakai while hakai itself is focused).
pub fn spawn_quit_hotkey<F: Fn() + Send + 'static>(on_quit: F) {
    if std::env::var("HAKAI_NO_GLOBAL_ESC").is_ok() {
        return;
    }
    std::thread::spawn(move || unsafe {
        const HOTKEY_ID: i32 = 1;
        if let Err(e) = RegisterHotKey(None, HOTKEY_ID, MOD_NOREPEAT, VK_ESCAPE.0 as u32) {
            log::warn!("global Esc hotkey unavailable ({e}) — Esc quits only while hakai is focused");
            return;
        }
        log::debug!("global Esc hotkey registered");
        let mut msg = MSG::default();
        loop {
            let got = GetMessageW(&mut msg, None, 0, 0);
            if got.0 <= 0 {
                break; // 0 = WM_QUIT, -1 = error
            }
            if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == HOTKEY_ID {
                on_quit();
                break;
            }
        }
    });
}
