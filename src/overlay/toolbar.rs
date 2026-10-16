//! Where the annotation strip goes, and what a click on it hits.
//!
//! The strip is drawn by the overlay rather than built from controls, so its geometry
//! has to be worked out here — and it is the kind of geometry that is wrong in ways a
//! picture does not show. A strip one pixel too wide covers a row of the capture; a
//! button whose rectangle is off by one is a button that does not respond at its own
//! edge. So the placement and the hit test are pure functions, tested without a window,
//! exactly like the annotation model beside them.
//!
//! # The rule
//!
//! **The strip never covers the selection.** It goes to the left of it when there is
//! room, then to the right, then above, then below. Only when there is no room anywhere
//! does it go inside, on the side with the most space, so that it covers as little of
//! what is being captured as possible.
//!
//! That is the whole point of not copying the tools that put a toolbar in a corner of
//! the screen: with the strip placed against the selection, the thing being selected is
//! never underneath it, and the strip follows the region as it is adjusted instead of
//! having to be chased.

use super::annotation::Tool;

/// One button of the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Every tool, in the order they appear.
    Tool(Tool),
    Colour,
    Thickness,
    Undo,
    Redo,
    Save,
    Copy,
    Cancel,
}

impl Button {
    /// The strip's buttons, top to bottom.
    ///
    /// Grouped: the tools, then how a stroke looks, then history, then the way out. The
    /// order is the order they are reached for, and the groups are separated in the
    /// drawing so the eye can skip the ones it is not using.
    pub const ALL: [Button; 12] = [
        Button::Tool(Tool::Move),
        Button::Tool(Tool::Arrow),
        Button::Tool(Tool::Rectangle),
        Button::Tool(Tool::Ellipse),
        Button::Tool(Tool::Pencil),
        Button::Colour,
        Button::Thickness,
        Button::Undo,
        Button::Redo,
        Button::Save,
        Button::Copy,
        Button::Cancel,
    ];

    /// Whether a separator line goes above this button.
    pub fn starts_a_group(self) -> bool {
        matches!(self, Button::Colour | Button::Undo | Button::Save)
    }
}

/// Which side of the selection the strip ended up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Above,
    Below,
    /// No room outside, so it covers part of the selection. Carries how much, so the
    /// caller can prefer the corner with the least of the capture under it.
    Inside { covered: u32 },
}

/// The strip's size and where it put itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub side: Side,
}

impl Placement {
    /// Whether a point on the desktop is over the strip.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x + self.width as i32
            && y < self.y + self.height as i32
    }
}

/// A selection, as the placement needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// The strip's sizes, in 96-DPI units, scaled by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    /// Side of a square button.
    pub button: i32,
    /// Gap between buttons.
    pub gap: i32,
    /// Padding inside the strip's own frame.
    pub padding: i32,
    /// Extra space above a button that starts a group, for the separator.
    pub group_gap: i32,
    /// How far the strip stands off the selection.
    pub standoff: i32,
}

impl Metrics {
    /// The sizes at 100%. Everything else follows from these.
    pub const BASE: Metrics = Metrics {
        button: 26,
        gap: 2,
        padding: 4,
        group_gap: 5,
        standoff: 8,
    };

    /// The strip's width, which is one button plus its padding on both sides.
    pub fn width(&self) -> u32 {
        (self.button + self.padding * 2).max(1) as u32
    }

    /// The strip's height: every button, every gap, every group gap, and the padding.
    pub fn height(&self) -> u32 {
        let buttons = Button::ALL.len() as i32;
        let gaps = (buttons - 1) * self.gap;
        let groups = Button::ALL
            .iter()
            .filter(|button| button.starts_a_group())
            .count() as i32
            * self.group_gap;
        (buttons * self.button + gaps + groups + self.padding * 2).max(1) as u32
    }

    /// The rectangle of each button, in the strip's own coordinates.
    ///
    /// Returned in the order of [`Button::ALL`], so a caller can pair them up.
    pub fn button_rects(&self) -> Vec<(Button, i32, i32, u32, u32)> {
        let mut rects = Vec::with_capacity(Button::ALL.len());
        let mut y = self.padding;
        for (index, button) in Button::ALL.iter().enumerate() {
            if index > 0 {
                y += self.gap;
            }
            if button.starts_a_group() {
                y += self.group_gap;
            }
            rects.push((
                *button,
                self.padding,
                y,
                self.button as u32,
                self.button as u32,
            ));
            y += self.button;
        }
        rects
    }
}

