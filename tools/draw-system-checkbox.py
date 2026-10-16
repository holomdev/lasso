"""Draw the system's own check box, so it can be compared with what the window does.

The settings window's check boxes come out as square outlines because the classic
`CHECKBOX` control asks the version 5 button library to draw it, and that library's
idea of a check box is the one from before Windows 11. The system's newer look is
available through the theme API, which this calls directly - no change to the
program, and nothing about the running window is modified.

The result is written next to the window's own check box, at its real size, so the
two can be looked at together.
"""
import ctypes
import ctypes.wintypes as wt

import numpy as np
from PIL import Image

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32
uxtheme = ctypes.windll.uxtheme

uxtheme.OpenThemeData.argtypes = [wt.HWND, wt.LPCWSTR]
uxtheme.OpenThemeData.restype = ctypes.c_void_p
uxtheme.CloseThemeData.argtypes = [ctypes.c_void_p]
uxtheme.GetThemePartSize.argtypes = [
    ctypes.c_void_p,
    wt.HDC,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_void_p,
    ctypes.c_int,
    ctypes.POINTER(wt.SIZE),
]
uxtheme.GetThemePartSize.restype = ctypes.c_long
uxtheme.DrawThemeBackground.argtypes = [
    ctypes.c_void_p,
    wt.HDC,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.POINTER(wt.RECT),
    ctypes.c_void_p,
]
uxtheme.DrawThemeBackground.restype = ctypes.c_long
uxtheme.IsThemeBackgroundPartiallyTransparent.argtypes = [
    ctypes.c_void_p,
    ctypes.c_int,
    ctypes.c_int,
]
uxtheme.IsThemeBackgroundPartiallyTransparent.restype = ctypes.c_bool

# From vssym32.h: the button part, and the check box states.
BP_CHECKBOX = 3
CBS_UNCHECKEDNORMAL = 1
CBS_CHECKEDNORMAL = 5
TS_TRUE = 1

themes = []


def find(hwnd, _):
    text = ctypes.create_unicode_buffer(256)
    user32.GetWindowTextW(hwnd, text, 256)
    if "lasso settings" in text.value:
        themes.append(hwnd)
    return True


ENUMPROC = ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
user32.EnumWindows(ENUMPROC(find), 0)
if not themes:
    raise SystemExit("open the settings window first: target\\debug\\lasso.exe settings")

hwnd = themes[0]
theme = uxtheme.OpenThemeData(hwnd, "BUTTON")
print("theme handle:", hex(theme) if theme else "null")
if not theme:
    raise SystemExit("the system has no theme for buttons")

for name, state in (("unchecked", CBS_UNCHECKEDNORMAL), ("checked", CBS_CHECKEDNORMAL)):
    size = wt.SIZE()
    result = uxtheme.GetThemePartSize(theme, None, BP_CHECKBOX, state, None, TS_TRUE,
                                      ctypes.byref(size))
    transparent = uxtheme.IsThemeBackgroundPartiallyTransparent(theme, BP_CHECKBOX, state)
    print(f"  {name}: size {size.cx}x{size.cy}  result 0x{result & 0xFFFFFFFF:08x}  "
          f"partially transparent: {bool(transparent)}")

# Draw both states side by side on a page-coloured background.
BOX = 32
canvas = Image.new("RGB", (BOX * 2 + 24, BOX + 16), (240, 240, 240))
memdc = gdi32.CreateCompatibleDC(None)


class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [
        ("biSize", wt.DWORD),
        ("biWidth", ctypes.c_long),
        ("biHeight", ctypes.c_long),
        ("biPlanes", wt.WORD),
        ("biBitCount", wt.WORD),
        ("biCompression", wt.DWORD),
        ("biSizeImage", wt.DWORD),
        ("biXPelsPerMeter", ctypes.c_long),
        ("biYPelsPerMeter", ctypes.c_long),
        ("biClrUsed", wt.DWORD),
        ("biClrImportant", wt.DWORD),
    ]


class BITMAPINFO(ctypes.Structure):
    _fields_ = [("bmiHeader", BITMAPINFOHEADER), ("bmiColors", wt.DWORD * 3)]


info = BITMAPINFO()
info.bmiHeader.biSize = ctypes.sizeof(BITMAPINFOHEADER)
info.bmiHeader.biWidth = canvas.width
info.bmiHeader.biHeight = -canvas.height  # top-down
info.bmiHeader.biPlanes = 1
info.bmiHeader.biBitCount = 32
info.bmiHeader.biCompression = 0  # BI_RGB

bits = ctypes.c_void_p()
gdi32.CreateDIBSection.restype = wt.HBITMAP
bitmap = gdi32.CreateDIBSection(memdc, ctypes.byref(info), 0, ctypes.byref(bits), None, 0)
print("dib section:", hex(bitmap) if bitmap else "null")

old = gdi32.SelectObject(memdc, bitmap)
page = gdi32.CreateSolidBrush(0x00F0F0F0)
full = wt.RECT(0, 0, canvas.width, canvas.height)
user32.FillRect(memdc, ctypes.byref(full), page)

for index, state in enumerate((CBS_UNCHECKEDNORMAL, CBS_CHECKEDNORMAL)):
    rect = wt.RECT(8 + index * (BOX + 8), 8, 8 + index * (BOX + 8) + BOX, 8 + BOX)
    uxtheme.DrawThemeBackground(theme, memdc, BP_CHECKBOX, state, ctypes.byref(rect), None)

buffer = (ctypes.c_char * (canvas.width * canvas.height * 4)).from_address(bits.value)
pixels = np.frombuffer(buffer, dtype=np.uint8).reshape(canvas.height, canvas.width, 4)
# The device context is BGRA, with the fourth byte unused in this format.
canvas = Image.fromarray(pixels[:, :, [2, 1, 0]].copy(), "RGB")
canvas = canvas.resize((canvas.width * 4, canvas.height * 4), Image.NEAREST)
canvas.save(r"target\test-output\system-checkbox.png")

gdi32.SelectObject(memdc, old)
gdi32.DeleteObject(bitmap)
gdi32.DeleteObject(page)
gdi32.DeleteDC(memdc)
uxtheme.CloseThemeData(theme)
print("wrote target\\test-output\\system-checkbox.png")
