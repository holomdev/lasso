//! Screen rotation.
//!
//! A monitor can be turned, and a turned monitor changes the shape of what a capture
//! has to put where. This module is the arithmetic of that and nothing else: it holds
//! no window, no device context and no device, so every case can be checked without a
//! rotated display to check it on — which matters, because a rotated display is
//! exactly what a development machine tends not to have.
//!
//! # Which way a duplicated frame comes out
//!
//! DXGI Output Duplication hands back the surface in the output's **unrotated**
//! orientation, with the mode's own width and height, and it is the presentation
//! engine that turns it for display. So a monitor standing on its side yields a
//! landscape image that has to be turned the *opposite* way before it goes onto the
//! virtual-desktop canvas, which is arranged the way the desktop is.
//!
//! That is the one part of this that cannot be established from the arithmetic: it is
//! a fact about what the API returns, and reversing it turns every rotated capture
//! sideways. `LASSO_FORCE_ROTATION` exists to exercise the path without a rotated
//! display, so at least the code is known to run; whether the direction is right needs
//! a monitor that can actually be turned.

/// How a display's contents are turned before being shown.
///
/// The four mirror variants exist because the display settings allow them, and a
/// capture taken with the mirror ignored would be correct but reflected. Nothing
/// constructs them yet: DXGI reports only the four plain turns, and the mirror is not
/// in `DXGI_MODE_ROTATION` at all, so a mirrored display has to be recognised some
/// other way. They are written and tested rather than left out, so that the shape of
/// the problem is settled when that route is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum Rotation {
    #[default]
    Identity,
    Rotate90,
    Rotate180,
    Rotate270,
    /// Turned 90° and mirrored across the horizontal axis.
    Rotate90Mirrored,
    Rotate180Mirrored,
    Rotate270Mirrored,
}

impl Rotation {
    /// Whether this rotation exchanges width and height.
    pub fn swaps_axes(self) -> bool {
        matches!(self, Rotation::Rotate90 | Rotation::Rotate270)
            || matches!(
                self,
                Rotation::Rotate90Mirrored | Rotation::Rotate270Mirrored
            )
    }

    /// The size of the image that stores a display of this size.
    ///
    /// The stored image has the **mode's** dimensions, and a monitor standing on its
    /// side is a portrait rectangle showing a landscape image: a 1080x1920 display
    /// stores 1920x1080. So a turned display and its image are the transpose of each
    /// other, and getting this the wrong way round makes every turned capture the
    /// wrong shape rather than merely the wrong way up.
    pub fn stored_from_display(self, display_width: u32, display_height: u32) -> (u32, u32) {
        if self.swaps_axes() {
            (display_height, display_width)
        } else {
            (display_width, display_height)
        }
    }

}

/// Where a point on screen is read from, in the image that stores this display.
///
/// The two rectangles involved are easy to confuse, so they are named here and
/// nowhere else in this file uses them loosely:
///
/// * The **desktop rectangle** is the monitor's place in virtual-desktop
///   coordinates, as `DesktopCoordinates` reports it. It is the size the compositor
///   writes, and every point of it has to be filled.
/// * The **stored image** is what output duplication hands back: the monitor's own
///   mode, which for a turned monitor is the transpose of its desktop rectangle. It
///   is the size the reading has to stay inside.
///
/// The arguments are the **desktop** rectangle and the point in it, because that is
/// what a capture walks. The image's dimensions are derived from them, so a caller
/// cannot pass the two the wrong way round.
///
/// It is written as an inverse on purpose. Reading a destination and taking from a
/// source is what a capture needs — every pixel of the canvas asks where its contents
/// come from — and expressing it directly avoids the off-by-one a forward map invites,
/// where several destinations round to the same source and others get none.
pub fn source_for(rotation: Rotation, x: i32, y: i32, desktop_width: u32, desktop_height: u32) -> (i32, i32) {
    // Work in i64 so that a large coordinate cannot wrap before it is used.
    let (image_width, image_height) = rotation.stored_from_display(desktop_width, desktop_height);
    let w = i64::from(image_width);
    let h = i64::from(image_height);
    let x = i64::from(x);
    let y = i64::from(y);

    // Undo the turn, then undo the mirror if the setting has one.
    //
    // These were **searched for, not derived**. Deriving them by hand crossed the two
    // quarter turns over three separate times, and the first version of the tests
    // agreed with the mistake because it used this function to build the dimensions it
    // then passed in. Instead a labelled grid is turned the way each display setting
    // describes, and every candidate formula built from `x`, `y`, `w-1-x`, `w-1-y`,
    // `h-1-x` and `h-1-y` is tried against the whole drawing. Exactly one formula
    // reproduces each rotation, and it is the one written here.
    //
    // The mirrored variants get a branch each rather than sharing the turn and
    // composing a flip, because they do not share one: a half turn mirrored flips its
    // column, a quarter turn mirrored flips both coordinates. Writing what the search
    // found is shorter than working out why, and it cannot be got wrong the same way
    // twice.
    //
    // `w` and `h` are the **stored image's** dimensions, which for a turned monitor are
    // the transpose of its desktop rectangle.
    let (column, row) = match rotation {
        Rotation::Identity => (x, y),
        Rotation::Rotate90 => (y, h - 1 - x),
        Rotation::Rotate180 => (w - 1 - x, h - 1 - y),
        Rotation::Rotate270 => (w - 1 - y, x),
        Rotation::Rotate90Mirrored => (w - 1 - y, h - 1 - x),
        Rotation::Rotate180Mirrored => (w - 1 - x, y),
        Rotation::Rotate270Mirrored => (y, x),
    };

    (column as i32, row as i32)
}

