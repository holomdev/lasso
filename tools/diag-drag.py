"""Does a synthetic drag still create a selection?

Prints what the program itself reports, because a screenshot only says whether something
looked right: `region` in its log is the selection it actually made.
"""

import ctypes
import ctypes.wintypes as wintypes
import pathlib
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
EXE = ROOT / "target" / "debug" / "lasso.exe"
ERR = ROOT / "target" / "diag-drag.err"
OUT = ROOT / "target" / "diag-drag.png"

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
    time.sleep(0.04)
    up = INPUT()
    up.type = 1
    up.u.ki.wVk = vk
    up.u.ki.dwFlags = 0x0002
    send(up)


for old in (ERR, OUT):
    old.unlink(missing_ok=True)

handle = open(ERR, "w", encoding="utf-8", errors="replace")
process = subprocess.Popen([str(EXE), "select", "--out", str(OUT)], stderr=handle, stdout=subprocess.DEVNULL)
time.sleep(2.5)
print("alive after start:", process.poll() is None)

# Where the pointer is, before anything: if SendInput is not reaching the desktop at all,
# this does not move either.
before = wintypes.POINT()
user32.GetCursorPos(ctypes.byref(before))
move_to(611, 263)
time.sleep(0.15)
after = wintypes.POINT()
user32.GetCursorPos(ctypes.byref(after))
print(f"pointer: ({before.x},{before.y}) -> ({after.x},{after.y})")

# A drag.
move_to(600, 250)
time.sleep(0.2)
mouse(0x0002)
time.sleep(0.1)
for step in range(1, 7):
    move_to(600 + 100 * step, 250 + 75 * step)
    time.sleep(0.04)
time.sleep(0.1)
mouse(0x0004)
time.sleep(0.4)

# Enter saves whatever is selected, which makes the program say what it thinks the region is.
key(0x0D)
try:
    process.wait(timeout=10)
except subprocess.TimeoutExpired:
    print("the overlay did not close on Enter")
    process.kill()
handle.close()

print("--- what the program reported ---")
for line in ERR.read_text(encoding="utf-8", errors="replace").splitlines():
    if any(word in line for word in ("region", "elapsed", "file", "build")):
        print("  " + line.strip())
print("saved file exists:", OUT.exists())
