//! What a person draws on a capture, and where.
//!
//! This module holds the annotations and nothing else: no device context, no window,
//! no drawing. That split is deliberate, and it is the lesson from the rotation work —
//! the arithmetic of a feature can be settled with tests long before anything on screen
//! can be checked, and a mistake found while drawing is a mistake found late.
//!
//! # The space annotations live in
//!
//! Every coordinate here is **relative to the selection's top-left corner**. Not to the
//! screen, and not to the capture.
//!
//! That choice decides what happens when the selection is moved after something has
//! been drawn. Anchored to the selection, the drawing goes with it: the arrow stays
//! pointing at the thing it was drawn to point at. Anchored to the screen, moving the
//! selection would leave the arrow behind, pointing at whatever happened to end up
//! underneath, which is worse than losing it. So the drawing belongs to the region, and
//! [`Annotations::clip_to`] handles the case where the region is made smaller than the
//! drawing.
//!
//! # The frame origin
//!
//! Callers work in screen coordinates — the overlay sees the pointer's position on the
//! desktop — and [`Frame::to_relative`] and [`Frame::to_screen`] are the only places
//! that conversion happens. Keeping it in one pair of functions is what stops a
//! coordinate from being converted twice, or not at all.

use windows::Win32::Foundation::COLORREF;

/// Which tool a drag uses.
///
/// [`Tool::Move`] is the default and is not a way of drawing: it exists so that
/// dragging inside the selection can still move it. Without it, choosing a drawing tool
/// would take away the ability to adjust the region, and there would be no way back
/// that a person could discover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Move,
    Arrow,
    Rectangle,
    Ellipse,
    Pencil,
}

impl Tool {
    /// Every tool. The strip lists buttons rather than tools, so this is only walked by
    /// the test that checks each tool agrees about whether it draws.
    #[cfg(test)]
    pub const ALL: [Tool; 5] = [
        Tool::Move,
        Tool::Arrow,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Pencil,
    ];

    /// Whether a drag with this tool produces a stroke.
    pub fn draws(self) -> bool {
        !matches!(self, Tool::Move)
    }
}

/// The colours a stroke can be, cycled through with one key.
///
/// A fixed set rather than a colour picker: a capture is annotated in a hurry, and
/// choosing from five colours is one keypress where a picker is a dialogue. The set is
/// chosen to be readable on both light and dark screenshots, which is why it holds no
/// pale grey and no mid-tone that could vanish into either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Colour {
    #[default]
    Red,
    Yellow,
    Green,
    Blue,
    White,
}

impl Colour {
    /// Every colour, in the order the key cycles them.
    pub const ALL: [Colour; 5] = [
        Colour::Red,
        Colour::Yellow,
        Colour::Green,
        Colour::Blue,
        Colour::White,
    ];

/// The next colour in the cycle, wrapping.
    pub fn next(self) -> Colour {
        let index = Colour::ALL.iter().position(|&c| c == self).unwrap_or(0);
        Colour::ALL[(index + 1) % Colour::ALL.len()]
    }

    /// The value GDI wants, which is `0x00BBGGRR`.
    ///
    /// A palette rather than the pure hues: red, yellow, green and blue at full saturation
    /// are the colours of a diagram, and on a photograph they shout. These are muted enough
    /// to mark something without taking the eye off it, and still four clearly different
    /// colours. White stays pure, because it is the one that has to read on dark content.
    pub fn to_colorref(self) -> COLORREF {
        COLORREF(match self {
            Colour::Red => 0x0044_44EF,
            Colour::Yellow => 0x0015_CCFA,
            Colour::Green => 0x005E_C522,
            Colour::Blue => 0x00F6_823B,
            Colour::White => 0x00FF_FFFF,
        })
    }
}

    /// The stroke widths on offer, thinnest first.
