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
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HDC, HGDIOBJ, HBITMAP,
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
        // Nearest-neighbour stretching: a magnifier wants blocky pixels, not a
        // smoothed average of them.
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

    /// Draw part of the bright capture into `dest`, scaled up. Both rectangles
    /// must already be inside their respective coordinate spaces.
    pub fn magnify(&self, dest: &RECT, source: &RECT) {
        let dest_width = dest.right - dest.left;
        let dest_height = dest.bottom - dest.top;
        let source_width = source.right - source.left;
        let source_height = source.bottom - source.top;
        if dest_width <= 0 || dest_height <= 0 || source_width <= 0 || source_height <= 0 {
            return;
        }

        // GDI measures `ysrc` from the bottom of the DIB even though this one is
        // declared top-down with a negative `biHeight`, so a sub-rectangle has to
        // be asked for at its mirrored offset. Whole-image blits are unaffected
        // because `height - 0 - height` is still 0, which is why only the
        // magnifier ever looked wrong. This is covered by a test that reads the
        // composed pixels back.
        let ysrc = self.height as i32 - source.top - source_height;

        unsafe {
            StretchDIBits(
                self.mem,
                dest.left,
                dest.top,
                dest_width,
                dest_height,
                source.left,
                ysrc,
                source_width,
                source_height,
                Some(self.base.as_ptr() as *const c_void),
                &self.bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
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
    use crate::overlay::geometry;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::GdiFlush;

    /// Capture where every pixel encodes its own coordinates, so a misplaced
    /// sample is identifiable rather than merely wrong-looking.
    fn coordinate_canvas(width: u32, height: u32) -> Canvas {
        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                // Blue carries x, green carries y, red is a constant marker.
                bgra.extend_from_slice(&[x as u8, y as u8, 0x40, 0xFF]);
            }
        }
        Canvas::new(&bgra, width, height).expect("the test canvas must build")
    }

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

    #[test]
    fn magnify_puts_the_pixel_under_the_pointer_at_the_marked_square() {
        // The panel is a screen-space overlay, so the test canvas has to be
        // large enough to hold it even though a real capture never is.
        let (width, height) = (200u32, 200u32);
        let canvas = coordinate_canvas(width, height);
        let pointer = (40, 30);

        let source = geometry::magnifier_source(pointer);
        let visible = geometry::intersect(&source, width, height);
        assert_eq!(visible, source, "the sample must lie fully on the capture");

        let panel = RECT {
            left: 0,
            top: 0,
            right: geometry::MAGNIFIER_PANEL,
            bottom: geometry::MAGNIFIER_PANEL,
        };
        let content = geometry::magnifier_content(&panel);
        let dest = geometry::magnifier_dest(&content, &source, &visible);
        canvas.magnify(&dest, &visible);

        let marked = geometry::magnifier_marked(&content);
        let centre_x = (marked.left + marked.right) / 2;
        let centre_y = (marked.top + marked.bottom) / 2;
        let (blue, green, _red) = canvas.back_pixel_for_test(centre_x, centre_y);

        assert_eq!(
            (blue as i32, green as i32),
            pointer,
            "the marked square must hold the pixel under the pointer"
        );
    }
}
