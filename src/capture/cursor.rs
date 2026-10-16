//! Drawing the mouse pointer into a capture.
//!
//! Neither capture route includes the pointer on its own. Output duplication
//! never does, and `BitBlt` from the screen only does with `CAPTUREBLT`, which
//! flickers the window being copied, refuses to copy layered windows at all, and
//! would give the two backends different pixels. So the pointer is drawn into the
//! finished frame, in one place, for both of them.
//!
//! The drawing is done by `DrawIconEx`, onto a **copy of what the pointer is
//! about to cover**. That is not a detail. A pointer is not a rectangle of
//! pixels: it is a shape drawn with an AND mask and an XOR mask, and the pixels
//! along its edge come out of those masks mixed with whatever was behind them.
//! Draw the shape into a bitmap of its own and those edges are mixed with black
//! instead, so the pointer lands on the frame with dark fringes where it should
//! be showing the screen. Give it the real neighbourhood and they come out right.
//!
//! Drawing straight into the frame is the obvious version and it is not
//! available: `CreateDIBSection` cannot be pointed at memory the caller already
//! owns, so there is no way to hand `DrawIconEx` a device context whose pixels
//! *are* the frame.

use anyhow::{bail, Context, Result};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, GetObjectW,
    HDC, ReleaseDC, SelectObject, SetDIBits, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DrawIconEx, GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, DI_NORMAL, HICON, ICONINFO,
};

/// The pointer as it was when it was asked for.
pub struct Cursor {
    /// The pointer's shape, owned by the system. Valid until something replaces
    /// the pointer, which is why it is used immediately and not stored.
    pub shape: HICON,
    /// Top-left of the shape on the virtual desktop.
    ///
    /// `CURSORINFO::ptScreenPos` is the point the pointer is **pointing at**,
    /// which is the hotspot inside the shape rather than its corner. Drawing at
    /// the raw position puts the pointer up and to the left of where it belongs,
    /// by exactly the hotspot.
    pub position: (i32, i32),
    pub width: u32,
    pub height: u32,
}

/// Ask the system for the pointer, or `None` when it is not being shown.
///
/// A hidden pointer is not a failure: it is the normal state over a window that
/// hides it, and the capture is simply taken without one.
pub fn current() -> Result<Option<Cursor>> {
    unsafe {
        let mut info = CURSORINFO {
            cbSize: std::mem::size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        GetCursorInfo(&mut info).context("GetCursorInfo failed")?;

        if info.flags != CURSOR_SHOWING || info.hCursor.is_invalid() {
            return Ok(None);
        }

        // A cursor handle and an icon handle are the same thing to the two calls
        // that read a pointer's shape; the crate keeps them as separate types.
        let shape = HICON(info.hCursor.0);

        let mut icon = ICONINFO::default();
        GetIconInfo(shape, &mut icon).context("GetIconInfo failed")?;
        // `GetIconInfo` creates both bitmaps, and nothing here draws with them:
        // their size is read and they are released again.
        let size = bitmap_size(icon.hbmMask).or_else(|| bitmap_size(icon.hbmColor));
        let hotspot = (icon.xHotspot as i32, icon.yHotspot as i32);
        release(icon);

        let Some((width, height)) = size else {
            bail!("the pointer has no measurable shape");
        };
        if width == 0 || height == 0 {
            bail!("the pointer reports a zero size");
        }

        Ok(Some(Cursor {
            shape,
            position: (
                info.ptScreenPos.x - hotspot.0,
                info.ptScreenPos.y - hotspot.1,
            ),
            width,
            height,
        }))
    }
}

/// Release the two bitmaps `GetIconInfo` produced.
fn release(icon: ICONINFO) {
    unsafe {
        if !icon.hbmColor.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(icon.hbmColor.0));
        }
        if !icon.hbmMask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(icon.hbmMask.0));
        }
    }
}

/// The size of a bitmap, in pixels.
///
/// `GetObjectW` on a bitmap fills in a `BITMAP`, and a pointer's shape is the
/// size of the mask it was built with.
unsafe fn bitmap_size(bitmap: HBITMAP) -> Option<(u32, u32)> {
    if bitmap.is_invalid() {
        return None;
    }
    let mut info = BITMAP::default();
    let read = unsafe {
        GetObjectW(
            HGDIOBJ(bitmap.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut info as *mut _ as *mut _),
        )
    };
    if read <= 0 || info.bmWidth <= 0 || info.bmHeight == 0 {
        return None;
    }
    Some((info.bmWidth as u32, info.bmHeight.unsigned_abs()))
}

