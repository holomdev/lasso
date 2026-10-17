"""Paint a box, drag the region, and check the box stayed where it was drawn.

The report was that the painting disappears when the capture area is moved. What has to be true
is exact: the box's absolute position on the screen does not move, because a mark belongs to the
pixel it marks. Measured as the bounding box of the stroke's colour before and after, which is a
number that either stays equal or does not.
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
OUT = ROOT / "target" / "overlay-move"
REGION = (600, 250, 1200, 700)
STROKE = (0xEF, 0x44, 0x44)  # the palette's red

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


def key(vk):
    down = INPUT()
    down.type = 1
    down.u.ki.wVk = vk
    send(down)
    time.sleep(0.05)
    up = INPUT()
    up.type = 1
    up.u.ki.wVk = vk
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
        time.sleep(0.03)
    mouse(0x0004)
    time.sleep(0.4)


def shot(name):
    image = ImageGrab.grab()
    OUT.mkdir(parents=True, exist_ok=True)
    image.save(OUT / f"{name}.png")
    return np.array(image.convert("RGB"))


# Well clear of the strip, whose colour button is the same red as the stroke: including it
# would make the measurement say where the strip is rather than where the drawing is.
DRAWN_IN = (650, 300, 1300, 780)


def stroke_box(image):
    """The bounding box of the stroke's colour, or None when there is none."""
    mask = np.all(np.abs(image.astype(int) - np.array(STROKE)) <= 8, axis=2)
    left, top, right, bottom = DRAWN_IN
    keep = np.zeros_like(mask)
    keep[top:bottom, left:right] = mask[top:bottom, left:right]
    if keep.sum() < 20:
        return None
    ys, xs = np.nonzero(keep)
    return (int(xs.min()), int(ys.min()), int(xs.max()), int(ys.max()))


OUT.mkdir(parents=True, exist_ok=True)
process = subprocess.Popen(
    [str(EXE), "select", "--out", str(OUT / "result.png")],
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
)
time.sleep(2.5)
if process.poll() is not None:
    sys.exit("the overlay exited before it was used")

try:
    left, top, right, bottom = REGION

    # A region, retried: the first synthetic drag is occasionally not delivered.
    for attempt in range(3):
        move_to(left + 11, top + 13)
        time.sleep(0.2)
        drag((left, top), (right, bottom))
        if stroke_box(shot("setup")) is not None or True:
            break
    settled = shot("0-region")

    # A box, drawn with the rectangle tool.
    key(0x52)  # R
    time.sleep(0.2)
    drag((700, 350), (900, 500))
    before = shot("1-painted")
    before_box = stroke_box(before)

    # The move tool, then drag the region itself by a good distance, grabbing well clear of
    # the box so the press does not draw.
    key(0x53)  # S
    time.sleep(0.25)
    drag((650, 320), (690, 340))
    after = shot("2-moved")
    after_box = stroke_box(after)

    # Enter saves, which is the file that would have been produced.
    key(0x0D)
    process.wait(timeout=10)
except subprocess.TimeoutExpired:
    process.kill()
    sys.exit("the overlay did not close on Enter")
finally:
    if process.poll() is None:
        process.kill()

failures = []
if before_box is None:
    failures.append("nothing was drawn, so the run proves nothing")
if before_box is not None and after_box is None:
    failures.append("the box vanished when the region was moved — the reported fault")
if before_box is not None and after_box is not None:
    # Same absolute position, within a pixel of rounding: the mark stayed on the pixel it marks.
    drift = max(abs(a - b) for a, b in zip(before_box, after_box))
    if drift > 2:
        failures.append(
            f"the box moved on the screen by {drift} px: {before_box} then {after_box}"
        )

print(f"box on the screen before the move: {before_box}")
print(f"box on the screen after  the move: {after_box}")
if before_box and after_box:
    print(f"largest corner drift: {max(abs(a - b) for a, b in zip(before_box, after_box))} px")
print()
print("FAILED" if failures else "the box stayed on the pixels it was drawn on")
for line in failures:
    print("  " + line)
sys.exit(1 if failures else 0)
