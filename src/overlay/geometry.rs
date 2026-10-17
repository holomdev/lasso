//! Pure geometry for the selection overlay.
//!
//! Kept apart from the window code so hit testing and resize maths can be
//! tested without ever opening a window. Everything here works on `RECT` in
//! client coordinates.

use windows::Win32::Foundation::RECT;

/// Tolerance, in pixels, for grabbing a selection edge.
pub const TOLERANCE: i32 = 5;

/// Drawn size, in pixels, of a resize handle.
pub const HANDLE: i32 = 6;

/// Which sides of a rectangle a drag moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub left: bool,
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
}

impl Edges {
    pub fn any(self) -> bool {
        self.left || self.top || self.right || self.bottom
    }

    /// True when the edges form a corner, which needs a diagonal cursor.
    pub fn diagonal(self) -> Option<Diagonal> {
        match (self.left || self.right, self.top || self.bottom) {
            (true, true) => {
                if (self.left && self.top) || (self.right && self.bottom) {
                    Some(Diagonal::NwSe)
                } else {
                    Some(Diagonal::NeSw)
                }
            }
            _ => None,
        }
    }
}

/// The two diagonal orientations of a resize cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Diagonal {
    NwSe,
    NeSw,
}

/// What a mouse press does to the current selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// Grabbed an edge or a corner; only these sides move.
    Resize(Edges),
    /// Pressed inside the selection; the whole rectangle moves.
    Move,
    /// Pressed away from the selection; a new rectangle starts here.
    Outside,
}

/// Classify a press `point` against `rect`.
pub fn zone_of(rect: &RECT, point: (i32, i32)) -> Zone {
    let (x, y) = point;
    let near_left = (x - rect.left).abs() <= TOLERANCE;
    let near_right = (x - rect.right).abs() <= TOLERANCE;
    let near_top = (y - rect.top).abs() <= TOLERANCE;
    let near_bottom = (y - rect.bottom).abs() <= TOLERANCE;
    let within_x = x >= rect.left - TOLERANCE && x <= rect.right + TOLERANCE;
    let within_y = y >= rect.top - TOLERANCE && y <= rect.bottom + TOLERANCE;

    if within_x && within_y {
        let edges = Edges {
            left: near_left,
            top: near_top,
            right: near_right,
            bottom: near_bottom,
        };
        if edges.any() {
            return Zone::Resize(edges);
        }
        if x > rect.left && x < rect.right && y > rect.top && y < rect.bottom {
            return Zone::Move;
        }
    }
    Zone::Outside
}

/// Rectangle spanned by two points, order independent.
pub fn rectangle(a: (i32, i32), b: (i32, i32)) -> RECT {
    RECT {
        left: a.0.min(b.0),
        top: a.1.min(b.1),
        right: a.0.max(b.0),
        bottom: a.1.max(b.1),
    }
}

/// Same rectangle with positive extents.
pub fn normalized(r: RECT) -> RECT {
    rectangle((r.left, r.top), (r.right, r.bottom))
}

