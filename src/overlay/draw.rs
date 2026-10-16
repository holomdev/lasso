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
    CreatePen, CreateSolidBrush, DeleteObject, Ellipse, FrameRect, GetStockObject, HDC, HGDIOBJ,
    LineTo, MoveToEx, NULL_BRUSH, PS_SOLID, Polyline, Rectangle, SelectObject,
};

use super::annotation::{Point, Shape, Stroke};
use super::toolbar::{Button, Metrics, Placement};

/// The colour a usable button's glyph is drawn in.
const BUTTON_INK: COLORREF = COLORREF(0x00E0_E0E0);
/// The colour of one that cannot be used: close to the plate, so it reads as unavailable.
const DIM_INK: COLORREF = COLORREF(0x0070_7070);

/// The narrowest a stroke is allowed to be drawn, so a set thickness of zero cannot
/// produce a shape that is invisible and looks like nothing happened.
const MIN_THICKNESS: i32 = 1;

/// The rectangle a selection covers, which annotations are clamped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Bounds {
    pub fn from_selection(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            left: x,
            top: y,
            right: x + width as i32,
            bottom: y + height as i32,
        }
    }

    /// Whether the rectangle has any area.
    pub fn has_area(&self) -> bool {
        self.right > self.left && self.bottom > self.top
    }

    /// A point pulled inside the selection.
    ///
    /// The far edge is `right - 1`, because a shape drawn *at* `right` would be the
    /// first pixel of the dimmed area outside the selection.
    fn clamp(&self, x: i32, y: i32) -> (i32, i32) {
        (
            x.clamp(self.left, (self.right - 1).max(self.left)),
            y.clamp(self.top, (self.bottom - 1).max(self.top)),
        )
    }
}

