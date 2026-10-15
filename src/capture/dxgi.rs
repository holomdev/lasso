//! Primary route: **DXGI Output Duplication**.
//!
//! Strategy:
//! 1. `CreateDXGIFactory1`, then walk adapters and outputs.
//! 2. Per output: a D3D11 device on **that** adapter plus `DuplicateOutput`
//!    (`DuplicateOutput` is a COM vtable method, so it never shows up in an
//!    import table).
//! 3. `AcquireNextFrame`, copy the texture into a *staging texture* and `Map`
//!    it to read the pixels on the CPU.
//! 4. Composite every monitor onto a virtual-desktop canvas, using the
//!    `DesktopCoordinates` each output reports rather than matching names.
//!
//! Two Desktop Duplication traps are handled here:
//!
//! * The *staging texture* must use **exactly** the `DXGI_FORMAT` the
//!   duplication reports. `CopyResource` across different formats copies
//!   nothing and **returns no error**, leaving a black image.
//! * `AcquireNextFrame` can hand back a frame with **no image** (the first one
//!   usually does). It has to be released with `ReleaseFrame` and retried;
//!   keeping it produces a black capture.
//!
//! Known limitations: screen rotation and the cursor are not handled (output
//! duplication never includes the cursor).

use anyhow::{bail, Context, Result};
use windows::core::{Interface, IUnknown};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1,
    IDXGIResource, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};

use super::{Backend, Frame};

/// How many times `AcquireNextFrame` is retried before giving up.
const ACQUIRE_ATTEMPTS: u32 = 15;
/// Timeout per attempt, in milliseconds.
const ACQUIRE_TIMEOUT_MS: u32 = 150;

pub fn capture(vx: i32, vy: i32, vw: u32, vh: u32) -> Result<Frame> {
    unsafe {
        let factory: IDXGIFactory1 =
            CreateDXGIFactory1().context("CreateDXGIFactory1 failed (is dxgi.dll missing?)")?;

        let mut canvas = vec![0u8; (vw as usize) * (vh as usize) * 4];
        let mut composited = 0usize;
        let mut first_error: Option<anyhow::Error> = None;

        let mut adapter_index = 0u32;
        loop {
            let adapter: IDXGIAdapter1 = match factory.EnumAdapters1(adapter_index) {
                Ok(a) => a,
                Err(_) => break, // DXGI_ERROR_NOT_FOUND: no more adapters
            };

            let mut output_index = 0u32;
            loop {
                let output: IDXGIOutput = match adapter.EnumOutputs(output_index) {
                    Ok(o) => o,
                    Err(_) => break, // no more outputs on this adapter
                };
                output_index += 1;

                match duplicate_into(&adapter, &output, vx, vy, vw, vh, &mut canvas) {
                    Ok(()) => composited += 1,
                    Err(err) => {
                        eprintln!("[capture] DXGI: output {output_index} skipped: {err:#}");
                        if first_error.is_none() {
                            first_error = Some(err);
                        }
                    }
                }
            }

            adapter_index += 1;
        }

        if composited == 0 {
            match first_error {
                Some(e) => return Err(e).context("no output could be duplicated with DXGI"),
                None => bail!("DXGI reported no video output"),
            }
        }

        eprintln!("[capture] DXGI composited {composited} output(s) into {vw}x{vh}");

        Ok(Frame {
            width: vw,
            height: vh,
            bgra: canvas,
            origin: (vx, vy),
            backend: Backend::Dxgi,
        })
    }
}

