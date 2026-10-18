"""Photograph the strip, so a change to how it looks can be looked at.

Drives a selection, crops the strip out of the screenshot and writes it next to the screenshots.
Used for judging the card, the buttons and later the icons; the automated checks measure pixels,
but nothing decides whether it looks right except looking.
"""

import ctypes
import ctypes.wintypes as wintypes
import pathlib
import subprocess
import sys
import time

import numpy as np
from PIL import Image, ImageGrab

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import layout  # noqa: E402
EXE = ROOT / "target" / "debug" / "lasso.exe"
OUT = ROOT / "target" / "overlay-strip"
REGION = (600, 250, 1200, 700)

# From the metrics: the strip stands off the region by the standoff, and is one button wide plus
# its padding.
BUTTON, PADDING = layout.BUTTON, layout.PADDING
STRIP_WIDTH = layout.STRIP_WIDTH

user32 = ctypes.windll.user32


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [
        ("dx", wintypes.LONG),
        ("dy", wintypes.LONG),
        ("mouseData", wintypes.DWORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [
        ("wVk", wintypes.WORD),
        ("wScan", wintypes.WORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]


class UNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", wintypes.DWORD), ("u", UNION)]


def send(packet):
    user32.SendInput(1, ctypes.byref(packet), ctypes.sizeof(INPUT))


def move_to(x, y):
    vx, vy = user32.GetSystemMetrics(76), user32.GetSystemMetrics(77)
    vw, vh = user32.GetSystemMetrics(78), user32.GetSystemMetrics(79)
    packet = INPUT()
    packet.type = 0
    packet.u.mi.dx = round((x - vx) * 65535 / (vw - 1))
    packet.u.mi.dy = round((y - vy) * 65535 / (vh - 1))
    packet.u.mi.dwFlags = 0x0001 | 0x8000 | 0x4000
    send(packet)


def mouse(flags):
    packet = INPUT()
    packet.type = 0
    packet.u.mi.dwFlags = flags
    send(packet)


def drag(start, end, steps=8):
    move_to(*start)
    time.sleep(0.2)
    mouse(0x0002)
    time.sleep(0.08)
    for step in range(1, steps + 1):
        move_to(
            start[0] + (end[0] - start[0]) * step // steps,
            start[1] + (end[1] - start[1]) * step // steps,
        )
        time.sleep(0.035)
    mouse(0x0004)
    time.sleep(0.35)


OUT.mkdir(parents=True, exist_ok=True)
# stderr kept: a fallback that cannot be seen is a difference nobody looks for, and the program
# says which icon family it ended up drawing with.
error_log = OUT / "stderr.txt"
process = subprocess.Popen(
    [str(EXE), "select"],
    stdout=subprocess.DEVNULL,
    stderr=error_log.open("w", encoding="utf-8", errors="replace"),
)
time.sleep(2.5)
if process.poll() is not None:
    sys.exit("the overlay exited before it was used")

try:
    left, top, right, bottom = REGION
    for attempt in range(3):
        move_to(left + 11, top + 13)
        time.sleep(0.2)
        drag((left, top), (right, bottom))
        screen = np.array(ImageGrab.grab().convert("RGB"))
        # The card is near white; anything that is not is the capture.
        light = np.all(screen.astype(int) > 230, axis=2)
        if light.sum() > 2000:
            break
        print(f"  the drag did not take (attempt {attempt + 1})")
    else:
        ImageGrab.grab().save(OUT / "failed.png")
        sys.exit("the setup failed: no strip on screen (see target/overlay-strip/failed.png)")

    image = ImageGrab.grab()
    image.save(OUT / "screen.png")
    # The card is taller than the region: it is 506 pixels and a region is often shorter, so the
    # crop has to be the card's own height, and its top is the region's unless it was pulled up.
    strip_left = layout.strip_x(REGION)
    strip_top = layout.strip_top(REGION)
    crop = image.crop(
        (strip_left, strip_top, strip_left + STRIP_WIDTH, strip_top + layout.STRIP_HEIGHT)
    )
    crop = crop.resize((crop.width * 2, crop.height * 2), Image.NEAREST)
    crop.save(OUT / "strip.png")
    print(f"strip crop: {crop.width}x{crop.height} at ({strip_left}, {top})")
finally:
    process.kill()
