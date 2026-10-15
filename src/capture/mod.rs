//! Screen capture.
//!
//! Two-tier design:
//!
//! * **Primary: DXGI Output Duplication.** One `IDXGIOutputDuplication` per
//!   output; every frame is composited onto a single canvas covering the whole
//!   virtual desktop.
//! * **Fallback: GDI `BitBlt`** from the screen DC. Slower, but it keeps
//!   working over RDP, in sessions without a usable GPU and with protected
//!   content, where duplication is unavailable.
//!
//! Everything is done in **physical pixels** with the process declared
//! per-monitor DPI aware, which is what allows a selection rectangle to span
//! monitors with different scale factors without drifting.

pub mod dxgi;
pub mod gdi;

use anyhow::{bail, Result};
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// `MONITORINFOF_PRIMARY`. The constant is not re-exported by the crate, so its
/// documented value is used instead.
const MONITORINFOF_PRIMARY: u32 = 0x1;

/// Which route produced a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Dxgi,
    Gdi,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Dxgi => "dxgi",
            Backend::Gdi => "gdi",
        }
    }
}

/// A captured frame: BGRA8, top-down rows, no padding.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    /// Frame origin in virtual-desktop coordinates.
    pub origin: (i32, i32),
    pub backend: Backend,
}

impl Frame {
    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }

    /// View as an `image::RgbaImage` so it can be written to PNG.
    /// BGRA -> RGBA by swapping channels on a copy.
    pub fn to_rgba_image(&self) -> image::RgbaImage {
        let mut buf = self.bgra.clone();
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2); // B <-> R
        }
        image::RgbaImage::from_raw(self.width, self.height, buf)
            .expect("buffer is sized by construction")
    }

    /// Copy a sub-rectangle of the frame as BGRA8 rows.
    ///
    /// Returns `None` for an empty rectangle or one that falls outside the
    /// frame, so callers cannot index past the capture.
    pub fn to_bgra_sub(&self, x: i32, y: i32, width: u32, height: u32) -> Option<Vec<u8>> {
        if x < 0 || y < 0 || width == 0 || height == 0 {
            return None;
        }
        if x as u32 + width > self.width || y as u32 + height > self.height {
            return None;
        }

        let row_bytes = width as usize * 4;
        let mut buf = Vec::with_capacity(row_bytes * height as usize);
        for row in 0..height as usize {
            let start = ((y as usize + row) * self.width as usize + x as usize) * 4;
            buf.extend_from_slice(&self.bgra[start..start + row_bytes]);
        }
        Some(buf)
    }

    /// The same sub-rectangle as an RGBA image, for writing a PNG.
    pub fn to_rgba_sub(&self, x: i32, y: i32, width: u32, height: u32) -> Option<image::RgbaImage> {
        let mut buf = self.to_bgra_sub(x, y, width, height)?;
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2); // B <-> R
        }
        image::RgbaImage::from_raw(width, height, buf)
    }
}

/// One monitor of the virtual desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    pub device: String,
    /// `(left, top, right, bottom)` in virtual-desktop coordinates.
    pub rect: (i32, i32, i32, i32),
    pub primary: bool,
}

impl MonitorInfo {
    pub fn width(&self) -> i32 {
        self.rect.2 - self.rect.0
    }
    pub fn height(&self) -> i32 {
        self.rect.3 - self.rect.1
    }
}

/// Virtual-desktop bounds in physical pixels.
/// `(x, y, width, height)`; `x`/`y` can be negative when a monitor sits left of
/// or above the primary one.
pub fn virtual_desktop() -> (i32, i32, u32, u32) {
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(0) as u32;
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(0) as u32;
        (x, y, w, h)
    }
}

