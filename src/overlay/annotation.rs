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
    pub fn to_colorref(self) -> COLORREF {
        COLORREF(match self {
            Colour::Red => 0x0000_00FF,
            Colour::Yellow => 0x0000_FFFF,
            Colour::Green => 0x0000_FF00,
            Colour::Blue => 0x00FF_0000,
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

    /// The shape with everything outside `[0, width) x [0, height)` removed.
    ///
    /// Used when the selection is made smaller than what has been drawn on it. `None`
    /// means nothing of the shape is left, which is reported rather than guessed at so
    /// the caller can tell "clipped" from "gone".
    ///
    /// The two-corner shapes are clipped by moving the offending corner to the edge
    /// rather than by rejecting the shape: a rectangle dragged past the edge should stop
    /// at the edge, which is what a person expects. But a shape that is **entirely**
    /// outside on one side is dropped, because dragging it onto the edge would invent a
    /// mark where none was drawn.
    pub fn clipped(&self, width: u32, height: u32) -> Option<Shape> {
        // A selection with no area holds nothing. `saturating_sub` below would turn its
        // no-such-pixel bounds into a point at the origin, which is how a stroke that
        // should have vanished came back as a dot in the corner.
        if width == 0 || height == 0 {
            return None;
        }
        let max_x = (width - 1) as i32;
        let max_y = (height - 1) as i32;
        let clamp = |p: Point| Point::new(p.x.clamp(0, max_x), p.y.clamp(0, max_y));

        match self {
            // Both corners strictly past the same edge: there is nothing to keep.
            Shape::Arrow { from, to } => two_corners(*from, *to, max_x, max_y)
                .map(|(from, to)| Shape::Arrow { from, to }),
            Shape::Rectangle { from, to } => two_corners(*from, *to, max_x, max_y)
                .map(|(from, to)| Shape::Rectangle { from, to }),
            Shape::Ellipse { from, to } => two_corners(*from, *to, max_x, max_y)
                .map(|(from, to)| Shape::Ellipse { from, to }),
            Shape::Pencil { points } => {
                // The pen keeps the runs that are still inside, rather than dragging
                // every stray point to the edge: a line pulled back to the border would
                // draw a false segment along it.
                let runs = inside_runs(points, &clamp);
                if runs.is_empty() {
                    None
                } else {
                    Some(Shape::Pencil { points: runs })
                }
            }
        }
    }
}

/// Two corners reduced to the region, or `None` when the shape is entirely outside it.
///
/// "Entirely outside" means both corners on the same side of the same edge, which is the
/// only arrangement where clamping would move a mark that was never inside.
fn two_corners(from: Point, to: Point, max_x: i32, max_y: i32) -> Option<(Point, Point)> {
    let past_the_left = from.x < 0 && to.x < 0;
    let past_the_right = from.x > max_x && to.x > max_x;
    let above = from.y < 0 && to.y < 0;
    let below = from.y > max_y && to.y > max_y;
    if past_the_left || past_the_right || above || below {
        return None;
    }
    let clamp = |p: Point| Point::new(p.x.clamp(0, max_x), p.y.clamp(0, max_y));
    Some((clamp(from), clamp(to)))
}

/// The points that lie inside the region, with a corner point added where a run leaves
/// it.
///
/// A freehand line that goes out of the selection and comes back should keep the part
/// that went out up to the edge, not stop short of it: the difference is a visible gap
/// between where the line was drawn and where the edge is.
fn inside_runs(points: &[Point], clamp: &impl Fn(Point) -> Point) -> Vec<Point> {
    let inside = |p: &Point| clamp(*p) == *p;
    let mut kept: Vec<Point> = Vec::with_capacity(points.len());
    for point in points {
        if inside(point) {
            kept.push(*point);
        } else if kept.last().is_some_and(inside) {
            // Leaving: stop at the edge.
            kept.push(clamp(*point));
            kept.push(*point);
        } else {
            kept.push(*point);
        }
    }
    // Trim anything at the ends that is outside, so a run cannot begin or end off the
    // selection, where it would never be drawn anyway.
    let first = kept.iter().position(inside);
    let last = kept.iter().rposition(inside);
    match (first, last) {
        (Some(first), Some(last)) if first <= last => kept[first..=last].to_vec(),
        _ => Vec::new(),
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

    /// Everything outside a selection of this size removed.
    ///
    /// The redo stack is not clipped along with the strokes. It is a stack of things
    /// that have already been taken back, so clipping it would silently change what redo
    /// puts back; leaving it alone is the smaller surprise of the two.
    pub fn clip_to(&self, width: u32, height: u32) -> Annotations {
        Annotations {
            strokes: self
                .strokes
                .iter()
                .filter_map(|stroke| clip_stroke(stroke, width, height))
                .collect(),
            undone: self.undone.clone(),
        }
    }
}

impl Stroke {
    pub fn new(shape: Shape, colour: Colour, thickness: u32) -> Self {
        Self {
            shape,
            colour,
            thickness: thickness.max(1),
        }
    }
}

/// A stroke reduced to the region, or dropped when nothing of it is left.
fn clip_stroke(stroke: &Stroke, width: u32, height: u32) -> Option<Stroke> {
    stroke.shape.clipped(width, height).map(|shape| Stroke {
        shape,
        colour: stroke.colour,
        thickness: stroke.thickness,
    })
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
    fn a_point_outside_the_selection_keeps_its_offset() {
        // Clamping here would flatten a line that leaves the region onto its edge, so
        // the offset is kept and clipping is left to `clipped`, which knows the shape.
        let frame = Frame::new(100, 100, 50, 50);
        assert_eq!(frame.to_relative(90, 100), Point::new(-10, 0));
        assert_eq!(frame.to_relative(400, 400), Point::new(300, 300));
    }

    #[test]
    fn moving_the_selection_takes_the_drawing_with_it() {
        // The property the whole coordinate choice exists for: after the region moves,
        // a stroke sits at the same place inside it as before. Expressed as a
        // round trip, so it holds whatever the numbers are.
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
    fn shrinking_the_selection_pulls_a_rectangle_back_to_the_edge() {
        // A rectangle dragged past the edge stops at the edge rather than disappearing.
        let mut annotations = Annotations::new();
        annotations.add(Stroke::new(
            Shape::Rectangle {
                from: Point::new(5, 5),
                to: Point::new(500, 500),
            },
            Colour::Red,
            2,
        ));

        let clipped = annotations.clip_to(100, 60);
        assert_eq!(clipped.strokes().len(), 1);
        assert_eq!(
            clipped.strokes()[0].shape,
            Shape::Rectangle {
                from: Point::new(5, 5),
                to: Point::new(99, 59),
            }
        );
    }

    #[test]
    fn shrinking_the_selection_drops_a_stroke_that_falls_outside() {
        let mut annotations = Annotations::new();
        annotations.add(arrow((200, 200), (300, 300)));
        let clipped = annotations.clip_to(100, 100);
        assert!(
            clipped.strokes().is_empty(),
            "a stroke entirely outside should not be dragged onto the edge"
        );
    }

    #[test]
    fn a_pencil_is_cut_at_the_edge_rather_than_flattened_onto_it() {
        // The case that makes the pen different from the two-corner shapes: points
        // outside are dropped and the run is stopped at the border. Flattening them onto
        // the edge would draw a false segment along it.
        let mut annotations = Annotations::new();
        annotations.add(Stroke::new(
            Shape::Pencil {
                points: vec![
                    Point::new(1, 1),
                    Point::new(2, 2),
                    Point::new(50, 50),
                    Point::new(60, 60),
                ],
            },
            Colour::Blue,
            1,
        ));

        let clipped = annotations.clip_to(10, 10);
        let Shape::Pencil { points } = &clipped.strokes()[0].shape else {
            panic!("the pen changed kind while clipping");
        };
        assert_eq!(points[0], Point::new(1, 1));
        assert_eq!(points[1], Point::new(2, 2));
        // The border is where the run stops, and nothing is kept past it.
        assert!(
            points.iter().all(|p| p.x < 10 && p.y < 10),
            "a point outside the selection survived: {points:?}"
        );
    }

    #[test]
    fn growing_the_selection_leaves_the_drawing_alone() {
        let mut annotations = Annotations::new();
        annotations.add(arrow((10, 10), (20, 20)));
        let before = annotations.strokes().to_vec();
        let clipped = annotations.clip_to(1000, 1000);
        assert_eq!(clipped.strokes(), before.as_slice());
    }

    #[test]
    fn an_ellipse_is_clipped_like_the_rectangle_it_is_inscribed_in() {
        // An ellipse is carried by the same two corners as a rectangle, so it has to
        // behave the same way under clipping — including being dropped when it is
        // entirely outside, which is the case a shared implementation gets wrong.
        let mut annotations = Annotations::new();
        annotations.add(Stroke::new(
            Shape::Ellipse {
                from: Point::new(5, 5),
                to: Point::new(500, 500),
            },
            Colour::Yellow,
            2,
        ));
        let clipped = annotations.clip_to(100, 60);
        assert_eq!(
            clipped.strokes()[0].shape,
            Shape::Ellipse {
                from: Point::new(5, 5),
                to: Point::new(99, 59),
            }
        );

        let mut outside = Annotations::new();
        outside.add(Stroke::new(
            Shape::Ellipse {
                from: Point::new(200, 10),
                to: Point::new(300, 90),
            },
            Colour::Yellow,
            2,
        ));
        assert!(
            outside.clip_to(100, 100).strokes().is_empty(),
            "an ellipse entirely past the edge should not be pulled onto it"
        );
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
        assert_eq!(Colour::Red.to_colorref().0, 0x0000_00FF);
        assert_eq!(Colour::Blue.to_colorref().0, 0x00FF_0000);
        assert_eq!(Colour::Green.to_colorref().0, 0x0000_FF00);
        assert_eq!(Colour::White.to_colorref().0, 0x00FF_FFFF);
        assert_eq!(Colour::Yellow.to_colorref().0, 0x0000_FFFF);
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

    #[test]
    fn clipping_to_a_selection_one_pixel_across_is_not_a_panic() {
        // The degenerate size, which `saturating_sub` is there for.
        let mut annotations = Annotations::new();
        annotations.add(arrow((0, 0), (50, 50)));
        let clipped = annotations.clip_to(1, 1);
        assert_eq!(clipped.strokes().len(), 1);
        assert_eq!(
            clipped.strokes()[0].shape,
            Shape::Arrow {
                from: Point::new(0, 0),
                to: Point::new(0, 0),
            }
        );
        // And a selection with no size at all keeps nothing.
        assert!(annotations.clip_to(0, 0).strokes().is_empty());
    }
}
