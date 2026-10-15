//! Pure geometry for the selection overlay.
//!
//! Kept apart from the window code so hit testing, resize maths and the
//! magnifier mapping can be tested without ever opening a window. Everything
//! here works on `RECT` in client coordinates.

use windows::Win32::Foundation::RECT;

/// Tolerance, in pixels, for grabbing a selection edge.
pub const TOLERANCE: i32 = 5;

/// Drawn size, in pixels, of a resize handle.
pub const HANDLE: i32 = 6;

// ---------------------------------------------------------------------------
// Magnifier
// ---------------------------------------------------------------------------

/// How far each source pixel is scaled up inside the magnifier.
pub const MAGNIFIER_ZOOM: i32 = 8;
/// Outer size of the magnifier panel, in client pixels.
pub const MAGNIFIER_PANEL: i32 = 120;
/// Frame thickness: one black pixel, then white.
pub const MAGNIFIER_BORDER: i32 = 4;
/// Distance kept between the pointer and the panel.
pub const MAGNIFIER_GAP: i32 = 20;

/// Half-length of the pointer marker's arms, and the gap left at its centre so
/// the marked pixel itself stays visible.
pub const MARKER_ARM: i32 = 9;
pub const MARKER_GAP: i32 = 2;

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

/// Source pixels the magnifier shows per axis.
///
/// The content area is an exact whole number of source pixels, and the span is
/// even, so the pixel under the pointer lands exactly on the centre of the
/// panel. Both properties are asserted in the tests below: if either breaks,
/// the magnifier starts pointing at the wrong pixel.
pub fn magnifier_span() -> i32 {
    (MAGNIFIER_PANEL - 2 * MAGNIFIER_BORDER) / MAGNIFIER_ZOOM
}

/// The area inside the panel's frame, where magnified pixels are drawn.
pub fn magnifier_content(panel: &RECT) -> RECT {
    inflated(panel, -MAGNIFIER_BORDER)
}

/// Which side of the pointer the magnifier should prefer. Each component is
/// -1, 0 or +1; 0 lets the placement use its default side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bias {
    pub horizontal: i32,
    pub vertical: i32,
}

/// Bias pointing away from the centre of `rect`, so a panel placed with it
/// steps outside the selection instead of over the handles on the far side.
pub fn outward(rect: &RECT, point: (i32, i32)) -> Bias {
    let centre_x = (rect.left + rect.right) / 2;
    let centre_y = (rect.top + rect.bottom) / 2;
    Bias {
        horizontal: if point.0 >= centre_x { 1 } else { -1 },
        vertical: if point.1 >= centre_y { 1 } else { -1 },
    }
}

/// Panel placement with the default bias, used by the tests.
#[cfg(test)]
pub fn magnifier_panel(pointer: (i32, i32), width: u32, height: u32) -> RECT {
    magnifier_panel_biased(pointer, width, height, Bias::default())
}

/// Panel placement preferring the side given by `bias`, falling back to the
/// opposite side when the preferred one has no room.
pub fn magnifier_panel_biased(pointer: (i32, i32), width: u32, height: u32, bias: Bias) -> RECT {
    let panel = MAGNIFIER_PANEL;
    let left = place(pointer.0, bias.horizontal, panel, width as i32);
    let top = place(pointer.1, bias.vertical, panel, height as i32);
    RECT {
        left,
        top,
        right: left + panel,
        bottom: top + panel,
    }
}

/// One axis of panel placement: after the pointer, or before it.
fn place(coordinate: i32, bias: i32, panel: i32, maximum: i32) -> i32 {
    let after = coordinate + MAGNIFIER_GAP;
    let before = coordinate - MAGNIFIER_GAP - panel;

    let (preferred, other) = if bias < 0 {
        (before, after)
    } else {
        (after, before)
    };

    let fits = |start: i32| start >= 0 && start + panel <= maximum;
    let chosen = if fits(preferred) {
        preferred
    } else if fits(other) {
        other
    } else {
        // Neither side fits. This only happens on a capture narrower than the
        // panel, where clamping is better than leaving the capture entirely.
        preferred
    };
    chosen.max(0).min((maximum - panel).max(0))
}

