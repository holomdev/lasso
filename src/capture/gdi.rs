//! GDI fallback: `BitBlt` from the screen DC into a `CreateDIBSection`.
//!
//! This route keeps working where output duplication is unavailable. A DIB
//! section is used instead of `CreateCompatibleBitmap` so the pixels can be read
//! directly, without an extra `GetDIBits` pass.

use anyhow::{bail, Context, Result};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
};

use super::{Backend, Frame};

/// Capture `(x, y, w, h)` of the virtual desktop into a top-down BGRA8 buffer.
///
/// `x`/`y` may be negative: `BitBlt` accepts virtual-desktop coordinates and the
/// screen DC covers every monitor.
pub fn capture(x: i32, y: i32, w: u32, h: u32) -> Result<Frame> {
    unsafe {
        let screen = GetDC(None);
        if screen.0.is_null() {
            bail!("GetDC(NULL) returned an invalid DC");
        }

        let mem = CreateCompatibleDC(Some(screen));
        if mem.0.is_null() {
            let _ = ReleaseDC(None, screen);
            bail!("CreateCompatibleDC failed");
        }

        // A negative biHeight requests top-down rows, which is what we want.
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let dib = match CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) => b,
            Err(e) => {
                let _ = DeleteDC(mem);
                let _ = ReleaseDC(None, screen);
                return Err(e).context("CreateDIBSection failed");
            }
        };

        let previous = SelectObject(mem, HGDIOBJ(dib.0));

        let blt = BitBlt(mem, 0, 0, w as i32, h as i32, Some(screen), x, y, SRCCOPY);

        let outcome = (|| -> Result<Frame> {
            blt.context("BitBlt failed")?;
            if bits.is_null() {
                bail!("the DIB section returned a null pixel pointer");
            }
            let len = (w as usize) * (h as usize) * 4;
            let mut bgra = vec![0u8; len];
            std::ptr::copy_nonoverlapping(bits as *const u8, bgra.as_mut_ptr(), len);
            // BitBlt from the screen DC leaves the alpha channel undefined, so
            // force it opaque and keep the PNG well-formed.
            for px in bgra.chunks_exact_mut(4) {
                px[3] = 0xFF;
            }
            Ok(Frame {
                width: w,
                height: h,
                bgra,
                origin: (x, y),
                backend: Backend::Gdi,
            })
        })();

        SelectObject(mem, previous);
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteDC(mem);
        let _ = ReleaseDC(None, screen);

        outcome
    }
}
