# Phase 5 — build & run walkthrough (chunk 1: the `AudioBackend` boundary)

Goal: real sound, via `cpal` — a 24-voice round-robin pool for one-shots, keyed loop
voices with a ~100ms gain glide, hand-summed stereo panning kept character-for-character
with `AudioEngine.swift`'s own formula. This chunk is the architecture underneath all of
that, not the `cpal` engine itself yet — see "What's next" below.

## What changed

**`hakai-core/src/audio.rs` gained an `AudioBackend` trait.** `AudioSink` was a genuine
no-op through Phases 3–4 (deliberately — matching `ToolSimulation.swift`'s rig, which
never starts the real audio engine either, so headless checks don't need an audio device).
It still is, *by default*. But it can now hold `Option<Box<dyn AudioBackend>>`, and every
one of its four calls (`play`/`start_loop`/`set_loop`/`stop_loop`) forwards to that backend
if one's plugged in, in addition to (in test builds) still recording the call as an event.
A fifth method, `update(dt)`, exists for the loop gain-glide.

The trait — not a concrete `cpal` dependency — is what lets `hakai-core` keep the promise
its own crate doc comment makes: "no window, no audio, no compositor... builds and runs
with plain `cargo test` on any OS." A real audio device is exactly the kind of thing that
would break that on a headless CI box or a container. `hakai`, the renderer binary, is
where `cpal` will actually live — it already isn't headless (it needs a live Wayland
compositor and a GPU), so it's the natural place for a real audio device dependency too.

**`AudioSink` moved from `GpuLayer` to `State` — i.e., from per-output to shared.** This
wasn't a Phase 5 requirement on its own; it was a latent bug this phase's own work
surfaced. `AudioEngine.swift` is one instance the app hands to *every* `GameScene`, not one
per screen (`AudioEngine.swift`'s own "only the first scene drives its update loop"
comment only makes sense if the loop state being advanced is shared in the first place) —
a real audio device is physically singular, so two independent per-output engines would
mean two overlapping output streams fighting over the same speakers. The per-output
`GpuLayer.audio: AudioSink` this port had built through chunks 1–4e never surfaced this,
since a no-op doesn't care how many copies of it exist. Fixed now, before there's a real
backend to actually double up: `audio` is `State`-level now, alongside `decals`/`icons`/
`sprites` (`particles`/`termites`/`rng` stay genuinely per-output — those really are
independent per screen). `State::advance` picked up an explicit `audio: &mut AudioSink`
parameter (same pattern as its existing `decals` one), and exactly one output — whichever
is `self.layers.first()`, computed once per real compositor frame before any `GpuLayer` is
borrowed — calls `self.audio.update(dt)`, so the shared gain-glide state only ever
advances once per frame, not once per monitor.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

(covers `audio.rs`'s new backend-delegation tests — a `RecordingBackend` behind
`Box<dyn AudioBackend>`, checked via a shared `Rc<RefCell<_>>` handle since the backend
itself gets moved into the sink)

```bash
cd /mnt/mac/hakai
cargo build && RUST_LOG=info cargo run
```

Nothing should look or sound any different yet — `self.audio` still has no backend plugged
in (`AudioSink::new()`, same as before), so this chunk is purely a wiring/architecture
change. Worth running anyway to confirm the relocation compiles and nothing regressed
(tool sounds — which don't exist yet — obviously still won't play; damage stamping, HUD,
everything else should be unaffected).

## What "done" looks like

- [ ] `hakai-core`'s tests pass, including the two new `audio.rs` backend tests
- [ ] `hakai` builds and runs with no visible/behavioral regression

## What's next

~~The actual `cpal` engine~~ — done, in this same phase's chunk 2 below.

---

# Phase 5 — chunk 2: the real `cpal` engine

`hakai/src/audio.rs` — a full `AudioBackend` implementation: the 35 bundled sounds
(`hakai/assets/sounds/*.wav`, copied from the macOS app's `Resources/sounds/`, all
confirmed mono/16-bit-PCM/44.1kHz via `file`), a 24-voice round-robin pool for one-shots,
keyed loop voices with the ~100ms gain glide, and equal-power stereo panning.

**The thread boundary was the real design question**, exactly as flagged at the end of
chunk 1 — resolved with a `Mutex<Mixer>` shared between the main thread (`AudioBackend`'s
methods, called from `ToolContext`) and `cpal`'s own dedicated realtime callback thread.
The main thread locks briefly to record intent ("play this," "this loop's target volume
changed"); the callback locks once per audio buffer to mix and advance every active voice.
A `Mutex` on a realtime audio thread is a known, real tradeoff — a slow lock holder can
cause an audible glitch — accepted deliberately, the same "correct first, revisit only if
it actually stutters" call already made once before in this port (the termites' per-frame
buffer allocation).