///
/// The thinnest is four and not two: GDI draws a filled ellipse three pixels across as
/// nothing at all, so a two-pixel dot in a palette of dots is an empty cell, and the eye
/// cannot tell two from four in a row anyway.
///
/// A list rather than an arithmetic progression, because the widths have to be told apart by
/// eye in a row of dots: two and four are different, four and six barely are, and twelve and
/// twenty-four are different again. The gaps widen as the width grows, which is how the eye
/// orders them. `+` and `-` move through this same list, so the keyboard and the palette never
/// disagree about what widths exist.
pub const THICKNESS_STEPS: [u32; 6] = [4, 6, 8, 12, 18, 24];

/// Where a width sits in the list, or the nearest place for one that is not in it.
pub fn thickness_index(thickness: u32) -> usize {
    let mut best = 0;
    let mut best_gap = u32::MAX;
    for (index, step) in THICKNESS_STEPS.iter().enumerate() {
        let gap = step.abs_diff(thickness);
        if gap < best_gap {
            best = index;
            best_gap = gap;
        }
    }
    best
}

/// The next width in the list, wrapping.
pub fn next_thickness(thickness: u32) -> u32 {
    THICKNESS_STEPS[(thickness_index(thickness) + 1) % THICKNESS_STEPS.len()]
}

/// The previous width in the list, wrapping.
pub fn previous_thickness(thickness: u32) -> u32 {
    let index = thickness_index(thickness);
    THICKNESS_STEPS[(index + THICKNESS_STEPS.len() - 1) % THICKNESS_STEPS.len()]
}


/// A point in selection-relative pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// One thing drawn in the selection.
///
/// The shapes carry their own geometry rather than a list of points, so that undo and
/// redo can keep a stroke intact and the drawing code does not have to guess what a
/// shape was meant to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    /// A line with a head at its second point, drawn from the two ends.
    Arrow { from: Point, to: Point },
    /// A rectangle between two opposite corners, normalised so the second may be any
    /// corner.
    Rectangle { from: Point, to: Point },
    /// An ellipse inscribed in the rectangle between two opposite corners.
    Ellipse { from: Point, to: Point },
    /// A freehand line. Always at least one point; a press with no movement still
    /// leaves a mark where it was pressed.
    Pencil { points: Vec<Point> },
}

impl Shape {
    /// The shape moved by an offset.
    pub fn moved(&self, dx: i32, dy: i32) -> Shape {
        let shift = |p: Point| Point::new(p.x + dx, p.y + dy);
        match self {
            Shape::Arrow { from, to } => Shape::Arrow {
                from: shift(*from),
                to: shift(*to),
            },
            Shape::Rectangle { from, to } => Shape::Rectangle {
                from: shift(*from),
                to: shift(*to),
            },
            Shape::Ellipse { from, to } => Shape::Ellipse {
                from: shift(*from),
                to: shift(*to),
            },
            Shape::Pencil { points } => Shape::Pencil {
                points: points.iter().copied().map(shift).collect(),
            },
        }
    }

}

/// One stroke: a shape, with the colour and thickness it was drawn in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stroke {
    pub shape: Shape,
    pub colour: Colour,
    pub thickness: u32,
}

/// What has been drawn on the selection, and how to take it back.
///
/// Undo and redo are two stacks of whole strokes. That is enough because a stroke is
/// only ever added or removed as a unit — nothing here edits a stroke in place — and it
/// keeps the meaning of "undo" obvious: the last thing drawn goes away.
///
/// Holding the strokes themselves rather than indices means the stacks cannot fall out
/// of step with the list, which is the usual way this goes wrong.
#[derive(Debug, Default, Clone)]
pub struct Annotations {
    strokes: Vec<Stroke>,
    undone: Vec<Stroke>,
}

impl Annotations {
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything drawn, oldest first.
    pub fn strokes(&self) -> &[Stroke] {
        &self.strokes
    }

    /// Whether there is anything for undo to take back.
    pub fn can_undo(&self) -> bool {
        !self.strokes.is_empty()
    }

