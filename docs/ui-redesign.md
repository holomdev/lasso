# Settings window: what is left

The window is functional and its behaviour is covered: values load from the
settings file, saving writes them back, `IsDialogMessage` provides Tab, Enter and
Escape, and the sizing takes the window frame into account. What remains is the
layout itself. This note records where it stands and how to check a change.

## Where the layout lives

All of it is in `src/settings_window.rs`:

- `PAD`, `LABEL_WIDTH`, `FIELD_HEIGHT`, `CLIENT_WIDTH`, `CLIENT_HEIGHT` and
  `BUTTON_WIDTH` at the top of the file.
- `build_controls()`, which creates every control at an absolute position.
- `child()`, which every control is created through.

The window is a plain two-column grid: labels and check boxes start at `PAD`, and
the fields — with their hints — start at `PAD + LABEL_WIDTH`. The button row sits
at the bottom right. The field labels are right-aligned and end just short of the
fields, so each reads as belonging to the one beside it.

## What is left

1. **Group the three areas.** The window is one flat list, so nothing tells the eye
   that the hotkey, the two check boxes and the output folder are separate
   concerns. `GROUPBOX` is the Windows idiom for this: three frames with captions,
   and every control inside a frame shifted down by that frame's height.

2. **Scale for the display.** The layout is in fixed pixels and the process is
   per-monitor DPI aware, so Windows does not scale the window itself: at 150% the
   window is too small for its contents. It has to be done everywhere at once,
   because scaling part of it looks worse than scaling none. Applying a
   `scale(pixels, dpi)` helper inside `child()`, plus the window size and the font
   height, covers every case in one place — and the helper is a pure function, so
   it is worth a test rather than a manual check on a second monitor.

## How to check a change

Do not trust that a control exists: a control can exist while sitting outside the
window, which is exactly how the button row was lost once. Capture the window and
look at it.

```powershell
Start-Process .\target\debug\lasso.exe -ArgumentList 'settings'
.\target\debug\lasso.exe capture --out target\test-output\ui.png
```

Then open `target\test-output\ui.png`. Three problems in this window were found
that way — a console left on top of it, the button row below its bottom edge, and a
clipped output path — and none of them would have been visible to a check that only
asked whether the controls had been created.