/// Move `start` so that the grabbed offset stays under `point`.
pub fn moved(start: &RECT, grab: (i32, i32), point: (i32, i32)) -> RECT {
    let width = start.right - start.left;
    let height = start.bottom - start.top;
    let left = point.0 - grab.0;
    let top = point.1 - grab.1;
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

/// Drag the selected `edges` to `point`, normalizing when an edge crosses the
/// opposite one.
pub fn resized(start: &RECT, edges: Edges, point: (i32, i32)) -> RECT {
    let mut r = *start;
    if edges.left {
        r.left = point.0;
    }
    if edges.right {
        r.right = point.0;
    }
    if edges.top {
        r.top = point.1;
    }
    if edges.bottom {
        r.bottom = point.1;
    }
    normalized(r)
}

/// Part of `r` that lies inside a `width` x `height` canvas.
pub fn intersect(r: &RECT, width: u32, height: u32) -> RECT {
    let max_x = width as i32;
    let max_y = height as i32;
    RECT {
        left: r.left.max(0).min(max_x),
        top: r.top.max(0).min(max_y),
        right: r.right.max(0).min(max_x),
        bottom: r.bottom.max(0).min(max_y),
    }
}

pub fn union(a: RECT, b: RECT) -> RECT {
    RECT {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }
}

pub fn inflated(r: &RECT, by: i32) -> RECT {
    RECT {
        left: r.left - by,
        top: r.top - by,
        right: r.right + by,
        bottom: r.bottom + by,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn press_inside_moves_and_press_away_creates() {
        let r = rect(100, 100, 300, 200);
        assert_eq!(zone_of(&r, (200, 150)), Zone::Move);
        assert_eq!(zone_of(&r, (20, 20)), Zone::Outside);
    }

    #[test]
    fn press_near_an_edge_or_corner_resizes() {
        let r = rect(100, 100, 300, 200);

        assert_eq!(
            zone_of(&r, (100, 150)),
            Zone::Resize(Edges {
                left: true,
                ..Default::default()
            })
        );
        assert_eq!(
            zone_of(&r, (300, 150)),
            Zone::Resize(Edges {
                right: true,
                ..Default::default()
            })
        );
        assert_eq!(
            zone_of(&r, (200, 100)),
            Zone::Resize(Edges {
                top: true,
                ..Default::default()
            })
        );
        assert_eq!(
            zone_of(&r, (200, 200)),
            Zone::Resize(Edges {
                bottom: true,
                ..Default::default()
            })
        );
        assert_eq!(
            zone_of(&r, (100, 100)),
            Zone::Resize(Edges {
                left: true,
                top: true,
                ..Default::default()
            })
        );
        assert_eq!(
            zone_of(&r, (300, 200)),
            Zone::Resize(Edges {
                right: true,
                bottom: true,
                ..Default::default()
            })
        );
    }

    #[test]
    fn the_tolerance_band_around_an_edge_still_counts_as_that_edge() {
        let r = rect(100, 100, 300, 200);
        // Two pixels outside the left edge, well inside the band.
        assert_eq!(
            zone_of(&r, (98, 150)),
            Zone::Resize(Edges {
                left: true,
                ..Default::default()
            })
        );
        // One pixel beyond the band is no longer a hit.
        assert_eq!(zone_of(&r, (100 - TOLERANCE - 1, 150)), Zone::Outside);
    }

    #[test]
    fn corners_report_diagonal_cursors() {
        let nw = Edges {
            left: true,
            top: true,
            ..Default::default()
        };
        let ne = Edges {
            right: true,
            top: true,
            ..Default::default()
        };
        let se = Edges {
            right: true,
            bottom: true,
            ..Default::default()
        };
        assert_eq!(nw.diagonal(), Some(Diagonal::NwSe));
        assert_eq!(se.diagonal(), Some(Diagonal::NwSe));
        assert_eq!(ne.diagonal(), Some(Diagonal::NeSw));
        assert_eq!(
            Edges {
                left: true,
                ..Default::default()
            }
            .diagonal(),
            None
        );
    }

    #[test]
    fn resizing_moves_only_the_selected_edges() {
        let start = rect(100, 100, 300, 200);
        let edges = Edges {
            right: true,
            bottom: true,
            ..Default::default()
        };
        assert_eq!(resized(&start, edges, (400, 260)), rect(100, 100, 400, 260));
    }

    #[test]
    fn dragging_an_edge_past_its_opposite_flips_the_rectangle() {
        let start = rect(100, 100, 300, 200);

        let flipped = resized(
            &start,
            Edges {
                left: true,
                ..Default::default()
            },
            (350, 150),
        );
        assert_eq!(flipped, rect(300, 100, 350, 200), "still ordered left < right");

        let flipped_vertical = resized(
            &start,
            Edges {
                top: true,
                ..Default::default()
            },
            (200, 260),
        );
        assert_eq!(flipped_vertical, rect(100, 200, 300, 260));
    }

    #[test]
    fn moving_keeps_the_size_and_the_grabbed_offset() {
        let start = rect(100, 100, 300, 200);
        // Grabbed 10 px inside from the left and 20 px down from the top.
        let moved_rect = moved(&start, (10, 20), (500, 400));
        assert_eq!(moved_rect, rect(490, 380, 690, 480));
        assert_eq!(moved_rect.right - moved_rect.left, start.right - start.left);
        assert_eq!(moved_rect.bottom - moved_rect.top, start.bottom - start.top);
    }

    #[test]
    fn intersecting_clamps_to_the_canvas() {
        assert_eq!(
            intersect(&rect(-50, -50, 100, 100), 1920, 1080),
            rect(0, 0, 100, 100)
        );
        assert_eq!(
            intersect(&rect(1800, 1000, 2200, 1200), 1920, 1080),
            rect(1800, 1000, 1920, 1080)
        );
        // Entirely outside collapses to zero extents, which the caller rejects.
        let gone = intersect(&rect(2000, 2000, 2100, 2100), 1920, 1080);
        assert_eq!(gone.right - gone.left, 0);
        assert_eq!(gone.bottom - gone.top, 0);
    }

    #[test]
    fn rectangle_ignores_point_order() {
        assert_eq!(rectangle((300, 200), (100, 100)), rect(100, 100, 300, 200));
        assert_eq!(rectangle((100, 100), (300, 200)), rect(100, 100, 300, 200));
    }

}