unsafe fn duplicate_into(
    adapter: &IDXGIAdapter1,
    output: &IDXGIOutput,
    vx: i32,
    vy: i32,
    vw: u32,
    vh: u32,
    canvas: &mut [u8],
) -> Result<()> {
    unsafe {
        let output_desc = output.GetDesc().context("IDXGIOutput::GetDesc failed")?;
        if !output_desc.AttachedToDesktop.as_bool() {
            bail!("output is not attached to the desktop");
        }

        let device = create_device_for(adapter)?;
        let duplication = {
            let output1: IDXGIOutput1 = output.cast().context("cast to IDXGIOutput1")?;
            let unknown: IUnknown = device.cast().context("cast of the device to IUnknown")?;
            output1
                .DuplicateOutput(Some(&unknown))
                .context("IDXGIOutput1::DuplicateOutput failed")?
        };

        let mode = duplication.GetDesc().ModeDesc;
        if mode.Width == 0 || mode.Height == 0 {
            bail!("the output reports a zero size");
        }
        // Only BGRA8 converts to RGBA for the PNG without guessing.
        if mode.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            bail!(
                "duplication format {:?} is not supported yet (expected B8G8R8A8_UNORM)",
                mode.Format
            );
        }

        let src_w = mode.Width;
        let src_h = mode.Height;

        // Build the staging texture once per output.
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        desc.Width = src_w;
        desc.Height = src_h;
        desc.MipLevels = 1;
        desc.ArraySize = 1;
        desc.Format = mode.Format; // <- must match, or CopyResource copies nothing
        desc.SampleDesc.Count = 1;
        desc.SampleDesc.Quality = 0;
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;

        let mut staging: Option<ID3D11Texture2D> = None;
        device
            .CreateTexture2D(&desc, None, Some(&mut staging))
            .context("CreateTexture2D (staging) failed")?;
        let staging = staging.context("CreateTexture2D returned no texture")?;
        let staging_res: ID3D11Resource = staging.cast().context("cast of the staging texture")?;
        let context = device.GetImmediateContext()?;

        // Retry until a frame WITH an image shows up.
        let mut pixels = vec![0u8; (src_w as usize) * (src_h as usize) * 4];
        let mut pitch = 0u32;
        let mut have_pixels = false;
        let mut seen_any = false;
        let mut empty = 0u32;

        for _ in 0..ACQUIRE_ATTEMPTS {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;

            match duplication.AcquireNextFrame(ACQUIRE_TIMEOUT_MS, &mut frame_info, &mut resource) {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
                Err(e) => return Err(e).context("AcquireNextFrame failed"),
            }

            // An image exists only if something was presented or frames
            // accumulated; otherwise the surface comes back black.
            let has_image = frame_info.LastPresentTime != 0 || frame_info.AccumulatedFrames != 0;

            let read = resource
                .take()
                .context("AcquireNextFrame returned no resource")
                .and_then(|res| {
                    read_back(
                        &context,
                        &staging_res,
                        &res,
                        &mut pixels,
                        &mut pitch,
                        src_w,
                        src_h,
                    )
                });

            // ALWAYS release the frame before acquiring again.
            let _ = duplication.ReleaseFrame();
            read?;
            seen_any = true;

            if has_image && !is_blank(&pixels) {
                have_pixels = true;
                break;
            }
            empty += 1;
        }

        if empty > 0 {
            eprintln!("[capture] DXGI: discarded {empty} frame(s) without a usable image");
        }

        if !seen_any {
            bail!("AcquireNextFrame returned no frame in {ACQUIRE_ATTEMPTS} attempts");
        }
        if !have_pixels {
            // Keep what was read last: a possibly stale capture beats failing
            // outright, and the caller can still fall back to GDI.
            eprintln!("[capture] DXGI: no fresh image; using the last frame read");
        }

        composite(
            &pixels,
            pitch,
            src_w,
            src_h,
            output_desc.DesktopCoordinates.left,
            output_desc.DesktopCoordinates.top,
            vx,
            vy,
            vw,
            vh,
            canvas,
        )
    }
}

/// Is the buffer all zeroes? Used to detect the black frame duplication hands
/// back before an image is available.
fn is_blank(pixels: &[u8]) -> bool {
    // Sample every 16 bytes: a real frame is almost never uniformly zero.
    pixels.iter().step_by(16).all(|&b| b == 0)
}

