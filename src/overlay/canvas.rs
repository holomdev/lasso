//! Off-screen pixel buffers for the selection overlay.
//!
//! Two copies of the capture are kept: an untouched one and a dimmed one. The
//! overlay paints the dimmed copy across the whole window and then the bright
//! copy clipped to the selection, so the dimming effect costs no per-frame CPU
//! work at all.
//!
//! Both buffers are plain BGRA8, top-down, with no padding, which is exactly
//! what `Frame` already holds, so this type stays free of GDI device contexts.

use std::ffi::c_void;

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    IntersectClipRect, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC,
    SRCCOPY,
};

/// How much of the original brightness the dimmed copy keeps, out of 256.
const DIM_KEEP: u32 = 112;

pub struct Canvas {
    base: Vec<u8>,
    dim: Vec<u8>,
    width: u32,
    height: u32,
}

impl Canvas {
    /// Build both buffers from a BGRA8 capture.
    pub fn new(bgra: &[u8], width: u32, height: u32) -> Self {
        let expected = width as usize * height as usize * 4;
        debug_assert_eq!(bgra.len(), expected, "capture size must match its bounds");

        let mut dim = Vec::with_capacity(bgra.len());
        for px in bgra.chunks_exact(4) {
            // Scale the colour channels; keep alpha opaque.
            let b = (px[0] as u32 * DIM_KEEP / 256) as u8;
            let g = (px[1] as u32 * DIM_KEEP / 256) as u8;
            let r = (px[2] as u32 * DIM_KEEP / 256) as u8;
            dim.extend_from_slice(&[b, g, r, 0xFF]);
        }

        Self {
            base: bgra.to_vec(),
            dim,
            width,
            height,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Top-down 32-bit header matching the buffers above.
    fn bitmap_info(&self) -> BITMAPINFO {
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: self.width as i32,
            biHeight: -(self.height as i32), // negative => top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        bmi
    }

    fn stretch(&self, dst: HDC, pixels: &[u8]) {
        let bmi = self.bitmap_info();
        unsafe {
            StretchDIBits(
                dst,
                0,
                0,
                self.width as i32,
                self.height as i32,
                0,
                0,
                self.width as i32,
                self.height as i32,
                Some(pixels.as_ptr() as *const c_void),
                &bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
    }

    /// Paint the dimmed capture across the whole window.
    pub fn paint_dimmed(&self, dst: HDC) {
        self.stretch(dst, &self.dim);
    }

    /// Paint the bright capture, clipped to `rect`.
    pub fn paint_bright(&self, dst: HDC, rect: &RECT) {
        unsafe {
            IntersectClipRect(dst, rect.left, rect.top, rect.right, rect.bottom);
        }
        self.stretch(dst, &self.base);
    }
}
