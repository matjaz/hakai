//! Hakai on Android.
//!
//! The window/event-loop/input shell is `hakai_core::shell`, shared with the Windows and
//! macOS builds and the Linux GNOME/X11 fallback; this is the Android hooks into it, run
//! from android-activity's NativeActivity glue. The only Java is the Quick Settings tile
//! (`java/`), which starts this same activity over whatever app is open.
//!
//! **The screen it smashes is the real one.** The activity uses a translucent theme, so
//! whatever it was launched over — the home screen, or with the tile any app — stays
//! visible behind it, the phone equivalent of the desktop overlay, with no special
//! permission. NativeActivity
//! forces its window to opaque RGB_565 on creation; [`make_window_translucent`] switches
//! it back to `PixelFormat.TRANSLUCENT` on the UI thread.
//!
//! Touch drives everything (see the shell): drag to use the tool, tap the status bar for
//! the palette, long-press it for credits (drag to scroll them), Back to quit. Leaving the
//! app quits it — a fresh start on return is simpler than rebuilding surfaces Android has
//! torn down.

use android_activity::AndroidApp;
use hakai_core::audio::AudioSink;
use hakai_core::render::HudText;
use hakai_core::shell::winit::event_loop::{EventLoopBuilder, OwnedDisplayHandle};
use hakai_core::shell::winit::platform::android::EventLoopBuilderExtAndroid;
use hakai_core::shell::{Coverage, Platform, ShellEvent};
use jni::{jni_sig, jni_str, JValue};

/// `android.graphics.PixelFormat.TRANSLUCENT`.
const PIXEL_FORMAT_TRANSLUCENT: i32 = -3;

struct Android {
    app: AndroidApp,
}

impl Platform for Android {
    fn name(&self) -> &'static str {
        "Android"
    }

    /// Vulkan on nearly every current phone; GLES (which needs the display handle) on the
    /// rest.
    fn instance(&self, display: OwnedDisplayHandle) -> wgpu::Instance {
        let mut desc = wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display));
        desc.backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::VULKAN | wgpu::Backends::GL);
        wgpu::Instance::new(desc)
    }

    fn audio(&mut self) -> AudioSink {
        hakai_core::playback::sink()
    }

    fn coverage(&self) -> Coverage {
        Coverage::Fullscreen
    }

    fn hud_text(&self) -> HudText {
        HudText::TOUCH
    }

    fn configure_event_loop(&mut self, builder: &mut EventLoopBuilder<ShellEvent>) {
        builder.with_android_app(self.app.clone());
    }

    fn quit_when_suspended(&self) -> bool {
        true
    }
}

/// Undoes NativeActivity's `getWindow().setFormat(PixelFormat.RGB_565)` so the translucent
/// theme actually shows what's underneath. `Window.setFormat` must run on the UI thread.
fn make_window_translucent(app: &AndroidApp) {
    let app2 = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        let vm = unsafe { jni::JavaVM::from_raw(app2.vm_as_ptr() as _) };
        let result = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let activity = unsafe { jni::objects::JObject::from_raw(env, app2.activity_as_ptr() as _) };
            let window = env
                .call_method(&activity, jni_str!("getWindow"), jni_sig!("()Landroid/view/Window;"), &[])?
                .l()?;
            env.call_method(&window, jni_str!("setFormat"), jni_sig!("(I)V"), &[JValue::Int(PIXEL_FORMAT_TRANSLUCENT)])?;
            Ok(())
        });
        match result {
            Ok(()) => log::info!("window format: translucent"),
            Err(e) => log::warn!("couldn't make the window translucent ({e:?}) — the background will be opaque"),
        }
    }));
}

#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("hakai"),
    );
    make_window_translucent(&app);
    hakai_core::shell::run(Android { app });
}
