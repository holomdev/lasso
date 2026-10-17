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
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BitBlt, COLORONCOLOR, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, HDC, HGDIOBJ, HBITMAP,
    IntersectClipRect, RestoreDC, SRCCOPY, SaveDC, SelectObject, SetStretchBltMode, StretchDIBits,
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
    /// The back buffer's pixels, as handed out by `CreateDIBSection`.
    ///
    /// Only the tests read this back, to check what was really composed.
    #[cfg_attr(not(test), allow(dead_code))]
    bits: *mut c_void,
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
        // Nearest-neighbour stretching, so the capture is never smoothed on its way
        // through: a blurred copy of the screen is not the screen.
        unsafe {
            SetStretchBltMode(mem, COLORONCOLOR);
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
            bits,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The back buffer's device context. Callers draw the border, the handles,
    /// the label and the magnifier here, never on the window DC.
    pub fn dc(&self) -> HDC {
        self.mem
    }

    /// The back buffer's pixels, as BGRA.
    ///
    /// The overlay draws the annotations into the back buffer, so this is the only way to
    /// get at what was drawn: the `base` and `dim` layers are the capture as it arrived
    /// and have never seen a stroke. Reading the capture instead of this is a mistake
    /// that leaves the annotations on screen and out of the saved file, which is exactly
    /// what it did until this was added.
    pub fn composed_bgra(&self) -> Vec<u8> {
        // GDI batches drawing, so the bits may still be empty without this.
        unsafe {
            let _ = GdiFlush();
        }
        let length = self.width as usize * self.height as usize * 4;
        let mut out = vec![0u8; length];
        if self.bits.is_null() {
            return out;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(self.bits as *const u8, out.as_mut_ptr(), length);
        }
        out
    }

    /// Write the back buffer to a BMP, for diagnosing what actually reached it.
    ///
    /// A fault that shows on screen as "the drawing went black" cannot be told apart from
    /// "the drawing was never composed" by looking at the window: both are a dark
    /// rectangle where something was. This writes out the buffer itself, so the two can be
    /// separated. BMP rather than PNG because it needs no encoder and the file is read
    /// back on this machine only.
    ///
    /// Called only when `LASSO_DUMP_CANVAS` is set, and only for the first few paints, so
    /// that leaving it in the program costs nothing and cannot fill a disk.
    pub fn dump_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;

        let pixels = self.composed_bgra();
        // A BMP row is padded to four bytes, which for BGRA is already true.
        let image_bytes = pixels.len();
        let header_size = 14 + 40;
        let file_size = header_size + image_bytes;

        let mut file = std::fs::File::create(path)?;
        file.write_all(b"BM")?;
        file.write_all(&(file_size as u32).to_le_bytes())?;
        file.write_all(&0u32.to_le_bytes())?;
        file.write_all(&(header_size as u32).to_le_bytes())?;
        file.write_all(&40u32.to_le_bytes())?;
        file.write_all(&(self.width as i32).to_le_bytes())?;
        // Negative height: the rows are stored top-down, like the DIB.
        file.write_all(&(-(self.height as i32)).to_le_bytes())?;
        file.write_all(&1u16.to_le_bytes())?;
        file.write_all(&32u16.to_le_bytes())?;
        file.write_all(&0u32.to_le_bytes())?;
        file.write_all(&(image_bytes as u32).to_le_bytes())?;
        file.write_all(&2835i32.to_le_bytes())?;
        file.write_all(&2835i32.to_le_bytes())?;
        file.write_all(&0u32.to_le_bytes())?;
        file.write_all(&0u32.to_le_bytes())?;
        file.write_all(&pixels)?;
        file.flush()
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

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::GdiFlush;

    impl Canvas {
        /// Read one composed pixel back as (blue, green, red).
        fn back_pixel(&self, x: i32, y: i32) -> (u8, u8, u8) {
            assert!(
                x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height,
                "({x}, {y}) is outside the {0}x{1} canvas",
                self.width,
                self.height
            );
            // GDI batches drawing; without this the bits may still be empty.
            unsafe {
                let _ = GdiFlush();
            }
            let offset = ((y as usize) * (self.width as usize) + x as usize) * 4;
            unsafe {
                let pixel = (self.bits as *const u8).add(offset);
                (*pixel, *pixel.add(1), *pixel.add(2))
            }
        }
    }

    impl Canvas {
        /// The pixel reader, reachable from the drawing tests in `super::draw`.
        ///
        /// A separate `impl` rather than an attribute on the one above: this whole block
        /// is inside the test module, so `pub(super)` would mean "visible to the test
        /// module's parent" and reach nothing useful.
        pub(crate) fn back_pixel_for_test(&self, x: i32, y: i32) -> (u8, u8, u8) {
            self.back_pixel(x, y)
        }
    }

}
