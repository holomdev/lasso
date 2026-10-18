//! Drawing the annotations and the strip.
//!
//! Kept apart from the overlay's own painting so that it can be checked by drawing into
//! a canvas and reading the pixels back, which is the only way to know that a shape ends
//! up where it was asked for. Nothing here decides *what* to draw: it is handed a stroke
//! and a device context and puts it there.
//!
//! # Coordinates
//!
//! A stroke's points are relative to the selection, and the canvas's device context is
//! in canvas coordinates, so drawing adds the selection's origin. That conversion
//! happens here and only here; the overlay works in screen coordinates and the model in
//! relative ones, and neither adds an origin of its own.
//!
//! Everything is clamped to the selection before it is drawn. The model already clips,
//! so this should never have anything to do — which is the point: a shape that escaped
//! the region would be drawn over the dimmed area outside it, and that is a fault worth
//! a clamp rather than a hope.

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BS_SOLID, CLIP_DEFAULT_PRECIS, CreateFontW, CreatePen, CreateSolidBrush,
    BACKGROUND_MODE, DEFAULT_CHARSET, DEFAULT_PITCH, DeleteObject, Ellipse, ExtCreatePen, ExtTextOutW,
    ETO_OPTIONS, FF_DONTCARE, FW_NORMAL, FrameRect, GetStockObject, GetTextExtentPoint32W,
    GetTextFaceW, HDC, HFONT, HGDIOBJ, HPEN, LineTo, LOGBRUSH, MoveToEx, NULL_BRUSH, NULL_PEN,
    OUT_TT_PRECIS, PEN_STYLE, PS_ENDCAP_ROUND, PS_GEOMETRIC, PS_JOIN_ROUND, PS_SOLID, Polyline,
    RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};

use super::annotation::{Point, Shape, Stroke};
use windows::Win32::Foundation::SIZE;
use super::toolbar::{Button, Metrics, Placement};

/// The narrowest a stroke is allowed to be drawn, so a set thickness of zero cannot
/// produce a shape that is invisible and looks like nothing happened.
const MIN_THICKNESS: i32 = 1;

/// Draw one stroke into a device context, in canvas coordinates.
///
/// Nothing is clamped to the selection. A mark may be drawn anywhere on the screen and is kept
/// where it was drawn; what is inside the selection is what gets saved, and that is decided when
/// the capture is cropped, not by refusing to draw. Clamping here is what made a stroke stop at
/// the region's edge instead of running past it.
pub fn draw_stroke(hdc: HDC, stroke: &Stroke, origin: (i32, i32)) {
    // A point in the stroke's own space becomes a point on the canvas.
    let to_canvas = |point: Point| -> (i32, i32) { (point.x + origin.0, point.y + origin.1) };

    let colour = stroke.colour.to_colorref();
    let thickness = (stroke.thickness as i32).max(MIN_THICKNESS);
    let pen = unsafe { CreatePen(PS_SOLID, thickness, colour) };
    if pen.is_invalid() {
        return;
    }
    let saved = unsafe { SelectObject(hdc, HGDIOBJ(pen.0)) };

    match &stroke.shape {
        Shape::Arrow { from, to } => {
            let from = to_canvas(*from);
            let to = to_canvas(*to);
            draw_arrow(hdc, from, to, thickness);
        }
        Shape::Rectangle { from, to } => {
            // Open, not filled: the point of a box on a screenshot is to mark something, and
            // filling it would hide exactly what is being marked.
            //
            // Four bars rather than `FrameRect`, which always draws a one-pixel frame with a
            // brush and so ignored the width while every other shape honoured it. And rather
            // than a pen, because a pen is centred on the path: a thick one would reach half
            // its width outside the box, and staying inside the box the two points describe is
            // the one property a marking box has to keep.
            //
            // The brush is never selected into the context — `FillRect` is given it directly —
            // so it can be deleted here.
            let (left, top, right, bottom) = normalized(to_canvas(*from), to_canvas(*to));
            let width = (right - left + 1).max(1);
            let height = (bottom - top + 1).max(1);
            // A box smaller than the stroke is all stroke, and that is what a thick outline
            // looks like: no bar may be wider than the box it sits in.
            let bar = thickness.min(width).min(height);

            let brush = unsafe { CreateSolidBrush(colour) };
            if !brush.is_invalid() {
                let bars = [
                    // Top and bottom, full width.
                    RECT {
                        left,
                        top,
                        right: right + 1,
                        bottom: top + bar,
                    },
                    RECT {
                        left,
                        top: bottom + 1 - bar,
                        right: right + 1,
                        bottom: bottom + 1,
                    },
                    // Left and right, between them.
                    RECT {
                        left,
                        top,
                        right: left + bar,
                        bottom: bottom + 1,
                    },
                    RECT {
                        left: right + 1 - bar,
                        top,
                        right: right + 1,
                        bottom: bottom + 1,
                    },
                ];
                for side in bars {
                    if side.right > side.left && side.bottom > side.top {
                        unsafe {
                            windows::Win32::Graphics::Gdi::FillRect(hdc, &side, brush);
                        }
                    }
                }
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(brush.0));
                }
            }
        }
        Shape::Ellipse { from, to } => {
            let (left, top, right, bottom) = normalized(to_canvas(*from), to_canvas(*to));
            // `Ellipse` fills its interior with whatever brush the context holds, so a
            // hollow one is selected for the duration. Without this the ellipse would be a
            // solid disc covering what it was drawn around.
            let hollow = unsafe { GetStockObject(NULL_BRUSH) };
            let previous = unsafe { SelectObject(hdc, hollow) };
            unsafe {
                let _ = Ellipse(hdc, left, top, right + 1, bottom + 1);
                SelectObject(hdc, previous);
            }
        }
        Shape::Pencil { points } => {
            if points.len() == 1 {
                // A press with no movement still leaves a mark, which is what makes a dot
                // possible. Filled with a brush rather than drawn as a one-pixel
                // rectangle, because `Rectangle` uses whatever brush the context holds
                // and the ellipse branch leaves a hollow one behind.
                let (x, y) = to_canvas(points[0]);
                let dot = RECT {
                    left: x,
                    top: y,
                    right: x + thickness,
                    bottom: y + thickness,
                };
                let brush = unsafe { CreateSolidBrush(colour) };
                if !brush.is_invalid() {
                    unsafe {
                        windows::Win32::Graphics::Gdi::FillRect(hdc, &dot, brush);
                        let _ = DeleteObject(HGDIOBJ(brush.0));
                    }
                }
            } else if points.len() > 1 {
                let flat: Vec<windows::Win32::Foundation::POINT> = points
                    .iter()
                    .map(|p| {
                        let (x, y) = to_canvas(*p);
                        windows::Win32::Foundation::POINT { x, y }
                    })
                    .collect();
                unsafe {
                    let _ = Polyline(hdc, &flat);
                }
            }
        }
    }

    unsafe {
        SelectObject(hdc, saved);
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
}

/// Two corners as a rectangle with `left <= right` and `top <= bottom`.
///
/// A drag can start at any corner, so the two points arrive in any order. GDI's
/// `Ellipse` and `Rectangle` will not draw a mirrored rectangle, so this is not
/// cosmetic: without it, half the directions a person drags in draw nothing.
fn normalized(from: (i32, i32), to: (i32, i32)) -> (i32, i32, i32, i32) {
    (
        from.0.min(to.0),
        from.1.min(to.1),
        from.0.max(to.0),
        from.1.max(to.1),
    )
}

/// A line from `from` to `to` with a head at `to`.
///
/// The head is two short lines rather than a filled triangle, so that it stays legible
/// at a thickness of one, where a triangle would be a dot.
fn draw_arrow(hdc: HDC, from: (i32, i32), to: (i32, i32), thickness: i32) {
    unsafe {
        let _ = MoveToEx(hdc, from.0, from.1, None);
        let _ = LineTo(hdc, to.0, to.1);
    }

    let dx = (to.0 - from.0) as f64;
    let dy = (to.1 - from.1) as f64;
    let length = (dx * dx + dy * dy).sqrt();
    if length < 1.0 {
        // Pressed and released on the same pixel: a line has no direction, so there is
        // no head to draw. The shaft above is still the mark.
        return;
    }

    // Long enough to read, and scaled with the stroke so that a thick arrow does not
    // get a head that vanishes into it.
    let head = (8.0 + thickness as f64 * 1.5).min(length);
    let angle = std::f64::consts::PI / 7.0; // about 26 degrees off the shaft
    let base = dy.atan2(dx);

    let wing = |sign: f64| -> windows::Win32::Foundation::POINT {
        let a = base + std::f64::consts::PI + sign * angle;
        windows::Win32::Foundation::POINT {
            x: to.0 + (head * a.cos()).round() as i32,
            y: to.1 + (head * a.sin()).round() as i32,
        }
    };
    let points = [wing(1.0), windows::Win32::Foundation::POINT { x: to.0, y: to.1 }, wing(-1.0)];
    unsafe {
        let _ = Polyline(hdc, &points);
    }
}