/// Capture pixels the magnifier samples, centred on the pointer.
pub fn magnifier_source(pointer: (i32, i32)) -> RECT {
    let span = magnifier_span();
    let left = pointer.0 - span / 2;
    let top = pointer.1 - span / 2;
    RECT {
        left,
        top,
        right: left + span,
        bottom: top + span,
    }
}

/// Where the `visible` part of `source` lands inside `content`.
pub fn magnifier_dest(content: &RECT, source: &RECT, visible: &RECT) -> RECT {
    RECT {
        left: content.left + (visible.left - source.left) * MAGNIFIER_ZOOM,
        top: content.top + (visible.top - source.top) * MAGNIFIER_ZOOM,
        right: content.left + (visible.right - source.left) * MAGNIFIER_ZOOM,
        bottom: content.top + (visible.bottom - source.top) * MAGNIFIER_ZOOM,
    }
}

/// The square marking the source pixel under the pointer.
pub fn magnifier_marked(content: &RECT) -> RECT {
    let index = magnifier_span() / 2;
    RECT {
        left: content.left + index * MAGNIFIER_ZOOM,
        top: content.top + index * MAGNIFIER_ZOOM,
        right: content.left + (index + 1) * MAGNIFIER_ZOOM,
        bottom: content.top + (index + 1) * MAGNIFIER_ZOOM,
    }
}

