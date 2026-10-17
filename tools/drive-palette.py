"""Drive the overlay with real input and measure the colour palette.

The same checks as `tools/overlay-palette.ps1` and `tools/check-palette.py`, in one process:
PowerShell kept hanging on the capture, and a check that has to be babysat is not much of a
check. Everything here is Windows-only and standard library plus Pillow.

What it proves, in order:

  0. With the pointer away, the palette is closed.
  1. Jumping **straight into the palette's area** does not open it. This is the fault that was
     reported: being inside the palette keeps it open, but only the colour button opens it.
  2. Pointing at the colour button opens it, showing all five colours.
  3. Moving on to a swatch keeps it open — the part a hover menu usually gets wrong.
  4. Clicking a swatch sets the colour, and the button keeps it once the palette closes.
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
OUT = ROOT / "target" / "overlay-palette"
REGION = (600, 250, 1200, 700)

# The strip's geometry at 100%, the same arithmetic the program does.
BUTTON, GAP, PADDING, GROUP_GAP, STANDOFF = 26, 2, 4, 5, 8
GROUPS = {5, 7, 9}


def button_top(index):
    y = PADDING
    for i in range(index):
        y += GAP
        if i in GROUPS:
            y += GROUP_GAP
        y += BUTTON
    if index in GROUPS:
        y += GROUP_GAP - GAP
    return y


STRIP_X = REGION[0] - STANDOFF - (BUTTON + PADDING * 2)
STRIP_Y = REGION[1]
COLOUR = (STRIP_X + PADDING + BUTTON // 2, STRIP_Y + button_top(5) + BUTTON // 2)
PALETTE_W = 5 * BUTTON + 4 * GAP + 2 * PADDING
PALETTE_H = BUTTON + 2 * PADDING
PALETTE_X = STRIP_X + (BUTTON + PADDING * 2) + STANDOFF
PALETTE_Y = COLOUR[1] - PALETTE_H // 2
PALETTE_BAND = (PALETTE_X, PALETTE_Y, PALETTE_X + PALETTE_W, PALETTE_Y + PALETTE_H)
BUTTON_BAND = (
    STRIP_X + PADDING,
    COLOUR[1] - BUTTON // 2,
    STRIP_X + PADDING + BUTTON,
    COLOUR[1] + BUTTON // 2,
)


def swatch(index):
    return (
        PALETTE_X + PADDING + index * (BUTTON + GAP) + BUTTON // 2,
        PALETTE_Y + PADDING + BUTTON // 2,
    )


# --- input ------------------------------------------------------------------------------

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


class INPUT(ctypes.Structure):
    _fields_ = [("type", wintypes.DWORD), ("mi", MOUSEINPUT)]


def move_to(x, y):
    vx, vy = user32.GetSystemMetrics(76), user32.GetSystemMetrics(77)
    vw, vh = user32.GetSystemMetrics(78), user32.GetSystemMetrics(79)
    packet = INPUT()
    packet.type = 0
    packet.mi.dx = round((x - vx) * 65535 / (vw - 1))
    packet.mi.dy = round((y - vy) * 65535 / (vh - 1))
    packet.mi.dwFlags = 0x0001 | 0x8000 | 0x4000
    user32.SendInput(1, ctypes.byref(packet), ctypes.sizeof(INPUT))


def press():
    packet = INPUT()
    packet.type = 0
    packet.mi.dwFlags = 0x0002
    user32.SendInput(1, ctypes.byref(packet), ctypes.sizeof(INPUT))


def release():
    packet = INPUT()
    packet.type = 0
    packet.mi.dwFlags = 0x0004
    user32.SendInput(1, ctypes.byref(packet), ctypes.sizeof(INPUT))


# --- measurement ------------------------------------------------------------------------

COLOURS = {
    "red": (255, 0, 0),
    "yellow": (255, 255, 0),
    "green": (0, 255, 0),
    "blue": (0, 0, 255),
    "white": (255, 255, 255),
}
SATURATED = ("red", "yellow", "green", "blue")


def shot(name):
    image = ImageGrab.grab()
    if OUT.exists():
        path = OUT / f"{name}.png"
        image.save(path)
    return np.array(image.convert("RGB"))


def count(image, colour, box, tolerance=6):
    band = image[box[1] : box[3], box[0] : box[2]]
    return int(np.all(np.abs(band.astype(int) - np.array(colour)) <= tolerance, axis=2).sum())


def palette_open(image):
    return all(count(image, COLOURS[c], PALETTE_BAND) >= 100 for c in SATURATED)


def strip_on_screen(image):
    """Whether the strip's plate is anywhere on screen, which says the drag worked."""
    plate = np.all(np.abs(image.astype(int) - np.array((42, 42, 42))) <= 2, axis=2)
    # The plate is a tall card: far more pixels than the scattered ones in a wallpaper.
    return int(plate.sum()) > 2000


