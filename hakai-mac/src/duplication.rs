//! Desktop capture on macOS — the producer for `capture::BrightnessMap`.
//!
//! `CGWindowListCreateImage(…, OnScreenBelowWindow, overlay)` captures the primary display
//! with everything at or above the overlay window left out — the same exclusion the Swift
//! original gets from `SCContentFilter(excludingApplications: [self])`, without having to
//! bridge ScreenCaptureKit's async API. A full-resolution capture of a Retina display
//! takes tens of milliseconds, so it runs on a worker thread; `capture` never blocks the
//! frame loop and just hands over the newest finished frame.
//!
//! Reading other apps' pixels needs the **Screen Recording** permission. Without it the
//! app works normally and every tool picks a random impact variant — exactly the Swift
//! original's (and the original original's) fallback. The permission is requested once;
//! macOS only shows its prompt the first time.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use hakai_core::shell::DesktopCapture;

use crate::macos::*;

const LIST_ON_SCREEN_BELOW_WINDOW: u32 = 1 << 2;
const IMAGE_DEFAULT: u32 = 0;
// kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little — BGRA in memory.
const BITMAP_BGRA: u32 = 2 | (2 << 12);

struct Frame {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
}

pub struct DesktopDuplication {
    request: Sender<()>,
    in_flight: Arc<AtomicBool>,
    latest: Arc<Mutex<Option<Frame>>>,
}

impl DesktopDuplication {
    /// `None` when Screen Recording isn't granted (and asks for it, so it is next launch).
    /// `below_window` is the overlay's `CGWindowID` on the primary display.
    pub fn new(below_window: u32) -> Option<Self> {
        if std::env::var("HAKAI_NO_CAPTURE").is_ok() {
            log::info!("desktop capture: disabled by HAKAI_NO_CAPTURE");
            return None;
        }
        unsafe {
            if !CGPreflightScreenCaptureAccess() {
                log::warn!(
                    "Screen Recording not granted — impact sounds will use a random variant \
                     (grant it in System Settings → Privacy & Security, then relaunch)"
                );
                CGRequestScreenCaptureAccess();
                return None;
            }
        }

        let (request, rx) = mpsc::channel();
        let in_flight = Arc::new(AtomicBool::new(false));
        let latest = Arc::new(Mutex::new(None));
        {
            let in_flight = in_flight.clone();
            let latest = latest.clone();
            std::thread::Builder::new()
                .name("hakai-capture".into())
                .spawn(move || worker(rx, below_window, in_flight, latest))
                .ok()?;
        }
        log::info!("desktop capture: active");
        Some(Self { request, in_flight, latest })
    }

}

impl DesktopCapture for DesktopDuplication {
    /// Hands the newest finished capture to `f` and queues the next one. `false` while
    /// the first capture is still in flight.
    fn capture(&mut self, f: &mut dyn FnMut(&[u8], u32, u32, u32)) -> bool {
        if !self.in_flight.swap(true, Ordering::AcqRel) {
            let _ = self.request.send(());
        }
        let Some(frame) = self.latest.lock().ok().and_then(|mut slot| slot.take()) else { return false };
        f(&frame.bytes, frame.width, frame.height, frame.width * 4);
        true
    }
}

fn worker(rx: Receiver<()>, below_window: u32, in_flight: Arc<AtomicBool>, latest: Arc<Mutex<Option<Frame>>>) {
    while rx.recv().is_ok() {
        if let Some(frame) = unsafe { grab(below_window) } {
            if let Ok(mut slot) = latest.lock() {
                *slot = Some(frame);
            }
        }
        in_flight.store(false, Ordering::Release);
    }
}

/// One capture of the primary display, drawn into a tightly packed BGRA buffer.
unsafe fn grab(below_window: u32) -> Option<Frame> {
    let bounds = CGDisplayBounds(CGMainDisplayID());
    let image = CGWindowListCreateImage(bounds, LIST_ON_SCREEN_BELOW_WINDOW, below_window, IMAGE_DEFAULT);
    if image.is_null() {
        log::debug!("CGWindowListCreateImage returned null");
        return None;
    }
    let (w, h) = (CGImageGetWidth(image), CGImageGetHeight(image));
    if w == 0 || h == 0 {
        CGImageRelease(image);
        return None;
    }
    let mut bytes = vec![0u8; w * h * 4];
    let space = CGColorSpaceCreateDeviceRGB();
    let ctx = CGBitmapContextCreate(bytes.as_mut_ptr().cast(), w, h, 8, w * 4, space, BITMAP_BGRA);
    CGColorSpaceRelease(space);
    if ctx.is_null() {
        CGImageRelease(image);
        return None;
    }
    let rect = CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: w as f64, height: h as f64 } };
    CGContextDrawImage(ctx, rect, image);
    CGContextRelease(ctx);
    CGImageRelease(image);
    Some(Frame { bytes, width: w as u32, height: h as u32 })
}
