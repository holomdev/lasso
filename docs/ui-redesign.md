# Settings window: what is left

The window is functional and its behaviour is covered: values load from the
settings file, saving writes them back, `IsDialogMessage` provides Tab, Enter and
Escape, and the sizing takes the window frame into account.

The layout is now grouped and scaled. This note records what that cost, how to
check a change, and what is genuinely still open.

## Where the layout lives

All of it is in two files:

- `src/ui_layout.rs` — `Layout`, the whole arrangement, and `scale`, which turns a
  value at 96 DPI into one at any scale. Nothing here touches a window, which is
  what makes it testable at every scale without a second monitor.
- `src/settings_window.rs` — `build_controls()` creates each control from a
  `Layout`, `child()` is what every control is created through, and `rescale()`
  rebuilds the window for a new display.

`Layout` is one base layout at 96 DPI passed through a single scale factor. It is a
three-column grid — indent, right-aligned label, field — cut into four frames:
Shortcut, After each capture, Saving to a file, and Tray icon. Each frame reserves
a caption row and then the controls, so a caption never runs into the field below
it.

Every coordinate comes from `Layout`. Reading a base value anywhere else is a
value that does not scale, which is the failure this arrangement exists to prevent.

## Grouping

`GROUPBOX` is the Windows idiom for "these settings belong together", and it is
what tells the eye that the shortcut, the destinations, the output folder and the
tray icon are separate concerns. As one flat column of controls the window read as
a list of unrelated lines.

## Scaling

The process is per-monitor DPI aware, so Windows does not scale this window: at
150% it would be drawn at its 96-DPI size, too small for its own contents. Three
things have to move together, and scaling two of the three looks worse than
scaling none:

1. **The coordinates**, through `scale(pixels, dpi)`.
2. **The window size**, which follows from the scaled client size.
3. **The font**, which is the one that is easy to miss. The stock GUI font is a
   single shared object drawn at 96 DPI, so a private font is created from its
   face name at the scaled height.

The buttons are measured rather than fixed: each is as wide as its own label, read
back from the font with `GetTextExtentPoint32W`, plus a margin for the border and
the focus rectangle. A width fixed for the English labels clips a translation.

## Changing scale while the window is open

`WM_DPICHANGED` is the notification that the display scale changed. It arrives in
two situations, and they are not the same problem:

- **Before the window is shown**, which is what happens when it opens on a display
  the process default did not describe. Nothing is on screen, so the controls are
  simply moved.
- **After the window is shown**, which is what dragging it to another monitor does.
  The controls are destroyed, the window is erased with nothing on it, and the
  controls are created again.

The second case is not a preference, it is the only version that looks right. A
window paints its own background *around* its children rather than over them, so a
control that moves leaves its old contents behind and a window that grows keeps
whatever was on the screen in the area it grew into. New controls drawn over the
old ones are correct in a list of controls and visibly wrong on screen. The window
class now names a background brush (`COLOR_BTNFACE`) so that there is something to
erase with at all.

What the controls hold is read out before they are destroyed and written back
afterwards, because the window may be carrying edits that have not been saved and a
change of display scale is not a reason to lose them.

## How to check a change

Do not trust that a control exists: a control can exist while sitting outside the
window, which is exactly how the button row was lost once. Capture the window and
look at it.

```powershell
.\tools\ui-shot.ps1 -Out target\test-output\ui.png
```

That starts the window, captures it and closes it again. It captures the window
rather than the desktop, so whatever else is on screen does not end up in the
frame, and it works for a window that a scaled layout has pushed off the display.

The scale is the other half of the problem, and there is no second display here to
drag the window onto. `--dpi` opens the window laid out for a display of that
scale:

```powershell
.\tools\ui-shot.ps1 -Arguments '--dpi 144' -Out target\test-output\ui-150.png
.\tools\ui-shot.ps1 -Arguments '--dpi 192' -Out target\test-output\ui-200.png
```

`tools/ui-probe.ps1` covers the other path. It captures the window twice — printed,
and copied from the screen — and `-Dpi` sends `WM_DPICHANGED` first, which is the
path a real display change takes. A window that is correct at `--dpi 192` and wrong
after `-Dpi 192` is a window whose rescale is broken, and the two captures side by
side say which.

`tools/ui-roundtrip.ps1` drives the window and reads back the file it wrote, which
is the only check that sees what the window actually handed on. It types into the
folder field, toggles both check boxes and the tray click, changes scale with those
edits in place, saves, and compares the result against the settings file.

Two things about that harness are worth knowing, because both cost time:

- **Reading a control's text from outside the process is not reliable here.** It
  works for some controls and not others, in the same run, and it is the harness
  talking to itself rather than to the window. Assert on the saved file.
- **Typing, not setting.** `SetWindowTextW` from another process is one message
  carrying a marshalled string; keystrokes are what a person does, and they are a
  different path through the control. The harness sends `WM_CHAR` one character at
  a time for that reason.

## What is left

1. **Long labels in a translated build.** The label column is sized for
   "Output folder". A longer translation needs the column measured from the font
   the same way the buttons now are.
2. **The output folder hint wraps to two lines** at 100% and one at 150%. It is
   given room for two, so it fits either way, but the group is taller than it needs
   to be on a large display.
3. **Nothing else is known to be wrong.** The window has not been looked at on a
   real second monitor at a different scale; the scale changes here are synthetic,
   which exercises the same code path but not the display switch itself.
