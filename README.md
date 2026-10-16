# lasso

A screenshot tool for Windows, written in Rust.

> **Status:** early development. The capture engine, the interactive region
> selection, the magnifier, the clipboard, the resident tray mode and the
> settings window work end to end; annotations and sharing are next.

## Overview

lasso captures the screen and lets you pick a region to keep. The capture engine
is built for real multi-monitor setups, including monitors with different DPI
scaling:

- **DXGI Output Duplication** as the primary route, with one duplication session
  per output.
- **GDI `BitBlt`** as an automatic fallback for RDP sessions, machines without a
  usable GPU, and protected content.
- The whole **virtual desktop** is captured in physical pixels, so a selection
  can span monitors with different scale factors without drifting.
- **Per-monitor DPI aware** (`PER_MONITOR_AWARE_V2`, falling back to the
  Windows 8.1 mode).
- **PNG output**, or straight to the **clipboard** when the image is going to be
  pasted somewhere rather than kept.
- The **mouse pointer**, drawn in on request. Neither backend includes it on its
  own, so it is composited into the finished frame: `--cursor` for one capture,
  or a check box in the settings for every capture.

Started with no arguments, lasso runs resident with an icon in the notification
area and a global hotkey, so a capture costs one keystroke instead of a terminal
command. A left click on the icon opens a menu, a right click starts a selection
straight away, and the chosen region goes to the clipboard.

That menu also opens the settings, where the shortcut, the destinations and what
the left click does are chosen, and an About dialog.

Selecting a region freezes the screen and dims everything outside the
selection, so the area you are about to keep is the only part at full
brightness.

A magnifier follows the pointer and shows the surrounding pixels scaled up, with
the pixel under the pointer marked. The overlay also draws its own crosshair at
the pointer, so which pixel is being marked never depends on how the system
cursor happens to be drawn.

## Requirements

| Component | Version used | Notes |
|---|---|---|
| Rust (rustup) | 1.99.0 | toolchain `stable-x86_64-pc-windows-msvc` |
| MSVC Build Tools | 2022 (17.14.41) | provides the linker |
| Windows SDK | 10.0.26100.0 | |
| Windows | 10 or later, x64 | output duplication needs a WDDM driver |

## Build and run

Build through the script, which keeps the output out of any directory carrying an
integrity label and reports whether the binary it produced carries one:

```powershell
.\tools\build.ps1                 # -Release to optimise, -Run to start it
```

Plain `cargo build` works as well, but the binary then lands inside this project
directory — and if that directory carries a `Low` integrity label, so does the
binary. The command-line commands below behave normally either way, while the
resident mode would never show an icon. The last item under Known limitations
explains why, and what the script does about it.

```powershell
cargo run -- monitors
cargo run -- capture
cargo run -- select
cargo run -- capture --backend gdi
cargo run -- capture --out C:\shots\full.png
cargo run -- capture --clipboard
cargo run -- select --clipboard
cargo run --                                  # resident: tray icon + global hotkey
cargo run -- tray --hotkey Ctrl+Shift+Y
```

### Commands

| Command | What it does |
|---|---|
| `lasso` or `lasso tray [--hotkey COMBO]` | Runs resident, with a notification-area icon and a global hotkey |
| `lasso settings` | Opens the settings window on its own, without running resident |
| `lasso settings --dpi N` | The same window, laid out for a display of that scale |
| `lasso about` | Shows the version, the license and the repository |
| `lasso monitors` | Prints the virtual-desktop bounds and every monitor |
| `lasso capture [--out FILE] [--backend dxgi\|gdi] [--clipboard] [--cursor]` | Captures the virtual desktop to a PNG |
| `lasso select [--out FILE] [--clipboard] [--cursor]` | Captures, shows the selection overlay, and saves the chosen region |

Both commands take `--clipboard` (`-c`) to put the result on the clipboard as
well, so it can be pasted somewhere without going through a file, and `--cursor`
to draw the mouse pointer into the capture.

### Settings

The resident mode reads `%APPDATA%\lasso\lasso.ini`, and edits it through the
settings window — from the tray menu, or with `lasso settings` when the resident
mode is not running. It is plain text, so it can also be edited by hand, copied
between machines and kept under version control:

```ini
hotkey = Ctrl+Shift+X
clipboard = yes
cursor = no
save_file = no
output_dir = C:\Users\you\Pictures\lasso
tray_left_click = menu
```

| Key | Meaning |
|---|---|
| `hotkey` | Global shortcut that starts a selection. At least one modifier is required |
| `clipboard` | Put each capture on the clipboard |
| `cursor` | Draw the mouse pointer into the capture |
| `save_file` | Also write each capture to `output_dir` |
| `output_dir` | Where saved captures go |
| `tray_left_click` | `menu` or `capture`: what a left click on the tray icon does |

