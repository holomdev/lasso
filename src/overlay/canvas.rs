//! Off-screen buffers for the selection overlay.
//!
//! Three layers are kept:
//!
//! * `base` - the untouched capture,
//! * `dim`  - the same capture darkened once, when the overlay opens,
//! * a back buffer (a DIB section plus its memory DC) that everything is
//!   composed into and then presented with a single blit.
//!
//! Composing off-screen matters: painting the dimmed copy, the bright copy and
//! the border straight onto the window DC makes those three steps visible one
//! after the other, which is exactly what flicker looks like while dragging.

use std::ffi::c_void;

use anyhow::{Context, Result};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BitBlt, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, HDC, HGDIOBJ, HBITMAP, IntersectClipRect, RestoreDC, SRCCOPY, SaveDC,
    SelectObject, StretchDIBits,
};

/// How much of the original brightness the dimmed copy keeps, out of 256.
const DIM_KEEP: u32 = 112;

pub struct Canvas {
    base: Vec<u8>,
    dim: Vec<u8>,
    width: u32,
    height: u32,
    bmi: BITMAPINFO,
    /// Memory DC holding the back buffer, where the overlay composes.
    mem: HDC,
    back: HBITMAP,
    original: HGDIOBJ,
}

impl Canvas {
    /// Build the layers and the back buffer from a BGRA8 capture.
    pub fn new(bgra: &[u8], width: u32, height: u32) -> Result<Self> {
        let expected = width as usize * height as usize * 4;
        debug_assert_eq!(bgra.len(), expected, "capture size must match its bounds");

        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32), // negative => top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };

        let mut dim = Vec::with_capacity(bgra.len());
        for px in bgra.chunks_exact(4) {
            let b = (px[0] as u32 * DIM_KEEP / 256) as u8;
            let g = (px[1] as u32 * DIM_KEEP / 256) as u8;
            let r = (px[2] as u32 * DIM_KEEP / 256) as u8;
            dim.extend_from_slice(&[b, g, r, 0xFF]);
        }

        let mem = unsafe { CreateCompatibleDC(None) };
        if mem.0.is_null() {
            anyhow::bail!("CreateCompatibleDC failed");
        }

        let mut bits: *mut c_void = std::ptr::null_mut();
        let back = match unsafe { CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) } {
            Ok(b) => b,
            Err(e) => {
                unsafe {
                    let _ = DeleteDC(mem);
                }
                return Err(e).context("CreateDIBSection failed for the back buffer");
            }
        };
        let original = unsafe { SelectObject(mem, HGDIOBJ(back.0)) };

        Ok(Self {
            base: bgra.to_vec(),
            dim,
            width,
            height,
            bmi,
            mem,
            back,
            original,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The back buffer's device context. Callers draw the border and the label
    /// here, never on the window DC.
    pub fn dc(&self) -> HDC {
        self.mem
    }

    fn stretch(&self, pixels: &[u8]) {
        unsafe {
            StretchDIBits(
                self.mem,
                0,
                0,
                self.width as i32,
                self.height as i32,
                0,
                0,
                self.width as i32,
                self.height as i32,
                Some(pixels.as_ptr() as *const c_void),
                &self.bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
    }

    /// Compose the dimmed capture, and the bright capture inside `selection`,
    /// into the back buffer, limited to `invalid`.
    pub fn compose(&self, invalid: &RECT, selection: Option<&RECT>) {
        let saved = unsafe { SaveDC(self.mem) };
        if saved == 0 {
            return;
        }
        let _ = unsafe {
            IntersectClipRect(
                self.mem,
                invalid.left,
                invalid.top,
                invalid.right,
                invalid.bottom,
            )
        };

        self.stretch(&self.dim);

        if let Some(sel) = selection {
            let inner = unsafe { SaveDC(self.mem) };
            let _ = unsafe { IntersectClipRect(self.mem, sel.left, sel.top, sel.right, sel.bottom) };
            self.stretch(&self.base);
            if inner != 0 {
                unsafe {
                    let _ = RestoreDC(self.mem, inner);
                }
            }
        }

        unsafe {
            let _ = RestoreDC(self.mem, saved);
        }
    }

    /// Copy the composed region to the window in a single blit.
    pub fn present(&self, dst: HDC, rect: &RECT) {
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return;
        }
        unsafe {
            let _ = BitBlt(
                dst,
                rect.left,
                rect.top,
                width,
                height,
                Some(self.mem),
                rect.left,
                rect.top,
                SRCCOPY,
            );
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem, self.original);
            let _ = DeleteObject(HGDIOBJ(self.back.0));
            let _ = DeleteDC(self.mem);
        }
    }
}
