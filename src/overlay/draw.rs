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
    BS_SOLID, CreatePen, CreateSolidBrush, DeleteObject, Ellipse, ExtCreatePen, FrameRect,
    GetObjectW, GetStockObject, HDC, HGDIOBJ, HPEN, LineTo, LOGBRUSH, MoveToEx, NULL_BRUSH,
    NULL_PEN, PEN_STYLE, PS_ENDCAP_ROUND, PS_GEOMETRIC, PS_JOIN_ROUND, PS_SOLID, Polyline, RoundRect,
    SelectObject,
};

use super::annotation::{Point, Shape, Stroke};
use super::toolbar::{Button, Metrics, Placement};

/// The colour a usable button's glyph is drawn in.
const BUTTON_INK: COLORREF = COLORREF(0x002A_2A2A);
/// The colour of one that cannot be used: close to the plate, so it reads as unavailable.
const DIM_INK: COLORREF = COLORREF(0x00B0_B0B0);
/// The rim around the colour swatch.
///
/// Near black, because the swatch has to be visible whatever colour it holds: white and
/// yellow on a light plate need an edge of their own, and a dark one reads against every
/// colour in the set.
const PLATE_SHADOW: COLORREF = COLORREF(0x0088_8888);

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
    plate: windows::Win32::Graphics::Gdi::HBRUSH,
    edge: windows::Win32::Graphics::Gdi::HBRUSH,
) {
    use super::annotation::{thickness_index, Colour, THICKNESS_STEPS};
    use super::toolbar::PaletteKind;

    let body = RECT {
        left: palette.x,
        top: palette.y,
        right: palette.x + palette.width as i32,
        bottom: palette.y + palette.height as i32,
    };
    unsafe {
        windows::Win32::Graphics::Gdi::FillRect(hdc, &body, plate);
        FrameRect(hdc, &body, edge);
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

    let rim = unsafe { CreateSolidBrush(PLATE_SHADOW) };
    for (index, square) in palette.swatches(metrics).into_iter().enumerate() {
        match palette.kind {
            PaletteKind::Colour => {
                let colour = Colour::ALL[index];
                let paint = unsafe { CreateSolidBrush(colour.to_colorref()) };
                if !paint.is_invalid() {
                    unsafe {
                        windows::Win32::Graphics::Gdi::FillRect(hdc, &square, paint);
                    }
                    if !rim.is_invalid() {
                        // Inside the swatch, so the colour keeps its full size and the rim is
                        // what separates it from the plate — which is what makes white visible.
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
                // A bar of that width, as the button shows it. A disc of the width was honest and
                // unreadable at the thin end, and a row of discs does not look like the widths it
                // offers; a row of bars does, and it matches the button that opens it.
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
                let dot = unsafe { CreateSolidBrush(BUTTON_INK) };
                if !dot.is_invalid() {
                    let no_pen = unsafe { GetStockObject(NULL_PEN) };
                    let previous_pen = unsafe { SelectObject(hdc, no_pen) };
                    let previous_brush = unsafe { SelectObject(hdc, HGDIOBJ(dot.0)) };
                    unsafe {
                        let _ = RoundRect(
                            hdc,
                            bar.left,
                            bar.top,
                            bar.right,
                            bar.bottom,
                            wide,
                            wide,
                        );
                        SelectObject(hdc, previous_brush);
                        SelectObject(hdc, previous_pen);
                        let _ = DeleteObject(HGDIOBJ(dot.0));
                    }
                }
                if !rim.is_invalid() {
                    unsafe {
                        FrameRect(hdc, &square, edge);
                    }
                }
            }
        }
        // The choice in force is outlined, so the palette answers "which one am I using".
        if index == selected {
            let ring = unsafe { CreateSolidBrush(BUTTON_INK) };
            if !ring.is_invalid() {
                let marker = RECT {
                    left: square.left - 1,
                    top: square.top - 1,
                    right: square.right + 1,
                    bottom: square.bottom + 1,
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
    }
}

/// Draw the strip: its plate, its separators, and every button.
/// The colour a solid brush was made from.
///
/// The brushes are made once and handed around, so the pen that outlines them has to take its
/// colour from the brush rather than from a second constant that could drift away from the first.
fn brush_colour(brush: windows::Win32::Graphics::Gdi::HBRUSH) -> COLORREF {
    let mut log = LOGBRUSH::default();
    let read = unsafe {
        GetObjectW(
            HGDIOBJ(brush.0),
            std::mem::size_of::<LOGBRUSH>() as i32,
            Some(&mut log as *mut LOGBRUSH as *mut std::ffi::c_void),
        )
    };
    if read == 0 {
        BUTTON_INK
    } else {
        log.lbColor
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

/// A rounded rectangle, filled and outlined with the objects it is given.
///
/// `RoundRect` takes the corner *diameter*, so the radius is doubled here rather than at every
/// call. Both objects are put back, because the context outlives this call.
fn rounded(hdc: HDC, rect: &RECT, radius: i32, fill: Option<windows::Win32::Graphics::Gdi::HBRUSH>, pen: HPEN) {
    let brush = match fill {
        Some(brush) => HGDIOBJ(brush.0),
        // No fill: the corner is round but the plate shows through the middle.
        None => unsafe { GetStockObject(NULL_BRUSH) },
    };
    let saved_brush = unsafe { SelectObject(hdc, brush) };
    let saved_pen = unsafe { SelectObject(hdc, HGDIOBJ(pen.0)) };
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

pub fn draw_strip(
    hdc: HDC,
    placement: &Placement,
    metrics: &Metrics,
    state: &StripState,
    plate: windows::Win32::Graphics::Gdi::HBRUSH,
    edge: windows::Win32::Graphics::Gdi::HBRUSH,
    face: windows::Win32::Graphics::Gdi::HBRUSH,
) {
    let body = RECT {
        left: placement.x,
        top: placement.y,
        right: placement.x + placement.width as i32,
        bottom: placement.y + placement.height as i32,
    };
    let edge_pen = unsafe { CreatePen(PS_SOLID, 1, brush_colour(edge)) };
    // The card first: a light plate with a hairline border, so it reads as a card laid on the
    // capture rather than a hole cut into it.
    if !edge_pen.is_invalid() {
        rounded(hdc, &body, metrics.panel_radius, Some(plate), edge_pen);
    } else {
        unsafe {
            windows::Win32::Graphics::Gdi::FillRect(hdc, &body, plate);
        }
    }

    // The rules between groups. One pixel, inset from the card's edges so they read as separators
    // inside it rather than as cuts through it.
    if !edge_pen.is_invalid() {
        for y in metrics.separators() {
            let rule = RECT {
                left: body.left + metrics.padding + 2,
                top: body.top + y,
                right: body.right - metrics.padding - 2,
                bottom: body.top + y + 1,
            };
            if rule.right > rule.left {
                unsafe {
                    windows::Win32::Graphics::Gdi::FillRect(hdc, &rule, edge);
                }
            }
        }
    }

    let ink_pen = unsafe { CreatePen(PS_SOLID, 1, BUTTON_INK) };
    let outline_in_ink = !ink_pen.is_invalid();

    for (button, x, y, width, height) in metrics.button_rects() {
        let rect = RECT {
            left: placement.x + x,
            top: placement.y + y,
            right: placement.x + x + width as i32,
            bottom: placement.y + y + height as i32,
        };

        // Every button is outlined, and the one in force is also filled and outlined darker: the
        // outline says "this is a button", the fill and the darker line say "this is the one".
        let filled = state.active == Some(button);
        let pen = if filled && outline_in_ink { ink_pen } else { edge_pen };
        if !pen.is_invalid() {
            rounded(hdc, &rect, metrics.radius, filled.then_some(face), pen);
        }

        // An unavailable button keeps its glyph, drawn in a colour close to the plate: it
        // reads as present but not available, which is what it is. Hiding it would move
        // every button below it and make the strip a different shape from one moment to
        // the next.
        let ink = if state.is_disabled(button) { DIM_INK } else { BUTTON_INK };
        draw_button_glyph(hdc, button, &rect, ink, state);
    }

    // Both pens, released. Nothing is selected into the context by the time they are freed —
    // `rounded` puts back what it took — and a pen that is still selected cannot be released,
    // which is the leak that took the icons away once already.
    unsafe {
        if !edge_pen.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(edge_pen.0));
        }
        if !ink_pen.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(ink_pen.0));
        }
    }
}

/// What the strip should show as being in a particular state.
///
/// Passed as one value rather than as separate arguments because the drawing needs to
/// know which button is armed, which is under the pointer, and which two are unavailable —
/// and three positional arguments of which two are booleans is how they end up swapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StripState {
    /// The button to draw standing out: the armed tool, or whatever the pointer is over.
    pub active: Option<Button>,
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
fn draw_button_glyph(hdc: HDC, button: Button, rect: &RECT, ink: COLORREF, state: &StripState) {
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
    let hollow = |rect: &RECT, radius: i32| rounded(hdc, rect, radius, None, pen);

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
            let paint = unsafe { CreateSolidBrush(state.colour.to_colorref()) };
            let rim = geometric_pen(1, PLATE_SHADOW);
            if !paint.is_invalid() {
                let saved_brush = unsafe { SelectObject(hdc, HGDIOBJ(paint.0)) };
                let saved_rim = if rim.is_invalid() {
                    None
                } else {
                    Some(unsafe { SelectObject(hdc, HGDIOBJ(rim.0)) })
                };
                unsafe {
                    let _ = Ellipse(
                        hdc,
                        centre.0 - arm,
                        centre.1 - arm,
                        centre.0 + arm,
                        centre.1 + arm,
                    );
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
            let bar = RECT {
                left: centre.0 - wide / 2,
                top: centre.1 - arm,
                right: centre.0 - wide / 2 + wide,
                bottom: centre.1 + arm,
            };
            let paint = unsafe { CreateSolidBrush(ink) };
            if !paint.is_invalid() {
                // Its corners are rounded to the width, so the thinnest reads as a line and the
                // widest as a pill rather than as a rectangle.
                let no_pen = unsafe { GetStockObject(NULL_PEN) };
                let saved_pen = unsafe { SelectObject(hdc, no_pen) };
                let saved_brush = unsafe { SelectObject(hdc, HGDIOBJ(paint.0)) };
                unsafe {
                    let _ = RoundRect(
                        hdc,
                        bar.left,
                        bar.top,
                        bar.right,
                        bar.bottom,
                        wide,
                        wide,
                    );
                    SelectObject(hdc, saved_brush);
                    SelectObject(hdc, saved_pen);
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
        let plate = unsafe { CreateSolidBrush(COLORREF(0x0020_2020)) };
        let edge = unsafe { CreateSolidBrush(COLORREF(0x0060_6060)) };
        let face = unsafe { CreateSolidBrush(COLORREF(0x0040_4040)) };
        let state = StripState {
            active: Some(Button::Tool(Tool::Arrow)),
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
                draw_strip(canvas.dc(), &placement, &metrics, &state, plate, edge, face);
            },
            9,
            40,
        );
        let growth = busy - idle;

        unsafe {
            let _ = DeleteObject(HGDIOBJ(plate.0));
            let _ = DeleteObject(HGDIOBJ(edge.0));
            let _ = DeleteObject(HGDIOBJ(face.0));
        }

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
        let plate = unsafe { CreateSolidBrush(COLORREF(0x0020_2020)) };
        let edge = unsafe { CreateSolidBrush(COLORREF(0x0060_6060)) };
        draw_palette(
            canvas.dc(),
            &palette,
            &metrics,
            &StripState {
                active: None,
                can_undo: false,
                can_redo: false,
                colour: crate::overlay::annotation::Colour::Red,
                thickness: THICKNESS_STEPS[0],
            },
            plate,
            edge,
        );
        unsafe {
            let _ = DeleteObject(HGDIOBJ(plate.0));
            let _ = DeleteObject(HGDIOBJ(edge.0));
        }
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
                    if pixel(&canvas, x, y) == ink_pixel(BUTTON_INK) {
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
            let plate = unsafe { CreateSolidBrush(COLORREF(0x0020_2020)) };
            let edge = unsafe { CreateSolidBrush(COLORREF(0x0060_6060)) };
            let face = unsafe { CreateSolidBrush(COLORREF(0x0040_4040)) };
            draw_strip(
                canvas.dc(),
                &placement,
                &metrics,
                &StripState {
                    active: None,
                    can_undo: false,
                    can_redo: false,
                    colour,
                    thickness: 2,
                },
                plate,
                edge,
                face,
            );
            unsafe {
                let _ = DeleteObject(HGDIOBJ(plate.0));
                let _ = DeleteObject(HGDIOBJ(edge.0));
                let _ = DeleteObject(HGDIOBJ(face.0));
            }

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
        let plate = unsafe { CreateSolidBrush(COLORREF(0x0020_2020)) };
        let edge = unsafe { CreateSolidBrush(COLORREF(0x0060_6060)) };
        let face = unsafe { CreateSolidBrush(COLORREF(0x0040_4040)) };
        let state = StripState {
            active: Some(Button::Tool(Tool::Arrow)),
            can_undo: true,
            colour: crate::overlay::annotation::Colour::Red,
            thickness: 2,
            can_redo: false,
        };
        draw_strip(canvas.dc(), &placement, &metrics, &state, plate, edge, face);
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
        // Everything but the four rounded corners. The corners are a quarter of a circle each, so
        // a square of the radius around every one of them is a generous bound on what is missing —
        // and a hole anywhere else would take the count far below it.
        let corners = 4 * (metrics.panel_radius * metrics.panel_radius) as usize;
        assert!(
            covered + corners >= area,
            "the plate covered {covered} of its {area} pixels, so something shows through"
        );
        assert!(
            covered < area,
            "the plate covered its corners too, so it is not rounded"
        );
        assert!(
            pixel(&canvas, body.left, body.top) == (0, 0, 0),
            "the corner is painted, so the card has square corners"
        );

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

        unsafe {
            let _ = DeleteObject(HGDIOBJ(plate.0));
            let _ = DeleteObject(HGDIOBJ(edge.0));
            let _ = DeleteObject(HGDIOBJ(face.0));
        }
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
            let plate = unsafe { CreateSolidBrush(COLORREF(0x0020_2020)) };
            let edge = unsafe { CreateSolidBrush(COLORREF(0x0060_6060)) };
            let face = unsafe { CreateSolidBrush(COLORREF(0x0040_4040)) };
            draw_strip(
                canvas.dc(),
                &placement,
                &metrics,
                // Every button usable, so a greyed glyph cannot be mistaken for a missing
                // one: the report was about the icons going, not about undo being dim.
                &StripState {
                    active: None,
                    can_undo: true,
                    colour: crate::overlay::annotation::Colour::Red,
                    thickness: 2,
                    can_redo: true,
                },
                plate,
                edge,
                face,
            );
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
                let wanted = match button {
                    // The state this test draws with is red.
                    Button::Colour => {
                        buffer_pixel(crate::overlay::annotation::Colour::Red)
                    }
                    _ => ink_pixel(BUTTON_INK),
                };
                let mut ink = 0;
                for py in rect.top..rect.bottom {
                    for px in rect.left..rect.right {
                        if pixel(&canvas, px, py) == wanted {
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

            unsafe {
                let _ = DeleteObject(HGDIOBJ(plate.0));
                let _ = DeleteObject(HGDIOBJ(edge.0));
                let _ = DeleteObject(HGDIOBJ(face.0));
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
            let plate = unsafe { CreateSolidBrush(COLORREF(0x0020_2020)) };
            let edge = unsafe { CreateSolidBrush(COLORREF(0x0060_6060)) };
            let face = unsafe { CreateSolidBrush(COLORREF(0x0040_4040)) };
            draw_strip(
                canvas.dc(),
                &placement,
                &metrics,
                &StripState {
                    active: None,
                    can_undo,
                    can_redo,
                    colour: crate::overlay::annotation::Colour::Red,
                thickness: 2,
                },
                plate,
                edge,
                face,
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
            let wanted = if can_undo { BUTTON_INK } else { DIM_INK };
            let want = (
                (wanted.0 & 0xFF) as u8,
                ((wanted.0 >> 8) & 0xFF) as u8,
                ((wanted.0 >> 16) & 0xFF) as u8,
            );
            let mut matching = 0;
            for py in rect.top..rect.bottom {
                for px in rect.left..rect.right {
                    if pixel(&canvas, px, py) == want {
                        matching += 1;
                    }
                }
            }
            counts.push((button, matching));

            unsafe {
                let _ = DeleteObject(HGDIOBJ(plate.0));
                let _ = DeleteObject(HGDIOBJ(edge.0));
                let _ = DeleteObject(HGDIOBJ(face.0));
            }
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