/// Draw a palette, and the ring that says which choice is in force.
///
/// The cells come from `Palette::swatches`, the same function the hit test reads, so a choice
/// cannot be drawn in one place and picked up in another. Written without early returns, for
/// the reason the strip's glyphs are: this runs on every repaint, and anything created here has
/// to be released.
pub fn draw_palette(
    hdc: HDC,
    palette: &super::toolbar::Palette,
    metrics: &Metrics,
    state: &StripState,
    chrome: &crate::theme::Chrome,
) {
    use super::annotation::{thickness_index, Colour, THICKNESS_STEPS};
    use super::toolbar::PaletteKind;
    use crate::theme as colours;

    let body = RECT {
        left: palette.x,
        top: palette.y,
        right: palette.x + palette.width as i32,
        bottom: palette.y + palette.height as i32,
    };

    // The same card as the strip, with the same shadow, and the same square corners: a popup that
    // floats on its own looks like it came from somewhere else.
    if let Some(source) = ShadowSource::new() {
        soft_shadow(hdc, &body, 0, &source);
    }
    let border = unsafe { CreateSolidBrush(colours::colorref(colours::POPUP_BORDER)) };
    unsafe {
        windows::Win32::Graphics::Gdi::FillRect(hdc, &body, chrome.popup);
        if !border.is_invalid() {
            FrameRect(hdc, &body, border);
            let _ = DeleteObject(HGDIOBJ(border.0));
        }
    }

    // Which cell is the one in force. A width between two steps marks the nearer one, which is
    // where `+` and `-` would take it next.
    let selected = match palette.kind {
        PaletteKind::Colour => Colour::ALL
            .iter()
            .position(|colour| *colour == state.colour)
            .unwrap_or(0),
        PaletteKind::Thickness => thickness_index(state.thickness),
    };

    let rim = unsafe { CreateSolidBrush(colours::colorref(colours::SWATCH_BORDER)) };
    for (index, square) in palette.swatches(metrics).into_iter().enumerate() {
        // The cell in force is filled, so the palette says which one is in use before the ring is
        // even looked at.
        if index == selected {
            unsafe {
                windows::Win32::Graphics::Gdi::FillRect(hdc, &square, chrome.active);
            }
        }
        match palette.kind {
            PaletteKind::Colour => {
                let colour = Colour::ALL[index];
                let paint = unsafe { CreateSolidBrush(colour.to_colorref()) };
                if !paint.is_invalid() {
                    unsafe {
                        windows::Win32::Graphics::Gdi::FillRect(hdc, &square, paint);
                    }
                    if !rim.is_invalid() {
                        // A hairline inside the swatch, so the colour keeps its size and white and
                        // yellow still have an edge against a white popup.
                        unsafe {
                            FrameRect(hdc, &square, rim);
                        }
                    }
                    unsafe {
                        let _ = DeleteObject(HGDIOBJ(paint.0));
                    }
                }
            }
            PaletteKind::Thickness => {
                // A bar of that width, as the button shows it: a row of bars is a row of widths,
                // and a row of discs is a row of dots that happen to differ.
                let thickness = THICKNESS_STEPS[index];
                let cell = (square.right - square.left).min(square.bottom - square.top);
                let wide = (thickness as i32).clamp(2, (cell - 6).max(2));
                let centre_x = (square.left + square.right) / 2;
                let bar = RECT {
                    left: centre_x - wide / 2,
                    top: square.top + 4,
                    right: centre_x - wide / 2 + wide,
                    bottom: square.bottom - 4,
                };
                let dot = unsafe { CreateSolidBrush(colours::colorref(colours::ICON)) };
                if !dot.is_invalid() {
                    unsafe {
                        windows::Win32::Graphics::Gdi::FillRect(hdc, &bar, dot);
                    }
                    unsafe {
                        let _ = DeleteObject(HGDIOBJ(dot.0));
                    }
                }
            }
        }
        // The choice in force, ringed two pixels clear of the swatch so the ring does not change
        // the colour it is pointing at.
        if index == selected {
            let ring = unsafe { CreateSolidBrush(colours::colorref(colours::SAVE_BG)) };
            if !ring.is_invalid() {
                let marker = RECT {
                    left: square.left - RING_OFFSET,
                    top: square.top - RING_OFFSET,
                    right: square.right + RING_OFFSET,
                    bottom: square.bottom + RING_OFFSET,
                };
                unsafe {
                    FrameRect(hdc, &marker, ring);
                    let _ = DeleteObject(HGDIOBJ(ring.0));
                }
            }
        }
    }
    unsafe {
        if !rim.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(rim.0));
        }
        if !border.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(border.0));
        }
    }
}


/// The face Windows draws its own interface with. It ships with the system, so nothing is bundled
/// and nothing has to be licensed: the glyphs are the operating system's.
const ICON_FACE: windows::core::PCWSTR = windows::core::w!("Segoe MDL2 Assets");

/// The glyph a button takes from that font, or `None` for the two that show a value instead.
///
/// Colour and Thickness are not icons: one is a disc of the colour in force and the other a bar of
/// the width in force, and replacing either with a fixed mark would leave the button unable to
/// answer the only question it is asked.
fn icon_glyph(button: Button) -> Option<u16> {
    Some(match button {
        Button::Tool(super::annotation::Tool::Move) => 0xE740,
        Button::Tool(super::annotation::Tool::Arrow) => 0xE741,
        Button::Tool(super::annotation::Tool::Rectangle) => 0xE739,
        Button::Tool(super::annotation::Tool::Ellipse) => 0xEA3A,
        Button::Tool(super::annotation::Tool::Pencil) => 0xE70F,
        Button::Undo => 0xE7A7,
        Button::Redo => 0xE7A6,
        Button::Save => 0xE74E,
        Button::Copy => 0xE8C8,
        Button::Cancel => 0xE711,
        Button::Colour | Button::Thickness => return None,
    })
}

/// The font, selected into the context, or `None` when the system does not have it.
///
/// `CreateFontW` does not fail for a missing family: it asks the mapper for a substitute and hands
/// back a font that is not the one asked for. So the check is the face that actually ended up
/// selected, read back with `GetTextFaceW`, rather than the handle being valid.
fn select_icon_font(hdc: HDC, side: i32) -> Option<(HFONT, HGDIOBJ)> {
    // Phosphor proportion is not in question here: MDL2 is drawn for a sixteen-pixel box, so the
    // icon is about half the button and the rest is the padding that makes a strip look calm.
    let height = -((side * 20 / 36).max(8));
    let font = unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            DEFAULT_PITCH.0 as u32 | FF_DONTCARE.0 as u32,
            ICON_FACE,
        )
    };
    if font.is_invalid() {
        return None;
    }
    let previous = unsafe { SelectObject(hdc, HGDIOBJ(font.0)) };
    let mut face = [0u16; 64];
    let _ = unsafe { GetTextFaceW(hdc, Some(&mut face)) };
    // `GetTextFaceW` counts the terminator in its answer, so a slice of that length carries a
    // trailing zero and compares unequal to the name it looks like. Cut at the first zero instead.
    let end = face.iter().position(|unit| *unit == 0).unwrap_or(face.len());
    let actual = String::from_utf16_lossy(&face[..end]);
    if actual != "Segoe MDL2 Assets" {
        // Said out loud: the fallback is invisible, and a strip drawn in the wrong icon family is
        // a difference nobody would think to look for.
        eprintln!("[strip] the icon font is missing, drawing the fallback: got {actual:?}");
        unsafe {
            SelectObject(hdc, previous);
            let _ = DeleteObject(HGDIOBJ(font.0));
        }
        return None;
    }
    Some((font, previous))
}

/// One glyph, centred on a point in the ink colour.
///
/// Grayscale antialiasing rather than ClearType: subpixel rendering puts coloured fringes on the
/// strokes, which is the last thing a capture tool should be drawing.
fn draw_icon_glyph(hdc: HDC, glyph: u16, centre: (i32, i32), ink: COLORREF) {
    // The glyph as a UTF-16 string of one unit, with a terminator so it can be handed over as a
    // pointer, and the count given separately.
    let text = [glyph, 0];
    let mut size = SIZE::default();
    let saved_colour = unsafe { SetTextColor(hdc, ink) };
    let saved_mode = unsafe { SetBkMode(hdc, TRANSPARENT) };
    unsafe {
        let _ = GetTextExtentPoint32W(hdc, &text[..1], &mut size);
        let _ = ExtTextOutW(
            hdc,
            centre.0 - size.cx / 2,
            centre.1 - size.cy / 2,
            ETO_OPTIONS(0),
            None,
            windows::core::PCWSTR(text.as_ptr()),
            1,
            None,
        );
        SetBkMode(hdc, BACKGROUND_MODE(saved_mode as u32));
        SetTextColor(hdc, saved_colour);
    }
}

/// A pen with round ends and round joins, which is what a Phosphor icon is drawn with.
///
/// A plain `CreatePen` makes square ends, and every stroke then has a chopped-off tip; the round
/// cap is most of what makes these icons look like themselves at eighteen pixels.
fn geometric_pen(width: i32, colour: COLORREF) -> HPEN {
    let brush = LOGBRUSH {
        lbStyle: BS_SOLID,
        lbColor: colour,
        lbHatch: 0,
    };
    unsafe {
        // The style is a `PEN_STYLE`, not the bare flags, and a solid line carries no dash pattern.
        ExtCreatePen(
            PEN_STYLE(PS_GEOMETRIC.0 | PS_SOLID.0 | PS_ENDCAP_ROUND.0 | PS_JOIN_ROUND.0),
            width.max(1) as u32,
            &brush,
            None,
        )
    }
}

