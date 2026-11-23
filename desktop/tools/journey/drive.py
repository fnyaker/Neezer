#!/usr/bin/env python3
"""Drive the real app on an X display the way a person does, with no root.

The journey bench has to press keys in a real GPUI window, resize it, hide it
(to prove MPRIS keeps moving on another workspace) and take pictures. `xdotool`
and `xte` are not part of a stock Arch install, and `tc`/`sudo` are out of the
question on the shared machine, so this module speaks X11 directly through
`python-xlib` (pure Python, `pip install python-xlib`; only the benchmark uses
it — the app has no Python dependency).

Everything here is XTEST: the app receives ordinary key and mouse events, the
same path a keyboard takes, so nothing is stubbed and no debug hook exists in
the shipped binary. `DISPLAY` selects the screen, and this module REFUSES to
run on a display it did not choose: the acceptance never touches the person's
Hyprland session (`docs/agents/desktop.md`, *Pitfalls found by measuring*).

The Go/Chrome-style details that cost D7 and D17 time:

* **No window manager means no focus.** Xvfb has no WM, so a new window is not
  focused and every `g h` goes nowhere. `focus()` sets X input focus on the
  biggest root child (the app's window; an input-method helper is a 1×1 named
  child, and it is not the largest).
* **A resize MOVES the window to 0,0 and, on this build, stops it receiving
  pointer clicks** (D7). The bench resizes after the first frame, drives the
  keyboard only, and asserts the geometry it measured.
* **`import -window root` can block on a software-rendered 2600×1700 root.** The
  screenshot here reads the pixels through `GetImage` and pipes them to
  ImageMagick's `convert`, which never takes the X server's grab.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import time

from pathlib import Path

from Xlib import X, XK, display as xdisplay
from Xlib.ext import xtest

# How long a key press is held. GPUI reads the press; the release follows in
# the same millisecond, as a keyboard does.
KEY_HOLD = 0.012
# Between two strokes of one `g h` sequence: long enough that the keymap's
# prefix state is established, short enough to stay one gesture.
STROKE_GAP = 0.030


class Screen:
    def __init__(self, display_name: str) -> None:
        assert_isolated(display_name)
        self.display_name = display_name
        self.display = xdisplay.Display(display_name)
        self.root = self.display.screen().root
        self._codes: dict[str, int] = {}
        self._modifiers: dict[str, int] = {}

    # -- windows -----------------------------------------------------------

    def _children(self) -> list:
        try:
            return self.root.query_tree().children
        except Exception:
            return []

    def window(self, title: str = "Neezer"):
        """The app's window: the biggest mapped root child whose name matches."""
        best = None
        best_area = 0
        for child in self._children():
            try:
                geometry = child.get_geometry()
                attrs = child.get_attributes()
            except Exception:
                continue
            if attrs.map_state != X.IsViewable:
                continue
            area = geometry.width * geometry.height
            if area <= 0:
                continue
            name = ""
            try:
                name = child.get_wm_name() or ""
            except Exception:
                pass
            if title and name and title.lower() not in name.lower():
                # An unnamed child (an input-method helper) is 1×1 in practice.
                if area < 100 * 100:
                    continue
            if area > best_area:
                best, best_area = child, area
        return best

    def wait_window(self, title: str = "Neezer", timeout: float = 30.0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            window = self.window(title)
            if window is not None:
                return window
            time.sleep(0.05)
        raise TimeoutError(f"no window titled {title!r} on {self.display_name}")

    def focus(self, window) -> None:
        window.set_input_focus(X.RevertToParent, X.CurrentTime)
        self.display.sync()

    def resize(self, window, width: int, height: int) -> tuple[int, int]:
        assert_isolated(self.display_name)
        window.configure(width=width, height=height)
        self.display.sync()
        time.sleep(0.25)
        geometry = window.get_geometry()
        return geometry.width, geometry.height

    def hide(self, window) -> None:
        assert_isolated(self.display_name)
        window.unmap()
        self.display.sync()
        time.sleep(0.25)

    def show(self, window) -> None:
        window.map()
        self.display.sync()
        time.sleep(0.25)

    def is_viewable(self, window) -> bool:
        try:
            return window.get_attributes().map_state == X.IsViewable
        except Exception:
            return False

    # -- keys --------------------------------------------------------------

    def _code(self, keysym_name: str) -> int:
        if keysym_name not in self._codes:
            keysym = XK.string_to_keysym(keysym_name)
            code = self.display.keysym_to_keycode(keysym)
            if not code:
                raise KeyError(f"no keycode for {keysym_name}")
            self._codes[keysym_name] = code
        return self._codes[keysym_name]

    def _modifier(self, name: str) -> int:
        if name not in self._modifiers:
            self._modifiers[name] = self._code(name)
        return self._modifiers[name]

    def stroke(self, spec: str) -> None:
        """One stroke: `h`, `g`, `Return`, `ctrl-right`, `shift-enter`."""
        assert_isolated(self.display_name)
        parts = spec.lower().split("-")
        key = parts[-1]
        modifiers = {"ctrl": "Control_L", "alt": "Alt_L", "shift": "Shift_L",
                     "super": "Super_L"}
        held = []
        for name in parts[:-1]:
            if name in ("ctrl", "alt", "shift", "super"):
                held.append(self._modifier(modifiers[name]))
        alias = {
            "return": "Return",
            "enter": "Return",
            "escape": "Escape",
            "space": "space",
            "left": "Left",
            "right": "Right",
            "up": "Up",
            "down": "Down",
            "home": "Home",
            "end": "End",
            "pagedown": "Next",
            "pageup": "Prior",
            "delete": "Delete",
            "menu": "Menu",
            ",": "comma",
            "/": "slash",
            "?": "question",
        }
        keysym = alias.get(key, key)
        code = self._code(keysym)
        for modifier in held:
            xtest.fake_input(self.display, X.KeyPress, modifier)
        self.display.sync()
        time.sleep(KEY_HOLD)
        xtest.fake_input(self.display, X.KeyPress, code)
        self.display.sync()
        time.sleep(KEY_HOLD)
        xtest.fake_input(self.display, X.KeyRelease, code)
        for modifier in reversed(held):
            xtest.fake_input(self.display, X.KeyRelease, modifier)
        self.display.sync()

    def keys(self, spec: str) -> None:
        """A sequence: `g h`, `g l`, `Escape` — one stroke per space."""
        strokes = [part for part in spec.split(" ") if part]
        for index, stroke in enumerate(strokes):
            self.stroke(stroke)
            if index + 1 < len(strokes):
                time.sleep(STROKE_GAP)

    def click(self, x: int, y: int) -> None:
        assert_isolated(self.display_name)
        xtest.fake_input(self.display, X.MotionNotify, x=x, y=y)
        self.display.sync()
        time.sleep(0.02)
        xtest.fake_input(self.display, X.ButtonPress, 1)
        self.display.sync()
        time.sleep(KEY_HOLD)
        xtest.fake_input(self.display, X.ButtonRelease, 1)
        self.display.sync()

    def drag(self, from_xy: tuple[int, int], to_xy: tuple[int, int]) -> None:
        """A press, a move in steps, a release: what `on_drag` + `on_drop` read."""
        assert_isolated(self.display_name)
        x, y = from_xy
        xtest.fake_input(self.display, X.MotionNotify, x=x, y=y)
        self.display.sync()
        time.sleep(0.02)
        xtest.fake_input(self.display, X.ButtonPress, 1)
        self.display.sync()
        time.sleep(0.05)
        tx, ty = to_xy
        steps = 12
        for step in range(1, steps + 1):
            xtest.fake_input(
                self.display,
                X.MotionNotify,
                x=int(x + (tx - x) * step / steps),
                y=int(y + (ty - y) * step / steps),
            )
            self.display.sync()
            time.sleep(0.02)
        time.sleep(0.05)
        xtest.fake_input(self.display, X.ButtonRelease, 1)
        self.display.sync()

    # -- pictures ----------------------------------------------------------

    def wheel(self, notches: int = 1, down: bool = True,
              at: tuple[int, int] | None = None) -> None:
        """Mouse-wheel notches: X11 buttons 4 (up) and 5 (down), which GPUI
        turns into `ScrollWheelEvent`s. What a person's fingers do on a page,
        so a scroller is measured through the same path in every build."""
        assert_isolated(self.display_name)
        button = 5 if down else 4
        if at is not None:
            x, y = at
            xtest.fake_input(self.display, X.MotionNotify, x=x, y=y)
            self.display.sync()
            time.sleep(0.01)
        for _ in range(notches):
            xtest.fake_input(self.display, X.ButtonPress, button)
            self.display.sync()
            time.sleep(0.01)
            xtest.fake_input(self.display, X.ButtonRelease, button)
            self.display.sync()
            time.sleep(0.01)

    def screenshot(self, path: str, window=None) -> tuple[int, int]:
        """The root (or one window's box), without `import`'s grab."""
        assert_isolated(self.display_name)
        geometry = self.root.get_geometry()
        image = self.root.get_image(
            0, 0, geometry.width, geometry.height, X.ZPixmap, 0xFFFFFFFF
        )
        raw = image.data
        pixels = geometry.width * geometry.height
        if len(raw) == pixels * 4:
            # A 24-bit root answers 32 bits per pixel, BGRX: the fourth byte is
            # undefined, so it is dropped rather than handed to `convert` as an
            # alpha channel (`convert` read the four-byte stream as three and
            # failed on the truncated tail).
            packed = bytearray(pixels * 3)
            packed[0::3] = raw[0::4]
            packed[1::3] = raw[1::4]
            packed[2::3] = raw[2::4]
            raw = bytes(packed)
        tool = "magick" if shutil.which("magick") else "convert"
        command = [
            tool,
            "-size",
            f"{geometry.width}x{geometry.height}",
            "-depth",
            "8",
            "bgr:-",
        ]
        if window is not None:
            box = window.get_geometry()
            command += [
                "-crop",
                f"{box.width}x{box.height}+{box.x}+{box.y}",
                "+repage",
            ]
        command.append(path)
        subprocess.run(command, input=raw, check=True)
        return geometry.width, geometry.height


# HARD-CODED, and nothing may widen it at run time: the only displays this
# driver may touch. `:0` is the person's session, and a worker has already typed
# into it once (D17's incident): a script that falls back to `DISPLAY=:0` is a
# script that presses keys in the user's window.
RESERVED_FIRST = 130
RESERVED_LAST = 199


def assert_isolated(display_name: str) -> None:
    """Refuse everything that is not the Xvfb THIS bench started.

    Called before `XOpenDisplay`, before any injection, and again before every
    key, click, drag, resize, hide and screenshot. Three conditions, all
    required:

    1. the display is a number in ``RESERVED_FIRST..=RESERVED_LAST`` (written
       here, never read from the environment beyond the number itself);
    2. ``/tmp/nz-d17-display-<n>`` exists — the marker the bench's own
       ``Xvfb.start`` writes with ``O_EXCL``, so a *different* X server on the
       same number is not ours either;
    3. neither ``WAYLAND_DISPLAY`` nor ``XDG_SESSION_TYPE=wayland`` is set:
       GPUI prefers Wayland whatever ``DISPLAY`` says, and a bench that inherits
       them opens the window (and the keys go) on the person's session.
    """
    if not display_name:
        raise SystemExit("refusing an empty DISPLAY: this driver only drives its own Xvfb")
    number_text = display_name.lstrip(":").split(".")[0]
    if not number_text.isdigit():
        raise SystemExit(f"refusing DISPLAY={display_name!r}: not an X display number")
    number = int(number_text)
    if not RESERVED_FIRST <= number <= RESERVED_LAST:
        raise SystemExit(
            f"refusing DISPLAY={display_name!r}: this driver only drives "
            f":{RESERVED_FIRST}..:{RESERVED_LAST} (the person's session is :0)"
        )
    if not Path(f"/tmp/nz-d17-display-{number}").exists():
        raise SystemExit(
            f"refusing DISPLAY={display_name!r}: no marker from this bench's own "
            "Xvfb (the number is not ours)"
        )
    if not Path(f"/tmp/.X{number}-lock").exists():
        raise SystemExit(f"DISPLAY={display_name!r} is not a running X server")
    # `WAYLAND_DISPLAY` is the variable GPUI actually reads: with it set, the
    # window opens on the person's compositor whatever `DISPLAY` says. The
    # session-type variable alone is not a risk (the bench sets it to `x11` for
    # the app anyway), and refusing on it made every run on a Wayland desktop
    # fail — measured, the hardened check broke the bench itself.
    if os.environ.get("WAYLAND_DISPLAY"):
        raise SystemExit(
            "WAYLAND_DISPLAY is set: GPUI would open the window on the person's "
            "session. Run the bench with `env -u WAYLAND_DISPLAY`."
        )
