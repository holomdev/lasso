//! Putting a captured frame on the Windows clipboard.
//!
//! The frame is published as `CF_DIB`: a `BITMAPINFOHEADER` followed by 32-bit
//! pixels, bottom-up, which is what every Windows application that accepts a
//! pasted image expects. A top-down DIB would be pasted upside down, and
//! `CF_DIBV5` is only worth it when transparency matters, which it does not for
//! a screen capture.
//!
//! The clipboard takes ownership of the memory handed to `SetClipboardData`, so
//! it must not be freed on success.

use std::mem::size_of;

use anyhow::{bail, Context, Result};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::Graphics::Gdi::{BITMAPINFOHEADER, BI_RGB};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

/// `CF_DIB`: the standard "device-independent bitmap" clipboard format.
///
/// The constant comes from the Win32 headers rather than the crate, so its
/// documented value is used.
const CF_DIB: u32 = 8;

/// Copy an opaque BGRA8 frame to the clipboard.
pub fn copy_bgra(bgra: &[u8], width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 {
        bail!("cannot copy an empty image");
    }
    let stride = width as usize * 4;
    let pixels = stride * height as usize;
    if bgra.len() != pixels {
        bail!(
            "buffer is {} bytes but {width}x{height} needs {pixels}",
            bgra.len()
        );
    }

    let total = size_of::<BITMAPINFOHEADER>() + pixels;

    unsafe {
        let handle: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, total).context("GlobalAlloc failed")?;
        let memory = GlobalLock(handle);
        if memory.is_null() {
            let _ = GlobalFree(Some(handle));
            bail!("GlobalLock returned a null pointer");
        }

        let header = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            // Positive height: clipboard bitmaps are bottom-up.
            biHeight: height as i32,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: pixels as u32,
            ..Default::default()
        };
        std::ptr::copy_nonoverlapping(
            &header as *const BITMAPINFOHEADER as *const u8,
            memory as *mut u8,
            size_of::<BITMAPINFOHEADER>(),
        );

        let destination = (memory as *mut u8).add(size_of::<BITMAPINFOHEADER>());
        for row in 0..height as usize {
            let source_row = height as usize - 1 - row;
            std::ptr::copy_nonoverlapping(
                bgra.as_ptr().add(source_row * stride),
                destination.add(row * stride),
                stride,
            );
        }

        let _ = GlobalUnlock(handle);

        OpenClipboard(None).context("OpenClipboard failed (another application may hold it)")?;
        let published = (|| -> Result<()> {
            EmptyClipboard().context("EmptyClipboard failed")?;
            SetClipboardData(CF_DIB, Some(HANDLE(handle.0)))
                .context("SetClipboardData(CF_DIB) failed")?;
            Ok(())
        })();
        let _ = CloseClipboard();

        if published.is_err() {
            // Nothing took ownership, so this memory is still ours to release.
            let _ = GlobalFree(Some(handle));
        }
        published
    }
}