unsafe extern "system" fn monitor_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let acc = unsafe { &mut *(data.0 as *mut Vec<MonitorInfo>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // MONITORINFOEXW starts with a MONITORINFO, so this cast is valid, and it is
    // what additionally yields the device name.
    let ok = unsafe {
        GetMonitorInfoW(
            hmonitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    };
    if ok.as_bool() {
        let end = info
            .szDevice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.szDevice.len());
        let r = info.monitorInfo.rcMonitor;
        acc.push(MonitorInfo {
            device: String::from_utf16_lossy(&info.szDevice[..end]),
            rect: (r.left, r.top, r.right, r.bottom),
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }
    TRUE
}

/// Enumerate monitors through `EnumDisplayMonitors`.
pub fn monitors() -> Result<Vec<MonitorInfo>> {
    let mut acc: Vec<MonitorInfo> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(monitor_proc),
            LPARAM(&mut acc as *mut Vec<MonitorInfo> as isize),
        );
    }
    if acc.is_empty() {
        bail!("EnumDisplayMonitors returned no monitor");
    }
    Ok(acc)
}

/// Capture the whole virtual desktop.
///
/// `prefer` forces one backend, which is useful for comparing and diagnosing.
/// With `None` it tries DXGI first and falls back to GDI, reporting why on
/// stderr.
pub fn capture_virtual_desktop(prefer: Option<Backend>) -> Result<Frame> {
    let (x, y, w, h) = virtual_desktop();
    if w == 0 || h == 0 {
        bail!("virtual desktop reports a zero size ({w}x{h})");
    }

    match prefer {
        Some(Backend::Gdi) => return gdi::capture(x, y, w, h),
        Some(Backend::Dxgi) => return dxgi::capture(x, y, w, h),
        None => {}
    }

    match dxgi::capture(x, y, w, h) {
        Ok(frame) => Ok(frame),
        Err(err) => {
            eprintln!("[capture] DXGI Output Duplication unavailable: {err:#}");
            eprintln!("[capture] falling back to GDI BitBlt");
            gdi::capture(x, y, w, h)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frame whose pixels encode their own coordinates, so a crop can be
    /// checked exactly instead of by eye.
    ///
    /// Stored BGRA holds `0x40` in blue, `y` in green and `x` in red, which
    /// after the channel swap must read back as R = x, G = y, B = 0x40.
    fn coordinate_frame(width: u32, height: u32) -> Frame {
        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                bgra.extend_from_slice(&[0x40, y as u8, x as u8, 0xFF]);
            }
        }
        Frame {
            width,
            height,
            bgra,
            origin: (0, 0),
            backend: Backend::Gdi,
        }
    }

    #[test]
    fn sub_image_matches_the_requested_rectangle() {
        let frame = coordinate_frame(6, 4);
        let image = frame
            .to_rgba_sub(2, 1, 3, 2)
            .expect("the region lies inside the frame");

        assert_eq!(image.width(), 3);
        assert_eq!(image.height(), 2);
        for row in 0..2u32 {
            for col in 0..3u32 {
                let px = image.get_pixel(col, row).0;
                assert_eq!(px[0], (2 + col) as u8, "red at ({col}, {row})");
                assert_eq!(px[1], (1 + row) as u8, "green at ({col}, {row})");
                assert_eq!(px[2], 0x40, "blue at ({col}, {row})");
                assert_eq!(px[3], 0xFF, "alpha at ({col}, {row})");
            }
        }
    }

    #[test]
    fn sub_image_rejects_regions_outside_the_frame() {
        let frame = coordinate_frame(4, 4);

        assert!(frame.to_rgba_sub(-1, 0, 1, 1).is_none(), "negative x");
        assert!(frame.to_rgba_sub(0, -1, 1, 1).is_none(), "negative y");
        assert!(frame.to_rgba_sub(0, 0, 0, 1).is_none(), "zero width");
        assert!(frame.to_rgba_sub(0, 0, 1, 0).is_none(), "zero height");
        assert!(frame.to_rgba_sub(3, 0, 2, 1).is_none(), "past the right edge");
        assert!(frame.to_rgba_sub(0, 3, 1, 2).is_none(), "past the bottom edge");
        assert!(frame.to_rgba_sub(3, 3, 1, 1).is_some(), "last pixel is valid");
    }

    #[test]
    fn whole_frame_swaps_bgra_to_rgba() {
        let frame = coordinate_frame(3, 2);
        let image = frame.to_rgba_image();
        let px = image.get_pixel(2, 1).0;
        assert_eq!([px[0], px[1], px[2], px[3]], [2, 1, 0x40, 0xFF]);
    }
}
