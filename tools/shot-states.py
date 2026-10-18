"""Photograph the strip in each of its states, side by side, with labels.

The states are the whole point of the restyle, and they are the one thing a single screenshot of
the resting strip cannot show. Each capture is cropped with a margin, because the bar's shadow lives
outside it — a crop to the bar alone would hide the very thing it is there to check.
"""

import ctypes
import ctypes.wintypes as wintypes
import pathlib
import subprocess
import sys
import time

from PIL import Image, ImageDraw, ImageFont

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import layout  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
EXE = ROOT / "target" / "debug" / "lasso.exe"
OUT = ROOT / "target" / "overlay-states"
REGION = (600, 250, 1200, 700)

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32
user32.SetProcessDPIAware()

ULONG_PTR = ctypes.c_ulonglong if ctypes.sizeof(ctypes.c_void_p) == 8 else ctypes.c_ulong

MOUSEINPUT = 0
INPUT_MOUSE = 0


class MOUSEINPUT_STRUCT(ctypes.Structure):
    _fields_ = [
        ("dx", wintypes.LONG),
        ("dy", wintypes.LONG),
        ("mouseData", wintypes.DWORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ULONG_PTR),
    ]


class INPUT(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [("mi", MOUSEINPUT_STRUCT)]

    _anonymous_ = ("u",)
    _fields_ = [("type", wintypes.DWORD), ("u", _U)]


def move_to(x, y):
    vx, vy = user32.GetSystemMetrics(76), user32.GetSystemMetrics(77)
    vw, vh = user32.GetSystemMetrics(78), user32.GetSystemMetrics(79)
    packet = INPUT()
    packet.type = MOUSEINPUT
    packet.mi.dx = round((x - vx) * 65535 / (vw - 1))
    packet.mi.dy = round((y - vy) * 65535 / (vh - 1))
    packet.mi.dwFlags = 0x0001 | 0x8000 | 0x4000
    user32.SendInput(1, ctypes.byref(packet), ctypes.sizeof(INPUT))


def press():
    packet = INPUT()
    packet.type = MOUSEINPUT
    packet.mi.dwFlags = 0x0002
    user32.SendInput(1, ctypes.byref(packet), ctypes.sizeof(INPUT))


def release():
    packet = INPUT()
    packet.type = MOUSEINPUT
    packet.mi.dwFlags = 0x0004
    user32.SendInput(1, ctypes.byref(packet), ctypes.sizeof(INPUT))


def screen():
    width = user32.GetSystemMetrics(78)
    height = user32.GetSystemMetrics(79)
    hdc = user32.GetDC(None)
    memory = gdi32.CreateCompatibleDC(hdc)
    bitmap = gdi32.CreateCompatibleBitmap(hdc, width, height)
    gdi32.SelectObject(memory, bitmap)
    gdi32.BitBlt(memory, 0, 0, width, height, hdc, 0, 0, 0x00CC0020)
    gdi32.GdiFlush()

    class BITMAPINFOHEADER(ctypes.Structure):
        _fields_ = [
            ("biSize", wintypes.DWORD),
            ("biWidth", wintypes.LONG),
            ("biHeight", wintypes.LONG),
            ("biPlanes", wintypes.WORD),
            ("biBitCount", wintypes.WORD),
            ("biCompression", wintypes.DWORD),
            ("biSizeImage", wintypes.DWORD),
            ("biXPelsPerMeter", wintypes.LONG),
            ("biYPelsPerMeter", wintypes.LONG),
            ("biClrUsed", wintypes.DWORD),
            ("biClrImportant", wintypes.DWORD),
        ]

    header = BITMAPINFOHEADER()
    header.biSize = ctypes.sizeof(BITMAPINFOHEADER)
    header.biWidth = width
    header.biHeight = -height
    header.biPlanes = 1
    header.biBitCount = 32
    header.biCompression = 0

    buffer = ctypes.create_string_buffer(width * height * 4)
    gdi32.GetDIBits(memory, bitmap, 0, height, buffer, ctypes.byref(header), 0)
    gdi32.DeleteObject(bitmap)
    gdi32.DeleteDC(memory)
    user32.ReleaseDC(None, hdc)
    return Image.frombuffer("RGBA", (width, height), buffer, "raw", "BGRA", 0, 1).convert("RGB")


def crop_with_margin(image):
    left = layout.strip_x(REGION) - 8
    top = layout.strip_top(REGION) - 8
    return image.crop(
        (
            left,
            top,
            left + layout.STRIP_WIDTH + 16,
            top + layout.STRIP_HEIGHT + 16,
        )
    )


# --- the run --------------------------------------------------------------------------------

OUT.mkdir(parents=True, exist_ok=True)
process = subprocess.Popen(
    [str(EXE), "select"],
    stdout=subprocess.DEVNULL,
    stderr=(OUT / "stderr.txt").open("w", encoding="utf-8", errors="replace"),
)
time.sleep(2.5)
if process.poll() is not None:
    sys.exit("the overlay exited before it was used")

shots = []
try:
    # Drag the region out.
    # Exactly the region the shared layout is written for: the strip is placed beside the
    # selection, so a drag that lands a few pixels off puts the crop a few pixels off with it.
    start = (REGION[0], REGION[1])
    end = (REGION[2] - 1, REGION[3] - 1)
    move_to(*start)
    press()
    time.sleep(0.15)
    for step in range(1, 9):
        move_to(
            start[0] + (end[0] - start[0]) * step // 8,
            start[1] + (end[1] - start[1]) * step // 8,
        )
        time.sleep(0.05)
    release()
    time.sleep(0.5)

    # Away from the strip: the resting state.
    move_to(REGION[0] + 260, REGION[1] + 120)
    time.sleep(0.4)
    shots.append(("rest", crop_with_margin(screen())))

    for name, index in (("hover arrow", 1), ("hover copy", 10), ("hover cancel", 11), ("hover save", 9)):
        move_to(*layout.button_centre(index, REGION))
        time.sleep(0.35)
        shots.append((name, crop_with_margin(screen())))

    # Held down, without releasing: the pressed state.
    move_to(*layout.button_centre(9, REGION))
    time.sleep(0.2)
    press()
    time.sleep(0.35)
    shots.append(("pressed save", crop_with_margin(screen())))
    # Let go off the button, so the click does not close the overlay.
    move_to(REGION[0] + 260, REGION[1] + 120)
    time.sleep(0.2)
    release()
    time.sleep(0.3)

    move_to(*layout.button_centre(11, REGION))
    time.sleep(0.2)
    press()
    time.sleep(0.35)
    shots.append(("pressed cancel", crop_with_margin(screen())))
    move_to(REGION[0] + 260, REGION[1] + 120)
    time.sleep(0.2)
    release()
    time.sleep(0.3)
finally:
    if process.poll() is None:
        process.kill()
    time.sleep(0.3)

if not shots:
    sys.exit("nothing was captured")

# Stitched, with the name of each state at the bottom.
gap = 10
width = sum(shot.width for _, shot in shots) + gap * (len(shots) - 1)
height = max(shot.height for _, shot in shots) + 22
sheet = Image.new("RGB", (width, height), (235, 237, 240))
draw = ImageDraw.Draw(sheet)
label = ImageFont.truetype(r"C:\Windows\Fonts\segoiui.ttf" if False else r"C:\Windows\Fonts\segoeui.ttf", 13)
x = 0
for name, shot in shots:
    sheet.paste(shot, (x, 0))
    draw.text((x + 2, shot.height + 3), name, font=label, fill=(40, 44, 52))
    x += shot.width + gap
sheet.save(OUT / "states.png")
for name, _ in shots:
    print(f"  {name}")
print(f"states: {sheet.width}x{sheet.height} -> {OUT / 'states.png'}")