**Every `cpal` API call here is checked against the crate's live docs, not just recalled.**
`cpal` is a stable, long-established crate, but this is still the first OS-audio-subsystem
integration in the whole port — a different subsystem (ALSA/PipeWire) than everything else
that's touched real hardware so far (`wgpu`/`smithay-client-toolkit`/`wayland-backend`,
all GPU/compositor-side). Fetched `docs.rs/cpal` directly for `HostTrait`/`DeviceTrait`/
`StreamTrait`'s exact signatures rather than trusting memory alone — worth calling out
since it caught a real mismatch before it ever reached a compiler: `build_output_stream`
takes `config` *by value* (`StreamConfig`), not `&StreamConfig`, which is what a first pass
here had written.

**Sample-rate mismatch is a known, accepted gap, not an oversight.** `select_stream_config`
asks the device for a stream at the sounds' own native 44.1kHz when it can supply one; if
the device can't (48kHz is common), no resampling happens, and every sound plays back at a
slightly wrong pitch/speed. Flagged in the module doc comment rather than silently
accepted as exact.

**`AudioEngine.swift`'s `stopLoop(_:fade:)` has an odd quirk, ported faithfully rather than
"fixed."** The `fade` parameter's actual numeric value is never read by `update`'s gain
glide (a fixed ~100ms step regardless) — only its *sign* matters (`fade <= 0` tears the
voice down instantly; any `fade > 0` glides out over the same fixed duration no matter
whether `0.1` or `2.0` was passed). Real, already present in the source; ported exactly,
flagged in a comment rather than silently corrected.

## Build & run

```bash
cd /mnt/mac/hakai-core
cargo test
```

(unchanged — this chunk touched only `hakai`)

```bash
cd /mnt/mac/hakai
cargo build && RUST_LOG=info cargo run
```

Watch the log for `audio: cpal backend started` and `preloaded 35 of 35 sounds` (if it
instead logs `no backend available`, audio failed to start but the app should still run
normally — see the RISK/fallback notes in `audio.rs`). Then: hammer (`1`, click) should
have an impact sound; chain-saw (`2`, just move the mouse without clicking) should idle,
and cutting should crossfade to a cutting sound; machine gun (`3`, hold) should fire
repeatedly with an occasional reverberation and shells ringing as they land; flame-thrower
(`4`, hold) should have a begin/loop/end sequence; color-thrower (`5`) should have
occasional paint-drop sounds; phaser (`6`), stamp (`7`), termites (`8`, listen for
chewing/squishing), washer (`9`, listen for a start + loop) should all have their own
sounds. Panning should follow the cursor's X position — left near the left edge, right
near the right.

## What "done" looks like

- [ ] `hakai` builds with `cpal`/`hound` in the dependency tree
- [ ] The log shows the backend starting and all 35 sounds preloading
- [ ] Every tool that plays a sound in the Swift original is audible here too
- [ ] Loop crossfades (chain-saw idle/cutting, flame begin/loop/end) sound smooth, not
      clicky or abrupt
- [ ] Panning audibly follows cursor position
- [ ] Nothing visual regressed

If that holds, Phase 5 is done — the last audio-shaped gap in this port is closed. Whatever
comes packaged as Phase 6+ (`wlr-screencopy`, multi-output scaling, the Omarchy skin,
packaging) per `OMARCHY-PORT.md` is next.

**Confirmed working** on a real run ("works!") — one fix needed first: `cpal::SampleRate`
turned out to be a plain `u32` type alias in the version that resolved here, not the
tuple-struct newtype older `cpal` releases had (`cpal::SampleRate(44_100)` and
`.sample_rate().0` both failed to parse — the compiler's own error, not a guess). Both
call sites fixed to treat it as a bare `u32`.

**Follow-up: audio wasn't actually stereo.** `select_stream_config` picked the *first*
device config matching the sounds' own sample rate and format, regardless of channel
count — on the machine tested on, that happened to be a mono config, silently flattening
every `pan_gains` computation to dead centre even though `Mixer::render` was already
fully stereo-capable end to end. Swift's `AVAudioEngine` never hits this: everything
mixes to its `mainMixerNode`, which is always stereo, so there was never a config to
"pick" in the first place. Fixed by explicitly preferring a 2-channel config among the
rate-matching ones (`max_by_key(|c| c.channels() == 2)`) instead of taking whichever
came first.

**A real underrun, on the first actual run — exactly the risk the module doc comment
already flagged.** `Mutex::lock()` *blocks* until available; blocking on a contended lock
from the realtime audio callback thread is precisely the anti-pattern that causes an
audible glitch, not just holding the lock briefly. Fixed properly: `try_lock()` instead —
if the main thread happens to be mid-update when a buffer's due, that one buffer is
silence, not a stalled stream — plus a larger fixed buffer size (`2048` frames, a guess at
"generous enough," not measured) for more scheduling headroom. Confirmed gone under a
machine-gun burst, the mixer's heaviest real workload.

**Also cleaned up leftover debug-level logging.** Several `log::info!` calls added during
earlier bug hunts (the frame-callback bug, the cursor entry-position bug) fired on every
tile upload, every pointer entry, and every click — reasonable while chasing those bugs,
noisy now that they're resolved. Downgraded to `log::debug!` (still there if a similar
bug needs the same kind of hunt again, just not printing under plain `RUST_LOG=info`);
genuine one-time startup diagnostics (audio backend status, asset counts, capability
binding) were left at `info`.