def held(image):
    counts = {c: count(image, COLOURS[c], BUTTON_BAND) for c in COLOURS}
    name = max(counts, key=lambda c: counts[c])
    return name if counts[name] >= 100 else None


# --- the run ----------------------------------------------------------------------------

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
    # A warm-up move, because the very first one after the window appears can be coalesced
    # with wherever the pointer already was: the drag then starts from the wrong place and
    # the whole run measures a screen with no strip on it.
    move_to(REGION[0] + 11, REGION[1] + 13)
    time.sleep(0.2)

    # A region, held and dragged: releasing on the corner would leave nothing selected.
    left, top, right, bottom = REGION
    move_to(left, top)
    time.sleep(0.2)
    press()
    time.sleep(0.08)
    for step in range(1, 7):
        move_to(
            left + (right - left) * step // 6,
            top + (bottom - top) * step // 6,
        )
        time.sleep(0.03)
    release()
    time.sleep(0.5)

    away = (left + 400, top + 300)

    move_to(*away)
    time.sleep(0.35)
    closed = shot("0-no-hover")

    # The setup has to be checked before the thing being tested, or a failure to select a
    # region is reported as a palette that does not work.
    if not strip_on_screen(closed):
        sys.exit(
            "the setup failed: no strip on screen after the drag, so nothing about the "
            "palette can be measured. The pointer may not have been delivered."
        )

    # Straight into the palette's area, which is the reported fault.
    move_to(*swatch(3))
    time.sleep(0.4)
    inside = shot("0b-inside-while-closed")

    # Away, then onto the colour button.
    move_to(*away)
    time.sleep(0.3)
    move_to(*COLOUR)
    time.sleep(0.4)
    hovered = shot("1-hover")

    # Past the swatches and back, which is what a hand does when it goes too far. Without
    # a grace the palette is gone the moment the pointer is in neither, and there is no way
    # back except through the colour button.
    move_to(PALETTE_BAND[2] + 120, COLOUR[1])
    time.sleep(0.10)
    move_to(*swatch(1))
    time.sleep(0.30)
    overshoot = shot("1c-back-after-overshoot")

    # Away, then onto the colour button again for the journey proper.
    move_to(*away)
    time.sleep(0.5)
    move_to(*COLOUR)
    time.sleep(0.35)

    # Onto a swatch in small steps, the way a hand moves. This is where it used to close:
    # the pointer crossed the gap between the strip and the palette and was in neither.
    for x in range(COLOUR[0], swatch(3)[0], 6):
        move_to(x, COLOUR[1])
        time.sleep(0.012)
    move_to(*swatch(3))
    time.sleep(0.35)
    on_swatch = shot("2-on-swatch")

    press()
    time.sleep(0.05)
    release()
    time.sleep(0.35)
    picked = shot("3-picked-blue")

    move_to(*away)
    time.sleep(0.4)
    after = shot("4-closed")
finally:
    process.kill()
    process.wait(timeout=5)

failures = []
if palette_open(closed):
    failures.append("the palette is on screen with the pointer away")
if palette_open(inside):
    failures.append(
        "the palette opened from empty space: the pointer entered its area without touching "
        "the colour button"
    )
if not palette_open(overshoot):
    failures.append(
        "the palette was gone after the pointer went past the swatches and came back"
    )
if not palette_open(hovered):
    failures.append("pointing at the colour button does not open the palette")
if not palette_open(on_swatch):
    failures.append("the palette closed when the pointer moved onto a swatch")
if held(hovered) != "red":
    failures.append(f"before choosing, the button should show red and shows {held(hovered)}")
if held(after) != "blue":
    failures.append(f"after choosing blue the button shows {held(after)}")
if palette_open(after):
    failures.append("the palette stays on screen after the pointer leaves it")

print(f"colour button centre {COLOUR}, palette {PALETTE_BAND}")
print(
    f"{'shot':<22} {'sat swatches':<18} button"
)
for name, image in [
    ("0-no-hover", closed),
    ("0b-inside-closed", inside),
    ("1-hover", hovered),
    ("1c-after-overshoot", overshoot),
    ("2-on-swatch", on_swatch),
    ("3-picked-blue", picked),
    ("4-closed", after),
]:
    swatches = "/".join(str(count(image, COLOURS[c], PALETTE_BAND)) for c in SATURATED)
    print(f"{name:<22} {swatches:<18} {held(image)}")

print()
print("FAILED" if failures else "the palette opens only from the colour button, and keeps the choice")
for line in failures:
    print("  " + line)
sys.exit(1 if failures else 0)