    /// Whether there is anything for redo to put back.
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Add a stroke.
    ///
    /// Adding discards the redo stack, which is what every editor does and what a person
    /// expects: drawing something new after undoing means the undone strokes are not
    /// coming back.
    pub fn add(&mut self, stroke: Stroke) {
        self.undone.clear();
        self.strokes.push(stroke);
    }

    /// Take back the last stroke. Returns whether anything was taken back.
    pub fn undo(&mut self) -> bool {
        match self.strokes.pop() {
            Some(stroke) => {
                self.undone.push(stroke);
                true
            }
            None => false,
        }
    }

    /// Put back the last stroke that was taken back.
    pub fn redo(&mut self) -> bool {
        match self.undone.pop() {
            Some(stroke) => {
                self.strokes.push(stroke);
                true
            }
            None => false,
        }
    }

    /// Move everything by an offset, in selection-relative pixels.
    ///
    /// Used when the selection itself moves, so the drawing travels with the region.
    pub fn moved(&self, dx: i32, dy: i32) -> Annotations {
        Annotations {
            strokes: self.strokes.iter().map(|s| s.moved(dx, dy)).collect(),
            undone: self.undone.iter().map(|s| s.moved(dx, dy)).collect(),
        }
    }
    /// The same drawing, re-based so it stays where it is on the screen.
    ///
    /// The region's origin has moved by `delta`, and these coordinates are relative to it, so
    /// each one has to move by the opposite amount to keep the mark on the pixel it marks. The
    /// sign is the whole of it, and it is here rather than in the window handler because a sign
    /// in a handler is a sign no test can be wrong about.
    pub fn rebased(&self, delta: (i32, i32)) -> Annotations {
        self.moved(-delta.0, -delta.1)
    }

}

impl Stroke {


    /// Where a stroke lies on the canvas, as left, top, right and bottom.
    ///
    /// Padded by the stroke's own width, because a thick line reaches half its width to each
    /// side of the points it is drawn between, and by one more pixel for the pen's corner.
    pub fn canvas_extent(&self, origin: (i32, i32)) -> (i32, i32, i32, i32) {
        let pad = self.thickness as i32 + 1;
        let shift = |p: Point| (p.x + origin.0, p.y + origin.1);
        let points: Vec<(i32, i32)> = match &self.shape {
            Shape::Arrow { from, to }
            | Shape::Rectangle { from, to }
            | Shape::Ellipse { from, to } => vec![shift(*from), shift(*to)],
            Shape::Pencil { points } => points.iter().copied().map(shift).collect(),
        };
        let mut left = i32::MAX;
        let mut top = i32::MAX;
        let mut right = i32::MIN;
        let mut bottom = i32::MIN;
        for (x, y) in points {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
        if left > right || top > bottom {
            return (0, 0, 0, 0);
        }
        (left - pad, top - pad, right + pad, bottom + pad)
    }

    pub fn new(shape: Shape, colour: Colour, thickness: u32) -> Self {
        Self {
            shape,
            colour,
            thickness: thickness.max(1),
        }
    }
}

/// Move a stroke, keeping its colour and thickness.
impl Stroke {
    fn moved(&self, dx: i32, dy: i32) -> Stroke {
        Stroke {
            shape: self.shape.moved(dx, dy),
            colour: self.colour,
            thickness: self.thickness,
        }
    }
}

/// The selection, as the overlay knows it: an origin on the desktop and a size.
///
/// The conversion between screen and relative coordinates lives here and nowhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Frame {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// A point on the desktop, as a point in the selection.
    ///
    /// Points outside the selection keep their offset rather than being clamped: a drag
    /// that leaves the region and comes back should come back to where the pointer is,
    /// and clamping here would flatten the line onto the edge.
    pub fn to_relative(&self, x: i32, y: i32) -> Point {
        Point::new(x - self.x, y - self.y)
    }

