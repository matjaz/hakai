//! Audio.
//!
//! Ported from the *logic* in `AudioEngine.swift`/`SoundBank.swift`, not the playback:
//! `smash_name` and `numbered_name` are pure, testable functions (rule #5 of the
//! interaction checks depends on `smash_name` directly), and `pan_for_x` is the
//! stereobase formula. `AudioSink` itself stays a genuine no-op *by default* — matching
//! `ToolSimulation.swift`'s own rig, which deliberately never starts the real audio
//! engine ("`play`/`startLoop` then do nothing, so the simulation emits no sound and
//! needs no audio device") — but can now delegate to a real [`AudioBackend`], which is
//! how Phase 5's `cpal` engine (in `hakai`, not here) actually gets called.
//!
//! **Why the backend is a trait living here, not a concrete `cpal` dependency.** This
//! crate's own doc comment promises "no window, no audio, no compositor... builds and
//! runs with plain `cargo test` on any OS" — a real audio device is exactly the kind of
//! thing that would break that on a headless CI box or a container. `AudioBackend` keeps
//! `hakai-core` itself free of any actual audio dependency while still giving `tools/`
//! and `ToolSimulation` a single, unchanged call surface
//! (`play`/`start_loop`/`set_loop`/`stop_loop`) regardless of whether a backend is
//! plugged in — `hakai`, the renderer binary (which already isn't headless — it needs a
//! live Wayland compositor and a GPU), is where `cpal` actually lives.

use crate::rng::SeededRng;

/// `sound stereobase`: the left side of the screen → left, the right side → right.
pub fn pan_for_x(x: f32, width: f32) -> f32 {
    if width <= 0.0 {
        return 0.0;
    }
    ((x / width) * 2.0 - 1.0).clamp(-1.0, 1.0)
}

const SMASH_COUNT: i64 = 8;
const SMASH_NAMES: [&str; 8] = ["smash1", "smash2", "smash3", "smash4", "smash5", "smash6", "smash7", "smash8"];

/// Picks the impact variant from the brightness of the surface — `sound by color
/// brightness below mouse cursor`. A dark surface gives a hollow/wooden impact, a light
/// one a glassy/metallic one. When the brightness is unknown (no screen capture yet —
/// that's Phase 6) the choice is random; the tool has to work in that case too.
pub fn smash_name(brightness: Option<u8>, rng: &mut SeededRng) -> &'static str {
    let index = match brightness {
        Some(b) => ((b as i64 * SMASH_COUNT) / 256 + 1).min(SMASH_COUNT),
        None => rng.int(1, SMASH_COUNT + 1),
    };
    SMASH_NAMES[(index - 1).clamp(0, SMASH_COUNT - 1) as usize]
}

/// A random name from a numbered group, e.g. `("shell", 3)` → `shell1`…`shell3`.
pub fn numbered_name(prefix: &str, count: i64, rng: &mut SeededRng) -> String {
    format!("{prefix}{}", rng.int(1, count + 1))
}

/// A real playback backend `AudioSink` can delegate to — see the module doc comment for
/// why this is a trait rather than a concrete dependency pulled straight into this crate.
/// Mirrors `AudioSink`'s own call surface exactly, plus `update` for the loop gain-glide
/// `AudioEngine.swift`'s own `update(dt:)` drives.
pub trait AudioBackend {
    fn play(&mut self, name: &str, pan: f32, volume: f32);
    fn start_loop(&mut self, name: &str, key: &str, volume: f32, pan: f32);
    fn set_loop(&mut self, key: &str, volume: Option<f32>, pan: Option<f32>);
    fn stop_loop(&mut self, key: &str, fade: f32);
    /// Advances every loop voice's gain glide by `dt` seconds. Only needs to be called
    /// once per frame *total*, not once per output — matching `AudioEngine.swift`'s "only
    /// the first scene drives its update loop" (loop voices are shared across every
    /// output, unlike `ParticleSystem`/`TermiteColony`, which genuinely are per-output).
    fn update(&mut self, dt: f32);
}

/// A no-op audio sink by default — every call is recorded as an event only in test builds
/// (`#[cfg(test)]`), so tool logic is exercised without depending on any of it actually
/// making sound, matching how the Swift simulation never starts its audio engine. Plug in
/// a real backend (`with_backend`) to make it actually play something.
pub struct AudioSink {
    backend: Option<Box<dyn AudioBackend>>,
    #[cfg(test)]
    pub events: Vec<String>,
}

impl AudioSink {
    pub fn new() -> Self {
        Self {
            backend: None,
            #[cfg(test)]
            events: Vec::new(),
        }
    }

    /// An `AudioSink` that forwards every call to a real backend, in addition to (in test
    /// builds) still recording it as an event — so a backend can be exercised under the
    /// same checks that already cover the no-op path, if that's ever useful.
    pub fn with_backend(backend: Box<dyn AudioBackend>) -> Self {
        Self {
            backend: Some(backend),
            #[cfg(test)]
            events: Vec::new(),
        }
    }

    pub fn play(&mut self, name: &str, pan: f32, volume: f32) {
        #[cfg(test)]
        self.events.push(format!("play {name}"));
        if let Some(backend) = &mut self.backend {
            backend.play(name, pan, volume);
        }
    }

