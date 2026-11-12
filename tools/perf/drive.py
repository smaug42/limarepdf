#!/usr/bin/env python3
"""REL-07's measurements of the app (the build plan's 6.3; the results in docs/developer/performance.md,
"Startup and responsiveness"): the release app driven on an X display of its own, timed from its timing
log (LIMAREPDF_TIMING) and from pictures of the screen.

    python3 -I tools/perf/drive.py MODE [ARG...] [--bin APP] [--set DIR] [--out DIR] [--runs N] [--cold] [--check]

Modes:
  startup FILE           the app started with FILE (a name in the set) until page 1 is drawn
  typing field|text|line|paragraph
                         200 keys, each 60 to 120 ms after the last one's frame: in form.pdf's first
                         field, in a new Add text box on letter.pdf (35 characters a line, then Enter),
                         at the end of letter.pdf's greeting line in Edit text (from about key 135 the
                         keys land past the view, "not seen"), or at the start of its first paragraph,
                         edited as a paragraph (set again and wrapped at each key)
  search 120|300         "annual" typed into Find on text-N.pdf, 50 ms between keys: the first search
                         after opening, then twice more (Ctrl+A, Backspace, a second's wait, typed again)
  working artwork|printers|search|save
                         keys ("e", then Backspace) or wheel turns, 100 ms apart, while artwork.pdf's page
                         is drawn, the printers are looked for (after Print is closed with Esc), or
                         text-10000.pdf is searched or saved
  print                  Ctrl+P on letter.pdf, the stand-in lpstat taking 4 s a call
  save letter|text-300   Ctrl+S with an Add text box open
  memory                 20 cycles of text-300.pdf opened (then from the start screen's Recent list), ten
                         pages down, searched and closed: the app's resident memory after each
  pages                  PB-37 on text-10000.pdf: page 1 moved to the end, undone, redone and undone;
                         page 1 deleted, undone and redone
  edits                  PB-43 on artwork.pdf, each edit in a launch of its own: a shape nudged with the
                         arrow key, a picture placed, a redaction applied
  redact                 PB-44: a redaction applied on text-10000.pdf
  select FILE            PB-52: the first line clicked in Edit text, on a corpus file (issue2618.pdf,
                         issue2956.pdf)
  ideograph              PB-61: 中 typed into a new Add text box set in Times, on letter.pdf
  try FILE STEP...       one launch for finding click points, a picture after each s:NAME step:
                         c:X,Y click   d:X,Y,X,Y drag   k:KEYS keys (ctrl+shift+z)   t:TEXT type
                         p:SECONDS wait   s:NAME

The documents come from --set (target/tmp/perf/refset; make it with `crates/limarepdf-core/tools/measure.py
--refset`); select's from the render corpus (tools/fetch-render-samples.sh). Each mode's run makes a folder
in --out (target/tmp/perf) named for it and the time, with result.json and summary.txt; each launch is a new
app there with settings, state and data folders of its own (light theme, recovery copies off) and copies of
the documents it changes. One Xvfb serves a mode's runs: 1600 x 1000, 24-bit, no window manager; the window
is 1200 x 850 at scale 1, at 0,0 (the points below are window pixels).

Warm: --runs N (10) after one discarded. --cold: N (5), the page cache dropped before each launch, which
needs root: run `sudo -v` first (this runs `sudo -n`). --check: one run, a picture of the window after
each step, for checking the points.

Times start when the input is sent (the launch, a key's press, a click's release) and end at "built", the
end of the frame that took the input (the timing log; the frame's last pass), and "shown", the end of the
first picture of the mode's area, taken every 5 ms, that differs from the last one before that frame
(startup: the first that matches the area's settled picture), before the next input; "not seen" when none
does. Each summary gives the pictures' achieved interval and cost: "shown" is good to about their sum,
"built" to about a millisecond. The accepted thresholds (6.3) are checked: "borderline" is within that
uncertainty of the limit.

The app runs with no desktop bus, CUPS_SERVER at a socket that isn't there, and stand-ins first on its
PATH: lpstat (4 s for each printer list, one printer, Stand_In), lpoptions and lp (both fail) and zenity
(the file chooser, answering docs/images/comments.png). No real printer is listed or used.

Needs Xvfb, libX11 and libXtst (through ctypes). Standard library only; Linux.
"""

import argparse
import ctypes
import ctypes.util
import hashlib
import json
import os
import random
import statistics
import struct
import subprocess
import sys
import textwrap
import threading
import time
import zlib
from bisect import bisect_left
from ctypes import CFUNCTYPE, POINTER, Structure, byref, c_char_p, c_int, c_ubyte, c_uint, c_ulong, c_void_p
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CORPUS = ROOT / "crates" / "limarepdf-core" / "tests" / "data" / "render"
PICTURE = ROOT / "docs" / "images" / "comments.png"
SCREEN, WINDOW, PARK = (1600, 1000), (0, 0, 1200, 850), (1590, 990)

# Where to click, in window pixels (the window at 0,0, 1200 x 850 at scale 1, the light theme).
AT = {
    "edit tab": (230, 59),
    "insert tab": (288, 59),
    "protect tab": (629, 59),
    "edit text": (583, 108),
    "select menu": (830, 108),
    "select objects": (916, 168),
    "font": (315, 93),
    "times": (283, 181),
    "add text": (52, 108),
    "image": (124, 108),
    "mark redaction": (113, 108),
    "apply redactions": (266, 108),
    "apply button": (653, 547),
    "close notice": (952, 170),
    "thumbnail 1": (94, 240),
    "recent 1": (447, 531),
    "field 1": (600, 348),
    "greeting": (280, 420),
    "paragraph 1": (300, 452),
    "edit as paragraph": (1084, 199),
    "new text": (230, 155),
    "hello text": (300, 240),
    "ideograph text": (300, 200),
    "shape": (249, 353),
    "picture": (400, 400),
    "view": (650, 500),
    "issue2618.pdf": (400, 304),
    "issue2956.pdf": (453, 269),
}
# Redaction marks dragged out, from one corner to the other.
MARK = {"artwork": ((300, 300), (420, 360)), "text-10000": ((280, 400), (500, 422))}
# The areas pictured, as x, y, width, height.
AREA = {
    "view": (185, 135, 790, 690),
    "field": (436, 330, 450, 38),
    "text": (220, 140, 420, 190),
    "line": (180, 405, 1020, 45),
    "paragraph": (270, 436, 420, 130),
    "results": (30, 228, 300, 152),
    "search box": (30, 165, 300, 40),
    "dialog": (300, 120, 600, 600),
    "status": (0, 822, 700, 28),
    "pages": (0, 140, 1200, 710),
}

# 200 keys of typing; the Add text box's in lines of 35 characters or fewer, Enter for the space between.
SCRIPT = (
    "the survey crews reached the northern camp before the rain and set up their tents along the river "
    "where the water ran fast and cold over the stones while the light faded and the wind came up off the lake"
)[:200]
SCRIPT_LINES = "\n".join(textwrap.wrap(SCRIPT, 35))
KEYS = {" ": "space", "\n": "Return", ",": "comma", ".": "period", "中": "ideograph"}
MODS = {"ctrl": "Control_L", "shift": "Shift_L", "alt": "Alt_L"}
IDEOGRAPH = 0x1004E2D  # the keysym for U+4E2D

