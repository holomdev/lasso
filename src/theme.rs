//! Every colour the strip is drawn with, in one place.
//!
//! The values are written `0xRRGGBB`, which is how they were specified, and `colorref` turns them
//! into what GDI wants — `0x00BBGGRR`. Keeping the source in the readable order and converting at
//! one boundary is the whole point of this module: a colour written straight into a drawing call
//! is a colour that will be written twice and disagree with itself.
//!
//! Nothing here is used by the capture itself: this is the interface's palette, not the pixels.

use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, DeleteObject, HBRUSH, HGDIOBJ};

/// The bar's plate.
pub const BAR_BG: u32 = 0xF7F8FA;
/// The bar's hairline.
pub const BAR_BORDER: u32 = 0xDCDFE6;
/// The rules between groups.
pub const SEPARATOR: u32 = 0xE1E4EA;

/// An icon at rest.
pub const ICON: u32 = 0x374151;
/// An icon whose button cannot act.
pub const ICON_DISABLED: u32 = 0xB4BAC6;

/// A button under the pointer.
pub const HOVER_BG: u32 = 0xE9ECF2;
/// A button held down.
pub const PRESSED_BG: u32 = 0xDDE1EA;

/// The tool in force.
pub const ACTIVE_BG: u32 = 0xE3E8FF;
/// The tool in force, as an icon.
pub const ACTIVE_ICON: u32 = 0x3B4FD8;

/// Save, which is the one filled button.
pub const SAVE_BG: u32 = 0x4150E0;
pub const SAVE_BG_HOVER: u32 = 0x3340C4;
pub const SAVE_BG_PRESSED: u32 = 0x2A35A6;
pub const SAVE_ICON: u32 = 0xFFFFFF;

/// Cancel, which is the one red mark.
pub const CANCEL_ICON: u32 = 0xD93F45;
pub const CANCEL_HOVER_BG: u32 = 0xFDECEC;
pub const CANCEL_PRESSED_BG: u32 = 0xFAD5D7;

/// A palette that has opened.
pub const POPUP_BG: u32 = 0xFFFFFF;
pub const POPUP_BORDER: u32 = 0xDCDFE6;

/// A swatch's edge: black at about eighteen per cent. GDI brushes carry no alpha, so the value is
/// the mix over the plate, which is what a swatch always sits on.
pub const SWATCH_BORDER: u32 = mix(0x000000, BAR_BG, 46, 255);

/// The bar's shadow: a soft dark, spread over four rings. Likewise mixed down for the same reason,
/// with each ring lighter than the last.
pub const SHADOW: [u32; 4] = [
    mix(0x101828, BAR_BG, 26, 255),
    mix(0x101828, BAR_BG, 18, 255),
    mix(0x101828, BAR_BG, 12, 255),
    mix(0x101828, BAR_BG, 7, 255),
];

/// `0xRRGGBB` as GDI's `0x00BBGGRR`.
pub const fn colorref(rgb: u32) -> COLORREF {
    let red = (rgb >> 16) & 0xFF;
    let green = (rgb >> 8) & 0xFF;
    let blue = rgb & 0xFF;
    COLORREF((blue << 16) | (green << 8) | red)
}

/// `foreground` over `background`, `weight` parts out of `total`.
///
/// Used where the specification asks for a translucent colour and GDI can give only an opaque one:
/// the result is the colour that would have been seen.
pub const fn mix(foreground: u32, background: u32, weight: u32, total: u32) -> u32 {
    (channel(foreground, background, weight, total, 16) << 16)
        | (channel(foreground, background, weight, total, 8) << 8)
        | channel(foreground, background, weight, total, 0)
}

const fn channel(foreground: u32, background: u32, weight: u32, total: u32, shift: u32) -> u32 {
    let fg = (foreground >> shift) & 0xFF;
    let bg = (background >> shift) & 0xFF;
    (((fg * weight) + (bg * (total - weight))) / total) & 0xFF
}

/// The brushes the strip is drawn with, made once and released together.
///
/// One struct rather than a dozen fields on the overlay's state: a brush that is made in one place
/// and forgotten in another is how a GDI handle is lost, and this keeps the making and the
/// releasing in the same file.
pub struct Chrome {
    pub bar_bg: HBRUSH,
    pub bar_border: HBRUSH,
    pub separator: HBRUSH,
    pub hover: HBRUSH,
    pub pressed: HBRUSH,
    pub active: HBRUSH,
    pub save: HBRUSH,
    pub save_hover: HBRUSH,
    pub save_pressed: HBRUSH,
    pub cancel_hover: HBRUSH,
    pub cancel_pressed: HBRUSH,
    pub swatch_border: HBRUSH,
    pub popup: HBRUSH,
    pub popup_border: HBRUSH,
    /// The shadow's rings, outermost first.
    pub shadow: [HBRUSH; 4],
}

impl Chrome {
    pub fn new() -> Self {
        let brush = |rgb: u32| unsafe { CreateSolidBrush(colorref(rgb)) };
        Self {
            bar_bg: brush(BAR_BG),
            bar_border: brush(BAR_BORDER),
            separator: brush(SEPARATOR),
            hover: brush(HOVER_BG),
            pressed: brush(PRESSED_BG),
            active: brush(ACTIVE_BG),
            save: brush(SAVE_BG),
            save_hover: brush(SAVE_BG_HOVER),
            save_pressed: brush(SAVE_BG_PRESSED),
            cancel_hover: brush(CANCEL_HOVER_BG),
            cancel_pressed: brush(CANCEL_PRESSED_BG),
            swatch_border: brush(SWATCH_BORDER),
            popup: brush(POPUP_BG),
            popup_border: brush(POPUP_BORDER),
            shadow: [
                brush(SHADOW[0]),
                brush(SHADOW[1]),
                brush(SHADOW[2]),
                brush(SHADOW[3]),
            ],
        }
    }
}

impl Drop for Chrome {
    fn drop(&mut self) {
        for brush in [
            self.bar_bg,
            self.bar_border,
            self.separator,
            self.hover,
            self.pressed,
            self.active,
            self.save,
            self.save_hover,
            self.save_pressed,
            self.cancel_hover,
            self.cancel_pressed,
            self.swatch_border,
            self.popup,
            self.popup_border,
            self.shadow[0],
            self.shadow[1],
            self.shadow[2],
            self.shadow[3],
        ] {
            if !brush.is_invalid() {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(brush.0));
                }
            }
        }
    }
}
