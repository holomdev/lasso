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

# The geometry comes from the one module that mirrors the program's metrics, so a change to the
# strip's size does not have to be made in every check by hand — which is what broke them all when
# the card was redrawn.
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import layout  # noqa: E402

# The icons are drawn in dark ink on the light card.
INK = (0x37, 0x41, 0x51)
BUTTON, GAP, PADDING = layout.BUTTON, layout.GAP, layout.PADDING
STRIP_X = layout.strip_x(REGION)
STRIP_Y = REGION[1]
COLOUR = layout.button_centre(5, REGION)
THICKNESS = layout.button_centre(6, REGION)

PALETTE_X, PALETTE_Y, PALETTE_W, PALETTE_H = layout.palette_box(5, 5, REGION)
PALETTE_BAND = (PALETTE_X, PALETTE_Y, PALETTE_X + PALETTE_W, PALETTE_Y + PALETTE_H)
BUTTON_BAND = (
    STRIP_X + PADDING,
    COLOUR[1] - BUTTON // 2,
    STRIP_X + PADDING + BUTTON,
    COLOUR[1] + BUTTON // 2,
)

THICK_PALETTE_X, THICK_PALETTE_Y, THICK_PALETTE_W, THICK_PALETTE_H = layout.palette_box(6, 6, REGION)
THICK_BAND = (
    THICK_PALETTE_X,
    THICK_PALETTE_Y,
    THICK_PALETTE_X + THICK_PALETTE_W,
    THICK_PALETTE_Y + THICK_PALETTE_H,
)
THICK_BUTTON_BAND = (
    STRIP_X + PADDING,
    THICKNESS[1] - BUTTON // 2,
    STRIP_X + PADDING + BUTTON,
    THICKNESS[1] + BUTTON // 2,
)


def thick_swatch(index):
    return layout.cell_centre(index, 6, 6, REGION)


def ink_area(image, band):
    """The ink pixels in a band, which for a dot is its area."""
    return count(image, INK, band)


def ink_per_cell(image):
    """Ink in each of the six thickness cells, in order."""
    return [ink_area(image, thick_cell(index)) for index in range(6)]


def thick_cell(index):
    x, y, w, h = layout.cell_box(index, 6, 6, REGION)
    return (x, y, x + w, y + h)


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
    "red": (239, 68, 68),  # #EF4444
    "yellow": (250, 204, 21),  # #FACC15
    "green": (34, 197, 94),  # #22C55E
    "blue": (59, 130, 246),  # #3B82F6
    "white": (255, 255, 255),  # #FFFFFF
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
    """Whether the card is on screen, which says the drag worked."""
    return layout.strip_visible(image, REGION)


def held(image):
    """The colour the button shows, read at its centre.

    Counting each palette colour over the button and taking the winner was the first version, and
    it started answering "white" for every colour: the card's plate is #FAFAFA, five levels from
    white, so a tolerance wide enough to read a screenshot also counted the plate as a white
    swatch. The swatch is a disc centred in its button, so one pixel is both enough and exact.
    """
    x, y = layout.button_centre(5, REGION)
    pixel = tuple(int(v) for v in image[y, x])
    for name, colour in COLOURS.items():
        if max(abs(a - b) for a, b in zip(pixel, colour)) <= 3:
            return name
    return "#{:02X}{:02X}{:02X}".format(*pixel)


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
    left, top, right, bottom = REGION

    def drag_a_region():
        # A warm-up move, because the very first one after the window appears can be
        # coalesced with wherever the pointer already was: the drag then starts from the
        # wrong place and the whole run measures a screen with no strip on it.
        move_to(left + 11, top + 13)
        time.sleep(0.2)
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
        return shot("setup")

    # Up to three attempts: the drag does not always land, and a check that reports "the
    # palette does not work" because the region was never made is worse than no check.
    for attempt in range(3):
        setup = drag_a_region()
        if strip_on_screen(setup):
            break
        print(f"  the drag did not take (attempt {attempt + 1}); trying again")
    else:
        sys.exit(
            "the setup failed: no strip on screen after three drags, so nothing about the "
            "palette can be measured. The pointer may not be reaching the window."
        )

    away = (left + 400, top + 300)

    move_to(*away)
    time.sleep(0.35)
    closed = shot("0-no-hover")


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

    # The thickness palette, the same way: open it from its button, then choose the widest.
    thick_before = shot("5-thickness-closed")
    move_to(*THICKNESS)
    time.sleep(0.4)
    thick_open = shot("6-thickness-open")
    for x in range(THICKNESS[0], thick_swatch(5)[0], 6):
        move_to(x, THICKNESS[1])
        time.sleep(0.012)
    move_to(*thick_swatch(5))
    time.sleep(0.3)
    press()
    time.sleep(0.05)
    release()
    time.sleep(0.35)
    thick_chosen = shot("7-thickness-chosen")
finally:
    process.kill()
    process.wait(timeout=5)

failures = []

# The thickness palette: a row of dots that get bigger, and a button that follows the choice.
sizes_open = ink_per_cell(thick_open)
if any(size < 4 for size in sizes_open):
    failures.append(f"a thickness cell is empty, so no dot was drawn there: {sizes_open}")
if not all(a < b for a, b in zip(sizes_open, sizes_open[1:])):
    failures.append(f"the thickness dots do not grow across the row: {sizes_open}")
if sizes_open and count(thick_open, layout.POPUP, THICK_BAND) < 1000:
    failures.append("the thickness palette is not on screen when its button is hovered")
if ink_area(thick_before, THICK_BUTTON_BAND) >= ink_area(thick_chosen, THICK_BUTTON_BAND):
    failures.append(
        "the thickness button did not grow after choosing the widest: "
        f"{ink_area(thick_before, THICK_BUTTON_BAND)} then "
        f"{ink_area(thick_chosen, THICK_BUTTON_BAND)}"
    )

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
print("thickness dots per cell:", ink_per_cell(thick_open))
print(
    "thickness button ink:",
    ink_area(thick_before, THICK_BUTTON_BAND),
    "->",
    ink_area(thick_chosen, THICK_BUTTON_BAND),
)
print()
print("FAILED" if failures else "the palette opens only from the colour button, and keeps the choice")
for line in failures:
    print("  " + line)
sys.exit(1 if failures else 0)
