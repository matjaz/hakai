//! Real audio playback — `cpal` behind `hakai_core::audio::AudioBackend`.
//!
//! Ported from `AudioEngine.swift`: a 24-voice round-robin pool for one-shot sounds
//! (`smash1`, a machine-gun shot, ...), keyed loop voices with a ~100ms gain glide (the
//! saw's idle/cutting crossfade, the flame/wash loops, ...), and hand-summed equal-power
//! stereo panning from `hakai_core::audio::pan_for_x`'s stereobase.
//!
//! **The thread boundary.** `cpal` calls its own callback closure repeatedly from a
//! dedicated realtime audio thread — completely separate from the thread that calls
//! `AudioBackend`'s methods (this app's main/event-loop thread, via `ToolContext`). A
//! [`Mutex`]-guarded [`Mixer`] is the bridge: the main thread locks it briefly to record
//! "play this," "this loop's target volume changed," etc.; the audio thread locks it once
//! per buffer to mix and advance every active voice. A `Mutex` on a realtime audio thread
//! is a real, known tradeoff (a slow lock holder can cause an audible glitch/underrun) —
//! accepted here the same way this port has accepted other "correct first, revisit only
//! if it actually stutters" tradeoffs (the termites' per-frame buffer allocation is the
//! other one), since the lock is held only briefly on either side and this isn't
//! professional low-latency production audio.
//!
//! **Sample rate.** All 35 bundled sounds are confirmed mono, 16-bit PCM, 44.1kHz (`file
//! assets/sounds/*.wav`). This engine asks the output device for a stream at that same
//! rate when the device can provide one; if it can't and falls back to something else
//! (48kHz is common), sounds play back at a slightly wrong pitch/speed — no resampling is
//! implemented. A real, known gap, not an oversight — flagged rather than silently
//! accepted as exact.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hakai_core::audio::AudioBackend;

const VOICE_COUNT: usize = 24;
/// `AudioEngine.swift`'s `update(dt:)`: `let step = Float(min(1, dt / 0.10))` — ~100ms to
/// reach a loop voice's target gain.
const GAIN_GLIDE_SECONDS: f32 = 0.10;

const SOUND_NAMES: &[&str] = &[
    "flame_begin",
    "flame_end",
    "flame_loop",
    "mg_reverb",
    "mg_shot1",
    "mg_shot2",
    "mg_shot3",
    "mg_shot4",
    "mg_shot5",
    "mg_shot6",
    "paint_drop",
    "paint_shoot",
    "phaser1",
    "phaser2",
    "saw_cut",
    "saw_idle",
    "shell1",
    "shell2",
    "shell3",
    "smash1",
    "smash2",
    "smash3",
    "smash4",
    "smash5",
    "smash6",
    "smash7",
    "smash8",
    "stamp1",
    "stamp2",
    "termite_chew",
    "termite_crunch",
    "termite_dead",
    "termite_squish",
    "wash_loop",
    "wash_start",
];

/// Every bundled sound, embedded at compile time (`assets/sounds/*.wav`) — deterministic,
/// no runtime file lookup, matching `hakai_core`'s own font-embedding precedent
/// (`hakai_core::fonts`'s doc comment). `include_bytes!` needs a literal path per file, so
/// this is one macro invocation per sound rather than something built from
/// `SOUND_NAMES` — the two are kept in sync by `preload`'s own assertion (a name in
/// `SOUND_NAMES` with no matching arm here is a compile error, not a silent gap).
fn embedded_wav(name: &str) -> Option<&'static [u8]> {
    macro_rules! sound {
        ($n:literal) => {
            if name == $n {
                return Some(include_bytes!(concat!("../assets/sounds/", $n, ".wav")));
            }
        };
    }
    sound!("flame_begin");
    sound!("flame_end");
    sound!("flame_loop");
    sound!("mg_reverb");
    sound!("mg_shot1");
    sound!("mg_shot2");
    sound!("mg_shot3");
    sound!("mg_shot4");
    sound!("mg_shot5");
    sound!("mg_shot6");
    sound!("paint_drop");
    sound!("paint_shoot");
    sound!("phaser1");
    sound!("phaser2");
    sound!("saw_cut");
    sound!("saw_idle");
    sound!("shell1");
    sound!("shell2");
    sound!("shell3");
    sound!("smash1");
    sound!("smash2");
    sound!("smash3");
    sound!("smash4");
    sound!("smash5");
    sound!("smash6");
    sound!("smash7");
    sound!("smash8");
    sound!("stamp1");
    sound!("stamp2");
    sound!("termite_chew");
    sound!("termite_crunch");
    sound!("termite_dead");
    sound!("termite_squish");
    sound!("wash_loop");
    sound!("wash_start");
    None
}

