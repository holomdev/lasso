# lasso

A screenshot tool for Windows, written in Rust.

> **Status:** early development. The capture engine works end to end; the
> region-selection overlay is next.

## Overview

lasso captures the screen and — soon — lets you pick a region to share. The
capture engine is built for real multi-monitor setups, including monitors with
different DPI scaling:

- **DXGI Output Duplication** as the primary route, with one duplication session
  per output.
- **GDI `BitBlt`** as an automatic fallback for RDP sessions, machines without a
  usable GPU, and protected content.
- The whole **virtual desktop** is captured in physical pixels, so a selection
  can span monitors with different scale factors without drifting.
- **Per-monitor DPI aware** (`PER_MONITOR_AWARE_V2`, falling back to the
  Windows 8.1 mode).
- PNG output.

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
cargo run -- capture --backend gdi
cargo run -- capture --out C:\shots\full.png
```

### Commands

| Command | What it does |
|---|---|
| `lasso monitors` | Prints the virtual-desktop bounds and every monitor |
| `lasso capture [--out FILE] [--backend dxgi\|gdi]` | Captures the virtual desktop to a PNG |

By default captures are written to `target/test-output/capture.png`. Keeping them
under `target/` means throwaway runs never leave files that could be committed by
accident. Pass `--out` to write somewhere else.

## Project layout

```
src/
  main.rs          CLI: monitors | capture [--out FILE] [--backend dxgi|gdi]
  dpi.rs           DPI awareness bootstrap
  capture/
    mod.rs         Frame, Backend, MonitorInfo, virtual desktop, fallback logic
    dxgi.rs        DXGI Output Duplication (primary)
    gdi.rs         BitBlt into a DIB section (fallback)
```

## Roadmap

- [x] Per-monitor DPI awareness, set before any DC is created
- [x] Monitor enumeration and virtual-desktop bounds
- [x] DXGI Output Duplication per output, composited onto a virtual-desktop canvas
- [x] Automatic, overridable GDI `BitBlt` fallback
- [x] PNG output
- [ ] Selection overlay: topmost borderless window, dimmed outside the selection, crosshair cursor
- [ ] Magnifier and `W x H` size label
- [ ] Resize handles and keyboard control (Esc / Enter / arrow keys)
- [ ] Cursor compositing
- [ ] Screen rotation
- [ ] Global hotkeys and tray icon
- [ ] Clipboard copy
- [ ] Annotation tools

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

### Known limitations

- Output duplication never includes the cursor, so it has to be composited
  separately.
- Screen rotation is not handled.
- Only `DXGI_FORMAT_B8G8R8A8_UNORM` is accepted; any other format fails
  explicitly and the caller falls back to GDI rather than returning a black
  frame.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