THRESHOLDS = "6.3's accepted thresholds"


class Failed(Exception):
    pass


def us():
    return time.time_ns() // 1000


# ---- X, through ctypes ----

class XImage(Structure):
    """The start of Xlib's XImage: as far as the fields read here."""

    _fields_ = [
        ("width", c_int), ("height", c_int), ("xoffset", c_int), ("format", c_int), ("data", c_void_p),
        ("byte_order", c_int), ("bitmap_unit", c_int), ("bitmap_bit_order", c_int), ("bitmap_pad", c_int),
        ("depth", c_int), ("bytes_per_line", c_int), ("bits_per_pixel", c_int),
    ]


def load(name):
    return ctypes.CDLL(ctypes.util.find_library(name) or f"lib{name}.so.6")


X11, XTST = load("X11"), load("Xtst")
P, W = c_void_p, c_ulong
for f, res, args in [
    (X11.XInitThreads, c_int, []),
    (X11.XOpenDisplay, P, [c_char_p]),
    (X11.XCloseDisplay, c_int, [P]),
    (X11.XDefaultRootWindow, W, [P]),
    (X11.XQueryTree, c_int, [P, W, POINTER(W), POINTER(W), POINTER(POINTER(W)), POINTER(c_uint)]),
    (X11.XGetGeometry, c_int, [P, W, POINTER(W), POINTER(c_int), POINTER(c_int)] + [POINTER(c_uint)] * 4),
    (X11.XFree, c_int, [P]),
    (X11.XSetInputFocus, c_int, [P, W, c_int, c_ulong]),
    (X11.XGetInputFocus, c_int, [P, POINTER(W), POINTER(c_int)]),
    (X11.XGetImage, POINTER(XImage), [P, W, c_int, c_int, c_uint, c_uint, c_ulong, c_int]),
    (X11.XGetSubImage, POINTER(XImage),
     [P, W, c_int, c_int, c_uint, c_uint, c_ulong, c_int, POINTER(XImage), c_int, c_int]),
    (X11.XDestroyImage, c_int, [POINTER(XImage)]),
    (X11.XStringToKeysym, c_ulong, [c_char_p]),
    (X11.XKeysymToKeycode, c_ubyte, [P, c_ulong]),
    (X11.XDisplayKeycodes, c_int, [P, POINTER(c_int), POINTER(c_int)]),
    (X11.XGetKeyboardMapping, POINTER(c_ulong), [P, c_ubyte, c_int, POINTER(c_int)]),
    (X11.XChangeKeyboardMapping, c_int, [P, c_int, c_int, POINTER(c_ulong), c_int]),
    (X11.XFlush, c_int, [P]),
    (X11.XSync, c_int, [P, c_int]),
    (XTST.XTestFakeKeyEvent, c_int, [P, c_uint, c_int, c_ulong]),
    (XTST.XTestFakeButtonEvent, c_int, [P, c_uint, c_int, c_ulong]),
    (XTST.XTestFakeMotionEvent, c_int, [P, c_int, c_int, c_int, c_ulong]),
]:
    f.restype, f.argtypes = res, args
ERRORS = CFUNCTYPE(c_int, P, P)
X11.XSetErrorHandler.restype, X11.XSetErrorHandler.argtypes = P, [ERRORS]
# Errors (focus asked of a window not yet shown) are ignored: what they'd stop is checked for.
IGNORE = ERRORS(lambda d, e: 0)
ALL_PLANES, ZPIXMAP = (1 << 64) - 1, 2


class Display:
    """A connection to the X display: input sent through XTEST, and pictures."""

    def __init__(self, name):
        self.d = X11.XOpenDisplay(name.encode())
        if not self.d:
            raise Failed(f"can't open the display {name}")
        self.root = X11.XDefaultRootWindow(self.d)
        self.codes = {}
        self.image = None

    def close(self):
        if self.image:
            X11.XDestroyImage(self.image)
        X11.XCloseDisplay(self.d)

    def code(self, name):
        if name not in self.codes:
            sym = X11.XStringToKeysym(name.encode())
            self.codes[name] = (X11.XKeysymToKeycode(self.d, sym) if sym else 0) or None
        if not self.codes[name]:
            raise Failed(f"no key for {name!r}")
        return self.codes[name]

    def bind(self, name, keysym):
        """Puts KEYSYM on a key that has none, known here as NAME: before the app starts, so its keymap
        has it."""
        lo, hi, per = c_int(), c_int(), c_int()
        X11.XDisplayKeycodes(self.d, byref(lo), byref(hi))
        count = hi.value - lo.value + 1
        syms = X11.XGetKeyboardMapping(self.d, lo.value, count, byref(per))
        free = [i for i in range(count) if not any(syms[i * per.value + j] for j in range(per.value))]
        X11.XFree(syms)
        if not free:
            raise Failed("no key is free for the ideograph")
        code = lo.value + free[-1]
        X11.XChangeKeyboardMapping(self.d, code, per.value, (c_ulong * per.value)(*[keysym] * per.value), 1)
        X11.XSync(self.d, 0)
        self.codes[name] = code

    def keys(self, spec):
        """Presses and releases a key, with modifiers held (ctrl+shift+z)."""
        *mods, name = spec.split("+") if spec != "+" else ["+"]
        held = [self.code(MODS[m]) for m in mods]
        k = self.code(KEYS.get(name, name))
        for m in held:
            XTST.XTestFakeKeyEvent(self.d, m, 1, 0)
        XTST.XTestFakeKeyEvent(self.d, k, 1, 0)
        XTST.XTestFakeKeyEvent(self.d, k, 0, 0)
        for m in reversed(held):
            XTST.XTestFakeKeyEvent(self.d, m, 0, 0)
        X11.XFlush(self.d)

    def move(self, x, y):
        XTST.XTestFakeMotionEvent(self.d, -1, x, y, 0)
        X11.XFlush(self.d)

    def button(self, b, down):
        XTST.XTestFakeButtonEvent(self.d, b, 1 if down else 0, 0)
        X11.XFlush(self.d)

    def window(self):
        """The app's window, the largest of the root's (there is no window manager), and where it is."""
        root, parent, kids, n = W(), W(), POINTER(W)(), c_uint()
        X11.XQueryTree(self.d, self.root, byref(root), byref(parent), byref(kids), byref(n))
        best = None
        for i in range(n.value):
            g = [c_int(), c_int(), c_uint(), c_uint()]
            if X11.XGetGeometry(self.d, kids[i], byref(W()), byref(g[0]), byref(g[1]), byref(g[2]),
                                byref(g[3]), byref(c_uint()), byref(c_uint())):
                g = tuple(v.value for v in g)
                if best is None or g[2] * g[3] > best[1][2] * best[1][3]:
                    best = (kids[i], g)
        if kids:
            X11.XFree(kids)
        return best

    def focus(self, w):
        """Gives W the keyboard (with no window manager, nothing else does)."""
        f, r = W(), c_int()
        for _ in range(500):
            X11.XSetInputFocus(self.d, w, 2, 0)
            X11.XSync(self.d, 0)
            X11.XGetInputFocus(self.d, byref(f), byref(r))
            if f.value == w:
                return
            time.sleep(0.01)
        raise Failed("the window never took the keyboard")

    def picture(self, path, area=WINDOW):
        """A PNG of AREA of the screen."""
        x, y, w, h = area
        if not self.image:
            self.image = X11.XGetImage(self.d, self.root, 0, 0, SCREEN[0], SCREEN[1], ALL_PLANES, ZPIXMAP)
        X11.XGetSubImage(self.d, self.root, x, y, w, h, ALL_PLANES, ZPIXMAP, self.image, 0, 0)
        stride = self.image.contents.bytes_per_line
        data = ctypes.string_at(self.image.contents.data, stride * h)
        png(path, data, w, h, stride)