/// Where the strip goes for a selection of this size in a space of this size.
///
/// `space` is the whole area the strip may be placed in — the virtual desktop, in the
/// overlay's case. Both are in the same coordinates.
pub fn place(region: Region, space_width: u32, space_height: u32, metrics: &Metrics) -> Placement {
    let width = metrics.width() as i32;
    let height = metrics.height() as i32;

    let left_of = region.x - metrics.standoff - width;
    let right_of = region.x + region.width as i32 + metrics.standoff;
    let above = region.y - metrics.standoff - height;
    let below = region.y + region.height as i32 + metrics.standoff;

    let fits_horizontally = |x: i32| x >= 0 && x + width <= space_width as i32;
    let fits_vertically = |y: i32| y >= 0 && y + height <= space_height as i32;

    // Aligned with the selection's top, then pulled back into the space so that a
    // selection near the bottom does not push the strip off the screen.
    let align_vertical = |top: i32| -> i32 {
        let lowest = (space_height as i32 - height).max(0);
        top.clamp(0, lowest)
    };
    let align_horizontal = |left: i32| -> i32 {
        let furthest = (space_width as i32 - width).max(0);
        left.clamp(0, furthest)
    };

    // Left first: reading order, and the side the eye returns to.
    if fits_horizontally(left_of) {
        return Placement {
            x: left_of,
            y: align_vertical(region.y),
            width: width as u32,
            height: height as u32,
            side: Side::Left,
        };
    }
    if fits_horizontally(right_of) {
        return Placement {
            x: right_of,
            y: align_vertical(region.y),
            width: width as u32,
            height: height as u32,
            side: Side::Right,
        };
    }
    if fits_vertically(above) {
        return Placement {
            x: align_horizontal(region.x),
            y: above,
            width: width as u32,
            height: height as u32,
            side: Side::Above,
        };
    }
    if fits_vertically(below) {
        return Placement {
            x: align_horizontal(region.x),
            y: below,
            width: width as u32,
            height: height as u32,
            side: Side::Below,
        };
    }

    // Nowhere outside. Go inside, on whichever side leaves fewer of the selection's
    // rows or columns underneath the strip.
    let space_on_left = region.x.max(0) + metrics.standoff;
    let space_on_right = (space_width as i32 - (region.x + region.width as i32)).max(0)
        + metrics.standoff;
    let space_above = region.y.max(0) + metrics.standoff;
    let space_below = (space_height as i32 - (region.y + region.height as i32)).max(0)
        + metrics.standoff;

    let best_horizontal = space_on_left.max(space_on_right);
    let best_vertical = space_above.max(space_below);
    let (x, y) = if best_horizontal >= best_vertical {
        let x = if space_on_left >= space_on_right {
            region.x + metrics.standoff
        } else {
            region.x + region.width as i32 - metrics.standoff - width
        };
        (x, align_vertical(region.y + metrics.standoff))
    } else {
        let y = if space_above >= space_below {
            region.y + metrics.standoff
        } else {
            region.y + region.height as i32 - metrics.standoff - height
        };
        (align_horizontal(region.x + metrics.standoff), y)
    };

    // How much of the selection the strip lies over, as a share of its height when the
    // strip is vertical. Reported so a caller that cares can say so; nothing decides
    // anything on it here.
    let overlap_y = (y + height - region.y).clamp(0, region.height as i32);
    let overlap_x = (x + width - region.x).clamp(0, region.width as i32);
    let covered = if best_horizontal >= best_vertical {
        overlap_y as u32
    } else {
        overlap_x as u32
    };

    Placement {
        x,
        y,
        width: width as u32,
        height: height as u32,
        side: Side::Inside { covered },
    }
}

/// Which button a point on the desktop is over.
///
/// `None` means the point is off the strip, which is what tells the caller to treat the
/// click as a selection gesture instead.
pub fn hit(placement: &Placement, metrics: &Metrics, x: i32, y: i32) -> Option<Button> {
    if !placement.contains(x, y) {
        return None;
    }
    let local_x = x - placement.x;
    let local_y = y - placement.y;
    metrics
        .button_rects()
        .into_iter()
        .find(|(_, bx, by, bw, bh)| {
            local_x >= *bx
                && local_y >= *by
                && local_x < bx + *bw as i32
                && local_y < by + *bh as i32
        })
        .map(|(button, _, _, _, _)| button)
}

