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

        // The capture's own alpha byte is not transparency: the screen has none, and both
        // capture backends leave the byte at zero. Copied through it made everything drawn on
        // this canvas transparent in anything that reads the fourth byte — the clipboard, where
        // a `CF_DIB` with `BI_RGB` has no alpha channel and the byte is undefined.
        let mut base = Vec::with_capacity(bgra.len());
        for px in bgra.chunks_exact(4) {
            base.extend_from_slice(&[px[0], px[1], px[2], 0xFF]);
        }

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
            base,
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
        // Everything a pen or a brush touched came back with its alpha at zero: GDI writes
        // colour and leaves the fourth byte alone. A screenshot is opaque, so the byte is set
        // here, at the one place pixels leave the buffer, rather than at each of the two places
        // they are written out.
        for pixel in out.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
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

    /// Copy the capture itself into the back buffer, undimmed and whole.
    ///
    /// What the saved image starts from. `compose` is for the window, which needs the dimmed
    /// area around a bright selection; the file needs neither, just the capture, so this is the
    /// one blit without the dressing.
    pub fn fill_with_capture(&self) {
        self.stretch(&self.base);
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


    #[test]
    fn a_new_canvas_is_empty_and_filling_it_brings_the_capture() {
        // The trap the saved image fell into: a canvas is constructed from the capture, but the
        // capture sits in its `base` layer and only reaches the back buffer when something blits
        // it there. Drawing on a fresh canvas draws on black, and reading it back returns black
        // with whatever was drawn on top — which is exactly what was written to the file and to
        // the clipboard.
        let capture: Vec<u8> = (0..8 * 8).flat_map(|_| [0x11u8, 0x22, 0x33, 0xFF]).collect();
        let canvas = Canvas::new(&capture, 8, 8).expect("a canvas");

        let empty = canvas.composed_bgra();
        assert!(
            empty.chunks_exact(4).all(|pixel| pixel[..3] == [0, 0, 0]),
            "a fresh canvas is not black, so this test no longer describes the trap"
        );

        canvas.fill_with_capture();
        let filled = canvas.composed_bgra();
        for pixel in filled.chunks_exact(4) {
            assert_eq!(
                &pixel[..3],
                &[0x11, 0x22, 0x33],
                "the capture did not reach the back buffer"
            );
            // A capture is opaque. Carrying its alpha byte through put a zero there, and a
            // `CF_DIB` with `BI_RGB` has no alpha channel, so the byte is undefined to whoever
            // reads it — which is a transparency nobody asked for.
            assert_eq!(pixel[3], 0xFF, "the composed pixels are not opaque");
        }
    }

    #[test]
    fn pixels_a_pen_touched_come_back_opaque() {
        // GDI writes colour into a 32-bit DIB and leaves the fourth byte alone, so every pixel a
        // stroke covers reads back with an alpha of zero. A viewer that honours alpha then shows
        // the drawing as a hole: the report was "the coloured lines are not there", while the
        // colours were present in the pixels. Anything that leaves this buffer has to be opaque,
        // and the check has to look at the alpha byte, since dropping it is what hid this.
        use crate::overlay::annotation::{Colour, Point, Shape, Stroke};
        use crate::overlay::draw::draw_stroke;

        let capture: Vec<u8> = (0..16 * 16).flat_map(|_| [0x40u8, 0x60, 0x80, 0x00]).collect();
        let canvas = Canvas::new(&capture, 16, 16).expect("a canvas");
        canvas.fill_with_capture();
        draw_stroke(
            canvas.dc(),
            &Stroke::new(
                Shape::Rectangle {
                    from: Point::new(2, 2),
                    to: Point::new(12, 12),
                },
                Colour::Red,
                2,
            ),
            (0, 0),
        );

        let pixels = canvas.composed_bgra();
        assert!(
            pixels.chunks_exact(4).all(|p| p[3] == 0xFF),
            "a pixel came back with an alpha below opaque"
        );
        // And the stroke is really there, so the test cannot pass by drawing nothing.
        let red = pixels
            .chunks_exact(4)
            .filter(|p| p[..3] == [0x44, 0x44, 0xEF])
            .count();
        assert!(red > 20, "the stroke did not reach the buffer: {red} red pixels");
    }
}