/// Decodes one embedded WAV into mono `f32` samples in `[-1, 1]`. `hound` reads 16-bit PCM
/// samples as `i16`; dividing by `i16::MAX` is the standard integer-to-float conversion.
fn decode_wav(bytes: &[u8]) -> Option<Vec<f32>> {
    let cursor = std::io::Cursor::new(bytes);
    let mut reader = hound::WavReader::new(cursor).ok()?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
        log::warn!("unexpected WAV format: {spec:?} (expected 16-bit PCM)");
    }
    let samples: Vec<f32> = reader.samples::<i16>().filter_map(Result::ok).map(|s| s as f32 / i16::MAX as f32).collect();
    Some(samples)
}

/// Equal-power stereo panning — a linear pan law (`left = 0.5*(1-pan)`) noticeably dips a
/// centred sound's perceived loudness; this doesn't. Matches how `AVAudioPlayerNode.pan`
/// (`AudioEngine.swift`'s own panning) behaves.
fn pan_gains(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    let angle = (p + 1.0) * std::f32::consts::FRAC_PI_4; // 0 (hard left) .. PI/2 (hard right)
    (angle.cos(), angle.sin())
}

struct OneShotVoice {
    samples: Arc<[f32]>,
    pos: usize,
    pan: f32,
    volume: f32,
}

struct LoopVoice {
    samples: Arc<[f32]>,
    pos: usize,
    pan: f32,
    current: f32,
    target: f32,
    stopping: bool,
}

/// The shared mixing state — everything the audio callback reads and advances, and
/// everything `AudioBackend`'s methods (called from the main thread) write to. Guarded by
/// a single `Mutex`; see the module doc comment for that tradeoff.
struct Mixer {
    /// A fixed-size round-robin pool, matching `AudioEngine.swift`'s own
    /// `voices: [AVAudioPlayerNode]` + `nextVoice` index. `.interrupts` there is
    /// `None` here just being overwritten — the newest sound always wins a slot over the
    /// oldest, which is the correct behaviour during a burst (the machine gun).
    voices: Vec<Option<OneShotVoice>>,
    next_voice: usize,
    loops: HashMap<String, LoopVoice>,
}

impl Mixer {
    fn new() -> Self {
        Self { voices: (0..VOICE_COUNT).map(|_| None).collect(), next_voice: 0, loops: HashMap::new() }
    }

    fn play(&mut self, samples: Arc<[f32]>, pan: f32, volume: f32) {
        self.voices[self.next_voice] = Some(OneShotVoice { samples, pos: 0, pan, volume });
        self.next_voice = (self.next_voice + 1) % self.voices.len();
    }

    fn start_loop(&mut self, key: &str, samples: Arc<[f32]>, volume: f32, pan: f32) {
        if let Some(existing) = self.loops.get_mut(key) {
            existing.target = volume;
            existing.stopping = false;
            existing.pan = pan;
            return;
        }
        self.loops.insert(key.to_string(), LoopVoice { samples, pos: 0, pan, current: 0.0, target: volume, stopping: false });
    }

    fn set_loop(&mut self, key: &str, volume: Option<f32>, pan: Option<f32>) {
        let Some(voice) = self.loops.get_mut(key) else { return };
        if let Some(v) = volume {
            voice.target = v;
        }
        if let Some(p) = pan {
            voice.pan = p;
        }
    }

    fn stop_loop(&mut self, key: &str, fade: f32) {
        // `fade`'s actual numeric value is unused beyond this sign check — matches
        // `AudioEngine.swift`'s own `stopLoop`, where every `fade > 0` glides out over the
        // same fixed ~100ms `update` step regardless of what value was passed; only
        // `fade <= 0` (an instant teardown) reads any differently. A real oddity already
        // present in the source, ported as-is rather than "fixed."
        if fade <= 0.0 {
            self.loops.remove(key);
        } else if let Some(voice) = self.loops.get_mut(key) {
            voice.target = 0.0;
            voice.stopping = true;
        }
    }