/// The rotation the environment asks this run to pretend, for exercising the path.
///
/// A rotated display is what a development machine does not have, so the arithmetic
/// would otherwise only ever run on someone else's. This reads
/// `LASSO_FORCE_ROTATION` as one of `90`, `180`, `270` or `0`, which is wrong on a
/// display that really is turned and is why it is not a CLI flag: forcing a rotation
/// gives a wrong capture, and that is not something to offer as a feature.
///
/// The case cannot be checked here. The disassembly had to be searched for the formula
/// for each rotation, and this switch has the same problem in a different form: the
/// only way to know the direction is right is a monitor that can be turned, and the
/// only way to know it is read is that the message below appears.
pub fn forced_by_environment() -> Option<Rotation> {
    match std::env::var("LASSO_FORCE_ROTATION").ok()?.trim() {
        "0" | "identity" => Some(Rotation::Identity),
        "90" => Some(Rotation::Rotate90),
        "180" => Some(Rotation::Rotate180),
        "270" => Some(Rotation::Rotate270),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every rotation, so a test that walks them cannot miss one.
    const ALL: [Rotation; 7] = [
        Rotation::Identity,
        Rotation::Rotate90,
        Rotation::Rotate180,
        Rotation::Rotate270,
        Rotation::Rotate90Mirrored,
        Rotation::Rotate180Mirrored,
        Rotation::Rotate270Mirrored,
    ];

    /// A monitor whose desktop rectangle is 5 wide and 3 tall.
    ///
    /// Chosen so that the two dimensions differ and neither divides the other: a map
    /// that confuses width with height cannot pass by symmetry, and a row length that
    /// is wrong shows up as a shift rather than as a coincidence.
    ///
    /// It is a **desktop rectangle**, which is not the same thing as a stored image:
    /// for a quarter turn the image storing it is 3 wide and 5 tall. Signs of a
    /// monitor turned to portrait are `(3, 5)` in the other order.
    const DESKTOP: (u32, u32) = (5, 3);

    /// The size of the image that stores `DESKTOP`.
    fn stored(rotation: Rotation) -> (u32, u32) {
        rotation.stored_from_display(DESKTOP.0, DESKTOP.1)
    }

    /// Every point of the desktop rectangle.
    ///
    /// A turned display still occupies the rectangle `DESKTOP` says: its contents are
    /// arranged differently, but the rectangle on the desk does not change shape.
    /// Walking the *stored* dimensions instead is the mistake this helper exists to
    /// prevent — it walks off the rectangle, and every point beyond it reads from a
    /// source the points inside already used.
    fn every_desktop_point() -> Vec<(i32, i32)> {
        let mut points = Vec::new();
        for y in 0..DESKTOP.1 as i32 {
            for x in 0..DESKTOP.0 as i32 {
                points.push((x, y));
            }
        }
        points
    }

    #[test]
    fn a_turned_monitor_is_stored_as_the_transpose_of_its_rectangle() {
        // A monitor stood on its side is a portrait rectangle showing a landscape
        // image, so the image is the transpose. Getting this the wrong way round makes
        // every turned capture the wrong shape rather than merely the wrong way up.
        assert_eq!(Rotation::Identity.stored_from_display(5, 3), (5, 3));
        assert_eq!(Rotation::Rotate180.stored_from_display(5, 3), (5, 3));
        assert_eq!(Rotation::Rotate90.stored_from_display(5, 3), (3, 5));
        assert_eq!(Rotation::Rotate270.stored_from_display(5, 3), (3, 5));
        assert!(Rotation::Rotate90.swaps_axes());
        assert!(!Rotation::Rotate180.swaps_axes());
    }

    #[test]
    fn every_point_of_the_rectangle_reads_from_inside_its_image() {
        // The property that matters: a rotation is a rearrangement, so it must not
        // read outside the image or leave a gap. Checked over the whole rectangle for
        // every rotation, which is cheap at this size.
        for rotation in ALL {
            let (width, height) = stored(rotation);
            for (x, y) in every_desktop_point() {
                let (sx, sy) = source_for(rotation, x, y, DESKTOP.0, DESKTOP.1);
                assert!(
                    sx >= 0 && sy >= 0 && (sx as u32) < width && (sy as u32) < height,
                    "{rotation:?}: rectangle ({x}, {y}) reads ({sx}, {sy}) \
                     outside a {width}x{height} image"
                );
            }
        }
    }

    #[test]
    fn every_pixel_of_the_image_is_read_exactly_once() {
        // The other half of being a rearrangement: nothing is dropped, and nothing is
        // used twice. A map that fails this produces a capture with a duplicated row
        // or a missing one, which is easy to miss by eye.
        for rotation in ALL {
            let (width, height) = stored(rotation);
            let mut seen = vec![false; (width * height) as usize];
            for (x, y) in every_desktop_point() {
                let (sx, sy) = source_for(rotation, x, y, DESKTOP.0, DESKTOP.1);
                let index = (sy as u32 * width + sx as u32) as usize;
                assert!(
                    !seen[index],
                    "{rotation:?}: image pixel ({sx}, {sy}) is read more than once"
                );
                seen[index] = true;
            }
            assert!(
                seen.iter().all(|&used| used),
                "{rotation:?}: some image pixel is never read"
            );
        }
    }

    #[test]
    fn the_unturned_case_is_the_identity() {
        for (x, y) in every_desktop_point() {
            assert_eq!(
                source_for(Rotation::Identity, x, y, DESKTOP.0, DESKTOP.1),
                (x, y)
            );
        }
    }

    #[test]
    fn the_corners_land_where_the_drawing_says() {
        // The expectations below were not derived from the implementation: the stored
        // image is drawn as a labelled grid 5 wide and 3 tall, turned the way each
        // display setting describes, and each corner of the rectangle is read off the
        // result. Two independent routes to the same table is the point — every
        // mistake made while writing this function was one of these corners, and each
        // time the arithmetic in the code and the arithmetic in the expectation
        // disagreed, both being hand-written.
        //
        // A monitor turned on its side occupies 3 wide and 5 tall; the half turns keep
        // the mode's own 5 by 3.
        // Both sizes are stated for every case rather than one being computed from the
        // other: `stored_from_display` is its own inverse, so a test that used it to
        // build the sizes it then passed in would agree with itself whatever it did.
        //
        // The rectangle a turned monitor occupies, and the image that stores it. A
        // monitor on its side is a portrait rectangle showing a landscape image.
        let cases: [(Rotation, (u32, u32), (u32, u32), [((i32, i32), (i32, i32)); 4]); 7] = [
            (
                Rotation::Identity,
                (5, 3),
                (5, 3),
                [((0, 0), (0, 0)), ((4, 0), (4, 0)), ((0, 2), (0, 2)), ((4, 2), (4, 2))],
            ),
            (
                Rotation::Rotate90,
                (3, 5),
                (5, 3),
                [((0, 0), (0, 2)), ((2, 0), (0, 0)), ((0, 4), (4, 2)), ((2, 4), (4, 0))],
            ),
            (
                Rotation::Rotate180,
                (5, 3),
                (5, 3),
                [((0, 0), (4, 2)), ((4, 0), (0, 2)), ((0, 2), (4, 0)), ((4, 2), (0, 0))],
            ),
            (
                Rotation::Rotate270,
                (3, 5),
                (5, 3),
                [((0, 0), (4, 0)), ((2, 0), (4, 2)), ((0, 4), (0, 0)), ((2, 4), (0, 2))],
            ),
            (
                Rotation::Rotate90Mirrored,
                (3, 5),
                (5, 3),
                [((0, 0), (4, 2)), ((2, 0), (4, 0)), ((0, 4), (0, 2)), ((2, 4), (0, 0))],
            ),
            (
                Rotation::Rotate180Mirrored,
                (5, 3),
                (5, 3),
                [((0, 0), (4, 0)), ((4, 0), (0, 0)), ((0, 2), (4, 2)), ((4, 2), (0, 2))],
            ),
            (
                Rotation::Rotate270Mirrored,
                (3, 5),
                (5, 3),
                [((0, 0), (0, 0)), ((2, 0), (0, 2)), ((0, 4), (4, 0)), ((2, 4), (4, 2))],
            ),
        ];

        for (rotation, rectangle, image, corners) in cases {
            // The rectangle and the image are each other's transpose, which is the one
            // thing that has to hold for the sizes to mean anything.
            assert_eq!(
                rotation.stored_from_display(rectangle.0, rectangle.1),
                image,
                "{rotation:?}: a {rectangle:?} rectangle should be stored as {image:?}"
            );
            for ((x, y), expected) in corners {
                assert_eq!(
                    source_for(rotation, x, y, rectangle.0, rectangle.1),
                    expected,
                    "{rotation:?}: rectangle ({x}, {y}) [rectangle {rectangle:?}, image {image:?}]"
                );
            }
        }
    }

    #[test]
    fn every_mirrored_rotation_is_a_rearrangement_too() {
        // The corner table covers the mirrored variants, but only at four points each.
        // The mirror composition is the easiest thing to get wrong, so each variant gets
        // the same whole-rectangle check as the plain ones.
        for rotation in [
            Rotation::Rotate90Mirrored,
            Rotation::Rotate180Mirrored,
            Rotation::Rotate270Mirrored,
        ] {
            let (image_width, image_height) = stored(rotation);
            let mut seen = vec![false; (image_width * image_height) as usize];
            for (x, y) in every_desktop_point() {
                let (sx, sy) = source_for(rotation, x, y, DESKTOP.0, DESKTOP.1);
                assert!(
                    sx >= 0 && sy >= 0 && (sx as u32) < image_width && (sy as u32) < image_height,
                    "{rotation:?}: ({x}, {y}) reads ({sx}, {sy}) outside the image"
                );
                let index = (sy as u32 * image_width + sx as u32) as usize;
                assert!(!seen[index], "{rotation:?}: ({sx}, {sy}) is read twice");
                seen[index] = true;
            }
            assert!(
                seen.iter().all(|&used| used),
                "{rotation:?}: a pixel of the image is never read"
            );
        }
    }

    #[test]
    fn every_mirrored_rotation_differs_from_its_unmirrored_twin() {
        // If the mirror were dropped, a mirrored setting would silently capture the
        // unmirrored picture, which is wrong in a way that is easy to look at and not
        // notice — the capture would be the right shape and the right way up.
        for (mirrored, plain) in [
            (Rotation::Rotate90Mirrored, Rotation::Rotate90),
            (Rotation::Rotate180Mirrored, Rotation::Rotate180),
            (Rotation::Rotate270Mirrored, Rotation::Rotate270),
        ] {
            let (width, height) = stored(mirrored);
            let differing = every_desktop_point()
                .into_iter()
                .filter(|&(x, y)| {
                    source_for(mirrored, x, y, DESKTOP.0, DESKTOP.1)
                        != source_for(plain, x, y, DESKTOP.0, DESKTOP.1)
                })
                .count();
            assert!(
                differing > 0,
                "{mirrored:?}: identical to {plain:?} over a {width}x{height} image, so the \
                 mirror is not being applied"
            );
        }
    }

    #[test]
    fn the_environment_switch_names_a_rotation() {
        // Set for the duration of this test only; the process is single-threaded here
        // and nothing else reads it.
        unsafe { std::env::set_var("LASSO_FORCE_ROTATION", "90") };
        assert_eq!(forced_by_environment(), Some(Rotation::Rotate90));
        unsafe { std::env::set_var("LASSO_FORCE_ROTATION", "180") };
        assert_eq!(forced_by_environment(), Some(Rotation::Rotate180));
        unsafe { std::env::set_var("LASSO_FORCE_ROTATION", "270") };
        assert_eq!(forced_by_environment(), Some(Rotation::Rotate270));
        unsafe { std::env::set_var("LASSO_FORCE_ROTATION", "0") };
        assert_eq!(forced_by_environment(), Some(Rotation::Identity));
        unsafe { std::env::set_var("LASSO_FORCE_ROTATION", "sideways") };
        assert_eq!(forced_by_environment(), None);
        unsafe { std::env::remove_var("LASSO_FORCE_ROTATION") };
        assert_eq!(forced_by_environment(), None);
    }
}