    /// A point in the selection, back on the desktop.
    ///
    /// Only the round-trip test uses this: the program converts one way, from screen to
    /// relative, and a second implementation of the other direction would be one more
    /// thing to keep in step.
    #[cfg(test)]
    pub fn to_screen(&self, point: Point) -> (i32, i32) {
        (point.x + self.x, point.y + self.y)
    }

    /// Whether a point on the desktop is inside the selection. Test-only, for the same
    /// reason as `to_screen`: the overlay has its own test of the same thing.
    #[cfg(test)]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x + self.width as i32
            && y < self.y + self.height as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrow(from: (i32, i32), to: (i32, i32)) -> Stroke {
        Stroke::new(
            Shape::Arrow {
                from: Point::new(from.0, from.1),
                to: Point::new(to.0, to.1),
            },
            Colour::Red,
            2,
        )
    }

    #[test]
    fn a_frame_converts_both_ways_without_drifting() {
        // The conversion is the one thing every other operation depends on, so it is
        // checked over a frame whose origin is not zero and whose size is not square:
        // an origin of (0, 0) hides a sign mistake, and a square hides an axis swap.
        let frame = Frame::new(-300, 120, 640, 480);
        for x in [-300, -299, 0, 339, 340] {
            for y in [120, 121, 0, 599, 600] {
                let relative = frame.to_relative(x, y);
                assert_eq!(frame.to_screen(relative), (x, y), "screen ({x}, {y})");
            }
        }
    }

    #[test]
    fn a_frame_knows_what_is_inside_it() {
        let frame = Frame::new(100, 50, 10, 4);
        assert!(frame.contains(100, 50), "its own top-left corner");
        assert!(frame.contains(109, 53), "its own bottom-right pixel");
        assert!(!frame.contains(99, 50), "one to the left");
        assert!(!frame.contains(100, 49), "one above");
        assert!(!frame.contains(110, 50), "one past the right edge");
        assert!(!frame.contains(100, 54), "one past the bottom edge");
    }

    #[test]
    fn a_strokes_extent_covers_where_it_is_drawn_and_its_width() {
        // The extent is what the overlay repaints while a stroke is being drawn. Too small and
        // the outside part of a mark is written into the buffer and never shown; the report was
        // a box that came out in fragments until Escape repainted everything.
        let stroke = Stroke::new(
            Shape::Arrow {
                from: Point::new(-40, -10),
                to: Point::new(60, 30),
            },
            Colour::Red,
            6,
        );
        let (left, top, right, bottom) = stroke.canvas_extent((100, 200));

        // The points, in canvas coordinates, then padded by the width.
        assert!(left <= 60 - 7, "the extent starts right of the stroke: {left}");
        assert!(top <= 190 - 7, "the extent starts below the stroke: {top}");
        assert!(right >= 160 + 7, "the extent ends left of the stroke: {right}");
        assert!(bottom >= 230 + 7, "the extent ends above the stroke: {bottom}");

        // And it is a rectangle, not a point.
        assert!(right > left && bottom > top);
    }

    #[test]
    fn a_strokes_extent_follows_the_origin() {
        // A stroke drawn over the dimmed area has negative offsets; its extent is still a real
        // rectangle on the canvas, which is the whole point of measuring it.
        let stroke = Stroke::new(
            Shape::Pencil {
                points: vec![Point::new(-100, -80), Point::new(-60, -40)],
            },
            Colour::Blue,
            4,
        );
        let (left, top, right, bottom) = stroke.canvas_extent((500, 500));
        assert!(left < 400 && top < 420, "outside the canvas: {left},{top}");
        assert!(right > 440 && bottom > 460, "the extent misses the stroke: {right},{bottom}");
    }

