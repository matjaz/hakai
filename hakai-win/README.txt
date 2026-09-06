Hakai (破壊) — Windows build
============================

Smash, burn and repaint your live desktop with nine tools, then watch it wander off
on its own (termites) or wipe it clean again (the washer).

A full-screen, transparent, always-on-top overlay sits above your real desktop.


Running
-------

Double-click hakai-win.exe. That's it — nothing to install, no dependencies.

SmartScreen ("Windows protected your PC") may show once, because this build is
unsigned: click "More info" -> "Run anyway".


Keys
----

  1-9        pick a tool
  Tab / Shift+Tab   cycle tools
  Up / Down  open / close the tool palette
  M          freeze the view to a snapshot (the real desktop keeps changing underneath)
  C          credits
  R          clear everything
  Esc        quit

Left mouse button uses the current tool. Alt+Tab, the Windows key and
Ctrl+Alt+Del all still work — the overlay can never trap you.


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


Notes
-----

- Multi-monitor: one overlay per display.
- If the overlay renders at the wrong size (some Remote Desktop sessions report a
  bogus DPI scale), set HAKAI_WINDOWED=1 to run it as a plain resizable window.
- The impact sound follows the brightness of whatever's under the cursor. If that
  never varies, Desktop Duplication isn't available (a Remote Desktop session, or
  some hybrid-graphics laptops) and a random variant is used instead.


License
-------

MIT (see LICENSE). Bundled fonts are SIL OFL 1.1; bundled sounds are a mix of
CC BY 4.0 / CC0 / Public Domain — see CREDITS.md for the full per-file breakdown.
Behaviour is derived from Desktop Destroyer by Miroslav Nemecek; no code or asset
from the original is included.