/// Where a shape of this size lands inside a frame, or `None` when it misses it.
///
/// This is the arithmetic the module turns on. It is what puts the pointer on the
/// point it is pointing at rather than at its own corner, and what keeps a
/// pointer near an edge — or on a monitor the frame does not cover — inside the
/// buffer. It is separate from every Win32 call here so it can be tested without
/// a pointer to move.
pub fn landing(
    frame_width: u32,
    frame_height: u32,
    frame_origin: (i32, i32),
    position: (i32, i32),
    shape_width: u32,
    shape_height: u32,
) -> Option<(i32, i32, u32, u32)> {
    let offset_x = position.0 - frame_origin.0;
    let offset_y = position.1 - frame_origin.1;

    let left = offset_x.max(0);
    let top = offset_y.max(0);
    let right = (offset_x + shape_width as i32).min(frame_width as i32);
    let bottom = (offset_y + shape_height as i32).min(frame_height as i32);
    if right <= left || bottom <= top {
        return None;
    }
    Some((left, top, (right - left) as u32, (bottom - top) as u32))
}

/// Draw a pointer's shape into a frame's pixels at a virtual-desktop position.
///
/// `frame_origin` is where the frame's pixel `(0, 0)` sits on the virtual
/// desktop, and `position` is where the shape's top-left corner sits there.
///
/// Returns `false` without drawing when the shape falls outside the frame, which
/// is what a pointer on a monitor this capture does not cover looks like.
pub fn draw_into(
    frame: &mut [u8],
    frame_width: u32,
    frame_height: u32,
    frame_origin: (i32, i32),
    shape: HICON,
    shape_width: u32,
    shape_height: u32,
    position: (i32, i32),
) -> Result<bool> {
    let stride = frame_width as usize * 4;
    let expected = stride * frame_height as usize;
    if frame.len() < expected {
        bail!(
            "the frame buffer holds {} bytes but {frame_width}x{frame_height} needs {expected}",
            frame.len()
        );
    }

    // The pointer's position is answered before its handle is looked at, so that
    // a pointer outside this capture is not reported as a bad handle.
    let Some((left, top, area_width, area_height)) = landing(
        frame_width,
        frame_height,
        frame_origin,
        position,
        shape_width,
        shape_height,
    ) else {
        return Ok(false);
    };
    if shape.is_invalid() {
        bail!("the pointer's shape is not a handle");
    }

    let offset_x = position.0 - frame_origin.0;
    let offset_y = position.1 - frame_origin.1;

    unsafe {
        let screen = GetDC(None);
        if screen.0.is_null() {
            bail!("GetDC(NULL) returned an invalid DC");
        }
        let mem = CreateCompatibleDC(Some(screen));
        if mem.0.is_null() {
            let _ = ReleaseDC(None, screen);
            bail!("CreateCompatibleDC failed for the pointer");
        }

        // A bitmap holding a copy of what is already at that spot, so the
        // pointer's masks are applied against the content they cover.
        let area = CreateCompatibleBitmap(screen, area_width as i32, area_height as i32);
        let _ = ReleaseDC(None, screen);
        if area.is_invalid() {
            let _ = DeleteDC(mem);
            bail!("CreateCompatibleBitmap failed for the pointer");
        }

        let outcome = (|| -> Result<()> {
            write_bitmap(area, area_width, area_height, frame, frame_width, left, top)?;

            let previous = SelectObject(mem, HGDIOBJ(area.0));
            // The shape goes down with its own top-left at the destination
            // position, because `DrawIconEx` applies the pointer's masks against
            // the destination's origin.
            let drawn = DrawIconEx(
                mem,
                offset_x - left,
                offset_y - top,
                shape,
                shape_width as i32,
                shape_height as i32,
                0,
                None,
                DI_NORMAL,
            );
            let result = drawn.context("DrawIconEx failed");
            SelectObject(mem, previous);
            result?;

            read_bitmap(mem, area, area_width, area_height, frame, frame_width, left, top)
        })();

        let _ = DeleteObject(HGDIOBJ(area.0));
        let _ = DeleteDC(mem);
        outcome?;
    }

    Ok(true)
}