/// A rounded rectangle, filled and optionally outlined.
///
/// `RoundRect` takes the corner *diameter*, so the radius is doubled here rather than at every call.
/// Both objects are put back, because the context outlives this call — and with no outline asked
/// for, the outline pen is the null one rather than a missing argument, so a button at rest draws
/// its background and no edge.
fn rounded(
    hdc: HDC,
    rect: &RECT,
    radius: i32,
    fill: Option<windows::Win32::Graphics::Gdi::HBRUSH>,
    pen: Option<HPEN>,
) {
    let brush = match fill {
        Some(brush) => HGDIOBJ(brush.0),
        None => unsafe { GetStockObject(NULL_BRUSH) },
    };
    let stroke = match pen {
        Some(pen) => HGDIOBJ(pen.0),
        None => unsafe { GetStockObject(NULL_PEN) },
    };
    let saved_brush = unsafe { SelectObject(hdc, brush) };
    let saved_pen = unsafe { SelectObject(hdc, stroke) };
    unsafe {
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius * 2,
            radius * 2,
        );
        SelectObject(hdc, saved_pen);
        SelectObject(hdc, saved_brush);
    }
}

/// A single black pixel, for `AlphaBlend` to stretch across a clip.
///
/// GDI has no blur, so the bar's shadow is a few rounded rings, each wider and fainter than the
/// last, blended over what is behind them. The source is this one pixel: blend it, do not paint
/// grey over a screenshot, or the shadow will look like a halo on a photograph.
struct ShadowSource {
    dc: HDC,
    previous: HGDIOBJ,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
}

impl ShadowSource {
    fn new() -> Option<Self> {
        let black = [0u8, 0, 0, 0];
        let bitmap = unsafe {
            windows::Win32::Graphics::Gdi::CreateBitmap(1, 1, 1, 32, Some(black.as_ptr() as *const _))
        };
        if bitmap.is_invalid() {
            return None;
        }
        let dc = unsafe { windows::Win32::Graphics::Gdi::CreateCompatibleDC(None) };
        if dc.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
            return None;
        }
        let previous = unsafe { SelectObject(dc, HGDIOBJ(bitmap.0)) };
        Some(Self {
            dc,
            previous,
            bitmap,
        })
    }
}

impl Drop for ShadowSource {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = windows::Win32::Graphics::Gdi::DeleteDC(self.dc);
        }
    }
}

/// Draw the bar's shadow under `rect`: four rings, outward and fainter, offset four pixels down.
fn soft_shadow(hdc: HDC, rect: &RECT, radius: i32, source: &ShadowSource) {
    use windows::Win32::Graphics::Gdi::{
        AC_SRC_OVER, AlphaBlend, BLENDFUNCTION, CombineRgn, CreateRoundRectRgn, RGN_DIFF,
        SelectClipRgn,
    };

    // A radius of zero is a rectangle: `CreateRoundRectRgn` takes the corner diameter, and GDI
    // treats a zero one as square. That is how the card and its shadow keep the same shape.
    //
    // Outermost first, so an inner ring lands on top of the fainter one outside it.
    //
    // The alphas are small because the rings *stack*: a pixel near the card is covered by all four,
    // and four passes at thirty per cent leave fifty-eight per cent of black, which is a bruise
    // rather than a shadow. These four compound to about sixteen per cent at the card and taper to
    // eight at the outside.
    for (spread, alpha) in [(5, 8u8), (4, 10), (3, 12), (2, 14)] {
        let outer = RECT {
            left: rect.left - spread,
            top: rect.top - spread + 4,
            right: rect.right + spread,
            bottom: rect.bottom + spread + 4,
        };
        // The hole is the bar itself, not a ring inside it: left one pixel larger, and every ring
        // stops short of the border and leaves a clean line of background between shadow and card,
        // which reads as no shadow at all.
        let inner = RECT {
            left: rect.left,
            top: rect.top + 4,
            right: rect.right,
            bottom: rect.bottom + 4,
        };
        unsafe {
            let ring = CreateRoundRectRgn(
                outer.left,
                outer.top,
                outer.right,
                outer.bottom,
                (radius + spread) * 2,
                (radius + spread) * 2,
            );
            let hole = CreateRoundRectRgn(
                inner.left,
                inner.top,
                inner.right,
                inner.bottom,
                radius * 2,
                radius * 2,
            );
            if ring.is_invalid() || hole.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(ring.0));
                let _ = DeleteObject(HGDIOBJ(hole.0));
                continue;
            }
            // Only the ring shows: the hole is subtracted so the middle is left alone until the
            // card itself is drawn over it.
            let _ = CombineRgn(Some(ring), Some(ring), Some(hole), RGN_DIFF);
            let _ = DeleteObject(HGDIOBJ(hole.0));

            let saved = windows::Win32::Graphics::Gdi::SaveDC(hdc);
            SelectClipRgn(hdc, Some(ring));
            let _ = AlphaBlend(
                hdc,
                outer.left,
                outer.top,
                outer.right - outer.left,
                outer.bottom - outer.top,
                source.dc,
                0,
                0,
                1,
                1,
                BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: alpha,
                    AlphaFormat: 0,
                },
            );
            if saved != 0 {
                let _ = windows::Win32::Graphics::Gdi::RestoreDC(hdc, saved);
            }
            let _ = DeleteObject(HGDIOBJ(ring.0));
        }
    }
}


/// How far clear of a swatch the ring that marks the choice sits.
const RING_OFFSET: i32 = 2;

/// The rules between the strip's groups: twenty-two pixels, centred.
const RULE_WIDTH: i32 = 22;

pub fn draw_strip(
    hdc: HDC,
    placement: &Placement,
    metrics: &Metrics,
    state: &StripState,
    chrome: &crate::theme::Chrome,
) {
    let body = RECT {
        left: placement.x,
        top: placement.y,
        right: placement.x + placement.width as i32,
        bottom: placement.y + placement.height as i32,
    };

    // The shadow, then the card, then everything else.
    //
    // The card's corners are square. Rounded ones softened the strip's own edge and, at this size,
    // the curve read as a smear on the corner of a bar that is otherwise a straight line: the
    // buttons are the things with corners worth drawing, and a rectangle is sharper than a radius.
    if let Some(source) = ShadowSource::new() {
        soft_shadow(hdc, &body, 0, &source);
    }
    let border_pen = unsafe { CreatePen(PS_SOLID, 1, crate::theme::colorref(crate::theme::BAR_BORDER)) };
    // A frame rather than four bars: `FrameRect` is one hairline and cannot leave a gap at a
    // corner.
    let edge = if border_pen.is_invalid() {
        None
    } else {
        unsafe { Some(CreateSolidBrush(crate::theme::colorref(crate::theme::BAR_BORDER))) }
            .filter(|brush| !brush.is_invalid())
    };
    unsafe {
        windows::Win32::Graphics::Gdi::FillRect(hdc, &body, chrome.bar_bg);
        if let Some(edge) = edge {
            FrameRect(hdc, &body, edge);
            let _ = DeleteObject(HGDIOBJ(edge.0));
        }
    }

    // The rules between groups: one pixel, short enough to read as a separator rather than as a cut.
    for y in metrics.separators() {
        let rule = RECT {
            left: body.left + (body.right - body.left) / 2 - RULE_WIDTH / 2,
            top: body.top + y,
            right: body.left + (body.right - body.left) / 2 + (RULE_WIDTH + 1) / 2,
            bottom: body.top + y + 1,
        };
        unsafe {
            windows::Win32::Graphics::Gdi::FillRect(hdc, &rule, chrome.separator);
        }
    }

    // One font for the whole strip, not one per glyph: twelve creations and deletions a repaint is
    // how the GDI handles ran out once already.
    let icon_font = select_icon_font(hdc, metrics.button);

    for (button, x, y, width, height) in metrics.button_rects() {
        let rect = RECT {
            left: placement.x + x,
            top: placement.y + y,
            right: placement.x + x + width as i32,
            bottom: placement.y + y + height as i32,
        };

        // Every button has a container: a filled one where the state gives it a colour, and
        // otherwise a hairline of its own. Square corners throughout — a radius on this many
        // rectangles at this size reads as a wobble rather than as a curve.
        match background_for(button, state, chrome) {
            Some(brush) => unsafe {
                windows::Win32::Graphics::Gdi::FillRect(hdc, &rect, brush);
            },
            None => unsafe {
                FrameRect(hdc, &rect, chrome.button_border);
            },
        }

        draw_button_glyph(
            hdc,
            button,
            &rect,
            ink_for(button, state),
            state,
            icon_font.map(|(font, _)| font),
        );
    }

    unsafe {
        if let Some((font, previous)) = icon_font {
            SelectObject(hdc, previous);
            let _ = DeleteObject(HGDIOBJ(font.0));
        }
        if !border_pen.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(border_pen.0));
        }
    }
}