/// Middle of the marked pixel, where the crosshair meets.
pub fn magnifier_crosshair(content: &RECT) -> (i32, i32) {
    let marked = magnifier_marked(content);
    (
        (marked.left + marked.right) / 2,
        (marked.top + marked.bottom) / 2,
    )
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

    #[test]
    fn magnifier_constants_fill_the_content_exactly() {
        let inner = MAGNIFIER_PANEL - 2 * MAGNIFIER_BORDER;
        assert_eq!(
            inner % MAGNIFIER_ZOOM,
            0,
            "the content area must hold a whole number of source pixels"
        );

        let span = magnifier_span();
        assert_eq!(
            span % 2,
            0,
            "an odd span would push the pointer's pixel off the centre"
        );
        assert!(
            span >= 8,
            "only {span} source pixels of context: too few to place an edge"
        );
    }

    #[test]
    fn the_marked_pixel_is_the_one_under_the_pointer() {
        for pointer in [(500, 400), (0, 0), (1, 1), (1919, 1079), (960, 540)] {
            let source = magnifier_source(pointer);
            let panel = magnifier_panel(pointer, 1920, 1080);
            let content = magnifier_content(&panel);
            let marked = magnifier_marked(&content);

            // Walk the marked square back to a capture pixel.
            let column = source.left + (marked.left - content.left) / MAGNIFIER_ZOOM;
            let row = source.top + (marked.top - content.top) / MAGNIFIER_ZOOM;
            assert_eq!(
                (column, row),
                pointer,
                "the magnifier must mark the pixel under the pointer"
            );
        }
    }

    #[test]
    fn the_marked_pixel_sits_at_the_centre_of_the_content() {
        let content = rect(300, 200, 300 + MAGNIFIER_PANEL, 200 + MAGNIFIER_PANEL);
        let content = magnifier_content(&content);
        let marked = magnifier_marked(&content);

        let centre_x = (content.left + content.right) / 2;
        let centre_y = (content.top + content.bottom) / 2;
        assert_eq!(marked.left, centre_x);
        assert_eq!(marked.top, centre_y);

        let (cross_x, cross_y) = magnifier_crosshair(&content);
        assert_eq!(cross_x, centre_x + MAGNIFIER_ZOOM / 2);
        assert_eq!(cross_y, centre_y + MAGNIFIER_ZOOM / 2);
    }

    #[test]
    fn a_source_wholly_on_the_capture_maps_onto_the_whole_content() {
        let pointer = (500, 400);
        let source = magnifier_source(pointer);
        let visible = intersect(&source, 1920, 1080);
        assert_eq!(visible, source, "the sample is fully on screen");

        let panel = magnifier_panel(pointer, 1920, 1080);
        let content = magnifier_content(&panel);
        assert_eq!(magnifier_dest(&content, &source, &visible), content);
    }

    #[test]
    fn an_offscreen_sample_maps_to_the_matching_part_of_the_content() {
        // The top-left pixel: half of the sample reaches off the capture.
        let pointer = (0, 0);
        let source = magnifier_source(pointer);
        assert!(source.left < 0 && source.top < 0);

        let visible = intersect(&source, 1920, 1080);
        assert_eq!(visible.left, 0);
        assert_eq!(visible.top, 0);

        let content = rect(0, 0, MAGNIFIER_PANEL, MAGNIFIER_PANEL);
        let content = magnifier_content(&content);
        let dest = magnifier_dest(&content, &source, &visible);

        let half = magnifier_span() / 2;
        assert_eq!(dest.left, content.left + half * MAGNIFIER_ZOOM);
        assert_eq!(dest.top, content.top + half * MAGNIFIER_ZOOM);
        assert_eq!(dest.right, content.right);
        assert_eq!(dest.bottom, content.bottom);
    }

    #[test]
    fn the_panel_flips_near_the_edges_and_never_leaves_the_capture() {
        // Middle of the screen: below and to the right of the pointer.
        let middle = magnifier_panel((500, 400), 1920, 1080);
        assert_eq!(middle.left, 500 + MAGNIFIER_GAP);
        assert_eq!(middle.top, 400 + MAGNIFIER_GAP);

        // Bottom-right corner: flips to the other side, stays on the capture.
        let corner = magnifier_panel((1910, 1070), 1920, 1080);
        assert!(corner.left >= 0 && corner.right <= 1920);
        assert!(corner.top >= 0 && corner.bottom <= 1080);
        assert!(corner.right < 1910, "flipped to the left");
        assert!(corner.bottom < 1070, "flipped above");

        // A capture smaller than the panel still yields an on-capture panel.
        let tiny = magnifier_panel((5, 5), 50, 50);
        assert_eq!(tiny.left, 0);
        assert_eq!(tiny.top, 0);
    }

    #[test]
    fn the_panel_can_be_biased_to_a_chosen_side() {
        let right = magnifier_panel_biased(
            (500, 400),
            1920,
            1080,
            Bias {
                horizontal: 1,
                vertical: 0,
            },
        );
        assert_eq!(right.left, 500 + MAGNIFIER_GAP);

        let left = magnifier_panel_biased(
            (500, 400),
            1920,
            1080,
            Bias {
                horizontal: -1,
                vertical: 0,
            },
        );
        assert_eq!(left.right, 500 - MAGNIFIER_GAP);

        let above = magnifier_panel_biased(
            (500, 400),
            1920,
            1080,
            Bias {
                horizontal: 0,
                vertical: -1,
            },
        );
        assert_eq!(above.bottom, 400 - MAGNIFIER_GAP);

        let below = magnifier_panel_biased(
            (500, 400),
            1920,
            1080,
            Bias {
                horizontal: 0,
                vertical: 1,
            },
        );
        assert_eq!(below.top, 400 + MAGNIFIER_GAP);
    }

    #[test]
    fn a_biased_panel_falls_back_when_the_preferred_side_has_no_room() {
        // Asking for the left side while the pointer is against the left edge:
        // there is no room, so it uses the right instead of leaving the capture.
        let panel = magnifier_panel_biased(
            (2, 400),
            1920,
            1080,
            Bias {
                horizontal: -1,
                vertical: 0,
            },
        );
        assert!(panel.left >= 0, "the panel must stay on the capture");
        assert_eq!(panel.left, 2 + MAGNIFIER_GAP, "it fell back to the right");
    }

    #[test]
    fn outward_points_away_from_the_selection_centre() {
        let selection = rect(100, 100, 300, 200);

        // Grabbing the right edge above its middle: right and up.
        assert_eq!(
            outward(&selection, (300, 120)),
            Bias {
                horizontal: 1,
                vertical: -1
            }
        );
        // Right edge below the middle: right and down.
        assert_eq!(
            outward(&selection, (300, 180)),
            Bias {
                horizontal: 1,
                vertical: 1
            }
        );
        // Left edge above the middle: left and up.
        assert_eq!(
            outward(&selection, (100, 120)),
            Bias {
                horizontal: -1,
                vertical: -1
            }
        );
    }
}