def png(path, data, w, h, stride):
    """Writes 32-bit BGRX pixels as an RGB PNG."""
    rows = bytearray()
    for y in range(h):
        px = data[y * stride : y * stride + w * 4]
        rgb = bytearray(w * 3)
        rgb[0::3], rgb[1::3], rgb[2::3] = px[2::4], px[1::4], px[0::4]
        rows += b"\0" + rgb

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    head = struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)
    Path(path).write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", head) + chunk(b"IDAT", zlib.compress(bytes(rows), 6))
                           + chunk(b"IEND", b""))


class Captures(threading.Thread):
    """Pictures of AREA every 5 ms (each requested 5 ms after the last was, or at once if it took longer),
    on a connection of its own: `shots` holds (start, cost, picture) in microseconds, the picture an
    index standing for its pixels, the same index for the same pixels."""

    def __init__(self, display, area):
        super().__init__(daemon=True)
        self.d = X11.XOpenDisplay(display.encode())
        if not self.d:
            raise Failed(f"can't open the display {display}")
        self.root = X11.XDefaultRootWindow(self.d)
        self.area, self.shots, self.done = area, [], threading.Event()
        self.starts = None

    def run(self):
        x, y, w, h = self.area
        image = X11.XGetImage(self.d, self.root, x, y, w, h, ALL_PLANES, ZPIXMAP)
        size = image.contents.bytes_per_line * h
        seen, last, index = {}, None, -1
        due = time.monotonic()
        while not self.done.is_set():
            t0 = time.time_ns()
            X11.XGetSubImage(self.d, self.root, x, y, w, h, ALL_PLANES, ZPIXMAP, image, 0, 0)
            data = ctypes.string_at(image.contents.data, size)
            if data != last:
                index = seen.setdefault(hashlib.blake2b(data, digest_size=16).digest(), len(seen))
                last = data
            self.shots.append((t0 // 1000, (time.time_ns() - t0) // 1000, index))
            due += 0.005
            wait = due - time.monotonic()
            if wait > 0:
                time.sleep(wait)
            else:
                due = time.monotonic()
        X11.XDestroyImage(image)
        X11.XCloseDisplay(self.d)

    def stop(self):
        self.done.set()
        self.join()
        self.starts = [s[0] for s in self.shots]

    def changed(self, after, until=None):
        """The end of the first picture started at or after AFTER that differs from the last one started
        before it, if one started before UNTIL."""
        i = bisect_left(self.starts, after)
        if i == 0 or i >= len(self.shots):
            return None
        before = self.shots[i - 1][2]
        for start, cost, k in self.shots[i:]:
            if until is not None and start >= until:
                return None
            if k != before:
                return start + cost
        return None

    def settled(self, after=0):
        """The end of the first picture started at or after AFTER that matches the last picture."""
        last = self.shots[-1][2]
        for start, cost, k in self.shots[bisect_left(self.starts, after):]:
            if k == last:
                return start + cost
        return None

    def spacing(self):
        """The intervals between pictures and their costs, in microseconds."""
        return [b[0] - a[0] for a, b in zip(self.shots, self.shots[1:])], [s[1] for s in self.shots]


# ---- The app and its timing log ----

class Log:
    """The app's timing log, read as it's written: `lines` of (epoch microseconds, event, detail)."""

    def __init__(self, path):
        self.path, self.file, self.part, self.lines, self.last_frame = path, None, b"", [], 0

    def poll(self):
        if self.file is None:
            try:
                self.file = open(self.path, "rb")
            except FileNotFoundError:
                return
        data = self.part + self.file.read()
        *whole, self.part = data.split(b"\n")
        for line in whole:
            epoch, _, event, detail = line.decode().split("\t", 3)
            self.lines.append((int(epoch), event, detail))
            if event == "frame built":
                self.last_frame = int(epoch)

    def find(self, event, start=0, after=0, test=lambda d: True):
        """The first line of EVENT from index START at or after the time AFTER whose detail passes TEST."""
        for i in range(start, len(self.lines)):
            t, e, d = self.lines[i]
            if e == event and t >= after and test(d):
                return i
        return None

    def frame_end(self, i):
        """The line ending the frame after line I: that frame's last pass."""
        j = self.find("frame built", i + 1)
        if j is None:
            return None
        frame = self.lines[j][2].split()[0]
        k = self.find("frame built", j + 1)
        while k is not None and self.lines[k][2].split()[0] == frame:
            j, k = k, self.find("frame built", k + 1)
        return j

    def at(self, i):
        return self.lines[i][0]


def pressed(detail):
    return detail.startswith("text") or detail.startswith("key") and detail.endswith("down")


class App:
    """One launch: the app on the session's display, its log, and pictures of AREA if given."""

    def __init__(self, s, args, area=None):
        s.count += 1
        self.s, self.x = s, s.x
        self.run = s.folder / f"run-{s.count:02d}"
        (self.run / "config" / "limarepdf").mkdir(parents=True)
        (self.run / "config" / "limarepdf" / "settings.txt").write_text("theme = light\nrecovery_copies = false\n")
        self.args = [self.copy(a) if isinstance(a, Copy) else a for a in args]
        self.log, self.sent, self.steps = Log(self.run / "timing.tsv"), 0, 0
        self.x.move(*PARK)
        if s.opts.cold:
            drop_caches()
        self.caps = None
        if area:
            self.caps = Captures(s.display, area)
            self.caps.start()
            while not self.caps.shots:
                time.sleep(0.001)
        env = {k: v for k, v in os.environ.items() if k not in ("WAYLAND_DISPLAY", "XAUTHORITY")}
        env.update(
            DISPLAY=s.display,
            PATH=f"{s.folder / 'standin'}:{os.environ.get('PATH', '/usr/bin:/bin')}",
            CUPS_SERVER=str(self.run / "no-such-cups.sock"),
            DBUS_SESSION_BUS_ADDRESS=f"unix:path={self.run / 'no-such-bus'}",
            XDG_CONFIG_HOME=str(self.run / "config"),
            XDG_STATE_HOME=str(self.run / "state"),
            XDG_DATA_HOME=str(self.run / "data"),
            WINIT_X11_SCALE_FACTOR="1",
            LIMAREPDF_TIMING=str(self.run / "timing.tsv"),
        )
        self.out = open(self.run / "app.log", "wb")
        self.started = self.sent = us()
        self.proc = subprocess.Popen([str(s.opts.bin), *map(str, self.args)], env=env, cwd=self.run,
                                     stdin=subprocess.DEVNULL, stdout=self.out, stderr=subprocess.STDOUT)
        self.wait(lambda: self.log.find("frame built"), "the first frame", 60)
        found = self.wait(lambda: self.placed(), "a 1200 x 850 window", 10)
        if found[1][:2] != (0, 0):
            raise Failed(f"the window is at {found[1][:2]}, not 0,0")
        self.window = found[0]
        self.x.focus(self.window)

    def placed(self):
        found = self.x.window()
        return found if found and found[1][2:] == tuple(WINDOW[2:]) else None

    def copy(self, c):
        to = self.run / c.path.name
        to.write_bytes(c.path.read_bytes())
        return to

    def alive(self):
        if self.proc.poll() is not None:
            raise Failed(f"the app ended (status {self.proc.returncode}); see {self.run / 'app.log'}")

    def wait(self, test, what, timeout=60, quiet=False):
        """Reads the log until TEST gives something, and gives it."""
        end = time.monotonic() + timeout
        while True:
            self.log.poll()
            found = test()
            if found is not None:
                return found
            self.alive()
            if time.monotonic() > end:
                if quiet:
                    return None
                raise Failed(f"no {what} in {timeout} s; see {self.run}")
            time.sleep(0.002)

    def settle(self, quiet=0.3, timeout=120):
        """Waits until no frame has been built for QUIET seconds since the last input."""
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            self.log.poll()
            if us() - max(self.log.last_frame, self.sent) >= quiet * 1e6:
                return
            self.alive()
            time.sleep(0.01)
        raise Failed(f"the app never settled; see {self.run}")

    def taken(self, sent, test, what, timeout=60):
        """The input line (passing TEST) of input sent at SENT, and the end of its frame."""
        start = max(0, len(self.log.lines) - 1)
        i = self.wait(lambda: self.log.find("input", start, sent, test), what, timeout)
        j = self.wait(lambda: self.log.find("frame built", i + 1), f"the frame of {what}", timeout)
        return {"sent": sent, "input": i, "first pass": j}

    def key(self, spec, wait=True, timeout=60):
        """Sends a key (time from its press), and waits for the frame that takes it."""
        self.sent = us()
        self.x.keys(spec)
        if not wait:
            return {"sent": self.sent}
        return self.taken(self.sent, pressed, f"the key {spec}", timeout)

    def type(self, text, gap):
        """Types TEXT, a key every GAP seconds, without waiting for frames: the last key's time."""
        for n, ch in enumerate(text):
            if n:
                time.sleep(max(0.0, self.sent / 1e6 + gap - time.time()))
            self.key(ch, wait=False)
        return self.sent

    def click(self, x, y, timeout=60):
        """A click at X,Y: the pointer moved there, pressed 30 ms later, released 40 ms after the press or
        once its frame is built, whichever is later. Both are timed; the release is the click's time."""
        self.x.move(x, y)
        time.sleep(0.03)
        down = self.sent = us()
        self.x.button(1, True)
        press = self.taken(down, lambda d: d == "button Primary down", "a press", timeout)
        time.sleep(max(0.0, (down + 40_000 - us()) / 1e6))
        self.sent = us()
        self.x.button(1, False)
        r = self.taken(self.sent, lambda d: d == "button Primary up", "a release", timeout)
        r["press"] = press
        return r

    def drag(self, a, b):
        self.x.move(*a)
        time.sleep(0.03)
        self.sent = us()
        self.x.button(1, True)
        self.taken(self.sent, lambda d: d == "button Primary down", "a press")
        for n in range(1, 11):
            time.sleep(0.02)
            self.x.move(a[0] + (b[0] - a[0]) * n // 10, a[1] + (b[1] - a[1]) * n // 10)
        time.sleep(0.05)
        self.sent = us()
        self.x.button(1, False)
        self.taken(self.sent, lambda d: d == "button Primary up", "a release")

    def wheel(self, down=True):
        """One notch of the wheel where the pointer is."""
        self.sent = us()
        b = 5 if down else 4
        self.x.button(b, True)
        self.x.button(b, False)
        return {"sent": self.sent}

    def step(self, what, quiet=0.3):
        """An untimed step (a click at a named point, the pointer then moved off the window; or keys), then a
        settled app; a picture with --check."""
        if what in AT:
            self.click(*AT[what])
            self.x.move(*PARK)
        else:
            self.key(what)
        self.settle(quiet)
        self.picture(what)

    def hello(self):
        """An edit to save: "hello" typed into a new Add text box, at a point that is margin on every
        page the saves use (text-10000.pdf's notice of a large document moves its page down)."""
        self.step("insert tab")
        self.step("add text")
        self.step("hello text")
        for ch in "hello":
            self.key(ch)
        self.settle()
        self.picture("hello")

    def picture(self, name):
        if self.s.opts.check:
            self.steps += 1
            self.x.picture(self.run / f"{self.steps:02d}-{name.replace(' ', '-').replace('+', '-')}.png")

    def built(self, r):
        """Input to the end of its frame (all passes), in ms."""
        j = self.log.frame_end(r["input"])
        r["built at"] = self.log.at(j)
        return (r["built at"] - r["sent"]) / 1000

    def shown(self, r, until=None):
        """Input to the first picture changed after its frame, in ms; None if none before UNTIL."""
        t = self.caps.changed(r["built at"], until)
        return None if t is None else (t - r["sent"]) / 1000

    def memory(self):
        """The resident memory now and at its highest, in MB (MiB)."""
        out = {}
        for line in Path(f"/proc/{self.proc.pid}/status").read_text().splitlines():
            k, _, v = line.partition(":")
            if k in ("VmRSS", "VmHWM"):
                out[k] = round(int(v.split()[0]) / 1024, 1)
        return out

    def drivers(self):
        """The Vulkan drivers the app has loaded (the loader loads every one it finds), and its threads
        that show which draws: lavapipe's are named llvmpipe-N."""
        maps = Path(f"/proc/{self.proc.pid}/maps").read_text()
        threads = {(t / "comm").read_text().strip() for t in Path(f"/proc/{self.proc.pid}/task").iterdir()}
        return sorted({Path(line.split()[-1]).name for line in maps.splitlines() if "libvulkan_" in line
                       or "libnvidia-glcore" in line}
                      | {f"thread {t.rstrip('0123456789')}N" for t in threads if t.startswith("llvmpipe")})

    def close(self):
        if self.caps and not self.caps.done.is_set():
            self.caps.stop()
        if self.proc.poll() is None:
            self.s.drivers.update(self.drivers())
            self.proc.terminate()
            try:
                self.proc.wait(5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        self.out.close()
        if self.log.file:
            self.log.file.close()


class Copy:
    """A document the app changes: copied into the launch's folder first."""

    def __init__(self, path):
        self.path = path


def drop_caches():
    r = subprocess.run(["sudo", "-n", "sh", "-c", "sync; echo 3 > /proc/sys/vm/drop_caches"],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if r.returncode:
        raise Failed("dropping the page cache needs root: run `sudo -v` first")


# ---- Results ----

class Result:
    """A mode's values by row, in ms (None for "not seen"); gates as (row, statistic, limit, kind)."""

    def __init__(self):
        self.rows, self.gates, self.samples, self.counted = {}, [], [], True
        self.intervals, self.costs, self.notes = [], [], []

    def add(self, row, value):
        self.samples[-1][row] = value
        if self.counted:
            self.rows.setdefault(row, []).append(value)

    def begin(self, counted, **about):
        self.counted = counted
        self.samples.append({"counted": counted, **about})

    def captures(self, app):
        if app.caps:
            i, c = app.caps.spacing()
            self.intervals += i
            self.costs += c

    def gate(self, row, stat, limit, kind):
        self.gates.append((row, stat, limit, kind))

    def uncertainty(self, kind):
        if kind == "shown" and self.intervals:
            return (statistics.median(self.intervals) + statistics.median(self.costs)) / 1000
        return 1.0

    def summary(self):
        out = []
        if self.intervals:
            ms = lambda v: f"{v / 1000:.1f}"
            out.append(f"pictures {ms(statistics.median(self.intervals))} / {ms(max(self.intervals))} ms apart, "
                       f"each {ms(statistics.median(self.costs))} / {ms(max(self.costs))} ms (median / worst)")
        for row, values in self.rows.items():
            seen = [v for v in values if v is not None]
            if not seen:
                out.append(f"  {row}: not seen ({len(values)})")
                continue
            line = f"  {row}: {num(statistics.median(seen))} / {num(max(seen))} ms ({len(seen)}"
            if len(seen) < len(values):
                line += f", {len(values) - len(seen)} not seen"
            line += f"; over 50 ms {sum(v > 50 for v in seen)}, over 100 ms {sum(v > 100 for v in seen)})"
            if len(seen) >= 20:
                line += f"; 95th percentile {num(statistics.quantiles(seen, n=20)[-1])} ms"
            out.append(line)
        for row, stat, limit, kind in self.gates:
            seen = [v for v in self.rows.get(row, []) if v is not None]
            if not seen:
                out.append(f"  {row} {stat} ≤ {num(limit)}: nothing to judge")
                continue
            v = statistics.median(seen) if stat == "median" else max(seen)
            u = self.uncertainty(kind)
            word = "fail" if v > limit + u else "borderline" if v > limit - u else "pass"
            missing = len(self.rows[row]) - len(seen)
            extra = f" ({missing} not seen)" if missing else ""
            out.append(f"  {row} {stat} ≤ {num(limit)} ms: {num(v)} ms ± {u:.1f}: {word}{extra}")
        return "\n".join(out + self.notes)


def num(v):
    return f"{v:,.1f}"


# ---- The modes ----

class Session:
    """A mode's run: its folder, its Xvfb and the stand-ins."""

    def __init__(self, opts, name):
        self.opts, self.count, self.drivers = opts, 0, set()
        stamp = time.strftime("%Y%m%d-%H%M%S", time.gmtime())
        self.folder = opts.out / f"{name}-{stamp}"
        self.folder.mkdir(parents=True)
        standin = self.folder / "standin"
        standin.mkdir()
        calls = self.folder / "standin.log"
        for tool, body in {
            "lpstat": f'echo "lpstat $*" >> "{calls}"\ncase "$1" in\n-e) sleep 4; echo Stand_In ;;\n'
                      '-d) echo "system default destination: Stand_In" ;;\nesac\n',
            "lpoptions": f'echo "lpoptions $*" >> "{calls}"\nexit 1\n',
            "lp": f'echo "lp $*" >> "{calls}"\necho "lp: a stand-in" >&2\nexit 1\n',
            "zenity": f'echo "zenity $*" >> "{calls}"\necho "{PICTURE}"\n',
        }.items():
            (standin / tool).write_text("#!/bin/sh\n" + body)
            (standin / tool).chmod(0o755)
        read, write = os.pipe()
        self.xvfb = subprocess.Popen(
            ["Xvfb", "-displayfd", str(write), "-screen", "0", f"{SCREEN[0]}x{SCREEN[1]}x24", "-nolisten", "tcp",
             "-noreset"],
            pass_fds=[write], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=open(self.folder / "xvfb.log", "wb"))
        os.close(write)
        with os.fdopen(read, "rb") as f:
            number = f.readline().strip().decode()
        if not number:
            raise Failed(f"Xvfb didn't start; see {self.folder / 'xvfb.log'}")
        self.display = f":{number}"
        self.x = Display(self.display)
        self.x.move(*PARK)

    def runs(self):
        """Whether each launch is counted: one discarded first when warm."""
        if self.opts.check:
            return [True]
        if self.opts.cold:
            return [True] * (self.opts.runs or 5)
        return [False] + [True] * (self.opts.runs or 10)

    def doc(self, name, copy=False):
        """A document of the set by name, or any by its path."""
        path = Path(name) if "/" in str(name) else self.opts.set / name
        if not path.is_file():
            raise Failed(f"no {path}: make the set with crates/limarepdf-core/tools/measure.py --refset")
        return Copy(path) if copy else path

    def close(self):
        self.x.close()
        self.xvfb.terminate()
        self.xvfb.wait()


def startup(s, res, name):
    path = s.doc(name)
    for counted in s.runs():
        res.begin(counted)
        app = App(s, [path], AREA["view"])
        try:
            drawn = app.wait(lambda: app.log.find("page drawn"), "page 1 drawn", 120)
            app.wait(lambda: app.log.frame_end(drawn), "page 1's frame")
            app.settle(0.5)
            app.caps.stop()
            app.picture("settled")
            t0, at = app.started, app.log.at
            res.add("built", (at(app.log.frame_end(drawn)) - t0) / 1000)
            settled = app.caps.settled(at(app.log.find("frame built")))
            res.add("shown", None if settled is None else (settled - t0) / 1000)
            res.add("first frame", (at(app.log.find("frame built")) - t0) / 1000)
            res.add("app made", (at(app.log.find("app made")) - t0) / 1000)
            res.samples[-1].update(app.memory(), drivers=app.drivers())
            res.captures(app)
        finally:
            app.close()
    if name == "letter.pdf":
        if s.opts.cold:
            res.gate("shown", "median", 1200, "shown")
            res.gate("shown", "worst", 1500, "shown")
        else:
            res.gate("shown", "median", 500, "shown")
            res.gate("shown", "worst", 700, "shown")


def typing(s, res, kind):
    rng = random.Random(1)
    doc, text = ("form.pdf", SCRIPT) if kind == "field" else ("letter.pdf", SCRIPT_LINES if kind == "text" else SCRIPT)
    for counted in s.runs():
        res.begin(counted)
        app = App(s, [s.doc(doc, copy=True)], AREA[kind])
        try:
            app.settle()
            if kind == "field":
                app.step("field 1")
            elif kind == "text":
                app.step("insert tab")
                app.step("add text")
                app.step("new text")
            elif kind == "line":
                app.step("edit text")
                app.step("greeting")
                app.step("End")
            else:
                app.step("edit text")
                app.step("paragraph 1")
                app.step("edit as paragraph")
                app.step("ctrl+Home")
            keys = []
            for ch in text:
                r = app.key(ch)
                keys.append(r)
                due = app.log.at(r["first pass"]) / 1e6 + rng.uniform(0.06, 0.12)
                app.wait(lambda: None if time.time() < due else True, "", 1)
            app.settle()
            app.caps.stop()
            app.picture("typed")
            for n, r in enumerate(keys):
                res.add("built", app.built(r))
            for n, r in enumerate(keys):
                res.add("shown", app.shown(r, keys[n + 1]["sent"] if n + 1 < len(keys) else None))
            res.captures(app)
        finally:
            app.close()
    res.gate("shown", "median", 50, "shown")
    res.gate("shown", "worst", 100, "shown")
    res.gate("built", "worst", 50, "built")


def search(s, res, pages):
    for counted in s.runs():
        res.begin(counted)
        app = App(s, [s.doc(f"text-{pages}.pdf")], AREA["results"])
        try:
            app.settle()
            app.step("ctrl+f")
            done = []
            for k in range(3):
                if k:
                    app.key("ctrl+a")
                    app.key("BackSpace")
                    app.settle()
                    time.sleep(1)
                start = len(app.log.lines)
                last = app.type("annual", 0.05)
                end = app.wait(lambda: app.log.find("search end", start, last), "the search's end", 120)
                app.settle()
                done.append((last, app.log.find("search start", start, last),
                             app.log.find("search first hit", start, last), end))
            app.caps.stop()
            app.picture("searched")
            for k, (last, begun, hit, end) in enumerate(done):
                which = "first search" if k == 0 else "later search"
                at, frame = app.log.at, lambda i: app.log.at(app.log.frame_end(i))
                res.add(f"{which}: pause", (at(begun) - last) / 1000)
                res.add(f"{which}: processing (built)", (frame(hit) - at(begun)) / 1000)
                res.add(f"{which}: total (built)", (frame(hit) - last) / 1000)
                nxt = done[k + 1][0] - 2_000_000 if k + 1 < len(done) else None
                seen = app.caps.changed(frame(hit), nxt)
                res.add(f"{which}: total (shown)", None if seen is None else (seen - last) / 1000)
                res.add(f"{which}: complete (built)", (frame(end) - last) / 1000)
                res.samples[-1][f"{which} {k} matches"] = app.log.lines[end][2]
            res.captures(app)
        finally:
            app.close()
    for which in ("first search", "later search"):
        if pages == "120":
            res.gate(f"{which}: processing (built)", "worst", 300, "built")
        res.gate(f"{which}: total (built)", "worst", 450, "built")
        res.gate(f"{which}: total (shown)", "worst", 450, "shown")


def busy_spans(lines, start_event, end_event, test=lambda d: True):
    """The spans (from, to) when at least one START_EVENT has no END_EVENT yet."""
    spans, running, began = [], 0, None
    for t, e, d in lines:
        if e == start_event and test(d):
            if not running:
                began = t
            running += 1
        elif e == end_event and test(d) and running:
            running -= 1
            if not running:
                spans.append((began, t))
    if running:
        spans.append((began, None))
    return spans


def within(t, spans):
    return any(a <= t and (b is None or t <= b) for a, b in spans)


def working(s, res, what):
    for counted in s.runs():
        res.begin(counted)
        doc = {"artwork": "artwork.pdf", "printers": "letter.pdf", "search": "text-10000.pdf",
               "save": "text-10000.pdf"}[what]
        app = App(s, [s.doc(doc, copy=what == "save")], AREA["search box" if what in ("artwork", "printers") else "view"])
        try:
            sent, first = [], time.time()
            if what == "artwork":
                app.key("ctrl+f")
                ended = lambda: busy_spans(app.log.lines, "render start", "render end")
            elif what == "printers":
                app.settle()
                app.key("ctrl+p")
                app.wait(lambda: app.log.find("print dialog"), "the Print dialog")
                time.sleep(0.3)  # not settled: the dialog draws until the printers are found
                app.picture("print")
                app.key("Escape")
                time.sleep(0.2)
                app.key("ctrl+f")
                time.sleep(0.2)
                final = lambda: app.log.find("printers found", 0, 0, lambda d: "at once" not in d)
            else:
                app.settle()
                if what == "search":
                    app.step("ctrl+f")
                    app.x.move(*AT["view"])
                    app.settle()
                    began = len(app.log.lines)
                    typed = app.type("annual", 0.05)
                    finished = lambda: app.log.find("search end", began, typed)
                else:
                    app.hello()
                    app.x.move(*AT["view"])
                    app.settle()
                    began = len(app.log.lines)
                    # Not waited for: the scrolling starts while Ctrl+S is taken.
                    pressed_at = app.key("ctrl+s", wait=False)["sent"]
                    first = pressed_at / 1e6 + 0.1
                    finished = lambda: app.log.find("save end", began)
            for n in range(1000):
                due = (sent[-1]["sent"] / 1e6 + 0.1) if sent else first
                time.sleep(max(0.0, due - time.time()))
                if what in ("artwork", "printers"):
                    sent.append(app.key("e" if n % 2 == 0 else "BackSpace", timeout=30))
                else:
                    sent.append(app.wheel(down=n % 2 == 0))
                app.log.poll()
                if what == "artwork":
                    spans = ended()
                    if n > 5 and spans and spans[-1][1] is not None and us() - spans[-1][1] > 500_000:
                        break
                elif what == "printers":
                    if final() is not None:
                        break
                elif finished() is not None:
                    break
                if n == 999:
                    raise Failed("the work never ended")
            app.settle()
            if what == "printers":
                app.key("BackSpace")
                app.settle()
            app.caps.stop()
            app.picture("done")
            lines = app.log.lines
            if what == "artwork":
                spans = busy_spans(lines, "render start", "render end")
            elif what == "printers":
                spans = [(sent[0]["sent"], app.log.at(final()))]
            elif what == "search":
                spans = busy_spans(lines, "search start", "search end")
            else:
                # From the key, as the user sees it: what the save does on the window's thread first counts.
                saved = busy_spans(lines, "save start", "save end")
                spans = [(pressed_at, saved[-1][1])]
                res.samples[-1]["to save start ms"] = (saved[0][0] - pressed_at) / 1000
            res.samples[-1]["busy ms"] = [((b or 0) - a) / 1000 for a, b in spans]
            for n, r in enumerate(sent):
                if not within(r["sent"], spans):
                    continue
                if "input" not in r:
                    start = 0
                    i = app.log.find("input", start, r["sent"], lambda d: d == "wheel")
                    if i is None:
                        continue
                    r.update(input=i)
                res.add("built", app.built(r))
                # Inputs sent before this one's frame was built are taken with it or after its picture.
                later = [x["sent"] for x in sent[n + 1:] if x["sent"] > r["built at"]]
                res.add("shown", app.shown(r, later[0] if later else None))
            res.captures(app)
        finally:
            app.close()
    res.gate("built", "worst", 50, "built")
    res.notes.append("  (shown is measured, not gated: the accepted threshold is on built)")


def print_dialog(s, res):
    for counted in s.runs():
        res.begin(counted)
        app = App(s, [s.doc("letter.pdf")], AREA["dialog"])
        try:
            app.settle()
            began = len(app.log.lines)
            r = app.key("ctrl+p")
            app.settle(0.3)
            dialog = app.log.find("print dialog", began)
            res.add("built", app.built(r))
            final = app.wait(lambda: app.log.find("printers found", began, 0, lambda d: "at once" not in d),
                             "the printers", 30)
            app.caps.stop()
            res.add("shown", app.shown(r))
            app.picture("printers")
            res.samples[-1].update(dialog_in_frame=dialog is not None and dialog < app.log.frame_end(r["input"]),
                                   listed=app.log.lines[final][2],
                                   listed_after_ms=(app.log.at(final) - r["sent"]) / 1000,
                                   before_listed=r["built at"] < app.log.at(final))
            res.captures(app)
        finally:
            app.close()
    res.gate("built", "worst", 200, "built")


def save(s, res, name):
    for counted in s.runs():
        res.begin(counted)
        app = App(s, [s.doc(f"{name}.pdf", copy=True)], AREA["status"])
        try:
            app.settle()
            app.hello()
            began = len(app.log.lines)
            r = app.key("ctrl+s")
            end = app.wait(lambda: app.log.find("save end", began), "the save's end", 120)
            app.settle()
            app.caps.stop()
            app.picture("saved")
            sign = app.log.find("save shown", began)
            frame = lambda i: app.log.at(app.log.frame_end(i))
            first = app.log.find("save start", began)
            if sign is not None:
                res.add("to the busy sign (built)", (frame(sign) - r["sent"]) / 1000)
                seen = app.caps.changed(frame(sign))
                res.add("to the busy sign (shown)", None if seen is None else (seen - r["sent"]) / 1000)
            else:
                res.notes.append("  a save showed no busy sign")
            res.add("to the saved frame (built)", (frame(end) - r["sent"]) / 1000)
            res.add("writing", (app.log.at(end) - app.log.at(first)) / 1000)
            res.samples[-1]["saved"] = app.log.lines[end][2]
            res.captures(app)
        finally:
            app.close()
    res.gate("to the busy sign (built)", "worst", 100, "built")
    res.gate("to the busy sign (shown)", "worst", 100, "shown")


def memory(s, res):
    res.begin(True)
    app = App(s, [s.doc("text-300.pdf")])
    try:
        rss = []
        for cycle in range(20 if not s.opts.check else 2):
            if cycle:
                began = len(app.log.lines)
                app.click(*AT["recent 1"])
                app.wait(lambda: app.log.find("open end", began), "the file opened")
            app.settle()
            for _ in range(10):
                app.key("Page_Down")
            app.settle()
            app.key("ctrl+f")
            began = len(app.log.lines)
            app.type("annual", 0.05)
            app.wait(lambda: app.log.find("search end", began), "the search's end", 120)
            app.settle()
            app.picture(f"cycle {cycle + 1}")
            app.key("ctrl+w")
            app.settle(0.4)
            app.picture(f"closed {cycle + 1}")
            rss.append(app.memory()["VmRSS"])
        res.samples[-1]["resident MB"] = rss
        res.notes.append(f"  resident after each cycle, MB: {', '.join(f'{v:.0f}' for v in rss)}")
        if len(rss) == 20:
            early, late = statistics.median(rss[2:10]), statistics.median(rss[-10:])
            word = "pass" if abs(late - early) <= 20 else "fail"
            res.notes.append(f"  the last 10's median {late:.1f} MB, cycles 3 to 10's {early:.1f} MB: "
                             f"{late - early:+.1f} MB, within 20 MB: {word}")
    finally:
        app.close()


def pages(s, res):
    steps = [("move", "alt+shift+End"), ("move undone", "ctrl+z"), ("move redone", "ctrl+y"),
             ("move undone again", "ctrl+z"), ("delete", "Delete"), ("delete undone", "ctrl+z"),
             ("delete redone", "ctrl+y")]
    for counted in s.runs():
        res.begin(counted)
        app = App(s, [s.doc("text-10000.pdf")], AREA["pages"])
        try:
            app.wait(lambda: app.log.find("page drawn"), "page 1 drawn", 120)
            app.settle()
            done = []
            for name, keys in steps:
                if name in ("move", "delete"):
                    app.step("thumbnail 1")
                r = app.key(keys)
                app.settle()
                app.picture(name)
                done.append((name, r))
            app.caps.stop()
            for n, (name, r) in enumerate(done):
                res.add(f"{name} (built)", app.built(r))
                res.add(f"{name} (shown)", app.shown(r, done[n + 1][1]["sent"] if n + 1 < len(done) else None))
            res.captures(app)
        finally:
            app.close()
    for name, _ in steps:
        res.gate(f"{name} (shown)", "worst", 100, "shown")


def edits(s, res):
    for edit in ("nudge", "picture", "redaction"):
        for counted in s.runs():
            res.begin(counted, edit=edit)
            app = App(s, [s.doc("artwork.pdf", copy=True)], AREA["view"])
            try:
                app.wait(lambda: app.log.find("page drawn"), "the page drawn", 120)
                app.settle(0.5)
                if edit == "nudge":
                    app.step("select menu")
                    app.step("select objects")
                    app.step("shape", quiet=0.5)
                    app.wait(lambda: None, "", 1.0, quiet=True)
                    app.settle(0.5)
                elif edit == "picture":
                    app.step("insert tab")
                    app.step("image", quiet=0.5)
                else:
                    app.step("protect tab")
                    app.step("mark redaction")
                    app.drag(*MARK["artwork"])
                    app.settle()
                    app.picture("marked")
                    app.step("apply redactions")
                before = app.memory()
                if edit == "nudge":
                    r = app.key("Right", timeout=120)
                else:
                    r = app.click(*AT["picture" if edit == "picture" else "apply button"], timeout=120)
                # Long: the frame after the edit's can take half a second, and it may be the first one seen.
                app.settle(1.5)
                after = app.memory()
                app.caps.stop()
                app.picture("done")
                res.add(f"{edit} (built)", app.built(r))
                res.add(f"{edit} (shown)", app.shown(r))
                if "press" in r:
                    res.samples[-1]["press to its frame ms"] = (
                        app.log.at(app.log.frame_end(r["press"]["input"])) - r["press"]["sent"]) / 1000
                res.samples[-1].update(before=before, after=after)
                res.captures(app)
            finally:
                app.close()
        res.gate(f"{edit} (shown)", "worst", 100, "shown")


def redact(s, res):
    for counted in s.runs():
        res.begin(counted)
        app = App(s, [s.doc("text-10000.pdf", copy=True)], AREA["view"])
        try:
            app.wait(lambda: app.log.find("page drawn"), "page 1 drawn", 120)
            app.settle()
            # With this notice and the unapplied mark's both up, the app draws without end: closed first.
            app.step("close notice")
            app.step("protect tab")
            app.step("mark redaction")
            app.drag(*MARK["text-10000"])
            app.settle()
            app.picture("marked")
            app.step("apply redactions")
            before = app.memory()
            r = app.click(*AT["apply button"], timeout=600)
            app.settle(1.5, 600)
            after = app.memory()
            app.caps.stop()
            app.picture("applied")
            res.add("apply (built)", app.built(r))
            res.add("apply (shown)", app.shown(r))
            res.samples[-1].update(before=before, after=after)
            res.captures(app)
        finally:
            app.close()
    res.gate("apply (shown)", "worst", 100, "shown")


def select(s, res, name):
    path = CORPUS / "pdfjs" / name
    if not path.is_file():
        raise Failed(f"no {path}: run tools/fetch-render-samples.sh first")
    for counted in s.runs():
        res.begin(counted)
        app = App(s, [path], AREA["view"])
        try:
            app.wait(lambda: app.log.find("page drawn"), "page 1 drawn", 120)
            app.settle(0.5)
            app.step("edit text")
            r = app.click(*AT[name])
            # The line shows selected a frame later, once its page's fonts are read (1.4 s on issue2618).
            # Not settled: the selected line's caret blinks.
            app.wait(lambda: app.log.find("frame built", app.log.frame_end(r["input"]) + 1),
                     "the frame after the click's", 60)
            time.sleep(0.3)
            app.caps.stop()
            app.picture("selected")
            res.add("select (built)", app.built(r))
            res.add("select (shown)", app.shown(r))
            res.samples[-1]["press to its frame ms"] = (
                app.log.at(app.log.frame_end(r["press"]["input"])) - r["press"]["sent"]) / 1000
            res.captures(app)
        finally:
            app.close()
    res.gate("select (shown)", "worst", 100, "shown")


def ideograph(s, res):
    s.x.bind("ideograph", IDEOGRAPH)
    for counted in s.runs():
        res.begin(counted)
        app = App(s, [s.doc("letter.pdf", copy=True)], AREA["view"])
        try:
            app.settle()
            app.step("edit text")
            app.step("font")
            app.step("times")
            # The font chosen with nothing selected arms Insert > Text: another click would disarm it.
            app.step("insert tab")
            app.step("ideograph text")
            r = app.key("中", timeout=120)
            app.settle(0.4)
            app.caps.stop()
            app.picture("typed")
            res.add("built", app.built(r))
            res.add("shown", app.shown(r))
            res.captures(app)
        finally:
            app.close()
    res.gate("shown", "median", 50, "shown")
    res.gate("shown", "worst", 100, "shown")
    res.gate("built", "worst", 50, "built")


def explore(s, res, name, *steps):
    s.opts.check = True
    res.begin(True)
    app = App(s, [s.doc(name)])
    try:
        # Settled alone isn't enough: no frame is built while a long document's first page is made.
        app.wait(lambda: app.log.find("page drawn"), "page 1 drawn", 120)
        app.settle()
        for step in steps:
            kind, _, arg = step.partition(":")
            if kind == "c":
                app.click(*map(int, arg.split(",")))
            elif kind == "d":
                x0, y0, x1, y1 = map(int, arg.split(","))
                app.drag((x0, y0), (x1, y1))
            elif kind == "k":
                app.key(arg)
            elif kind == "t":
                app.type(arg, 0.05)
            elif kind == "p":
                time.sleep(float(arg))
                continue
            elif kind == "s":
                app.picture(arg)
                continue
            else:
                raise Failed(f"no step {step!r}")
            app.settle()
        app.picture("end")
    finally:
        app.close()


MODES = {
    "startup": (startup, 1), "typing": (typing, 1), "search": (search, 1), "working": (working, 1),
    "print": (print_dialog, 0), "save": (save, 1), "memory": (memory, 0), "pages": (pages, 0),
    "edits": (edits, 0), "redact": (redact, 0), "select": (select, 1), "ideograph": (ideograph, 0),
    "try": (explore, None),
}
CHOICES = {"typing": ("field", "text", "line", "paragraph"), "search": ("120", "300"),
           "working": ("artwork", "printers", "search", "save"), "save": ("letter", "text-300")}


def setup(opts):
    """What a result needs said with it."""
    git = lambda *a: subprocess.run(["git", "-C", str(ROOT), *a], capture_output=True, text=True).stdout.strip()
    mem = dict(line.split(":", 1) for line in Path("/proc/meminfo").read_text().splitlines())
    return {
        "date": time.strftime("%Y-%m-%d %H:%M UTC", time.gmtime()),
        "commit": git("rev-parse", "--short", "HEAD"),
        "changed": git("status", "--porcelain", "--", "crates", "Cargo.lock").splitlines(),
        "app": str(opts.bin),
        "app built": time.strftime("%Y-%m-%d %H:%M UTC", time.gmtime(opts.bin.stat().st_mtime)),
        "load": Path("/proc/loadavg").read_text().split()[:3],
        "available MB": int(mem["MemAvailable"].split()[0]) // 1024,
        "documents": {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in sorted(opts.set.glob("*.pdf"))} if opts.set.is_dir() else {},
    }


def main(argv):
    p = argparse.ArgumentParser(prog="drive.py", usage=__doc__.split("\n\n")[1].strip())
    p.add_argument("mode", choices=MODES)
    p.add_argument("args", nargs="*")
    p.add_argument("--bin", type=Path, default=ROOT / "target" / "release" / "limarepdf")
    p.add_argument("--set", type=Path, default=ROOT / "target" / "tmp" / "perf" / "refset")
    p.add_argument("--out", type=Path, default=ROOT / "target" / "tmp" / "perf")
    p.add_argument("--runs", type=int)
    p.add_argument("--cold", action="store_true")
    p.add_argument("--check", action="store_true")
    opts = p.parse_args(argv)
    run, count = MODES[opts.mode]
    if count is not None and len(opts.args) != count:
        p.error(f"{opts.mode} takes {count} argument{'s' if count != 1 else ''}")
    if opts.mode in CHOICES and opts.args[0] not in CHOICES[opts.mode]:
        p.error(f"{opts.mode} takes one of {', '.join(CHOICES[opts.mode])}")
    if not opts.bin.is_file():
        p.error(f"no app at {opts.bin}: build it (cargo build --release) or name it with --bin")
    X11.XInitThreads()
    X11.XSetErrorHandler(IGNORE)
    name = "-".join([opts.mode, *(Path(a).stem for a in opts.args[:1])] + (["cold"] if opts.cold else [])
                    + (["check"] if opts.check else []))
    about = setup(opts)
    s = Session(opts, name)
    res = Result()
    try:
        run(s, res, *opts.args)
    finally:
        s.close()
    about["load after"] = Path("/proc/loadavg").read_text().split()[:3]
    about["drivers"] = sorted(s.drivers)
    title = f"{name}: {about['commit']}, {about['date']}, " + (
        "check" if opts.check else f"{sum(x['counted'] for x in res.samples)} runs"
        + ("" if opts.cold or opts.mode in ("memory", "try") else " after one discarded")
        + (", the page cache dropped before each" if opts.cold else ""))
    text = title + "\n" + res.summary()
    print(text)
    (s.folder / "summary.txt").write_text(text + "\n")
    (s.folder / "result.json").write_text(json.dumps(
        {"setup": about, "mode": opts.mode, "args": opts.args, "cold": opts.cold, "rows": res.rows,
         "samples": res.samples, "gates": res.gates, "notes": res.notes}, indent=1, default=str))
    print(f"in {s.folder}")


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except Failed as e:
        sys.exit(f"drive.py: {e}")