/// The background a button draws, if any.
///
/// One place for the order of precedence, because the same question asked in two places is the
/// question that gets two answers: pressed beats hover beats the tool in force, Save is filled in
/// every state, and Cancel only shows itself when it is being used.
fn background_for(
    button: Button,
    state: &StripState,
    chrome: &crate::theme::Chrome,
) -> Option<windows::Win32::Graphics::Gdi::HBRUSH> {
    if state.is_disabled(button) {
        return None;
    }
    if button == Button::Save {
        return Some(match state.pressed {
            Some(pressed) if pressed == button => chrome.save_pressed,
            _ if state.hovered == Some(button) => chrome.save_hover,
            _ => chrome.save,
        });
    }
    if button == Button::Cancel {
        return match state.pressed {
            Some(pressed) if pressed == button => Some(chrome.cancel_pressed),
            _ if state.hovered == Some(button) => Some(chrome.cancel_hover),
            _ => None,
        };
    }
    if state.pressed == Some(button) {
        return Some(chrome.pressed);
    }
    if state.hovered == Some(button) {
        return Some(chrome.hover);
    }
    if state.armed == Some(button) {
        return Some(chrome.active);
    }
    None
}

/// The colour a button's mark is drawn in.
fn ink_for(button: Button, state: &StripState) -> COLORREF {
    use crate::theme as colours;
    if state.is_disabled(button) {
        return colours::colorref(colours::ICON_DISABLED);
    }
    match button {
        Button::Save => colours::colorref(colours::SAVE_ICON),
        Button::Cancel => colours::colorref(colours::CANCEL_ICON),
        _ if state.armed == Some(button) => colours::colorref(colours::ACTIVE_ICON),
        _ => colours::colorref(colours::ICON),
    }
}

/// What the strip should show as being in a particular state.
///
/// Passed as one value rather than as separate arguments because the drawing needs to
/// know which button is armed, which is under the pointer, and which two are unavailable —
/// and three positional arguments of which two are booleans is how they end up swapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StripState {
    /// The tool in force, which is the one button drawn as chosen.
    ///
    /// One button only: hover is not a choice, and a strip that lights two buttons at once makes
    /// the person look twice to find out which one they are painting with.
    pub armed: Option<Button>,
    /// The button the pointer is over, if any.
    pub hovered: Option<Button>,
    /// The button being held down, if any.
    pub pressed: Option<Button>,
    /// Whether there is anything for undo to take back.
    pub can_undo: bool,
    /// Whether there is anything for redo to put back.
    pub can_redo: bool,
    /// The colour a stroke would be drawn in, which the colour button shows.
    ///
    /// The button is the only place the colour appears: without it the strip says nothing
    /// about what will be drawn, and the only way to find out is to draw something.
    pub colour: crate::overlay::annotation::Colour,
    /// The width a stroke would be drawn at, which the thickness button shows as a dot.
    pub thickness: u32,
}

impl StripState {
    /// Whether a button is drawn as unavailable.
    ///
    /// Undo and redo are the only two that can be, and they are worth showing: a person
    /// who has just drawn something looks for undo, and seeing it greyed out before
    /// anything is drawn says where it will be.
    pub fn is_disabled(&self, button: Button) -> bool {
        match button {
            Button::Undo => !self.can_undo,
            Button::Redo => !self.can_redo,
            _ => false,
        }
    }
}

/// The GDI objects this process currently holds, and the ceiling it is allowed.
///
/// A strip whose plate and separators are drawn and whose glyphs are not is what running
/// out of them looks like: `FillRect` creates nothing, while every glyph needs a pen and
/// possibly a brush. Printing both numbers next to a failure is what turns "the icons
/// went" into a figure.
fn user_objects() -> u32 {
    use windows::Win32::System::Threading::GetCurrentProcess;
    use windows::Win32::System::Threading::{GetGuiResources, GR_GDIOBJECTS};
    unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) }
}