A line the program does not recognise is **kept exactly as written**, including
keys written by a newer build, so editing the file by hand never silently loses
entries. A value that cannot be understood leaves that one setting at its default
and reports the line it came from, rather than refusing to start.

`--hotkey` overrides the file's combination for a single run and does not write
back, which is what makes it usable from a shortcut.

### Selection keys

| Input | Result |
|---|---|
| left drag | defines the region; dragging again replaces it |
| Enter | saves the region, or the whole screen when nothing is selected |
| Esc or right click | cancels |

The magnifier steps outside the selection while an edge is dragged, so it never
covers the handles on the far side, and it stays out of the way entirely while
the whole region is being moved.

By default output goes to `target/test-output/`. Keeping it under `target/` means
throwaway runs never leave files that could be committed by accident. Pass
`--out` to write somewhere else.

## Project layout

```
src/
  main.rs            CLI dispatch: tray | monitors | capture | select
  clipboard.rs       Publishing a frame as CF_DIB
  dpi.rs             DPI awareness bootstrap
  hotkey.rs          Global hotkey combinations, parsed and rendered
  icon.rs            Reading the embedded .ico for the size Windows asks for
  settings.rs        The settings model, and the lasso.ini they live in
  settings_window.rs The window that edits them
  ui_layout.rs       Where that window puts every control, at any display scale
  tray.rs            Resident mode: tray icon, menu, hotkey, message loop
  capture/
    mod.rs           Frame, Backend, MonitorInfo, virtual desktop, fallback
    dxgi.rs          DXGI Output Duplication (primary)
    gdi.rs           BitBlt into a DIB section (fallback)
  overlay/
    mod.rs           Selection window, input handling, painting
    canvas.rs        Off-screen bright and dimmed copies of the capture
    geometry.rs      Pure hit-testing and layout arithmetic
assets/
  lasso.ico          Generated by tools/make_icon.py
tools/
  make_icon.py       Renders the icon at every size it ships
  build.ps1          Builds outside any labelled directory, for tray mode
  ui-shot.ps1        Captures the settings window, optionally at a chosen scale
  ui-probe.ps1       Captures it printed and copied, with or without a scale change
  ui-size.ps1        Prints the window and client rectangles
  ui-roundtrip.ps1   Drives the window and checks the settings file it wrote
```

## Roadmap

- [x] Per-monitor DPI awareness, set before any DC is created
- [x] Monitor enumeration and virtual-desktop bounds
- [x] DXGI Output Duplication per output, composited onto a virtual-desktop canvas
- [x] Automatic, overridable GDI `BitBlt` fallback
- [x] PNG output
- [x] Selection overlay: topmost borderless window, dimmed outside the region,
      crosshair cursor, live size label
- [x] Move, resize handles and arrow-key nudging of the region
- [x] Magnifier and a pointer crosshair, for placing an edge precisely
- [x] Cursor compositing, opt-in, on both backends and in the resident mode
- [ ] Screen rotation
- [ ] Global hotkeys and tray icon
- [x] Clipboard copy
- [x] Resident mode: notification-area icon, context menu, global hotkey
- [x] Embedded application icon, generated at every size it ships
- [x] Settings window: hotkey, output and tray click behaviour, saved to `lasso.ini`
- [x] About dialog
- [ ] Annotation tools
- [ ] Upload and share

## Implementation notes

### Desktop Duplication pitfalls

Two behaviours cost real debugging time and are handled explicitly in
`capture/dxgi.rs`:

1. **The staging texture must use the exact `DXGI_FORMAT` the duplication
   reports.** `CopyResource` across different formats copies nothing and returns
   no error at all, so the capture silently comes out black.
2. **The first `AcquireNextFrame` often returns a frame with no image**
   (`LastPresentTime == 0`, black surface). It has to be released with
   `ReleaseFrame` and retried; keeping it produces a black capture.

Both were pinned down by comparing the two capture routes on the same frame: the
DXGI route produced a single colour and 0% non-black pixels while the GDI route
produced the real desktop. With the fixes in place both routes agree.

### The mouse pointer

Neither route includes the pointer on its own, and the two do not agree about it
when asked: output duplication never draws it, and `BitBlt` from the screen only
does with `CAPTUREBLT`, which flickers the window being copied and refuses to copy
layered windows at all. So the pointer is drawn into the finished frame by
`capture/cursor.rs`, for both backends and for both the capture and the selection.