    #[test]
    fn a_mark_stays_on_the_screen_when_the_region_moves() {
        // The report: paint something, drag the region, and the painting is gone. Coordinates
        // are held relative to the region's origin, and the origin moves with the drag, so a mark
        // shifted the same way as the region travels twice as far as it and leaves the frame. It
        // has to be shifted the opposite way to stay put, and `rebased` is what the overlay
        // calls, so this exercises the code that runs rather than repeating its arithmetic.
        let frame = Frame::new(100, 100, 200, 200);
        let mut annotations = Annotations::new();
        annotations.add(Stroke::new(
            Shape::Rectangle {
                from: Point::new(10, 10),
                to: Point::new(60, 60),
            },
            Colour::Red,
            2,
        ));
        let corner = Point::new(10, 10);
        let on_screen_before = frame.to_screen(corner);

        let delta = (30, 20);
        let moved_frame = Frame::new(
            frame.x + delta.0,
            frame.y + delta.1,
            frame.width,
            frame.height,
        );
        let after = annotations.rebased(delta);

        let stroke = &after.strokes()[0];
        let corner_now = match &stroke.shape {
            Shape::Rectangle { from, .. } => *from,
            other => panic!("the shape changed: {other:?}"),
        };
        assert_eq!(
            moved_frame.to_screen(corner_now),
            on_screen_before,
            "the mark moved on the screen when only the region was dragged"
        );
    }

    #[test]
    fn a_mark_may_sit_outside_the_region_and_stays_there() {
        // Drawing is not confined to the region any more: a mark made over the dimmed area is
        // held at a negative offset from the origin, which is a coordinate and not an error. What
        // saves only the inside is the crop, so nothing here may throw the outside part away.
        let mut annotations = Annotations::new();
        annotations.add(Stroke::new(
            Shape::Arrow {
                from: Point::new(-30, -20),
                to: Point::new(80, 70),
            },
            Colour::Green,
            4,
        ));

        // A move does not discard it, however far outside it is.
        let moved = annotations.rebased((120, 90));
        assert_eq!(
            moved.strokes().len(),
            1,
            "a mark outside the region was discarded by moving the region"
        );
        let Shape::Arrow { from, to } = moved.strokes()[0].shape else {
            panic!("the shape changed");
        };
        // The whole stroke is still there, both ends, at the offsets the move implies.
        assert_eq!(from, Point::new(-150, -110));
        assert_eq!(to, Point::new(-40, -20));
    }

    #[test]
    fn moving_the_drawing_and_moving_it_back_is_a_no_op() {
        // A round trip, so it holds whatever the numbers are. It used to be called "moving the
        // selection takes the drawing with it", which described the coordinates rather than
        // what anybody wants to happen: a mark stays on the screen, and the shift exists to
        // keep it there as the origin moves under it.
        let annotations = {
            let mut a = Annotations::new();
            a.add(arrow((10, 20), (30, 40)));
            a
        };

        let moved = annotations.moved(7, -3);
        let back = moved.moved(-7, 3);
        assert_eq!(back.strokes(), annotations.strokes(), "a move and back is a no-op");
    }

    #[test]
    fn a_pencil_keeps_every_point_when_it_moves() {
        let mut annotations = Annotations::new();
        annotations.add(Stroke::new(
            Shape::Pencil {
                points: vec![Point::new(0, 0), Point::new(1, 1), Point::new(2, 4)],
            },
            Colour::Green,
            3,
        ));
        let moved = annotations.moved(5, 5);
        let Shape::Pencil { points } = &moved.strokes()[0].shape else {
            panic!("the shape changed kind while moving");
        };
        assert_eq!(
            points,
            &vec![Point::new(5, 5), Point::new(6, 6), Point::new(7, 9)]
        );
    }