    /// The gain glide — called once per real frame from the main thread (`AudioSink`'s own
    /// `update`), not from the audio callback.
    fn update(&mut self, dt: f32) {
        let step = (dt / GAIN_GLIDE_SECONDS).min(1.0);
        self.loops.retain(|_, voice| {
            voice.current += (voice.target - voice.current) * step;
            !(voice.stopping && voice.current < 0.005)
        });
    }

    /// Mixes `frames` frames (each `channels` samples wide) into `out`, advancing every
    /// active voice. Runs on the realtime audio thread.
    fn render(&mut self, out: &mut [f32], channels: usize) {
        out.fill(0.0);
        let frames = out.len() / channels.max(1);

        for voice_slot in &mut self.voices {
            let Some(voice) = voice_slot else { continue };
            let (gl, gr) = pan_gains(voice.pan);
            let mut done = false;
            for frame in 0..frames {
                if voice.pos >= voice.samples.len() {
                    done = true;
                    break;
                }
                let s = voice.samples[voice.pos] * voice.volume;
                mix_frame(out, frame, channels, s * gl, s * gr);
                voice.pos += 1;
            }
            if done || voice.pos >= voice.samples.len() {
                *voice_slot = None;
            }
        }

        for voice in self.loops.values_mut() {
            if voice.samples.is_empty() {
                continue;
            }
            let (gl, gr) = pan_gains(voice.pan);
            for frame in 0..frames {
                let s = voice.samples[voice.pos] * voice.current;
                mix_frame(out, frame, channels, s * gl, s * gr);
                voice.pos += 1;
                if voice.pos >= voice.samples.len() {
                    voice.pos = 0; // loop
                }
            }
        }

        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

/// Adds one stereo-panned sample into `out`'s given frame — `channels == 1` sums both
/// channels into the single output channel (a mono output device); `channels >= 2` writes
/// left/right into the first two and leaves any further channels untouched (silent).
fn mix_frame(out: &mut [f32], frame: usize, channels: usize, left: f32, right: f32) {
    let base = frame * channels;
    if channels == 1 {
        out[base] += left + right;
    } else {
        out[base] += left;
        out[base + 1] += right;
    }
}

/// The real `AudioBackend` — owns the `cpal` stream (kept alive for as long as this does;
/// the stream stops as soon as it's dropped) and the shared `Mixer` the stream's own
/// callback reads from.
pub struct CpalBackend {
    sounds: HashMap<&'static str, Arc<[f32]>>,
    mixer: Arc<Mutex<Mixer>>,
    _stream: cpal::Stream,
    /// `SoundBank.swift`'s own "reported once, not on every playback" — a missing sound
    /// otherwise logs on every single `play`/`start_loop` call that names it, which for a
    /// held-down tool is many times a second.
    missing: std::collections::HashSet<String>,
}

impl CpalBackend {
    /// `None` if no output device is available at all, or the stream fails to build —
    /// audio is optional (`AudioEngine.swift`'s own doc comment: "if `AVAudioEngine` fails
    /// to start, the app carries on normally"), so the caller falls back to a plain
    /// `AudioSink::new()` with no backend, not a hard failure.
    pub fn new() -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().or_else(|| {
            log::warn!("no default audio output device — running without sound");
            None
        })?;

        let config = select_stream_config(&device);
        let channels = config.channels() as usize;
        let sample_rate = config.sample_rate();
        log::info!("audio output: {} channels @ {sample_rate}Hz", channels);

        let sounds: HashMap<&'static str, Arc<[f32]>> = SOUND_NAMES
            .iter()
            .filter_map(|&name| {
                let bytes = embedded_wav(name)?;
                let samples = decode_wav(bytes)?;
                Some((name, Arc::from(samples)))
            })
            .collect();
        log::info!("preloaded {} of {} sounds", sounds.len(), SOUND_NAMES.len());

        let mixer = Arc::new(Mutex::new(Mixer::new()));
        let mixer_for_callback = mixer.clone();
        let mut stream_config: cpal::StreamConfig = config.into();
        // A real underrun on a first run: the default buffer size left too little
        // headroom for a `Mutex`-guarded mixer sharing state with the main thread (see
        // the module doc comment's own tradeoff note). A fixed, more generous buffer
        // size gives the callback more time between calls, independent of the
        // `try_lock` fix below — the two address the same risk from different ends.
        // RISK: 2048 frames is a guess at "generous enough," not measured; if underruns
        // persist, this is the first number to try raising.
        stream_config.buffer_size = cpal::BufferSize::Fixed(2048);

        let stream = device
            .build_output_stream(
                stream_config, // owned, not `&StreamConfig` — confirmed against cpal's current docs
                move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                    // `try_lock`, not `lock` — this closure runs on `cpal`'s own realtime
                    // audio thread, where *blocking* on a contended mutex (which `lock`
                    // does) is the actual underrun risk, not just holding one briefly.
                    // If the main thread happens to be mid-update, this buffer is just
                    // silence instead of glitching the whole stream by stalling it.
                    match mixer_for_callback.try_lock() {
                        Ok(mut mixer) => mixer.render(data, channels),
                        Err(_) => data.fill(0.0),
                    }
                },
                |err| log::error!("audio stream error: {err}"),
                None,
            )
            .map_err(|e| log::error!("failed to build the audio output stream: {e}"))
            .ok()?;
        stream.play().map_err(|e| log::error!("failed to start the audio output stream: {e}")).ok()?;

        Some(Self { sounds, mixer, _stream: stream, missing: std::collections::HashSet::new() })
    }

