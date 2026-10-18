"""Compare the saved file with the screen it was supposed to capture, on a still region.

Two lessons are built in. The region is over the wallpaper, not over an application window: this
desktop's windows change while the check runs, and comparing against a moving picture measures the
picture, not the file. And the crop's origin is **searched for** rather than assumed — the first
version assumed the region moved exactly as far as the pointer, read a 17% difference, and blamed
the program for its own arithmetic.

Every pixel that is not a stroke has to match the screen, and the strokes have to be there.
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
OUT = ROOT / "target" / "overlay-saved"
SAVED = OUT / "result.png"

# Wallpaper, well clear of the taskbar, the desktop icons and every window.
REGION = (200, 640, 680, 1020)
MOVE = "--move" in sys.argv
MOVED_BY = (60, 40)
RED = (0xEF, 0x44, 0x44)

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
    time.sleep(0.3)


def strokes(image):
    return np.all(np.abs(image.astype(int) - np.array(RED)) <= 8, axis=2)


OUT.mkdir(parents=True, exist_ok=True)
SAVED.unlink(missing_ok=True)
left, top, right, bottom = REGION
if MOVE:
    left += MOVED_BY[0]
    top += MOVED_BY[1]
    right += MOVED_BY[0]
    bottom += MOVED_BY[1]

process = subprocess.Popen(
    [str(EXE), "select", "--out", str(SAVED)],
    stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL,
    text=True,
)
time.sleep(2.5)
if process.poll() is not None:
    sys.exit("the overlay exited before it was used")

try:
    # The region as it starts, even in the moved run: it is moved afterwards.
    start = (
        left - MOVED_BY[0] if MOVE else left,
        top - MOVED_BY[1] if MOVE else top,
        right - MOVED_BY[0] if MOVE else right,
        bottom - MOVED_BY[1] if MOVE else bottom,
    )
    for attempt in range(3):
        move_to(start[0] + 11, start[1] + 13)
        time.sleep(0.2)
        drag((start[0], start[1]), (start[2], start[3]))
        screen = np.array(ImageGrab.grab().convert("RGB"))
        if screen[start[1]:start[3], start[0]:start[2]].mean() > 40:
            break
        print(f"  the drag did not take (attempt {attempt + 1})")
    else:
        sys.exit("the setup failed: no selection on screen")

    # Two boxes with the rectangle tool, inside the region and away from the strip.
    key(0x52)  # R
    time.sleep(0.25)
    drag((start[0] + 60, start[1] + 60), (start[0] + 260, start[1] + 160))
    drag((start[0] + 300, start[1] + 200), (start[0] + 430, start[1] + 320))

    if MOVE:
        # The move tool, then drag the region by the middle, clear of both boxes.
        key(0x53)  # S
        time.sleep(0.25)
        grab = (start[0] + 20, start[1] + 350)
        drag(grab, (grab[0] + MOVED_BY[0], grab[1] + MOVED_BY[1]))

    key(0x0D)  # Enter
    process.wait(timeout=10)
    stamp = [line for line in (process.stdout.read() or "").splitlines() if "build" in line]
except subprocess.TimeoutExpired:
    process.kill()
    sys.exit("the overlay did not close on Enter")
finally:
    if process.poll() is None:
        process.kill()

time.sleep(0.5)
after = np.array(ImageGrab.grab().convert("RGB"))

failures = []
if not SAVED.exists():
    failures.append("no file was saved")
    saved = None
else:
    saved = np.array(Image.open(SAVED).convert("RGB"))
    want = (right - left, bottom - top)
    if (saved.shape[1], saved.shape[0]) != want:
        failures.append(
            f"the file is {saved.shape[1]}x{saved.shape[0]} and the region is {want[0]}x{want[1]}"
        )
        saved = None

if saved is not None:
    height, width = saved.shape[:2]
    drawn = strokes(saved)

    # Where the file actually came from: the best match within a region's width of the expected
    # corner. Searched rather than assumed, so a wrong assumption cannot pass for a wrong program.
    best = None
    for dy in range(-90, 91):
        for dx in range(-90, 91):
            y0, x0 = top + dy, left + dx
            if y0 < 0 or x0 < 0 or y0 + height > after.shape[0] or x0 + width > after.shape[1]:
                continue
            candidate = after[y0:y0 + height, x0:x0 + width]
            difference = np.abs(saved.astype(int) - candidate.astype(int)).max(axis=2)
            share = ((difference > 24) & ~drawn).sum() / (~drawn).sum()
            if best is None or share < best[0]:
                best = (share, dx, dy)

    share, dx, dy = best
    print(f"the file matches the screen at an offset of ({dx}, {dy}), disagreeing on {share:.2%}")
    print(f"stroke pixels in the file: {int(drawn.sum())}")
    if share > 0.02:
        failures.append(
            f"{share:.1%} of the file is not the screen and is not a stroke, even at the best "
            "offset: the saved image is not the capture"
        )
    if abs(dx) > 2 or abs(dy) > 2:
        failures.append(
            f"the file's content comes from ({left + dx}, {top + dy}) rather than the region at "
            f"({left}, {top})"
        )
    if drawn.sum() < 200:
        failures.append("the file has no stroke in it")

print()
print("FAILED" if failures else "the file is the screen, with the strokes on it")
for line in failures:
    print("  " + line)
if stamp:
    print("  " + stamp[0].strip())
sys.exit(1 if failures else 0)