/// The strip's metrics for a display scale.
pub fn metrics_for(factor: f64) -> Metrics {
    let scale = |value: i32| crate::ui_layout::scale(value, factor);
    Metrics {
        button: scale(Metrics::BASE.button),
        gap: scale(Metrics::BASE.gap),
        padding: scale(Metrics::BASE.padding),
        group_gap: scale(Metrics::BASE.group_gap),
        standoff: scale(Metrics::BASE.standoff),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> Metrics {
        Metrics::BASE
    }

    /// A selection of a size that leaves room on both sides in a 1920x1080 space.
    fn a_roomy_selection() -> Region {
        Region {
            x: 600,
            y: 300,
            width: 640,
            height: 400,
        }
    }

    #[test]
    fn the_strip_is_taller_than_its_buttons_and_wider_than_one() {
        // The two measurements everything else is placed against. A strip whose height
        // did not account for the gaps would overlap its own buttons; one whose width
        // was a single button would clip the padding.
        let m = metrics();
        let buttons = Button::ALL.len() as i32 * m.button;
        assert!(
            m.height() as i32 > buttons,
            "the height has to include the gaps and the padding"
        );
        assert!(
            m.width() as i32 > m.button,
            "the width has to include the padding"
        );
        assert_eq!(
            m.height() as i32,
            buttons + (Button::ALL.len() as i32 - 1) * m.gap + 3 * m.group_gap + m.padding * 2,
            "height does not match its own parts"
        );
    }

    #[test]
    fn the_buttons_do_not_overlap_and_stay_inside_the_strip() {
        // A mis-added gap puts one button on top of the next, which looks like a
        // drawing fault and is really an arithmetic one.
        let m = metrics();
        let rects = m.button_rects();
        assert_eq!(rects.len(), Button::ALL.len());

        let mut previous_bottom = 0;
        for (index, (button, x, y, w, h)) in rects.iter().enumerate() {
            assert!(*x >= 0 && *y >= 0, "{button:?} starts outside the strip");
            assert!(
                *x + *w as i32 <= m.width() as i32,
                "{button:?} is wider than the strip"
            );
            if index > 0 {
                assert!(
                    *y >= previous_bottom,
                    "{button:?} starts at {y}, above the button before it which ends at {previous_bottom}"
                );
            }
            previous_bottom = *y + *h as i32;
        }
        assert!(
            previous_bottom + m.padding <= m.height() as i32,
            "the last button ends at {previous_bottom}, past the strip's {}",
            m.height()
        );
    }

    #[test]
    fn every_button_can_be_hit_at_its_own_centre() {
        // The hit test and the drawing must agree, or a button responds somewhere other
        // than where it is drawn — the worst kind of fault to look for by eye.
        let m = metrics();
        let placement = place(a_roomy_selection(), 1920, 1080, &m);
        for (button, x, y, w, h) in m.button_rects() {
            let centre_x = placement.x + x + w as i32 / 2;
            let centre_y = placement.y + y + h as i32 / 2;
            assert_eq!(
                hit(&placement, &m, centre_x, centre_y),
                Some(button),
                "{button:?} is not hit at its own centre"
            );
        }
    }

    #[test]
    fn a_point_off_the_strip_hits_nothing() {
        let m = metrics();
        let placement = place(a_roomy_selection(), 1920, 1080, &m);
        assert_eq!(hit(&placement, &m, placement.x - 1, placement.y), None);
        assert_eq!(hit(&placement, &m, placement.x, placement.y - 1), None);
        assert_eq!(
            hit(&placement, &m, placement.x + placement.width as i32, placement.y),
            None
        );
        assert_eq!(
            hit(&placement, &m, placement.x, placement.y + placement.height as i32),
            None
        );
    }

    #[test]
    fn the_strip_goes_to_the_left_when_there_is_room() {
        let m = metrics();
        let region = a_roomy_selection();
        let placement = place(region, 1920, 1080, &m);
        assert_eq!(placement.side, Side::Left);
        assert!(
            placement.x + placement.width as i32 <= region.x,
            "the strip reaches into the selection"
        );
    }

    #[test]
    fn the_strip_moves_to_the_right_when_the_left_is_taken() {
        let m = metrics();
        let region = Region {
            x: 4,
            y: 300,
            width: 600,
            height: 300,
        };
        let placement = place(region, 1920, 1080, &m);
        assert_eq!(placement.side, Side::Right, "there was no room on the left");
        assert!(
            placement.x >= region.x + region.width as i32,
            "the strip overlaps the selection"
        );
    }

    #[test]
    fn the_strip_stands_off_the_selection_rather_than_touching_it() {
        // Touching the edge makes the strip look like part of the selection's frame.
        let m = metrics();
        let region = a_roomy_selection();
        let placement = place(region, 1920, 1080, &m);
        assert_eq!(
            region.x - (placement.x + placement.width as i32),
            m.standoff,
            "the gap is not the standoff"
        );
    }

    #[test]
    fn the_strip_never_covers_the_selection_when_there_is_room() {
        // The rule the whole module exists for, checked over a spread of selections and
        // spaces rather than one example.
        let m = metrics();
        let selections = [
            (10, 10, 100, 100),
            (600, 300, 640, 400),
            (1500, 900, 400, 160),
            (900, 40, 300, 900),
            (100, 500, 1700, 400),
        ];
        for (x, y, width, height) in selections {
            let region = Region { x, y, width, height };
            for (space_w, space_h) in [(1920, 1080), (3840, 2160), (1280, 720)] {
                let placement = place(region, space_w, space_h, &m);
                if let Side::Inside { .. } = placement.side {
                    continue; // no room outside; the rule cannot apply
                }
                let overlap_x = placement.x < region.x + region.width as i32
                    && placement.x + placement.width as i32 > region.x;
                let overlap_y = placement.y < region.y + region.height as i32
                    && placement.y + placement.height as i32 > region.y;
                assert!(
                    !(overlap_x && overlap_y),
                    "a {space_w}x{space_h} space with {region:?} put the strip on the selection \
                     at {:?} on the {:?} side",
                    (placement.x, placement.y),
                    placement.side
                );
            }
        }
    }

    #[test]
    fn the_strip_stays_inside_the_space_whatever_the_selection() {
        // A strip pushed off the screen cannot be clicked, and on a single-monitor setup
        // there is nothing to scroll to reach it.
        let m = metrics();
        for (x, y, width, height) in [
            (0, 0, 100, 100),
            (0, 0, 1920, 1080),
            (1820, 100, 100, 100),
            (100, 1000, 100, 80),
            (-400, -200, 300, 300),
        ] {
            let region = Region { x, y, width, height };
            let placement = place(region, 1920, 1080, &m);
            assert!(
                placement.x >= 0 && placement.y >= 0,
                "{region:?} put the strip at a negative position: {:?}",
                (placement.x, placement.y)
            );
            assert!(
                placement.x + placement.width as i32 <= 1920,
                "{region:?} pushed the strip past the right edge"
            );
            assert!(
                placement.y + placement.height as i32 <= 1080,
                "{region:?} pushed the strip past the bottom edge"
            );
        }
    }

    #[test]
    fn a_selection_filling_the_screen_gets_the_strip_inside_it() {
        // The degenerate case: there is no room anywhere, so covering part of the
        // capture is the only option left. It is reported as such rather than presented
        // as a normal placement.
        let m = metrics();
        let region = Region {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let placement = place(region, 1920, 1080, &m);
        assert!(
            matches!(placement.side, Side::Inside { .. }),
            "expected an inside placement, got {:?}",
            placement.side
        );
    }

    #[test]
    fn a_narrow_selection_still_gets_a_strip_that_can_be_clicked() {
        // A one-pixel-wide column has no room either side. The strip must still be
        // somewhere a person can reach, not clipped to nothing.
        let m = metrics();
        let region = Region {
            x: 500,
            y: 100,
            width: 1,
            height: 800,
        };
        let placement = place(region, 1920, 1080, &m);
        assert_eq!(placement.width, m.width(), "the strip was made narrower");
        assert_eq!(placement.height, m.height(), "the strip was made shorter");
        assert!(placement.x >= 0 && placement.x + placement.width as i32 <= 1920);
    }

    #[test]
    fn the_metrics_scale_with_the_display() {
        // The strip is drawn, not built from controls, so it does not scale by itself.
        // At 200% every part has to double, or the buttons are too small to hit.
        let base = metrics_for(1.0);
        assert_eq!(base, Metrics::BASE);

        let double = metrics_for(2.0);
        assert_eq!(double.button, Metrics::BASE.button * 2);
        assert_eq!(double.padding, Metrics::BASE.padding * 2);
        assert_eq!(double.standoff, Metrics::BASE.standoff * 2);
        assert!(
            double.height() > base.height(),
            "the strip did not grow with the scale"
        );
        assert!(double.width() > base.width());
    }

    #[test]
    fn a_scaled_strip_can_still_be_hit() {
        // The hit test uses the same metrics as the drawing, so a scaled strip has to
        // work through the same path rather than only at 100%.
        for factor in [1.0, 1.25, 1.5, 2.0, 3.0] {
            let m = metrics_for(factor);
            let placement = place(a_roomy_selection(), 1920, 1080, &m);
            for (button, x, y, w, h) in m.button_rects() {
                let centre_x = placement.x + x + w as i32 / 2;
                let centre_y = placement.y + y + h as i32 / 2;
                assert_eq!(
                    hit(&placement, &m, centre_x, centre_y),
                    Some(button),
                    "{button:?} is not hit at {factor}x"
                );
            }
        }
    }

    #[test]
    fn the_groups_start_where_they_should() {
        // The first button does not need a separator: it is already against the strip's
        // own top edge, and a line there would read as a border.
        assert!(
            !Button::Tool(Tool::Move).starts_a_group(),
            "the first button would draw a separator under the strip's edge"
        );
        assert!(!Button::Tool(Tool::Arrow).starts_a_group());
        assert!(
            Button::Colour.starts_a_group(),
            "the appearance group needs a separator above it"
        );
        assert!(Button::Undo.starts_a_group());
        assert!(!Button::Redo.starts_a_group());
        assert!(Button::Save.starts_a_group(), "the way out is its own group");
        assert!(!Button::Copy.starts_a_group());
    }
}
