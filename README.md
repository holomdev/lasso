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

```powershell
cargo build
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
| `lasso about` | Shows the version, the license and the repository |
| `lasso monitors` | Prints the virtual-desktop bounds and every monitor |
| `lasso capture [--out FILE] [--backend dxgi\|gdi] [--clipboard]` | Captures the virtual desktop to a PNG |
| `lasso select [--out FILE] [--clipboard]` | Captures, shows the selection overlay, and saves the chosen region |

Both commands take `--clipboard` (`-c`) to put the result on the clipboard as
well, so it can be pasted somewhere without going through a file.

### Settings

The resident mode reads `%APPDATA%\lasso\lasso.ini`, and edits it through the
settings window — from the tray menu, or with `lasso settings` when the resident
mode is not running. It is plain text, so it can also be edited by hand, copied
between machines and kept under version control:

```ini
hotkey = Ctrl+Shift+X
clipboard = yes
save_file = no
output_dir = C:\Users\you\Pictures\lasso
tray_left_click = menu
```

| Key | Meaning |
|---|---|
| `hotkey` | Global shortcut that starts a selection. At least one modifier is required |
| `clipboard` | Put each capture on the clipboard |
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
- [ ] Cursor compositing
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
from.

The hotkey field is a read-only `EDIT` whose window procedure is replaced. A
read-only edit still takes focus and still receives key messages, which makes it
the smallest thing that can capture a combination; a control painted from scratch
would be far more code for a field that shows one line.

The window pumps a nested message loop and never posts `WM_QUIT`, because that
would end the resident mode's loop as well. It reports completion through a flag
instead.

A new shortcut is registered **before** the previous one is released, so a
combination another program already owns leaves the old one working rather than
leaving the mode with no shortcut at all.

### Notification area

Adding the icon is treated as optional. `Shell_NotifyIconW` reports failure with
`ERROR_ACCESS_DENIED` (5) in a session with no working shell — one where
`Shell_TrayWnd` exists but no `explorer.exe` runs — and refusing to start because
of that would take the hotkey down with it, even though the hotkey does not
depend on the shell at all. The mode warns, keeps the hotkey, and leaves the
console visible: without an icon, the console is the only way to stop it.

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

### Known limitations

- Output duplication never includes the cursor, so it has to be composited
  separately.
- The tray icon needs a working notification area. Without one the resident mode
  falls back to the hotkey alone.
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
