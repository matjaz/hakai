Hakai (破壊) — macOS build
==========================

Smash, burn and repaint your live desktop with nine tools, then watch it wander off
on its own (termites) or wipe it clean again (the washer).

A transparent overlay sits above your real desktop — menu bar and Dock included —
on every display.


Installing
----------

Drag Hakai.app onto Applications, then launch it.

This build isn't notarised by Apple, so the first launch is blocked ("Apple could
not verify Hakai…"). Either:

  - open System Settings → Privacy & Security, scroll down and click
    "Open Anyway" next to the Hakai message, then launch it again; or
  - in Terminal:  xattr -dr com.apple.quarantine /Applications/Hakai.app

Universal binary: runs natively on Apple silicon and Intel Macs, macOS 11 or newer.


Keys
----

  1-9        pick a tool
  Tab / Shift+Tab   cycle tools
  Up / Down  open / close the tool palette
  M          freeze the view to a snapshot (the real desktop keeps changing underneath)
  C          credits
  R          clear everything
  Esc / ⌘Q   quit

Left mouse button uses the current tool.


Tools
-----

  1  Hammer         cracks the desktop where you strike
  2  Chain-saw      cuts continuously while dragged
  3  Machine gun    bullet holes, ejected shells, muzzle flash
  4  Flame-thrower  standing fires that spread and burn out into scorch marks
  5  Color-thrower  paint splats
  6  Phaser         a sustained beam
  7  Stamp          REJECTED / APPROVED / TOP SECRET / ...
  8  Termites       bugs that wander the desktop and eat it
  9  Washer         the only repair tool — wipes damage away along the stroke


Screen Recording permission (optional)
--------------------------------------

The impact sound follows the brightness of whatever's under the cursor — a hit on a
dark background sounds hollow, on a light one glassy — and M's frozen snapshot is a
picture of the desktop. Both need to read the screen, which macOS gates behind the
Screen Recording permission. The first launch asks for it; grant it in System
Settings → Privacy & Security → Screen & System Audio Recording, then relaunch.

Without it everything else works the same and a random impact variant is used.


Notes
-----

- Multi-monitor: one overlay per display. Brightness is sampled from the main display.
- HAKAI_WINDOWED=1 runs it in an ordinary window instead of over the desktop;
  HAKAI_NO_CAPTURE=1 skips the screen capture (and its permission prompt).


Building
--------

From a checkout of the source repository:

    cargo run --release --manifest-path hakai-mac/Cargo.toml
    hakai-mac/package.sh

package.sh builds a universal binary (Apple silicon + Intel) and writes
hakai-mac/target/hakai-<version>-macos-universal.dmg. Ad-hoc signed by default.
Set SIGN_IDENTITY to a Developer ID and NOTARY_PROFILE to a keychain profile to
sign, notarise, and staple instead — see the header of package.sh.

Metal draws into a non-opaque CAMetalLayer. The overlay is lifted to the
screensaver window level so it covers the menu bar and the Dock, on every Space
and every display. The brightness-driven impact sound reads the screen with
CGWindowListCreateImage, excluding the overlay itself.


Status
------

Functional — overlay above the menu bar and Dock, all nine tools, audio, a
universal DMG — verified on Apple silicon (M3 Pro). Intel and multi-monitor
setups are built but not yet tried on real hardware.


Releases
--------

CI builds the universal DMG on each push. A v* tag that matches this crate's
version (and the same version in hakai-linux, hakai-win, and hakai-android)
publishes it. See .github/workflows/ci.yml.


License
-------

MIT (see LICENSE.txt). Bundled fonts are SIL OFL 1.1; bundled sounds are a mix of
CC BY 4.0 / CC0 / Public Domain — see CREDITS.md for the full per-file breakdown.
Behaviour is derived from Desktop Destroyer by Miroslav Nemecek; no code or asset
from the original is included.