/// A button's icon, drawn from lines so that it stays crisp at any scale.
///
/// `ink` colours both the lines and, where a shape needs it, the fill. A button that
/// cannot be used is drawn in a colour close to the plate: present, and visibly not
/// available.
fn draw_button_glyph(
    hdc: HDC,
    button: Button,
    rect: &RECT,
    ink: COLORREF,
    state: &StripState,
    font: Option<HFONT>,
) {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let centre = (rect.left + width / 2, rect.top + height / 2);
    let side = width.min(height);

    // Phosphor draws for a twenty-four pixel box with a two-pixel stroke, and its marks reach nine
    // pixels from the centre. Both are derived from the button's side, so the weight and the
    // proportion survive a strip drawn at any scale.
    let stroke = (side * 2 / 24).max(2);
    let arm = (side * 9 / 24).max(4);
    // A chevron's legs are shorter than the arm they cap: at half the arm the four of them met
    // in the middle and the mark read as a diamond with a cross through it.
    let head = (arm / 3).max(2);

    // The system's own icon, when there is one for this button. Everything below is the fallback,
    // and it is kept because a machine without the font should still show an icon rather than a
    // blank button.
    if let (Some(_), Some(glyph)) = (font, icon_glyph(button)) {
        draw_icon_glyph(hdc, glyph, centre, ink);
        return;
    }

    let pen = geometric_pen(stroke, ink);
    if pen.is_invalid() {
        // A pen that cannot be created is a glyph that vanishes while the plate and the rules —
        // which create nothing — carry on looking right. That is the shape of the fault that was
        // reported once, so it says so out loud instead of returning quietly.
        eprintln!(
            "[strip] the pen for {button:?} could not be created: {} GDI objects held",
            user_objects()
        );
        return;
    }
    let saved_pen = unsafe { SelectObject(hdc, HGDIOBJ(pen.0)) };

    let line = |from: (i32, i32), to: (i32, i32)| unsafe {
        let _ = MoveToEx(hdc, from.0, from.1, None);
        let _ = LineTo(hdc, to.0, to.1);
    };
    // Every straight mark is a line with round ends; `rounded` covers the two glyphs that are
    // outlines. Nothing here fills, so no brush is selected and nothing has to be put back.
    let hollow = |rect: &RECT, radius: i32| rounded(hdc, rect, radius, None, Some(pen));

    match button {
        Button::Tool(super::annotation::Tool::Move) => {
            // `arrows-out-cardinal`: a cross with a chevron at each of its four ends.
            line((centre.0 - arm, centre.1), (centre.0 + arm, centre.1));
            line((centre.0, centre.1 - arm), (centre.0, centre.1 + arm));
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let tip = (centre.0 + dx * arm, centre.1 + dy * arm);
                // The two strokes of the chevron point back along the arm they cap.
                line(tip, (tip.0 - dx * head + dy * head, tip.1 - dy * head + dx * head));
                line(tip, (tip.0 - dx * head - dy * head, tip.1 - dy * head - dx * head));
            }
        }
        Button::Tool(super::annotation::Tool::Arrow) => {
            // `arrow-up-right`: the shaft, and a chevron opening back down the shaft.
            let head_at = (centre.0 + arm, centre.1 - arm);
            let tail = (centre.0 - arm, centre.1 + arm);
            line(tail, head_at);
            line(head_at, (head_at.0 - head, head_at.1));
            line(head_at, (head_at.0, head_at.1 + head));
        }
        Button::Tool(super::annotation::Tool::Rectangle) => {
            // `square`, with a corner radius small enough to read as a square at this size.
            let box_rect = RECT {
                left: centre.0 - arm,
                top: centre.1 - arm,
                right: centre.0 + arm,
                bottom: centre.1 + arm,
            };
            hollow(&box_rect, (stroke * 2).max(2));
        }
        Button::Tool(super::annotation::Tool::Ellipse) => {
            // `circle`: an open outline, which is also what the tool draws.
            let (rx, ry) = (arm, arm);
            let background = unsafe { GetStockObject(NULL_BRUSH) };
            let saved = unsafe { SelectObject(hdc, background) };
            unsafe {
                let _ = Ellipse(
                    hdc,
                    centre.0 - rx,
                    centre.1 - ry,
                    centre.0 + rx,
                    centre.1 + ry,
                );
                SelectObject(hdc, saved);
            }
        }
        Button::Tool(super::annotation::Tool::Pencil) => {
            // `pencil-simple`: a body running up to the right with a point at the lower left. A
            // single stroke with a chevron at its end read as an arrow, which is the next button
            // along; the body has to have a width for the mark to say "pencil".
            let tip = (centre.0 - arm, centre.1 + arm);
            let far = (centre.0 + arm, centre.1 - arm);
            let half = (stroke * 3 / 2).max(2);
            // The body runs at forty-five degrees, so its two sides are offset along the other
            // diagonal by half its width.
            let points = [
                windows::Win32::Foundation::POINT {
                    x: tip.0,
                    y: tip.1,
                },
                windows::Win32::Foundation::POINT {
                    x: tip.0 + half,
                    y: tip.1 + half,
                },
                windows::Win32::Foundation::POINT {
                    x: far.0 + half,
                    y: far.1 + half,
                },
                windows::Win32::Foundation::POINT {
                    x: far.0 - half,
                    y: far.1 - half,
                },
                windows::Win32::Foundation::POINT {
                    x: tip.0 - half,
                    y: tip.1 - half,
                },
                windows::Win32::Foundation::POINT {
                    x: tip.0,
                    y: tip.1,
                },
            ];
            unsafe {
                let _ = Polyline(hdc, &points);
            }
        }
        Button::Colour => {
            // The colour itself, as a disc: the only place the colour appears, and a swatch that
            // is not the colour is a button with no answer. The rim is what lets white and yellow
            // read against a light card.
            // The same seventeen-pixel box the font glyphs occupy, so the disc does not look like a
            // button and a half next to them.
            let reach = (side * 22 / 72).max(5);
            let paint = unsafe { CreateSolidBrush(state.colour.to_colorref()) };
            let rim = geometric_pen(1, crate::theme::colorref(crate::theme::SWATCH_BORDER));
            if !paint.is_invalid() {
                let saved_brush = unsafe { SelectObject(hdc, HGDIOBJ(paint.0)) };
                let saved_rim = if rim.is_invalid() {
                    None
                } else {
                    Some(unsafe { SelectObject(hdc, HGDIOBJ(rim.0)) })
                };
                unsafe {
                    // Square, like everything else in the strip now: a rounded swatch beside square
            // buttons reads as a different kind of control.
                    let swatch = RECT {
                        left: centre.0 - reach,
                        top: centre.1 - reach,
                        right: centre.0 + reach,
                        bottom: centre.1 + reach,
                    };
                    let _ =
                        windows::Win32::Graphics::Gdi::FillRect(hdc, &swatch, paint);
                }
                unsafe {
                    if let Some(previous) = saved_rim {
                        SelectObject(hdc, previous);
                        let _ = DeleteObject(HGDIOBJ(rim.0));
                    }
                    SelectObject(hdc, saved_brush);
                    let _ = DeleteObject(HGDIOBJ(paint.0));
                }
            }
        }
        Button::Thickness => {
            // A bar as wide as the stroke, standing the height of the icon box.
            //
            // A disc of the width was honest and unreadable: the thinnest stroke is four pixels, so
            // the mark was a speck in a thirty-six pixel button. A bar keeps the width as its width
            // and takes its length from the icon, which is how the reference shows it — and it is
            // the width that the button is for.
            let wide = (state.thickness as i32).clamp(4, (side - 6).max(4));
            // The bar's height is the icon box, not the full reach: the width is the thing being
            // shown and the length only has to be long enough to show it against.
            let tall = (side * 20 / 72).max(5);
            let bar = RECT {
                left: centre.0 - wide / 2,
                top: centre.1 - tall,
                right: centre.0 - wide / 2 + wide,
                bottom: centre.1 + tall,
            };
            let paint = unsafe { CreateSolidBrush(ink) };
            if !paint.is_invalid() {
                // Square ends, like the bars in its own palette and like everything else in the
                // strip: the two are the same mark at two sizes, and a rounded end here would make
                // the thinnest width a dot rather than a line.
                unsafe {
                    windows::Win32::Graphics::Gdi::FillRect(hdc, &bar, paint);
                    let _ = DeleteObject(HGDIOBJ(paint.0));
                }
            }
        }
        Button::Undo | Button::Redo => {
            // `arrow-u-up-left` and its mirror: a leg down one side, a half circle over the top,
            // and a chevron at the other end pointing back the way it came.
            //
            // The curve is a polyline rather than `Arc`. `Arc` starts and ends at the points where
            // two rays cross the ellipse and sweeps one way only, and which way that looks on
            // screen depends on the sign of y — the first attempt drew a shape like an s. Eight
            // points and round joins give the half circle that is wanted, with no convention to
            // get wrong.
            let sign = if button == Button::Undo { 1 } else { -1 };
            let r = arm;
            let mut curve = Vec::with_capacity(9);
            for step in 0..=8 {
                let angle = std::f64::consts::PI * step as f64 / 8.0;
                let x = centre.0 + sign * (r as f64 * angle.cos()).round() as i32;
                let y = centre.1 - (r as f64 * angle.sin()).round() as i32;
                curve.push(windows::Win32::Foundation::POINT { x, y });
            }
            // The leg, from where the curve starts down to the bottom of the icon box.
            let start = (curve[0].x, curve[0].y);
            line(start, (start.0, centre.1 + r));
            unsafe {
                let _ = Polyline(hdc, &curve);
            }
            // The chevron at the other end, opening back along the curve.
            let end = (curve[8].x, curve[8].y);
            line(end, (end.0 - sign * head, end.1 - head));
            line(end, (end.0 - sign * head, end.1 + head));
        }
        Button::Save => {
            // `floppy-disk`: the case, the shutter at the top and the label at the bottom.
            let case = RECT {
                left: centre.0 - arm,
                top: centre.1 - arm,
                right: centre.0 + arm,
                bottom: centre.1 + arm,
            };
            hollow(&case, (stroke * 2).max(2));
            let shutter = RECT {
                left: centre.0 - head,
                top: centre.1 - arm,
                right: centre.0 + head,
                bottom: centre.1 - head,
            };
            hollow(&shutter, 1);
            line(
                (centre.0 - head, centre.1 + head),
                (centre.0 + head, centre.1 + head),
            );
            line(
                (centre.0 - head, centre.1 + arm),
                (centre.0 + head, centre.1 + arm),
            );
        }
        Button::Copy => {
            // `copy`: the sheet in front, and the corner of the one behind it.
            let front = RECT {
                left: centre.0 - arm,
                top: centre.1 - arm + head,
                right: centre.0 + arm - head,
                bottom: centre.1 + arm,
            };
            hollow(&front, (stroke * 2).max(2));
            line(
                (centre.0 - arm + head, centre.1 - arm),
                (centre.0 + arm, centre.1 - arm),
            );
            line(
                (centre.0 + arm, centre.1 - arm),
                (centre.0 + arm, centre.1 + arm - head),
            );
        }
        Button::Cancel => {
            // `x`, two crossing strokes with round ends.
            line(
                (centre.0 - arm, centre.1 - arm),
                (centre.0 + arm, centre.1 + arm),
            );
            line(
                (centre.0 - arm, centre.1 + arm),
                (centre.0 + arm, centre.1 - arm),
            );
        }
    }

    unsafe {
        // Put back what was there before the pen is released: an object that is still the one a
        // context holds cannot be deleted, and `DeleteObject` on it fails and leaves the handle.
        SelectObject(hdc, saved_pen);
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::annotation::{Colour, Tool};
    use crate::overlay::canvas::Canvas;
    use windows::Win32::Graphics::Gdi::GdiFlush;

    /// A canvas big enough for the shapes and the strip, with every pixel set to a known
    /// value so that anything drawn on it is unmistakable.
    fn blank(width: u32, height: u32) -> Canvas {
        let bgra = vec![0u8; (width * height * 4) as usize];
        Canvas::new(&bgra, width, height).expect("the test canvas must build")
    }

    /// Read a composed pixel back as (blue, green, red).
    fn pixel(canvas: &Canvas, x: i32, y: i32) -> (u8, u8, u8) {
        canvas.back_pixel_for_test(x, y)
    }

    /// Count the pixels of a rectangle that are not the blank value.
    /// A colour as the canvas buffer holds it: blue, green, red.
    ///
    /// The buffer is BGRA, and a `COLORREF` is `0x00BBGGRR`, so the two orders are the same
    /// reversed. Deriving it means a change of palette does not have to be made twice; the
    /// values themselves are pinned in `annotation`, where they are a decision rather than an
    /// accident of layout.
    /// A `COLORREF`, which is 0x00BBGGRR, in the order the buffer holds: blue, green, red.
    /// Whether two colours are close enough to be the same mark.
///
/// The icons come from a font, so their edges are drawn in shades between the ink and whatever is
/// behind them. A comparison that demanded the exact colour would count the middle of a two-pixel
/// stroke in some glyphs and nothing at all in others, which is a test failing over its own
/// arithmetic rather than over the picture.
fn close(a: (u8, u8, u8), b: (u8, u8, u8), tolerance: u8) -> bool {
    a.0.abs_diff(b.0) <= tolerance
        && a.1.abs_diff(b.1) <= tolerance
        && a.2.abs_diff(b.2) <= tolerance
}

fn ink_pixel(colour: COLORREF) -> (u8, u8, u8) {
        let raw = colour.0;
        (
            ((raw >> 16) & 0xFF) as u8,
            ((raw >> 8) & 0xFF) as u8,
            (raw & 0xFF) as u8,
        )
    }

    fn buffer_pixel(colour: crate::overlay::annotation::Colour) -> (u8, u8, u8) {
        // `COLORREF` is 0x00BBGGRR, so the low byte is red and the high one is blue; the buffer
        // wants blue first. Reading them in the same order is what made this return red in the
        // blue slot, and the test said so with a pixel value.
        let raw = colour.to_colorref().0;
        (
            ((raw >> 16) & 0xFF) as u8,
            ((raw >> 8) & 0xFF) as u8,
            (raw & 0xFF) as u8,
        )
    }

    fn painted(canvas: &Canvas, rect: &RECT) -> usize {
        let mut count = 0;
        for y in rect.top..rect.bottom {
            for x in rect.left..rect.right {
                if pixel(canvas, x, y) != (0, 0, 0) {
                    count += 1;
                }
            }
        }
        count
    }

    /// The growth in GDI objects caused by `draw`, as the **median** over several rounds.
    ///
    /// `GetGuiResources` reports the whole process, and Rust runs tests in threads, so the
    /// count also moves while other tests draw. Taking the median of several rounds is what
    /// makes the measurement survive that: a leak adds to every round, while another test
    /// finishing adds a burst to one or two. The median of nine rounds is the fifth
    /// largest, which a burst cannot reach and a leak cannot avoid.
    fn growth_per_round(mut draw: impl FnMut(), rounds: usize, passes: usize) -> i64 {
        // One warm-up pass first: the first call may create things the context keeps, and
        // that is not a leak.
        for _ in 0..3 {
            draw();
        }
        let mut growths = Vec::with_capacity(rounds);
        for _ in 0..rounds {
            let before = user_objects() as i64;
            for _ in 0..passes {
                draw();
            }
            growths.push(user_objects() as i64 - before);
        }
        growths.sort_unstable();
        growths[growths.len() / 2]
    }

    #[test]
    fn drawing_the_strip_does_not_leak_gdi_objects() {
        // The fault that took the icons away, caught the way it has to be caught: by
        // counting. A brush was selected into the context by every glyph and never put
        // back, and `DeleteObject` on an object that is still selected does not release the
        // handle. Twelve glyphs a repaint leaked about seven objects a frame; the process's
        // ceiling is around ten thousand, so after a minute and a half of use `CreatePen`
        // began to fail, and the strip drew its plate and separators — which create nothing
        // — with no icons in them.
        //
        // Nothing about a screenshot shows this. Two numbers do.
        use crate::overlay::toolbar::{self, Region};

        let metrics = toolbar::Metrics::BASE;
        let canvas = blank(200, 400);
        let placement = toolbar::place(
            Region {
                x: 160,
                y: 20,
                width: 30,
                height: 30,
            },
            200,
            400,
            &metrics,
        );
        let chrome = crate::theme::Chrome::new();
        let state = StripState {
            armed: Some(Button::Tool(Tool::Arrow)),
            hovered: None,
            pressed: None,
            can_undo: true,
            colour: crate::overlay::annotation::Colour::Red,
            thickness: 2,
            can_redo: false,
        };

        // The count is process-wide, so other tests running in parallel move it while this one
        // measures. Rather than loosening the threshold until noise fits under it — which made
        // the check blind to a leak of one object a repaint — the same measurement is taken with
        // nothing drawing, and subtracted. Other threads' objects land in both numbers; only
        // what this loop leaks survives the difference.
        let idle = growth_per_round(|| {}, 9, 40);
        let busy = growth_per_round(
            || {
                draw_strip(canvas.dc(), &placement, &metrics, &state, &chrome);
            },
            9,
            40,
        );
        let growth = busy - idle;

        // One glyph's pen is one object. Forty repaints that leak one apiece leave forty behind;
        // what survives the idle subtraction is a few objects of timing noise, so ten separates
        // the two without pretending the noise is not there.
        assert!(
            growth <= 10,
            "forty repaints of the strip add {growth} more GDI objects than an idle round, so something created \
             per glyph is never released"
        );
    }

    #[test]
    fn drawing_a_stroke_does_not_leak_gdi_objects() {
        // The same check for the annotations, because a stroke is the other thing drawn on
        // every frame while a drag is in progress.
        let canvas = blank(120, 120);
        let strokes = [
            Stroke::new(
                Shape::Rectangle {
                    from: Point::new(10, 10),
                    to: Point::new(100, 100),
                },
                Colour::Red,
                2,
            ),
            Stroke::new(
                Shape::Ellipse {
                    from: Point::new(10, 10),
                    to: Point::new(100, 100),
                },
                Colour::Green,
                3,
            ),
            Stroke::new(
                Shape::Arrow {
                    from: Point::new(10, 10),
                    to: Point::new(100, 100),
                },
                Colour::Blue,
                2,
            ),
            Stroke::new(
                Shape::Pencil {
                    points: vec![Point::new(10, 10), Point::new(50, 50), Point::new(90, 90)],
                },
                Colour::White,
                4,
            ),
            Stroke::new(
                Shape::Pencil {
                    points: vec![Point::new(40, 40)],
                },
                Colour::Yellow,
                4,
            ),
        ];

        let growth = growth_per_round(
            || {
                for stroke in &strokes {
                    draw_stroke(canvas.dc(), stroke, (0, 0));
                }
            },
            9,
            10,
        );

        assert!(
            growth <= 2,
            "every ten passes over every shape add {growth} GDI objects, so a pen or brush \
             is never released"
        );
    }

    #[test]
    fn the_thickness_palette_draws_a_dot_that_grows_across_the_row() {
        // Two faults have lived here. A dot can be drawn with a brush that was created and
        // never selected into the context, which leaves the cell outlined and empty; and the
        // widths can be chosen so that two of them look the same, which makes the row a row of
        // marks that says nothing about what it offers.
        use crate::overlay::annotation::THICKNESS_STEPS;
        use crate::overlay::toolbar::{self, PaletteKind, Region};

        let metrics = toolbar::Metrics::BASE;
        let height = metrics.height() + 40;
        let strip = toolbar::place(
            Region {
                x: metrics.width() as i32 + 40,
                y: 20,
                width: 60,
                height: 60,
            },
            340,
            height,
            &metrics,
        );
        let palette = toolbar::palette_for(PaletteKind::Thickness, &strip, 340, height, &metrics);

        let canvas = blank(340, height);
        let chrome = crate::theme::Chrome::new();
        draw_palette(
            canvas.dc(),
            &palette,
            &metrics,
            &StripState {
                armed: None,
                hovered: None,
                pressed: None,
                can_undo: false,
                can_redo: false,
                colour: crate::overlay::annotation::Colour::Red,
                thickness: THICKNESS_STEPS[0],
            },
            &chrome,
        );
        unsafe {
            let _ = GdiFlush();
        }

        let mut per_cell = Vec::new();
        for cell in palette.swatches(&metrics) {
            let mut ink = 0;
            for y in cell.top..cell.bottom {
                for x in cell.left..cell.right {
                    // The dots are drawn in the button ink; the cell's own outline is the edge
                    // colour, so counting only the ink counts only the dot.
                    if pixel(&canvas, x, y) == ink_pixel(crate::theme::colorref(crate::theme::ICON)) {
                        ink += 1;
                    }
                }
            }
            per_cell.push(ink);
        }

        assert!(
            per_cell.iter().all(|count| *count >= 4),
            "a thickness cell came out with no dot in it: {per_cell:?}. A cell that is \
             outlined and empty is a brush that was created and never selected"
        );
        assert!(
            per_cell.windows(2).all(|pair| pair[0] < pair[1]),
            "the dots do not grow across the row: {per_cell:?}"
        );
    }

    #[test]
    fn the_colour_button_is_filled_with_the_colour_a_stroke_would_use() {
        // The button carried a comment saying the colour was filled in by the caller's face
        // drawing, and the face is one brush shared by all twelve buttons — so the colour
        // was in fact never shown anywhere. The only way to find out what would be drawn was
        // to draw something.
        use crate::overlay::annotation::Colour;
        use crate::overlay::toolbar::{self, Region};

        let metrics = toolbar::Metrics::BASE;
        let placement = toolbar::place(
            Region {
                x: metrics.width() as i32 + 40,
                y: 20,
                width: 60,
                height: 60,
            },
            200,
            metrics.height() + 40,
            &metrics,
        );
        // The colour button's own square, from the same arithmetic the drawing uses.
        let (_, x, y, w, h) = metrics
            .button_rects()
            .into_iter()
            .find(|(button, ..)| *button == Button::Colour)
            .expect("the colour button is in the strip");
        let square = RECT {
            left: placement.x + x,
            top: placement.y + y,
            right: placement.x + x + w as i32,
            bottom: placement.y + y + h as i32,
        };

        for colour in [Colour::Red, Colour::Blue, Colour::White] {
            let canvas = blank(200, metrics.height() + 40);
            let chrome = crate::theme::Chrome::new();
            draw_strip(
                canvas.dc(),
                &placement,
                &metrics,
                &StripState {
                    armed: None,
                    hovered: None,
                    pressed: None,
                    can_undo: false,
                    can_redo: false,
                    colour,
                    thickness: 2,
                },
                &chrome,
            );

            let wanted = buffer_pixel(colour);
            let mut found = 0;
            for y in square.top..square.bottom {
                for x in square.left..square.right {
                    if pixel(&canvas, x, y) == wanted {
                        found += 1;
                    }
                }
            }
            assert!(
                found > 20,
                "{colour:?} should fill the colour button, and only {found} pixels of it \
                 were drawn in that square: the swatch does not show the colour in force"
            );
        }
    }

    #[test]
    fn a_rectangle_thickens_with_the_stroke() {
        // Reported: the arrow, the ellipse and the pencil took the chosen width and the box did
        // not. It was drawn with `FrameRect`, which always makes a one-pixel frame with a brush,
        // so the width was never consulted. Nothing about a picture of one rectangle shows it;
        // two of them at different widths do.
        let box_of = || Shape::Rectangle {
            from: Point::new(20, 30),
            to: Point::new(100, 90),
        };

        let mut painted_at = Vec::new();
        for thickness in [2u32, 12] {
            let canvas = blank(120, 120);
            draw_stroke(
                canvas.dc(),
                &Stroke::new(box_of(), Colour::Red, thickness),
                (0, 0),
            );
            let area = painted(
                &canvas,
                &RECT {
                    left: 0,
                    top: 0,
                    right: 120,
                    bottom: 120,
                },
            );
            painted_at.push(area);

            // And it still stays inside the box its two points describe, whatever its width.
            let outside = painted(
                &canvas,
                &RECT {
                    left: 0,
                    top: 0,
                    right: 120,
                    bottom: 120,
                },
            ) - painted(
                &canvas,
                &RECT {
                    left: 20,
                    top: 30,
                    right: 101,
                    bottom: 91,
                },
            );
            assert_eq!(
                outside, 0,
                "a {thickness}px box drew {outside} pixels outside its own corners"
            );
        }

        assert!(
            painted_at[1] > painted_at[0] * 2,
            "a 12px box painted {} pixels and a 2px one painted {}: the width is being \
             ignored",
            painted_at[1],
            painted_at[0]
        );
    }

    #[test]
    fn a_stroke_is_not_confined_to_the_selection() {
        // The opposite of what this module used to test. A mark may be made anywhere on the
        // screen, so a stroke drawn from outside the region keeps its outside part; what is
        // saved is decided by cropping the capture, not by refusing to draw.
        let canvas = blank(120, 120);
        // An origin in the middle, and a stroke whose tail starts well to its upper left: the
        // old clamp would have pulled that tail to the origin and painted nothing to the left
        // of it.
        draw_stroke(
            canvas.dc(),
            &Stroke::new(
                Shape::Arrow {
                    from: Point::new(-20, -20),
                    to: Point::new(40, 40),
                },
                Colour::Red,
                2,
            ),
            (50, 50),
        );
        unsafe {
            let _ = GdiFlush();
        }

        let left_of_the_origin = painted(
            &canvas,
            &RECT {
                left: 0,
                top: 0,
                right: 50,
                bottom: 120,
            },
        );
        assert!(
            left_of_the_origin > 0,
            "nothing was drawn to the upper left of the origin, so the stroke is still being \
             clamped to the region"
        );
    }

    #[test]
    fn a_rectangle_is_drawn_inside_its_own_outline() {
        // The property that matters for every shape: whatever it draws is within the
        // bounding box its two points describe. Checked by counting what changed outside
        // that box, which is the failure a person would not see until a corner of the
        // drawing was over something it should not cover.
        let canvas = blank(120, 120);
        let stroke = Stroke::new(
            Shape::Rectangle {
                from: Point::new(20, 30),
                to: Point::new(100, 90),
            },
            Colour::Red,
            2,
        );
        draw_stroke(canvas.dc(), &stroke, (0, 0));
        unsafe {
            let _ = GdiFlush();
        }

        let outline = RECT {
            left: 19,
            top: 29,
            right: 102,
            bottom: 92,
        };
        assert!(painted(&canvas, &outline) > 0, "nothing was drawn at all");

        // Outside the outline, with a margin for the pen's width, nothing may appear.
        let mut strays = 0;
        for y in 0..120 {
            for x in 0..120 {
                let inside = x >= outline.left && x < outline.right && y >= outline.top && y < outline.bottom;
                if !inside && pixel(&canvas, x, y) != (0, 0, 0) {
                    strays += 1;
                }
            }
        }
        assert_eq!(strays, 0, "the rectangle drew outside its own outline");
    }

    #[test]
    fn a_rectangle_drawn_in_any_direction_comes_out_the_same() {
        // A drag can start at any corner. GDI will not draw a mirrored rectangle, so
        // without normalising, two of the four directions draw nothing — which looks
        // like the tool being broken half the time.
        let directions = [
            ((20, 30), (100, 90)),
            ((100, 90), (20, 30)),
            ((100, 30), (20, 90)),
            ((20, 90), (100, 30)),
        ];
        let mut counts = Vec::new();
        for (from, to) in directions {
            let canvas = blank(120, 120);
            draw_stroke(
                canvas.dc(),
                &Stroke::new(
                    Shape::Rectangle {
                        from: Point::new(from.0, from.1),
                        to: Point::new(to.0, to.1),
                    },
                    Colour::Red,
                    2,
                ),
                (0, 0),
            );
            unsafe {
                let _ = GdiFlush();
            }
            counts.push(painted(&canvas, &RECT { left: 0, top: 0, right: 120, bottom: 120 }));
        }
        assert!(
            counts.iter().all(|&c| c > 0),
            "a direction drew nothing: {counts:?}"
        );
        // The four are the same rectangle, so they should cover the same amount.
        assert!(
            counts.iter().all(|&c| c == counts[0]),
            "the directions do not agree: {counts:?}"
        );
    }

    #[test]
    fn an_ellipse_is_not_filled_in() {
        // A filled ellipse hides whatever it was drawn around, which defeats the point of
        // drawing on a screenshot. The centre must stay untouched.
        let canvas = blank(120, 120);
        draw_stroke(
            canvas.dc(),
            &Stroke::new(
                Shape::Ellipse {
                    from: Point::new(20, 20),
                    to: Point::new(100, 100),
                },
                Colour::Red,
                2,
            ),
            (0, 0),
        );
        unsafe {
            let _ = GdiFlush();
        }
        assert_eq!(
            pixel(&canvas, 60, 60),
            (0, 0, 0),
            "the inside of the ellipse was painted over"
        );
        // But the outline exists.
        let edge = painted(&canvas, &RECT { left: 20, top: 18, right: 101, bottom: 22 });
        assert!(edge > 0, "the ellipse has no top edge");
    }

    #[test]
    fn a_colour_reaches_the_canvas_as_the_colour_it_names() {
        // `COLORREF` is 0x00BBGGRR, so red and blue are the way round that surprises
        // people. Checked by reading the pixel back, because getting it wrong produces a
        // drawing in the wrong colour, which is easy to blame on anything else.
        let canvas = blank(60, 60);
        draw_stroke(
            canvas.dc(),
            &Stroke::new(
                Shape::Rectangle {
                    from: Point::new(10, 10),
                    to: Point::new(50, 50),
                },
                Colour::Blue,
                2,
            ),
            (0, 0),
        );
        unsafe {
            let _ = GdiFlush();
        }

        // Find a painted pixel on the top edge and check it is blue: the DIB holds BGRA,
        // so a blue stroke has a high first byte and a low third.
        let mut found: Option<(u8, u8, u8)> = None;
        for x in 10..50 {
            let px = pixel(&canvas, x, 10);
            if px != (0, 0, 0) {
                found = Some(px);
                break;
            }
        }
        let (blue, green, red) = found.expect("the rectangle's top edge was not drawn");
        assert!(
            blue > 128 && red < 128,
            "expected a blue stroke, read ({blue}, {green}, {red})"
        );
    }

    #[test]
    fn a_pencil_of_one_point_still_leaves_a_mark() {
        // A press with no movement. Drawing nothing would look like the tool had not
        // registered the click.
        let canvas = blank(40, 40);
        draw_stroke(
            canvas.dc(),
            &Stroke::new(
                Shape::Pencil {
                    points: vec![Point::new(20, 20)],
                },
                Colour::Red,
                3,
            ),
            (0, 0),
        );
        unsafe {
            let _ = GdiFlush();
        }
        assert!(
            painted(&canvas, &RECT { left: 18, top: 18, right: 24, bottom: 24 }) > 0,
            "a single-point pencil left nothing"
        );
    }

    #[test]
    fn an_arrow_has_a_head_when_it_has_a_direction() {
        // A shaft without a head is a line, and the difference is the whole reason the
        // tool exists. The head is checked by counting what was drawn near the tip and
        // comparing it with the same count for a zero-length arrow, which has no head.
        let canvas = blank(100, 100);
        draw_stroke(
            canvas.dc(),
            &Stroke::new(
                Shape::Arrow {
                    from: Point::new(10, 10),
                    to: Point::new(80, 80),
                },
                Colour::Red,
                2,
            ),
            (0, 0),
        );
        unsafe {
            let _ = GdiFlush();
        }
        let near_tip = painted(&canvas, &RECT { left: 60, top: 60, right: 90, bottom: 90 });
        assert!(near_tip > 10, "the head is missing: only {near_tip} pixels near the tip");

        // The pixels the head adds must not reach beyond the bounding box of the shaft
        // by more than the head's own length.
        let mut strays = 0;
        for y in 0..100 {
            for x in 0..100 {
                if pixel(&canvas, x, y) == (0, 0, 0) {
                    continue;
                }
                let far_from_line = (x as i32 - y as i32).abs() > 14;
                let beyond_tip = x > 90 || y > 90;
                if far_from_line || beyond_tip {
                    strays += 1;
                }
            }
        }
        assert_eq!(strays, 0, "the arrow drew outside its own sweep");
    }

    #[test]
    fn a_zero_length_arrow_draws_a_shaft_and_no_head() {
        // Pressed and released on one pixel: there is no direction, so there is nothing
        // for a head to point along. It must not divide by zero or draw a stray triangle.
        let canvas = blank(40, 40);
        draw_stroke(
            canvas.dc(),
            &Stroke::new(
                Shape::Arrow {
                    from: Point::new(20, 20),
                    to: Point::new(20, 20),
                },
                Colour::Red,
                2,
            ),
            (0, 0),
        );
        unsafe {
            let _ = GdiFlush();
        }
        let all = painted(&canvas, &RECT { left: 0, top: 0, right: 40, bottom: 40 });
        assert!(all > 0, "nothing was drawn");
        assert!(all < 40, "a head was drawn for an arrow with no direction: {all} pixels");
    }

    #[test]
    fn a_shape_is_placed_by_the_selection_origin() {
        // The conversion the whole module is about: the same stroke, drawn with two different
        // origins, must land in two different places.
        let stroke = Stroke::new(
            Shape::Rectangle {
                from: Point::new(5, 5),
                to: Point::new(25, 25),
            },
            Colour::Red,
            2,
        );

        let first = blank(100, 100);
        draw_stroke(first.dc(), &stroke, (10, 10));
        let second = blank(100, 100);
        draw_stroke(second.dc(), &stroke, (50, 50));
        unsafe {
            let _ = GdiFlush();
        }

        assert!(
            painted(&first, &RECT { left: 14, top: 14, right: 37, bottom: 37 }) > 0,
            "the first placement is empty"
        );
        assert!(
            painted(&second, &RECT { left: 54, top: 54, right: 77, bottom: 77 }) > 0,
            "the second placement is empty"
        );
        assert_eq!(
            painted(&first, &RECT { left: 0, top: 0, right: 50, bottom: 50 })
                + painted(&second, &RECT { left: 50, top: 50, right: 100, bottom: 100 }),
            painted(&first, &RECT { left: 0, top: 0, right: 100, bottom: 100 })
                + painted(&second, &RECT { left: 0, top: 0, right: 100, bottom: 100 }),
            "the two placements do not account for all the drawing"
        );
    }

    #[test]
    fn the_strip_is_drawn_inside_its_placement_and_covers_it() {
        // The plate must fill exactly the placement: a plate one pixel short leaves a
        // line of the capture showing through, which reads as a drawing fault.
        use crate::overlay::toolbar::{self, Button, Region};

        // Tall enough for the strip: it is taller than any region it sits beside.
        let canvas = blank(300, 700);
        let metrics = toolbar::Metrics::BASE;
        let placement = toolbar::place(
            Region {
                x: 150,
                y: 40,
                width: 120,
                height: 120,
            },
            300,
            400,
            &metrics,
        );
        let chrome = crate::theme::Chrome::new();
        let state = StripState {
            armed: Some(Button::Tool(Tool::Arrow)),
            hovered: None,
            pressed: None,
            can_undo: true,
            colour: crate::overlay::annotation::Colour::Red,
            thickness: 2,
            can_redo: false,
        };
        draw_strip(canvas.dc(), &placement, &metrics, &state, &chrome);
        unsafe {
            let _ = GdiFlush();
        }

        let body = RECT {
            left: placement.x,
            top: placement.y,
            right: placement.x + placement.width as i32,
            bottom: placement.y + placement.height as i32,
        };
        let covered = painted(&canvas, &body);
        let area = (body.right - body.left) as usize * (body.bottom - body.top) as usize;
        // Every pixel of the placement, corners included: the card is a rectangle, and a card one
        // pixel short anywhere leaves a line of the capture showing through.
        assert_eq!(
            covered, area,
            "the plate covered {covered} of its {area} pixels, so something shows through"
        );
        for corner in [
            (body.left, body.top),
            (body.right - 1, body.top),
            (body.left, body.bottom - 1),
            (body.right - 1, body.bottom - 1),
        ] {
            assert_ne!(
                pixel(&canvas, corner.0, corner.1),
                (0, 0, 0),
                "the corner at {corner:?} is not painted, so the card is not square"
            );
        }

        // And nothing outside it.
        let mut strays = 0;
        for y in 0..700 {
            for x in 0..300 {
                let inside = x >= body.left && x < body.right && y >= body.top && y < body.bottom;
                if !inside && pixel(&canvas, x, y) != (0, 0, 0) {
                    strays += 1;
                }
            }
        }
        assert_eq!(strays, 0, "the strip drew outside its placement");
    }

    #[test]
    fn every_button_draws_its_icon_at_every_scale() {
        // A person reported the strip losing its icons and the drawing going with it, and
        // it could not be reproduced at 100%. The strip's size scales with the display, so
        // the scale is the first thing to suspect: a glyph is built from a quarter of the
        // button's side and a fixed two-pixel pen, so a small enough or large enough button
        // could produce something that is there and cannot be seen.
        use crate::overlay::toolbar::{self, Region};

        for factor in [1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
            let metrics = toolbar::metrics_for(factor);
            // The canvas grows with the strip. Pinning it would read outside the buffer at
            // the larger factors, which is the test being wrong rather than the drawing.
            let width = metrics.width() * 2 + 200;
            let height = metrics.height() + 40;
            let placement = toolbar::place(
                Region {
                    x: metrics.width() as i32 + 100,
                    y: 20,
                    width: 80,
                    height: 80,
                },
                width,
                height,
                &metrics,
            );
            let canvas = blank(width, height);
            let chrome = crate::theme::Chrome::new();
            // Named, so the ink each button is expected to use can be asked of the program below.
            // Every button usable, so a greyed glyph cannot be mistaken for a missing one: the
            // report was about the icons going, not about undo being dim.
            let state = StripState {
                armed: None,
                hovered: None,
                pressed: None,
                can_undo: true,
                colour: crate::overlay::annotation::Colour::Red,
                thickness: 2,
                can_redo: true,
            };
            draw_strip(canvas.dc(), &placement, &metrics, &state, &chrome);
            unsafe {
                let _ = GdiFlush();
            }

            for (button, x, y, w, h) in metrics.button_rects() {
                let rect = RECT {
                    left: placement.x + x,
                    top: placement.y + y,
                    right: placement.x + x + w as i32,
                    bottom: placement.y + y + h as i32,
                };
                // The colour button does not draw a glyph in ink: it is a swatch of the
                // colour itself, so it is counted by that colour instead. Every other button
                // is counted by the ink, which is what "the icons went" was about.
                // Asked of the program, not restated: Save is drawn in white on its fill and
                // Cancel in red, and a test that kept its own table would be testing the table.
                let wanted = match button {
                    // The state this test draws with is red.
                    Button::Colour => buffer_pixel(crate::overlay::annotation::Colour::Red),
                    _ => ink_pixel(ink_for(button, &state)),
                };
                let mut ink = 0;
                for py in rect.top..rect.bottom {
                    for px in rect.left..rect.right {
                        if close(pixel(&canvas, px, py), wanted, 40) {
                            ink += 1;
                        }
                    }
                }
                // A glyph is a handful of two-pixel strokes. Four is a low bar that still
                // fails if nothing at all was drawn.
                assert!(
                    ink >= 4,
                    "at {factor}x, {} drew {ink} pixels of {:?} in a {}x{} button",
                    button.as_str_for_test(),
                    wanted,
                    w,
                    h
                );
            }
        }
    }

    #[test]
    fn the_strip_fits_inside_the_place_it_was_given_at_every_scale() {
        // The plate is drawn from the placement's own size, so a metrics height that
        // disagreed with the button walk would leave the last buttons outside the plate —
        // drawn on the capture, over whatever is underneath, and outside every rectangle the
        // hit test knows about.
        use crate::overlay::toolbar;

        for factor in [1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
            let metrics = toolbar::metrics_for(factor);
            let rects = metrics.button_rects();
            let last = rects.last().expect("the strip has buttons");
            let last_bottom = last.2 + last.4 as i32;
            assert!(
                last_bottom + metrics.padding <= metrics.height() as i32,
                "at {factor}x the last button ends at {last_bottom}, past the strip's {}",
                metrics.height()
            );
            for (index, (_, x, _, w, _)) in rects.iter().enumerate() {
                assert!(
                    x + *w as i32 <= metrics.width() as i32,
                    "at {factor}x button {index} is wider than the strip"
                );
            }
        }
    }

    /// Undo and redo, drawn in the two inks so the states can be told apart.
    #[test]
    fn an_unavailable_button_still_draws_something() {
        use crate::overlay::toolbar::{self, Button, Region};

        let mut counts = Vec::new();
        for (can_undo, can_redo) in [(false, false), (true, true)] {
            let canvas = blank(300, 400);
            let metrics = toolbar::Metrics::BASE;
            let placement = toolbar::place(
                Region {
                    x: 150,
                    y: 40,
                    width: 120,
                    height: 120,
                },
                300,
                700,
                &metrics,
            );
            let chrome = crate::theme::Chrome::new();
            draw_strip(
                canvas.dc(),
                &placement,
                &metrics,
                &StripState {
                    armed: None,
                    hovered: None,
                    pressed: None,
                    can_undo,
                    can_redo,
                    colour: crate::overlay::annotation::Colour::Red,
                thickness: 2,
                },
                &chrome,
            );
            unsafe {
                let _ = GdiFlush();
            }

            let (button, x, y, w, h) = metrics
                .button_rects()
                .into_iter()
                .find(|(b, _, _, _, _)| *b == Button::Undo)
                .expect("undo is always in the strip");
            let rect = RECT {
                left: placement.x + x,
                top: placement.y + y,
                right: placement.x + x + w as i32,
                bottom: placement.y + y + h as i32,
            };
            let wanted = if can_undo { crate::theme::colorref(crate::theme::ICON) } else { crate::theme::colorref(crate::theme::ICON_DISABLED) };
            let want = ink_pixel(wanted);
            let mut matching = 0;
            for py in rect.top..rect.bottom {
                for px in rect.left..rect.right {
                    // Antialiased, so the stroke's core is the ink and its edge is not.
                    if close(pixel(&canvas, px, py), want, 40) {
                        matching += 1;
                    }
                }
            }
            counts.push((button, matching));
        }

        let (button, unavailable) = counts[0];
        let (_, available) = counts[1];
        assert!(
            unavailable > 0,
            "{button:?} drew nothing in the dim ink, so it looks like a hole"
        );
        assert!(
            available > 0,
            "{button:?} drew nothing in the bright ink while it could be used"
        );
    }
}