    pub fn start_loop(&mut self, name: &str, key: &str, volume: f32, pan: f32) {
        #[cfg(test)]
        self.events.push(format!("start_loop {key} <- {name}"));
        if let Some(backend) = &mut self.backend {
            backend.start_loop(name, key, volume, pan);
        }
    }

    pub fn set_loop(&mut self, key: &str, volume: Option<f32>, pan: Option<f32>) {
        #[cfg(test)]
        self.events.push(format!("set_loop {key}"));
        if let Some(backend) = &mut self.backend {
            backend.set_loop(key, volume, pan);
        }
    }

    pub fn stop_loop(&mut self, key: &str, fade: f32) {
        #[cfg(test)]
        self.events.push(format!("stop_loop {key}"));
        if let Some(backend) = &mut self.backend {
            backend.stop_loop(key, fade);
        }
    }

    /// Advances the backend's loop gain-glide by `dt`, if a backend is plugged in —
    /// otherwise a no-op. See `AudioBackend::update`'s doc comment for why this should be
    /// called once per frame, not once per `ToolContext`/output.
    pub fn update(&mut self, dt: f32) {
        if let Some(backend) = &mut self.backend {
            backend.update(dt);
        }
    }
}

impl Default for AudioSink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_selects_the_impact_variant() {
        let mut rng = SeededRng::new(5);
        assert_eq!(smash_name(Some(10), &mut rng), "smash1");
        assert_eq!(smash_name(Some(245), &mut rng), "smash8");
    }

    #[test]
    fn brightness_boundaries_are_clamped_into_range() {
        let mut rng = SeededRng::new(1);
        assert_eq!(smash_name(Some(0), &mut rng), "smash1");
        assert_eq!(smash_name(Some(255), &mut rng), "smash8");
    }

    #[test]
    fn unknown_brightness_is_random_but_in_range() {
        let mut rng = SeededRng::new(9);
        for _ in 0..200 {
            let name = smash_name(None, &mut rng);
            assert!(SMASH_NAMES.contains(&name));
        }
    }

    #[test]
    fn pan_follows_screen_position() {
        assert_eq!(pan_for_x(0.0, 1000.0), -1.0);
        assert_eq!(pan_for_x(1000.0, 1000.0), 1.0);
        assert_eq!(pan_for_x(500.0, 1000.0), 0.0);
    }

    /// A backend that just records every call it receives, into a `Rc<RefCell<_>>` the
    /// test keeps its own handle to — since the backend itself ends up moved into the
    /// sink's `Box<dyn AudioBackend>`, this is what lets the test still inspect what it
    /// saw afterwards, rather than only proving the sink's own separate `#[cfg(test)]`
    /// event log fired.
    #[derive(Default, Clone)]
    struct RecordingBackend {
        calls: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }
    impl AudioBackend for RecordingBackend {
        fn play(&mut self, name: &str, pan: f32, volume: f32) {
            self.calls.borrow_mut().push(format!("play {name} pan={pan} volume={volume}"));
        }
        fn start_loop(&mut self, name: &str, key: &str, volume: f32, pan: f32) {
            self.calls.borrow_mut().push(format!("start_loop {key} <- {name} volume={volume} pan={pan}"));
        }
        fn set_loop(&mut self, key: &str, volume: Option<f32>, pan: Option<f32>) {
            self.calls.borrow_mut().push(format!("set_loop {key} volume={volume:?} pan={pan:?}"));
        }
        fn stop_loop(&mut self, key: &str, fade: f32) {
            self.calls.borrow_mut().push(format!("stop_loop {key} fade={fade}"));
        }
        fn update(&mut self, dt: f32) {
            self.calls.borrow_mut().push(format!("update {dt}"));
        }
    }

    #[test]
    fn a_sink_without_a_backend_never_panics_and_records_nothing_but_test_events() {
        let mut sink = AudioSink::new();
        sink.play("smash1", 0.0, 1.0);
        sink.start_loop("flame", "flame", 0.5, 0.0);
        sink.set_loop("flame", Some(0.2), None);
        sink.stop_loop("flame", 0.1);
        sink.update(1.0 / 60.0);
        assert_eq!(sink.events, vec!["play smash1", "start_loop flame <- flame", "set_loop flame", "stop_loop flame"]);
    }

    #[test]
    fn a_sink_with_a_backend_forwards_every_call() {
        let backend = RecordingBackend::default();
        let calls = backend.calls.clone(); // the test's own handle, surviving the move below

        let mut sink = AudioSink::with_backend(Box::new(backend));
        sink.play("smash1", 0.5, 0.9);
        sink.start_loop("saw_idle", "saw_idle", 0.28, -0.2);
        sink.set_loop("saw_idle", Some(0.0), Some(0.3));
        sink.stop_loop("saw_idle", 0.18);
        sink.update(0.016);

        assert_eq!(
            *calls.borrow(),
            vec![
                "play smash1 pan=0.5 volume=0.9",
                "start_loop saw_idle <- saw_idle volume=0.28 pan=-0.2",
                "set_loop saw_idle volume=Some(0.0) pan=Some(0.3)",
                "stop_loop saw_idle fade=0.18",
                "update 0.016",
            ],
            "the backend should have received every call, not just the sink's own test-event log"
        );
    }
}
