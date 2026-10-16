"""Open the built settings window and report its client size.

The client size is the quickest way to tell one build of this window from another:
the layout is a fixed width and grew taller when the group heights were fixed, so a
window that is not the expected height is not the current build.
"""
import ctypes
import ctypes.wintypes as wt
import subprocess
import sys
import time

EXE = sys.argv[1] if len(sys.argv) > 1 else r"target\debug\lasso.exe"
EXPECTED = {
    (468, 436): "before the group heights were fixed",
    (468, 496): "after the DPI-aware window frame, before the group heights",
    (468, 624): "current",
}

user32 = ctypes.windll.user32
process = subprocess.Popen([EXE, "settings"])
time.sleep(2.0)

found = []


def each(hwnd, _):
    text = ctypes.create_unicode_buffer(256)
    user32.GetWindowTextW(hwnd, text, 256)
    if "lasso settings" in text.value:
        found.append(hwnd)
    return True


ENUMPROC = ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
user32.EnumWindows(ENUMPROC(each), 0)

if not found:
    print("  no settings window appeared")
else:
    hwnd = found[0]
    client = wt.RECT()
    window = wt.RECT()
    user32.GetClientRect(hwnd, ctypes.byref(client))
    user32.GetWindowRect(hwnd, ctypes.byref(window))
    width = client.right - client.left
    height = client.bottom - client.top
    print(f"  client {width} x {height}   window "
          f"{window.right - window.left} x {window.bottom - window.top}")
    for (w, h), what in EXPECTED.items():
        if (w, h) == (width, height):
            print(f"  -> {what}")
            break
    else:
        print("  -> not a size this layout has ever produced at 100%")
    dpi = user32.GetDpiForWindow(hwnd)
    print(f"  dpi {dpi}")

if process.poll() is None:
    process.kill()
