"""Ask each control in the live settings window whether it has a theme.

`GetWindowTheme` returns a theme handle for a themed control and zero for one
without, which is the only direct answer to whether the manifest took effect in the
running process.
"""
import ctypes
import ctypes.wintypes as wintypes

user32 = ctypes.windll.user32
uxtheme = ctypes.windll.uxtheme

uxtheme.GetWindowTheme.argtypes = [wintypes.HWND]
uxtheme.GetWindowTheme.restype = ctypes.c_void_p
uxtheme.IsThemeActive.restype = ctypes.c_bool
uxtheme.IsAppThemed.restype = ctypes.c_bool

print("IsThemeActive:", bool(uxtheme.IsThemeActive()))
print("IsAppThemed:  ", bool(uxtheme.IsAppThemed()))

windows = []


def find(hwnd, _):
    text = ctypes.create_unicode_buffer(256)
    user32.GetWindowTextW(hwnd, text, 256)
    if "lasso settings" in text.value:
        windows.append(hwnd)
    return True


ENUMPROC = ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)
user32.EnumWindows(ENUMPROC(find), 0)
print("settings windows found:", len(windows))
if not windows:
    raise SystemExit("no window to inspect")

parent = windows[0]
rows = []


def each(hwnd, _):
    class_name = ctypes.create_unicode_buffer(64)
    user32.GetClassNameW(hwnd, class_name, 64)
    text = ctypes.create_unicode_buffer(64)
    user32.GetWindowTextW(hwnd, text, 64)
    theme = uxtheme.GetWindowTheme(hwnd)
    rows.append((class_name.value, text.value[:24], bool(theme)))
    return True


user32.EnumChildWindows(parent, ENUMPROC(each), 0)
themed = sum(1 for _, _, t in rows if t)
print(f"child controls: {len(rows)}, themed: {themed}")
for class_name, text, has_theme in rows:
    mark = "themed" if has_theme else "CLASSIC"
    print(f"  {class_name:8} {mark:8} {text!r}")