    #[test]
    fn undo_and_redo_walk_the_strokes_in_order() {
        let mut annotations = Annotations::new();
        annotations.add(arrow((0, 0), (1, 1)));
        annotations.add(arrow((2, 2), (3, 3)));
        annotations.add(arrow((4, 4), (5, 5)));
        assert_eq!(annotations.strokes().len(), 3);

        assert!(annotations.undo());
        assert_eq!(annotations.strokes().len(), 2);
        assert_eq!(annotations.strokes()[1].shape, Shape::Arrow {
            from: Point::new(2, 2),
            to: Point::new(3, 3)
        });

        assert!(annotations.undo());
        assert!(annotations.undo());
        assert!(!annotations.undo(), "nothing left to take back");
        assert!(annotations.strokes().is_empty());

        assert!(annotations.redo());
        assert_eq!(annotations.strokes().len(), 1, "the oldest comes back first");
        assert!(annotations.redo());
        assert!(annotations.redo());
        assert!(!annotations.redo(), "nothing left to put back");
        assert_eq!(annotations.strokes().len(), 3);
    }

    #[test]
    fn drawing_after_an_undo_drops_what_was_undone() {
        // What every editor does, and what a person expects: the undone strokes are not
        // coming back once something new is drawn.
        let mut annotations = Annotations::new();
        annotations.add(arrow((0, 0), (1, 1)));
        annotations.add(arrow((2, 2), (3, 3)));
        assert!(annotations.undo());
        assert!(annotations.can_redo());

        annotations.add(arrow((9, 9), (9, 9)));
        assert!(!annotations.can_redo(), "redo was not discarded");
        assert_eq!(annotations.strokes().len(), 2);
    }

    #[test]
    fn undoing_everything_and_redoing_it_all_comes_back_identical() {
        let mut annotations = Annotations::new();
        for i in 0..5 {
            annotations.add(arrow((i, i), (i + 1, i + 2)));
        }
        let original = annotations.strokes().to_vec();

        while annotations.undo() {}
        while annotations.redo() {}
        assert_eq!(annotations.strokes(), original.as_slice());
    }

    #[test]
    fn a_colour_cycle_returns_to_where_it_started() {
        let mut colour = Colour::Red;
        for _ in 0..Colour::ALL.len() {
            colour = colour.next();
        }
        assert_eq!(colour, Colour::Red, "the cycle did not close");
        // And it actually moves, rather than being the identity.
        assert_ne!(Colour::Red.next(), Colour::Red);
    }

    #[test]
    fn a_colour_is_worth_what_gdi_expects() {
        // GDI's `COLORREF` is 0x00BBGGRR, so red and blue are the way round that
        // surprises people. Checked explicitly because getting it wrong produces a
        // capture that is the right drawing in the wrong colour, which is easy to
        // attribute to anything else.
        assert_eq!(
            Colour::Red.to_colorref().0,
            0x0044_44EF,
            "#EF4444 must stay the colour it is documented as"
        );
        assert_eq!(
            Colour::Yellow.to_colorref().0,
            0x0015_CCFA,
            "#FACC15 must stay the colour it is documented as"
        );
        assert_eq!(
            Colour::Green.to_colorref().0,
            0x005E_C522,
            "#22C55E must stay the colour it is documented as"
        );
        assert_eq!(
            Colour::Blue.to_colorref().0,
            0x00F6_823B,
            "#3B82F6 must stay the colour it is documented as"
        );
        assert_eq!(
            Colour::White.to_colorref().0,
            0x00FF_FFFF,
            "#FFFFFF must stay the colour it is documented as"
        );
    }

    #[test]
    fn a_thickness_is_never_zero() {
        // A zero width would draw nothing and leave a stroke that looks like a bug.
        let stroke = Stroke::new(
            Shape::Arrow {
                from: Point::new(0, 0),
                to: Point::new(1, 1),
            },
            Colour::Red,
            0,
        );
        assert_eq!(stroke.thickness, 1);
    }

    #[test]
    fn only_the_move_tool_refuses_to_draw() {
        assert!(!Tool::Move.draws());
        for tool in Tool::ALL.iter().filter(|t| **t != Tool::Move) {
            assert!(tool.draws(), "{tool:?} should draw");
        }
    }

}