A pointer is not a rectangle of pixels. It is a shape drawn with an AND mask and
an XOR mask, and the pixels along its edge come out of those masks mixed with
whatever was behind them. `DrawIconEx` is given a copy of the pixels the pointer
is about to cover, so those edges are mixed with the screen they were over; draw
the shape into a bitmap of its own and they are mixed with black instead, and the
pointer arrives with dark fringes. That was the first version of this and the
fringes are what it looked like.

Two details decide whether the pointer lands in the right place:

- `CURSORINFO::ptScreenPos` is the point the pointer is **pointing at**, not the
  corner of its shape. The shape's top-left is that point less the hotspot from
  `GetIconInfo`, and using the raw position puts the pointer up and to the left of
  where it belongs.
- `DrawIconEx` applies the masks against the destination origin, so the
  coordinates it is given have to be in the bitmap it is drawing into, which is a
  small rectangle around the pointer rather than the whole frame.

`tools/cursor-check.ps1` checks it by putting a flat window of its own on screen,
capturing the same rectangle with and without the pointer, and comparing. The
window is what makes it work: on a live desktop the two captures differ over the
whole screen as the desktop repaints itself, and the pointer is invisible in that
noise. The check then asks that the changed pixels form a small cluster covering
the pointer's own position - which is what catches a hotspot error - and that they
came out partly transparent rather than fully replaced.

### Overlay rendering

The overlay keeps two copies of the capture: the untouched pixels and a dimmed
one, built once when the overlay opens. Painting is then two blits — the dimmed
copy across the window, and the bright copy clipped to the selection — so the
dimming effect costs no per-frame CPU work while dragging.

Repaints are limited to the union of the previous and the new selection
rectangle. Rebuilding the whole window on every mouse move is what makes naive
implementations flicker. Border and label are drawn with GDI on top of the
blits, with a transparent text background so the label reads over any content.

### Magnifier

The magnifier samples a small square around the pointer and scales it up by an
integer factor, with the pixel under the pointer landing exactly on the centre of
the panel. That property is asserted in the tests, because it silently broke
once: the panel and the marked square were placed correctly, yet the image
inside showed a different part of the screen.

The cause is a GDI quirk worth remembering. `StretchDIBits` measures `ysrc` from
the **bottom** of the DIB even when the header declares it top-down with a
negative `biHeight`, so a sub-rectangle has to be requested at its mirrored
offset. Whole-image blits are unaffected, because `height - 0 - height` is still
zero, which is why only the magnifier ever looked wrong.

The magnifier is drawn with nearest-neighbour stretching, since a magnifier wants
blocky pixels rather than a smoothed average of them.

### Settings window

The controls are created in plain code after the window exists, not in
`WM_CREATE`: that runs inside `CreateWindowExW`, before the caller has had any
chance to hand over the owner and the settings the controls have to be filled
from. The window is cut into four group frames — the shortcut, what happens after
a capture, where a file goes, and the tray icon — rather than one flat column of
controls, which read as a list of unrelated lines.

The hotkey field is a read-only `EDIT` whose window procedure is replaced. A
read-only edit still takes focus and still receives key messages, which makes it
the smallest thing that can capture a combination; a control painted from scratch
would be far more code for a field that shows one line.

Every coordinate comes from `ui_layout::Layout`, which is one base layout at 96 DPI
passed through a single scale factor. The window is per-monitor DPI aware, so
Windows does not scale it: at 150% it would be drawn at its 96-DPI size and be too
small for its own contents. The font is created per display for the same reason —
the stock GUI font does not follow a window to a denser display.

Moving the window to a display with a different scale rebuilds its controls rather
than moving them, because a window paints its own background around its children
rather than over them: a control that moves leaves its old contents behind. What
the controls hold is read out first and written back afterwards, so an edit that
has not been saved survives the change.

`lasso settings --dpi N` builds the layout for a display of that scale instead of
the one in use. There is otherwise no way to see the window at 150% on a machine
with one display, and a layout that is only ever checked at 100% is only known to
work at 100%. `tools/ui-shot.ps1` captures the result.

The window pumps a nested message loop and never posts `WM_QUIT`, because that
would end the resident mode's loop as well. It reports completion through a flag
instead.

Each row's label is given the **same rectangle height and top as the field beside
it**, rather than being sized to its own text and centred. Sizing it to the text
cannot line the two up: when the two heights have different parity there is no
whole pixel that centres one on the other, and the half pixel it is out by is
visible on a row that is otherwise all straight lines. A static control centres
its own text vertically, so the taller rectangle costs nothing.