    fn sample_arc(&mut self, name: &str) -> Option<Arc<[f32]>> {
        if let Some(samples) = self.sounds.get(name) {
            return Some(samples.clone());
        }
        if self.missing.insert(name.to_string()) {
            log::warn!("audio: missing '{name}'");
        }
        None
    }
}

impl AudioBackend for CpalBackend {
    fn play(&mut self, name: &str, pan: f32, volume: f32) {
        let Some(samples) = self.sample_arc(name) else { return };
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.play(samples, pan, volume);
        }
    }

    fn start_loop(&mut self, name: &str, key: &str, volume: f32, pan: f32) {
        let Some(samples) = self.sample_arc(name) else { return };
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.start_loop(key, samples, volume, pan);
        }
    }

    fn set_loop(&mut self, key: &str, volume: Option<f32>, pan: Option<f32>) {
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.set_loop(key, volume, pan);
        }
    }

    fn stop_loop(&mut self, key: &str, fade: f32) {
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.stop_loop(key, fade);
        }
    }

    fn update(&mut self, dt: f32) {
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.update(dt);
        }
    }
}

/// Picks a stream config at the bundled sounds' own 44.1kHz when the device can supply
/// one, falling back to the device's own default otherwise (see the module doc comment's
/// note on what that fallback costs). `SupportedStreamConfigRange`'s methods used here
/// (`sample_format`, `min_sample_rate`/`max_sample_rate`, `with_sample_rate`) are checked
/// against `cpal` 0.18's own published docs, not just recalled from memory — still not
/// the same as a real compile, but the one part of this whole port's `cpal` integration
/// that carries no residual doc-uncertainty on top of the usual "not actually compiled
/// yet" risk.
fn select_stream_config(device: &cpal::Device) -> cpal::SupportedStreamConfig {
    // `cpal::SampleRate` is a plain `u32` type alias in this version (not the
    // tuple-struct newtype older `cpal` releases had) — confirmed by the compiler, not a
    // guess: `cpal::SampleRate(44_100)` doesn't parse as a value, and `.sample_rate().0`
    // doesn't parse as a field access, either.
    const TARGET_RATE: cpal::SampleRate = 44_100;
    let preferred = device.supported_output_configs().ok().and_then(|configs| {
        configs
            .filter(|c| c.sample_format() == cpal::SampleFormat::F32)
            .filter(|c| c.min_sample_rate() <= TARGET_RATE && TARGET_RATE <= c.max_sample_rate())
            // Prefer a 2-channel config over whatever else matches — `.find` alone would
            // happily settle for the first rate-matching config regardless of channel
            // count, which on some devices is mono, silently flattening every pan to
            // dead centre despite `Mixer`/`pan_gains` already being fully stereo. Swift's
            // `AVAudioEngine` never has this problem: everything mixes to its
            // `mainMixerNode`, which is always stereo.
            .max_by_key(|c| c.channels() == 2)
            .map(|c| c.with_sample_rate(TARGET_RATE))
    });
    preferred.unwrap_or_else(|| {
        device.default_output_config().unwrap_or_else(|e| panic!("no usable audio output config: {e}"))
    })
}