/// Copy a rectangle of the frame into a device-dependent bitmap.
unsafe fn write_bitmap(
    bitmap: HBITMAP,
    width: u32,
    height: u32,
    frame: &[u8],
    frame_width: u32,
    left: i32,
    top: i32,
) -> Result<()> {
    let stride = frame_width as usize * 4;
    let row_bytes = width as usize * 4;
    let mut rows = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = (top as usize + row) * stride + left as usize * 4;
        rows.extend_from_slice(&frame[start..start + row_bytes]);
    }

    let bmi = top_down(width, height);
    let copied = unsafe {
        SetDIBits(
            None,
            bitmap,
            0,
            height,
            rows.as_ptr() as *const core::ffi::c_void,
            &bmi,
            DIB_RGB_COLORS,
        )
    };
    if copied == 0 {
        bail!("SetDIBits failed while filling the pointer's bitmap");
    }
    Ok(())
}

/// Copy a device-dependent bitmap's pixels back into the frame.
///
/// `dc` is only used to answer questions about the bitmap's palette, so a memory
/// context will do; `GetDIBits` is the one call that insists on having one.
unsafe fn read_bitmap(
    dc: HDC,
    bitmap: HBITMAP,
    width: u32,
    height: u32,
    frame: &mut [u8],
    frame_width: u32,
    left: i32,
    top: i32,
) -> Result<()> {
    let stride = frame_width as usize * 4;
    let row_bytes = width as usize * 4;
    let mut rows = vec![0u8; row_bytes * height as usize];

    let mut bmi = top_down(width, height);
    let copied = unsafe {
        GetDIBits(
            dc,
            bitmap,
            0,
            height,
            Some(rows.as_mut_ptr() as *mut core::ffi::c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };
    if copied == 0 {
        bail!("GetDIBits failed while reading the pointer's bitmap back");
    }

    for row in 0..height as usize {
        let start = (top as usize + row) * stride + left as usize * 4;
        for column in 0..width as usize {
            let from = row * row_bytes + column * 4;
            let to = start + column * 4;
            frame[to] = rows[from];
            frame[to + 1] = rows[from + 1];
            frame[to + 2] = rows[from + 2];
            // The frame is the screen, which is opaque. A device-dependent bitmap
            // has no alpha of its own, and the pointer's mask must not punch a
            // hole in the frame where it was.
            frame[to + 3] = 0xFF;
        }
    }
    Ok(())
}

/// The header describing a 32-bit top-down DIB of this size.
fn top_down(width: u32, height: u32) -> BITMAPINFO {
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width as i32,
        // A negative height asks for top-down rows, which is what a frame is.
        biHeight: -(height as i32),
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    bmi
}

/// Ask for the pointer and draw it into a frame.
///
/// A pointer that cannot be read is reported and otherwise ignored: a capture
/// without a pointer is better than no capture at all.
pub fn compose(frame: &mut [u8], width: u32, height: u32, origin: (i32, i32)) {
    let cursor = match current() {
        Ok(Some(cursor)) => cursor,
        Ok(None) => {
            eprintln!("[cursor] the pointer is not being shown");
            return;
        }
        Err(err) => {
            eprintln!("[cursor] could not read the pointer: {err:#}");
            return;
        }
    };

    eprintln!(
        "[cursor] {}x{} at ({}, {})",
        cursor.width, cursor.height, cursor.position.0, cursor.position.1
    );
    match draw_into(
        frame,
        width,
        height,
        origin,
        cursor.shape,
        cursor.width,
        cursor.height,
        cursor.position,
    ) {
        Ok(true) => {}
        Ok(false) => eprintln!("[cursor] the pointer is outside this capture"),
        Err(err) => eprintln!("[cursor] could not draw the pointer: {err:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: u32 = 100;

    #[test]
    fn a_shape_inside_the_frame_lands_where_it_is_put() {
        let where_it_lands = landing(FRAME, FRAME, (0, 0), (10, 20), 16, 16).expect("inside");
        assert_eq!(where_it_lands, (10, 20, 16, 16));
    }

    #[test]
    fn a_shape_over_an_edge_is_clipped_rather_than_dropped() {
        // Over the top-left corner: the part inside the frame is kept.
        let corner = landing(FRAME, FRAME, (0, 0), (-8, -8), 16, 16).expect("corner");
        assert_eq!(corner, (0, 0, 8, 8));

        // Over the bottom-right corner.
        let far = landing(FRAME, FRAME, (0, 0), (95, 95), 16, 16).expect("far");
        assert_eq!(far, (95, 95, 5, 5));
    }

    #[test]
    fn a_shape_wholly_outside_the_frame_does_not_land() {
        assert!(
            landing(FRAME, FRAME, (0, 0), (-16, 0), 16, 16).is_none(),
            "left"
        );
        assert!(
            landing(FRAME, FRAME, (0, 0), (0, -16), 16, 16).is_none(),
            "above"
        );
        assert!(
            landing(FRAME, FRAME, (0, 0), (100, 0), 16, 16).is_none(),
            "right"
        );
        assert!(
            landing(FRAME, FRAME, (0, 0), (0, 100), 16, 16).is_none(),
            "below"
        );
    }

    #[test]
    fn landing_follows_the_frame_origin() {
        // A frame whose left edge is at x = 100 puts virtual x = 110 at pixel 10.
        let shifted = landing(FRAME, FRAME, (100, 50), (110, 60), 16, 16).expect("shifted");
        assert_eq!(shifted, (10, 10, 16, 16));

        // The same virtual position is outside a frame that does not cover it.
        assert!(landing(FRAME, FRAME, (0, 0), (110, 60), 16, 16).is_none());
    }

    #[test]
    fn landing_handles_a_negative_frame_origin() {
        // A virtual desktop whose primary monitor is not at the origin, which is
        // the ordinary case for a monitor arranged to the left or above.
        let corner = landing(50, 50, (-100, -50), (-100, -50), 16, 16).expect("corner");
        assert_eq!(corner, (0, 0, 16, 16));
    }

    #[test]
    fn the_hotspot_is_what_decides_where_a_shape_lands() {
        // The system reports the point the pointer is pointing at, and a shape's
        // own top-left is that point less its hotspot. Landing the shape at the
        // hotspot instead would put it one hotspot down and to the right.
        let pointer = (200, 300);
        let hotspot = (3, 5);
        let corner = (pointer.0 - hotspot.0, pointer.1 - hotspot.1);

        let landed = landing(400, 400, (0, 0), corner, 32, 32).expect("landed");
        assert_eq!(landed.0, 197, "left edge");
        assert_eq!(landed.1, 295, "top edge");
        // The point the pointer is pointing at is inside the shape, which is the
        // property that makes it point at what it was pointing at.
        assert!(landed.0 <= pointer.0 && pointer.0 < landed.0 + landed.2 as i32);
        assert!(landed.1 <= pointer.1 && pointer.1 < landed.1 + landed.3 as i32);
    }

    #[test]
    fn drawing_into_a_short_buffer_is_refused() {
        // A caller that hands over the wrong length is told so rather than having
        // a write past the end.
        let mut too_short = vec![0u8; 16];
        let result = draw_into(
            &mut too_short,
            10,
            10,
            (0, 0),
            HICON(std::ptr::null_mut()),
            8,
            8,
            (0, 0),
        );
        assert!(result.is_err(), "a short buffer is an error");
    }

    #[test]
    fn a_shape_outside_the_frame_is_not_an_error() {
        // A null handle is fine here: the position is answered before the handle
        // is looked at, which is what keeps "the pointer is on another monitor"
        // from being reported as a broken pointer.
        let mut canvas = vec![0u8; 10 * 10 * 4];
        let result = draw_into(
            &mut canvas,
            10,
            10,
            (0, 0),
            HICON(std::ptr::null_mut()),
            8,
            8,
            (50, 50),
        );
        assert_eq!(result.ok(), Some(false));
    }

    #[test]
    fn drawing_an_invalid_shape_is_refused() {
        let mut canvas = vec![0u8; 10 * 10 * 4];
        let result = draw_into(
            &mut canvas,
            10,
            10,
            (0, 0),
            HICON(std::ptr::null_mut()),
            8,
            8,
            (0, 0),
        );
        assert!(result.is_err(), "a null shape is an error");
    }
}