The labels and the read-only fields are painted in the window's own colour, which
is what `WM_CTLCOLORSTATIC` is for. Left alone, a static and a read-only edit both
paint themselves on `COLOR_WINDOW`, so they came out white while the window around
them was `COLOR_BTNFACE` grey, and a page of white boxes with grey gaps between
them reads as an unfinished window. The brush is created from the same system
colour the window class names as its background, so the page and the boxes on it
are one surface.

A new shortcut is registered **before** the previous one is released, so a
combination another program already owns leaves the old one working rather than
leaving the mode with no shortcut at all.

### Notification area

Adding the icon is treated as optional. `Shell_NotifyIconW` can refuse it, and in
the environment this was developed in it refuses with `ERROR_ACCESS_DENIED` (5)
for every process tried: one started by the development agent, one started as an
interactive scheduled task, and an independent implementation using a system icon
and a matching structure. Comparing integrity levels found the difference: those
processes ran at Low, while `Shell_TrayWnd` and the applications whose icons do
appear run at Medium. Windows blocks a lower-integrity process from sending
messages to a higher-integrity window, and registering an icon is such a message,
so the call is refused before the shell ever sees it. The scheduled-task run is the
one case whose own integrity level was not confirmed.

That is a property of the environment the code ran in and not of the code, which is
the ordinary `Shell_NotifyIconW` sequence. Refusing to start over it would take the
hotkey down with it, even though the hotkey does not depend on the shell at all.
The mode warns, keeps the hotkey, and leaves the console visible: without an icon,
the console is the only way to stop it.

The attempt is repeated on a timer for the first half minute, because the mode can
be started before the shell is ready, and again whenever the shell announces that it
has restarted: an icon belongs to the shell that accepted it, so a restart takes it
away.

The console is hidden only when this process owns it. A console shared with a shell
belongs to that shell, and hiding it would take the user's own terminal window with
it.

### Clipboard

A capture is published as `CF_DIB`: a `BITMAPINFOHEADER` followed by 32-bit
pixels. Two details matter.

The rows are written **bottom-up**, with a positive `biHeight`. The capture
naturally holds them top-down, but a DIB pasted that way would show the image
upside down.

`CF_DIBV5` is deliberately not used. It carries an alpha channel, and a screen
capture is opaque, so it would only add a format that some applications handle
worse than the plain one.

The clipboard takes ownership of the memory handed to `SetClipboardData`, so it
is released only when publishing failed.

The result is checked by reading the clipboard back from a separate process and
comparing pixels against the saved PNG, which also rules out a vertical flip.

### Tests

```powershell
cargo test
```

Pixels encode their own coordinates, so a wrong offset or a flipped channel order
fails a test instead of producing a plausible-looking image. The crop used by the
selection is covered this way, and so is the magnifier: that test composes through
GDI and reads the back buffer's pixels back, which is what caught the mirrored
sampling described above.

The settings layout is covered the same way. `ui_layout.rs` touches no window, so
its tests build the arrangement at every scale Windows offers, from 100% to 300%,
and check that the button row ends inside the window, that no group overlaps the
next, that every column stays inside its frame and that a hint follows its field.
Those are the properties a capture cannot check, because a control that has been
placed outside the window still exists.

### Known limitations

- The token the pointer is drawn from is read when the window is, so a pointer
  that changes shape between the capture and the drawing is drawn with the newer
  shape. It is in the right place either way.
- Screen rotation is still not handled: the capture sizes come from the virtual
  desktop, which reports the rotated dimensions, but nothing detects a rotated
  output or corrects for one.
- The tray icon needs a working notification area. Without one the resident mode
  falls back to the hotkey alone.
- A binary carrying a `Low` mandatory integrity label can never show a tray icon.
  Windows applies the label of a file to the process started from it, and a
  low-integrity process cannot send messages to the higher-integrity window that
  owns the notification area, so `Shell_NotifyIcon` fails with
  `ERROR_ACCESS_DENIED` (5) however the program is started and by whomever — the
  program runs, the hotkey works, and no icon ever appears. This is not
  hypothetical: it is how it was found, after every launch context had been ruled
  out, because the directory the code was built in carried that label.
  `tools/build.ps1` keeps the output out of any labelled directory and reports
  whether the binary it produced carries a label. To check one by hand:

  ```powershell
  icacls lasso.exe | findstr Mandatory
  ```

  An installation should place the binary somewhere ordinary, such as
  `%LOCALAPPDATA%\Programs\lasso\`, for the same reason.
- Screen rotation is not handled.
- Only `DXGI_FORMAT_B8G8R8A8_UNORM` is accepted; any other format fails
  explicitly and the caller falls back to GDI rather than returning a black
  frame.
- The overlay covers every monitor but the selection is a single rectangle, so a
  selection crossing monitors is captured as one region.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