/// Draw one stroke into a device context, in canvas coordinates.
pub fn draw_stroke(hdc: HDC, stroke: &Stroke, bounds: &Bounds, origin: (i32, i32)) {
    if !bounds.has_area() {
        return;
    }
    // A point in the stroke's own space becomes a point on the canvas, then is clamped
    // to the selection. The order matters: clamping first would compare coordinates from
    // two different spaces.
    let to_canvas = |point: Point| -> (i32, i32) {
        bounds.clamp(point.x + origin.0, point.y + origin.1)
    };

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
            // Open, not filled: the point of a box on a screenshot is to mark something,
            // and filling it would hide exactly what is being marked.
            //
            // `FrameRect` takes a brush rather than using the pen, so the stroke's
            // colour is made into one here and released with the pen.
            let (left, top, right, bottom) = normalized(to_canvas(*from), to_canvas(*to));
            let outline = RECT {
                left,
                top,
                right: right + 1,
                bottom: bottom + 1,
            };
            let brush = unsafe { CreateSolidBrush(colour) };
            if !brush.is_invalid() {
                unsafe {
                    FrameRect(hdc, &outline, brush);
                    let _ = DeleteObject(HGDIOBJ(brush.0));
                }
            }
        }
        Shape::Ellipse { from, to } => {
            let (left, top, right, bottom) = normalized(to_canvas(*from), to_canvas(*to));
            // `Ellipse` fills its interior with whatever brush the context holds, so a
            // hollow one is selected for the duration. Without this the ellipse would be
            // a solid disc covering what it was drawn around.
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

/// Draw the strip: its plate, its separators, and every button.
/// Draw the strip: its plate, its separators, and every button.
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
    unsafe {
        windows::Win32::Graphics::Gdi::FillRect(hdc, &body, plate);
    }
    // A one-pixel border, then a lighter inner line: the plate reads as a card rather
    // than as a hole in the capture, which matters because it sits next to one.
    unsafe {
        windows::Win32::Graphics::Gdi::FrameRect(hdc, &body, edge);
    }

    for (button, x, y, width, height) in metrics.button_rects() {
        let rect = RECT {
            left: placement.x + x,
            top: placement.y + y,
            right: placement.x + x + width as i32,
            bottom: placement.y + y + height as i32,
        };

        if button.starts_a_group() {
            // The separator goes in the gap above the button, which is why the metrics
            // reserve a wider gap there.
            let line = RECT {
                left: rect.left + 4,
                top: rect.top - metrics.group_gap / 2 - 1,
                right: rect.right - 4,
                bottom: rect.top - metrics.group_gap / 2,
            };
            unsafe {
                windows::Win32::Graphics::Gdi::FillRect(hdc, &line, edge);
            }
        }

        if state.active == Some(button) {
            unsafe {
                windows::Win32::Graphics::Gdi::FillRect(hdc, &rect, face);
                windows::Win32::Graphics::Gdi::FrameRect(hdc, &rect, edge);
            }
        }

        // An unavailable button keeps its glyph, drawn in a colour close to the plate: it
        // reads as present but not available, which is what it is. Hiding it would move
        // every button below it and make the strip a different shape from one moment to
        // the next.
        let ink = if state.is_disabled(button) { DIM_INK } else { BUTTON_INK };
        draw_button_glyph(hdc, button, &rect, ink);
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

/// A button's icon, drawn from lines so that it stays crisp at any scale.
///
/// `ink` colours both the lines and, where a shape needs it, the fill. A button that
/// cannot be used is drawn in a colour close to the plate: present, and visibly not
/// available.
fn draw_button_glyph(hdc: HDC, button: Button, rect: &RECT, ink: COLORREF) {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let centre = (rect.left + width / 2, rect.top + height / 2);
    // A quarter of the button's side: big enough to read, small enough to leave the
    // padding that makes the strip look uncrowded.
    let arm = (width.min(height) / 4).max(2);
    let pen = unsafe { CreatePen(PS_SOLID, 2, ink) };
    if pen.is_invalid() {
        return;
    }
    // The same colour as a brush, for the two glyphs that are framed rather than stroked.
    let brush = unsafe { CreateSolidBrush(ink) };
    let saved = unsafe { SelectObject(hdc, HGDIOBJ(pen.0)) };

    let line = |from: (i32, i32), to: (i32, i32)| unsafe {
        let _ = MoveToEx(hdc, from.0, from.1, None);
        let _ = LineTo(hdc, to.0, to.1);
    };

    match button {
        Button::Tool(super::annotation::Tool::Move) => {
            // Four small arrows: the universal "move" mark.
            line((centre.0 - arm, centre.1), (centre.0 + arm, centre.1));
            line((centre.0, centre.1 - arm), (centre.0, centre.1 + arm));
            line((centre.0 - arm, centre.1), (centre.0 - arm + 2, centre.1 - 2));
            line((centre.0 - arm, centre.1), (centre.0 - arm + 2, centre.1 + 2));
            line((centre.0 + arm, centre.1), (centre.0 + arm - 2, centre.1 - 2));
            line((centre.0 + arm, centre.1), (centre.0 + arm - 2, centre.1 + 2));
            line((centre.0, centre.1 - arm), (centre.0 - 2, centre.1 - arm + 2));
            line((centre.0, centre.1 - arm), (centre.0 + 2, centre.1 - arm + 2));
            line((centre.0, centre.1 + arm), (centre.0 - 2, centre.1 + arm - 2));
            line((centre.0, centre.1 + arm), (centre.0 + 2, centre.1 + arm - 2));
        }
        Button::Tool(super::annotation::Tool::Arrow) => {
            line(
                (centre.0 - arm, centre.1 + arm),
                (centre.0 + arm, centre.1 - arm),
            );
            let head = arm;
            line(
                (centre.0 + arm, centre.1 - arm),
                (centre.0 + arm - head, centre.1 - arm),
            );
            line(
                (centre.0 + arm, centre.1 - arm),
                (centre.0 + arm, centre.1 - arm + head),
            );
        }
        Button::Tool(super::annotation::Tool::Rectangle) => {
            let box_rect = RECT {
                left: centre.0 - arm,
                top: centre.1 - arm,
                right: centre.0 + arm + 1,
                bottom: centre.1 + arm + 1,
            };
            unsafe {
                let _ = Rectangle(hdc, box_rect.left, box_rect.top, box_rect.right, box_rect.bottom);
            }
        }
        Button::Tool(super::annotation::Tool::Ellipse) => {
            unsafe {
                let _ = Ellipse(
                    hdc,
                    centre.0 - arm,
                    centre.1 - arm,
                    centre.0 + arm + 1,
                    centre.1 + arm + 1,
                );
            }
        }
        Button::Tool(super::annotation::Tool::Pencil) => {
            line(
                (centre.0 - arm, centre.1 + arm),
                (centre.0 + arm, centre.1 - arm),
            );
            line(
                (centre.0 - arm, centre.1 + arm),
                (centre.0 - arm + 2, centre.1 + arm),
            );
        }
        Button::Colour => {
            // A filled disc: the colour itself is filled in by the caller's face
            // drawing, and this is its outline.
            unsafe {
                let _ = Ellipse(
                    hdc,
                    centre.0 - arm,
                    centre.1 - arm,
                    centre.0 + arm + 1,
                    centre.1 + arm + 1,
                );
            }
            line((centre.0 - arm, centre.1), (centre.0 + arm, centre.1));
        }
        Button::Thickness => {
            // Three lines of increasing weight, drawn as separate pens would be
            // expensive; two lines of different lengths read the same.
            line((centre.0 - arm, centre.1 - 2), (centre.0 + arm, centre.1 - 2));
            line((centre.0 - arm, centre.1 + 2), (centre.0 + arm, centre.1 + 2));
        }
        Button::Undo => {
            line((centre.0 + arm, centre.1 + arm), (centre.0 - arm, centre.1 + arm));
            line(
                (centre.0 - arm, centre.1 + arm),
                (centre.0 - arm + 2, centre.1 + arm - 2),
            );
            line(
                (centre.0 - arm, centre.1 + arm),
                (centre.0 - arm + 2, centre.1 + arm + 2),
            );
        }
        Button::Redo => {
            line((centre.0 - arm, centre.1 + arm), (centre.0 + arm, centre.1 + arm));
            line(
                (centre.0 + arm, centre.1 + arm),
                (centre.0 + arm - 2, centre.1 + arm - 2),
            );
            line(
                (centre.0 + arm, centre.1 + arm),
                (centre.0 + arm - 2, centre.1 + arm + 2),
            );
        }
        Button::Save => {
            let box_rect = RECT {
                left: centre.0 - arm,
                top: centre.1 - arm,
                right: centre.0 + arm + 1,
                bottom: centre.1 + arm + 1,
            };
            unsafe {
                windows::Win32::Graphics::Gdi::FrameRect(hdc, &box_rect, brush);
            }
            line((centre.0 - arm, centre.1 + arm), (centre.0 + arm, centre.1 - arm));
            line((centre.0 + arm, centre.1 - arm), (centre.0 + arm, centre.1));
            line((centre.0 + arm, centre.1), (centre.0 + arm - 2, centre.1 - 2));
        }
        Button::Copy => {
            let front = RECT {
                left: centre.0 - arm + 2,
                top: centre.1 - arm + 2,
                right: centre.0 + arm + 1,
                bottom: centre.1 + arm + 1,
            };
            unsafe {
                windows::Win32::Graphics::Gdi::FrameRect(hdc, &front, brush);
            }
            line((centre.0 - arm, centre.1 - arm), (centre.0 + arm - 2, centre.1 - arm));
            line((centre.0 - arm, centre.1 - arm), (centre.0 - arm, centre.1 + arm - 2));
        }
        Button::Cancel => {
            line((centre.0 - arm, centre.1 - arm), (centre.0 + arm, centre.1 + arm));
            line((centre.0 - arm, centre.1 + arm), (centre.0 + arm, centre.1 - arm));
        }
    }

    unsafe {
        SelectObject(hdc, saved);
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

    #[test]
    fn a_rectangle_is_drawn_inside_its_own_outline() {
        // The property that matters for every shape: whatever it draws is within the
        // bounding box its two points describe. Checked by counting what changed outside
        // that box, which is the failure a person would not see until a corner of the
        // drawing was over something it should not cover.
        let canvas = blank(120, 120);
        let bounds = Bounds::from_selection(0, 0, 120, 120);
        let stroke = Stroke::new(
            Shape::Rectangle {
                from: Point::new(20, 30),
                to: Point::new(100, 90),
            },
            Colour::Red,
            2,
        );
        draw_stroke(canvas.dc(), &stroke, &bounds, (0, 0));
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
            let bounds = Bounds::from_selection(0, 0, 120, 120);
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
                &bounds,
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
        let bounds = Bounds::from_selection(0, 0, 120, 120);
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
            &bounds,
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
    fn a_stroke_is_clamped_to_the_selection() {
        // A shape asked for outside the selection must not appear there: the area beyond
        // it is the dimmed capture, and a stroke over it would be drawn outside the
        // region the person is capturing.
        let canvas = blank(200, 200);
        let bounds = Bounds::from_selection(50, 50, 40, 40);
        draw_stroke(
            canvas.dc(),
            &Stroke::new(
                Shape::Rectangle {
                    from: Point::new(-100, -100),
                    to: Point::new(500, 500),
                },
                Colour::Red,
                2,
            ),
            &bounds,
            (0, 0),
        );
        unsafe {
            let _ = GdiFlush();
        }

        assert!(
            painted(&canvas, &RECT { left: 49, top: 49, right: 91, bottom: 91 }) > 0,
            "nothing was drawn inside the selection"
        );
        let mut strays = 0;
        for y in 0..200 {
            for x in 0..200 {
                let inside = x >= 49 && x < 91 && y >= 49 && y < 91;
                if !inside && pixel(&canvas, x, y) != (0, 0, 0) {
                    strays += 1;
                }
            }
        }
        assert_eq!(strays, 0, "the stroke reached outside the selection");
    }

    #[test]
    fn a_colour_reaches_the_canvas_as_the_colour_it_names() {
        // `COLORREF` is 0x00BBGGRR, so red and blue are the way round that surprises
        // people. Checked by reading the pixel back, because getting it wrong produces a
        // drawing in the wrong colour, which is easy to blame on anything else.
        let canvas = blank(60, 60);
        let bounds = Bounds::from_selection(0, 0, 60, 60);
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
            &bounds,
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
        let bounds = Bounds::from_selection(0, 0, 40, 40);
        draw_stroke(
            canvas.dc(),
            &Stroke::new(
                Shape::Pencil {
                    points: vec![Point::new(20, 20)],
                },
                Colour::Red,
                3,
            ),
            &bounds,
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
        let bounds = Bounds::from_selection(0, 0, 100, 100);
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
            &bounds,
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
        let bounds = Bounds::from_selection(0, 0, 40, 40);
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
            &bounds,
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
    fn nothing_is_drawn_into_a_selection_with_no_area() {
        // The degenerate case, which the model also guards. Two guards is correct here:
        // this one is the last thing between a stale coordinate and the canvas.
        let canvas = blank(40, 40);
        let bounds = Bounds::from_selection(10, 10, 0, 0);
        draw_stroke(
            canvas.dc(),
            &Stroke::new(
                Shape::Rectangle {
                    from: Point::new(0, 0),
                    to: Point::new(10, 10),
                },
                Colour::Red,
                2,
            ),
            &bounds,
            (0, 0),
        );
        unsafe {
            let _ = GdiFlush();
        }
        assert_eq!(
            painted(&canvas, &RECT { left: 0, top: 0, right: 40, bottom: 40 }),
            0,
            "something was drawn for a selection with no area"
        );
    }

    #[test]
    fn a_shape_is_placed_by_the_selection_origin() {
        // The conversion the whole module is about: the same stroke, drawn with two
        // different origins, must land in two different places — and each within its own
        // selection.
        let stroke = Stroke::new(
            Shape::Rectangle {
                from: Point::new(5, 5),
                to: Point::new(25, 25),
            },
            Colour::Red,
            2,
        );

        let first = blank(100, 100);
        draw_stroke(
            first.dc(),
            &stroke,
            &Bounds::from_selection(10, 10, 40, 40),
            (10, 10),
        );
        let second = blank(100, 100);
        draw_stroke(
            second.dc(),
            &stroke,
            &Bounds::from_selection(50, 50, 40, 40),
            (50, 50),
        );
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
            400,
            &metrics,
        );
        let plate = unsafe { CreateSolidBrush(COLORREF(0x0020_2020)) };
        let edge = unsafe { CreateSolidBrush(COLORREF(0x0060_6060)) };
        let face = unsafe { CreateSolidBrush(COLORREF(0x0040_4040)) };
        let state = StripState {
            active: Some(Button::Tool(Tool::Arrow)),
            can_undo: true,
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
        assert_eq!(
            covered, area,
            "the plate covered {covered} of its {area} pixels, so something shows through"
        );

        // And nothing outside it.
        let mut strays = 0;
        for y in 0..400 {
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
    fn an_unavailable_button_still_draws_something() {
        // Undo starts unavailable, and it has to look like a button rather than a hole:
        // hiding it would move every button below it, and the strip's shape would change
        // from one moment to the next.
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
                400,
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
                },
                plate,
                edge,
                face,
            );
            unsafe {
                let _ = GdiFlush();
            }

            // Counted by the ink's own colour. Counting "anything other than the plate"
            // was the first version of this, and it proved nothing: both inks differ from
            // the plate, so both states came out with the same number and the test passed
            // without telling them apart.
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
            // `COLORREF` is 0x00BBGGRR, and the DIB holds BGRA.
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
            "{button:?} drew nothing in the dim ink, so it looks like a hole rather than \
             a button that cannot be used"
        );
        assert!(
            available > 0,
            "{button:?} drew nothing in the bright ink while it could be used"
        );
    }
}
