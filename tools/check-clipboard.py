"""Click the strip's copy button and read the clipboard back.

The report was that copying put the drawings on the clipboard and the capture in black. Both the
file and the clipboard are cut from the same composed pixels, so the file check covers the
composition — but the clipboard is what was reported, so it is measured directly.
"""

import ctypes
import ctypes.wintypes as wintypes
import pathlib
import subprocess
import sys
import time

import numpy as np
from PIL import ImageGrab

ROOT = pathlib.Path(__file__).resolve().parent.parent
EXE = ROOT / "target" / "debug" / "lasso.exe"
OUT = ROOT / "target" / "overlay-clipboard"
REGION = (600, 250, 1200, 700)
# The copy button, from the strip's layout: the eleventh of twelve, in the last group.
COPY_BUTTON = (575, 557)
STROKE = (0xEF, 0x44, 0x44)

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


def click(x, y):
    move_to(x, y)
    time.sleep(0.25)
    mouse(0x0002)
    time.sleep(0.08)
    mouse(0x0004)
    time.sleep(0.4)


def key(vk):
    down = INPUT()
    down.type = 1
    down.u.ki.wVk = vk
    send(down)
    time.sleep(0.05)
    up = INPUT()
    up.type = 1
    up.u.ki.dwFlags = 0x0002
    send(up)


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


def empty_clipboard():
    """Clear it first, so a stale image from an earlier run cannot pass for this one's."""
    if user32.OpenClipboard(None):
        user32.EmptyClipboard()
        user32.CloseClipboard()


empty_clipboard()
process = subprocess.Popen(
    [str(EXE), "select", "--clipboard", "--out", str(OUT / "unused.png")],
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
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
        plate = np.all(np.abs(screen.astype(int) - np.array((42, 42, 42))) <= 2, axis=2)
        if plate.sum() > 2000:
            break
        print(f"  the drag did not take (attempt {attempt + 1})")
    else:
        sys.exit("the setup failed: no strip on screen")

    # A box inside the region, so the clipboard should carry it too.
    key(0x52)  # R
    time.sleep(0.25)
    drag((700, 350), (900, 500))

    # The copy button.
    click(*COPY_BUTTON)
    process.wait(timeout=10)
except subprocess.TimeoutExpired:
    process.kill()
    sys.exit("the overlay did not close after the copy button")
finally:
    if process.poll() is None:
        process.kill()

time.sleep(0.4)
failures = []
image = ImageGrab.grabclipboard()
if image is None:
    failures.append("the clipboard holds no image — nothing was copied")
    saved = None
else:
    saved = np.array(image.convert("RGB"))
    brightness = float(saved.astype(float).mean())
    distinct = len(np.unique(saved.reshape(-1, 3), axis=0))
    print(f"clipboard image: {saved.shape[1]}x{saved.shape[0]}, brightness {brightness:.1f}, {distinct} colours")
    if (saved.shape[1], saved.shape[0]) != (right - left, bottom - top):
        failures.append(
            f"the clipboard image is {saved.shape[1]}x{saved.shape[0]} and the region is "
            f"{right - left}x{bottom - top}"
        )
    if brightness < 20:
        failures.append(
            f"the clipboard image is essentially black, mean brightness {brightness:.1f}: the "
            "capture never reached the canvas it was drawn on"
        )
    elif distinct < 100:
        failures.append(f"the clipboard image has only {distinct} colours, so it is not the screen")
    else:
        mask = np.all(np.abs(saved.astype(int) - np.array(STROKE)) <= 8, axis=2)
        if mask.sum() < 20:
            failures.append("the clipboard image has no stroke in it, so the drawing was lost")

print()
print("FAILED" if failures else "the clipboard holds the capture with the drawing on it")
for line in failures:
    print("  " + line)
sys.exit(1 if failures else 0)