unsafe fn create_device_for(adapter: &IDXGIAdapter1) -> Result<ID3D11Device> {
    unsafe {
        let adapter_base: IDXGIAdapter = adapter.cast().context("cast to IDXGIAdapter")?;
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;

        // D3D_DRIVER_TYPE_UNKNOWN is mandatory when an adapter is supplied.
        D3D11CreateDevice(
            Some(&adapter_base),
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .context("D3D11CreateDevice failed")?;

        device.context("D3D11CreateDevice returned no device")
    }
}

/// Copy the duplicated texture into the staging texture and read it back to the
/// CPU.
unsafe fn read_back(
    context: &ID3D11DeviceContext,
    staging: &ID3D11Resource,
    resource: &IDXGIResource,
    pixels: &mut [u8],
    pitch_out: &mut u32,
    src_w: u32,
    src_h: u32,
) -> Result<()> {
    unsafe {
        let texture: ID3D11Texture2D = resource
            .cast()
            .context("cast of IDXGIResource to ID3D11Texture2D")?;
        let src_res: ID3D11Resource = texture.cast().context("cast of the texture")?;

        context.CopyResource(Some(staging), Some(&src_res));

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context
            .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .context("Map of the staging texture failed")?;

        let result = (|| -> Result<()> {
            if mapped.pData.is_null() {
                bail!("the mapped frame has a null pointer");
            }
            let row_bytes = (src_w as usize) * 4;
            let pitch = mapped.RowPitch as usize;
            if pitch < row_bytes {
                bail!("RowPitch ({pitch}) is smaller than the row width ({row_bytes})");
            }
            *pitch_out = mapped.RowPitch;
            // Copy row by row, dropping the driver padding.
            let src = mapped.pData as *const u8;
            for row in 0..src_h as usize {
                let src_off = row * pitch;
                let dst_off = row * row_bytes;
                std::ptr::copy_nonoverlapping(
                    src.add(src_off),
                    pixels.as_mut_ptr().add(dst_off),
                    row_bytes,
                );
            }
            Ok(())
        })();

        context.Unmap(staging, 0);
        result
    }
}

/// Copy one monitor's frame onto the virtual-desktop canvas, clipping whatever
/// falls outside and never overflowing the buffer.
#[allow(clippy::too_many_arguments)]
fn composite(
    pixels: &[u8],
    pitch: u32,
    src_w: u32,
    src_h: u32,
    left: i32,
    top: i32,
    vx: i32,
    vy: i32,
    vw: u32,
    vh: u32,
    canvas: &mut [u8],
) -> Result<()> {
    // Destination in canvas coordinates.
    let dx = left - vx;
    let dy = top - vy;

    // Range of source rows/columns that lands inside the canvas.
    let x0 = dx.max(0);
    let y0 = dy.max(0);
    let x1 = (dx + src_w as i32).min(vw as i32);
    let y1 = (dy + src_h as i32).min(vh as i32);
    if x1 <= x0 || y1 <= y0 {
        return Ok(()); // monitor entirely outside the canvas
    }

    let rows = (y1 - y0) as usize;
    let cols_bytes = ((x1 - x0) as usize) * 4;
    let dst_stride = (vw as usize) * 4;
    let src_stride = if pitch == 0 {
        (src_w as usize) * 4
    } else {
        pitch as usize
    };

    for row in 0..rows {
        let sy = (y0 - dy) as usize + row;
        let sx = (x0 - dx) as usize;
        let src_off = sy * src_stride + sx * 4;
        let dst_off = ((y0 as usize) + row) * dst_stride + (x0 as usize) * 4;
        canvas[dst_off..dst_off + cols_bytes]
            .copy_from_slice(&pixels[src_off..src_off + cols_bytes]);
    }
    Ok(())
}
