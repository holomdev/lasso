# lasso

A screenshot tool for Windows, written in Rust.

> **Status:** early development. The capture engine, the interactive region
> selection and the magnifier work end to end; the toolbar, annotations and
> sharing are next.

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
- PNG output.

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
```

### Commands

| Command | What it does |
|---|---|
| `lasso monitors` | Prints the virtual-desktop bounds and every monitor |
| `lasso capture [--out FILE] [--backend dxgi\|gdi]` | Captures the virtual desktop to a PNG |
| `lasso select [--out FILE]` | Captures, shows the selection overlay, and saves the chosen region |

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
  main.rs            CLI: monitors | capture | select
  dpi.rs             DPI awareness bootstrap
  capture/
    mod.rs           Frame, Backend, MonitorInfo, virtual desktop, fallback
    dxgi.rs          DXGI Output Duplication (primary)
    gdi.rs           BitBlt into a DIB section (fallback)
  overlay/
    mod.rs           Selection window, input handling, painting
    canvas.rs        Off-screen bright and dimmed copies of the capture
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
- [ ] Clipboard copy
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
