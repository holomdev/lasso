"""Paint a box that starts outside the region, then save and look at what was kept.

Two things the report asks for, and they are separate:

  * a mark may be made anywhere on the screen, including over the dimmed area outside the
    region, and it is visible there;
  * the saved file contains only the part the region covers.

Measured, not asserted: the red pixels outside the region on screen, and the red pixels in the
saved image, which is the region cropped — so a box crossing the left edge must leave red
touching the crop's left column.
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
EXE = ROOT / "target" / "debug" / "lasso.exe"
OUT = ROOT / "target" / "overlay-outside"
REGION = (600, 250, 1200, 700)
STROKE = (0xEF, 0x44, 0x44)
SAVED = OUT / "result.png"

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


def red_mask(image):
    return np.all(np.abs(image.astype(int) - np.array(STROKE)) <= 8, axis=2)


def strip_swatch_band(region_left):
    """The columns the strip's own colour swatch occupies, which is the same red."""
    return (region_left - 60, region_left)


OUT.mkdir(parents=True, exist_ok=True)
SAVED.unlink(missing_ok=True)

process = subprocess.Popen(
    [str(EXE), "select", "--out", str(SAVED)],
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
)
time.sleep(2.5)
if process.poll() is not None:
    sys.exit("the overlay exited before it was used")

try:
    left, top, right, bottom = REGION

    # A region, with a warm-up move and a retry: the first synthetic drag is sometimes lost.
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

    # The rectangle tool, then a box that begins ABOVE the region and ends inside it, so it
    # crosses the region's top edge. Above and not to the left, because the strip stands
    # immediately to the left of the region and its own colour button is the same red.
    key(0x52)  # R
    time.sleep(0.25)
    drag((700, 180), (900, 400))

    screen = np.array(ImageGrab.grab().convert("RGB"))
    mask = red_mask(screen)
    # Clear of the region, and limited to the columns the box occupies.
    outside_pixels = int(mask[: top - 20, 690:910].sum())

    # Save.
    key(0x0D)
    process.wait(timeout=10)
except subprocess.TimeoutExpired:
    process.kill()
    sys.exit("the overlay did not close on Enter")
finally:
    if process.poll() is None:
        process.kill()

failures = []
if outside_pixels < 20:
    failures.append(
        f"only {outside_pixels} pixels were painted outside the region, so a mark outside it "
        "is still being refused"
    )

if not SAVED.exists():
    failures.append("no file was saved")
    saved = None
else:
    saved = np.array(Image.open(SAVED).convert("RGB"))
    if saved.shape[1] != right - left or saved.shape[0] != bottom - top:
        failures.append(
            f"the saved image is {saved.shape[1]}x{saved.shape[0]} and the region is "
            f"{right - left}x{bottom - top}"
        )
    else:
        inside = red_mask(saved)
        if inside.sum() < 20:
            failures.append("the saved image has no stroke in it at all")
        # The box crossed the top edge, so the crop must have stroke in its first rows.
        elif inside[:3, :].sum() == 0:
            failures.append(
                "the saved image has no stroke against its top edge, so the part that was "
                "inside the region was lost with the part that was outside"
            )

print(f"painted outside the region, on screen: {outside_pixels} px")
if saved is not None:
    mask = red_mask(saved)
    columns = np.nonzero(mask.any(axis=0))[0]
    rows = np.nonzero(mask.any(axis=1))[0]
    if len(columns) == 0:
        print(f"saved image: {saved.shape[1]}x{saved.shape[0]}, and no stroke in it at all")
    else:
        print(
            f"saved image: {saved.shape[1]}x{saved.shape[0]}, stroke columns "
            f"{columns.min()}..{columns.max()}, rows {rows.min()}..{rows.max()}"
        )
print()
print("FAILED" if failures else "painted outside the region, and only the inside was saved")
for line in failures:
    print("  " + line)
sys.exit(1 if failures else 0)
