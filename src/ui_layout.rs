//! Where the settings window puts its controls.
//!
//! The window is per-monitor DPI aware, so Windows does not scale it: at 150% a
//! layout written in fixed pixels is too small for its own contents. Every
//! coordinate therefore comes from [`Layout`], which is one base layout at 96 DPI
//! passed through a single scale factor.
//!
//! Scaling it in one place is not only shorter, it is the only version that looks
//! right. A window whose fields grew while its captions did not is worse than one
//! that never grew at all, and the two easiest things to forget — the window size
//! and the font height — are exactly the two that show up first.
//!
//! Nothing here touches a window, so the arithmetic can be tested at any scale
//! without a second monitor to check it on. `settings_window` reads a `Layout`
//! and creates controls from it; nothing else is allowed to invent a coordinate.

/// The DPI Windows reports for a 100% display.
pub const BASE_DPI: u32 = 96;

/// The base layout, in 96-DPI pixels.
///
/// Every one of these is multiplied by the scale factor exactly once, in
/// [`Layout::new`]. A value read from here anywhere else would be a value that
/// does not scale.
mod base {
    pub const PAD: i32 = 16;
    pub const CLIENT_WIDTH: i32 = 468;
    pub const LABEL_WIDTH: i32 = 96;
    pub const LABEL_GAP: i32 = 8;
    pub const LABEL_HEIGHT: i32 = 18;
    pub const LABEL_DY: i32 = 4;
    pub const FIELD_HEIGHT: i32 = 24;
    pub const ROW: i32 = 24;
    pub const HINT_GAP: i32 = 4;
    pub const HINT_HEIGHT: i32 = 30;
    pub const CHECK_HEIGHT: i32 = 22;
    pub const CHECK_STEP: i32 = 26;
    pub const INSET: i32 = 12;
    pub const GROUP_GAP: i32 = 10;
    pub const AFTER_CAPTION: i32 = 24;
    pub const BUTTON_HEIGHT: i32 = 26;
    pub const BUTTON_GAP: i32 = 8;
    /// Room reserved below a group caption for the shortcut's hint, and for the
    /// folder's, which wraps onto a second line.
    pub const SHORTCUT_TAIL: i32 = 32;
    pub const FILE_TAIL: i32 = 32;
}

/// Convert a value in 96-DPI pixels to the given scale.
///
/// Rounds to the nearest pixel rather than truncating: at 125% truncation loses
/// a pixel from each of the columns independently, which is what makes a scaled
/// grid stop lining up with itself. A half pixel rounds up, which is what the
/// system's own scaling does.
pub fn scale(value: i32, factor: f64) -> i32 {
    if value < 0 {
        return -scale(-value, factor);
    }
    // `f64::from(i32)` is exact, so the only rounding here is the one intended.
    (f64::from(value) * factor).round() as i32
}

/// The scale factor for a DPI value.
///
/// A DPI Windows reports as zero would divide by zero and collapse the whole
/// layout onto the origin, so it is treated as the base instead.
pub fn factor(dpi: u32) -> f64 {
    if dpi == 0 {
        return 1.0;
    }
    f64::from(dpi) / f64::from(BASE_DPI)
}

/// The width of a push button whose label measures `text_width`.
///
/// The margin grows with the label, so a scaled button is exactly its scaled
/// self: a fixed margin would make the text a slightly larger share of the
/// button on a denser display than on a coarser one.
pub fn button_width(text_width: i32, factor: f64) -> i32 {
    scale(text_width + BUTTON_TEXT_MARGIN, factor)
}

/// The room a button keeps around its label, in 96-DPI pixels.
///
/// Nine pixels either side of the text: enough for the border and the focus
/// rectangle, which is drawn inside the border and is what gets cut off first
/// when a button is sized to its text exactly.
const BUTTON_TEXT_MARGIN: i32 = 18;

/// A rectangle in client coordinates, as `(x, y, width, height)`.
pub type Rect = (i32, i32, i32, i32);

/// Where every control in the settings window goes, in client coordinates.
///
/// The window is a three-column grid — indent, label, field — cut into groups by
/// frames. Each frame reserves a caption row, then the controls, and the indent
/// applies to everything inside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// The display this layout was built for. Kept rather than the scale factor
    /// so a DPI notification can be compared with the value it came from.
    pub dpi: u32,
    /// Window client size. The caller turns it into a window size.
    pub client_width: i32,
    pub client_height: i32,
    /// The margin around the whole window.
    pub pad: i32,
    /// Width of the label column, right-aligned.
    pub label_width: i32,
    /// The gap between a right-aligned label and the field it belongs to.
    pub label_gap: i32,
    pub label_height: i32,
    pub label_dy: i32,
    /// Height of a single-line edit.
    pub field_height: i32,
    pub row: i32,
    /// The gap between a field and the hint that explains it.
    pub hint_gap: i32,
    pub hint_height: i32,
    pub check_height: i32,
    pub check_step: i32,
    /// How far the contents of a group sit inside its frame.
    pub inset: i32,
    /// The gap between one group and the next.
    pub group_gap: i32,
    /// How far the controls of a group sit below its caption.
    pub after_caption: i32,

    /// Left and right edges shared by every frame, and the width between them.
    pub frame_left: i32,
    pub frame_right: i32,
    pub frame_width: i32,
    /// The label and field columns, inside a frame.
    pub label_left: i32,
    pub field_left: i32,
    pub field_width: i32,

    /// Width of the browse button.
    pub small_button: i32,

    /// The shortcut group.
    pub shortcut_top: i32,
    pub shortcut_height: i32,
    /// The two destination check boxes, which share a group.
    pub after_top: i32,
    pub after_height: i32,
    /// The output folder, which the check box above it depends on.
    pub file_top: i32,
    pub file_height: i32,
    /// What a left click on the tray icon does.
    pub tray_top: i32,
    pub tray_height: i32,

    /// The button row, anchored to the bottom right.
    pub button_width: i32,
    pub button_height: i32,
    pub button_gap: i32,
    pub button_y: i32,
    pub save_x: i32,
    pub cancel_x: i32,
}

impl Layout {
    /// The layout for a display, with the two button widths supplied by the
    /// caller because they depend on the font rather than on a constant.
    pub fn new(dpi: u32, small_button: i32, wide_button: i32) -> Self {
        let factor = factor(dpi);
        let at = |value: i32| scale(value, factor);

        let pad = at(base::PAD);
        let client_width = at(base::CLIENT_WIDTH);
        let label_width = at(base::LABEL_WIDTH);
        let label_gap = at(base::LABEL_GAP);
        let label_height = at(base::LABEL_HEIGHT);
        let label_dy = at(base::LABEL_DY);
        let field_height = at(base::FIELD_HEIGHT);
        let row = at(base::ROW);
        let hint_gap = at(base::HINT_GAP);
        let hint_height = at(base::HINT_HEIGHT);
        let check_height = at(base::CHECK_HEIGHT);
        let check_step = at(base::CHECK_STEP);
        let inset = at(base::INSET);
        let group_gap = at(base::GROUP_GAP);
        let after_caption = at(base::AFTER_CAPTION);
        let button_height = at(base::BUTTON_HEIGHT);
        let button_gap = at(base::BUTTON_GAP);

        // The frame spans the window less the outer margin. The label and field
        // columns are measured inside the frame, so the caption above them has
        // the full width and never runs into the field beside it.
        let frame_left = pad;
        let frame_right = client_width - pad;
        let frame_width = frame_right - frame_left;
        let label_left = frame_left + inset;
        let field_left = label_left + label_width + label_gap;
        let field_width = frame_right - inset - field_left;

        let hint_top = field_height + hint_gap;

        // The shortcut group: a label and a field on one row, its hint below.
        let shortcut_top = pad;
        let shortcut_height = after_caption + hint_top + at(base::SHORTCUT_TAIL);

        // The two destination check boxes, each a full-width row.
        let after_top = shortcut_top + shortcut_height + group_gap;
        let after_height = after_caption + check_height + check_step + hint_gap;

        // The output folder: the same label-and-field row, with the browse button
        // sharing the field's row and a wrapping hint below.
        let file_top = after_top + after_height + group_gap;
        let file_height = after_caption + hint_top + at(base::FILE_TAIL);

        // The tray group: one full-width button and its hint.
        let tray_top = file_top + file_height + group_gap;
        let tray_height = after_caption + row + hint_gap + label_height;

        let button_y = tray_top + tray_height + pad;
        let client_height = button_y + button_height + pad;

        let cancel_x = frame_right - wide_button;
        let save_x = cancel_x - wide_button - button_gap;

        Self {
            dpi,
            client_width,
            client_height,
            pad,
            label_width,
            label_gap,
            label_height,
            label_dy,
            field_height,
            row,
            hint_gap,
            hint_height,
            check_height,
            check_step,
            inset,
            group_gap,
            after_caption,
            frame_left,
            frame_right,
            frame_width,
            label_left,
            field_left,
            field_width,
            small_button,
            shortcut_top,
            shortcut_height,
            after_top,
            after_height,
            file_top,
            file_height,
            tray_top,
            tray_height,
            button_width: wide_button,
            button_height,
            button_gap,
            button_y,
            save_x,
            cancel_x,
        }
    }

    /// The top of the first control inside a group, given the group's top.
    pub fn content_top(&self, group_top: i32) -> i32 {
        group_top + self.after_caption
    }

    /// The right edge shared by every control inside a frame.
    pub fn inner_right(&self) -> i32 {
        self.frame_right - self.inset
    }

    /// The rectangle of a group's frame.
    pub fn group(&self, top: i32, height: i32) -> Rect {
        (self.frame_left, top, self.frame_width, height)
    }

    /// The rectangle of a right-aligned label on the row whose top is `top`.
    pub fn label(&self, top: i32) -> Rect {
        (
            self.label_left,
            top + self.label_dy,
            self.label_width,
            self.label_height,
        )
    }

    /// The rectangle of a field on the row whose top is `top`.
    pub fn field(&self, top: i32, width: i32) -> Rect {
        (self.field_left, top, width, self.field_height)
    }

    /// The rectangle of a hint below the row whose top is `top`.
    ///
    /// A hint spans the whole inner width rather than the field above it. It is
    /// the one thing in a group that is not a label or a field, and a line long
    /// enough to wrap should have the room to do it: a static control does not
    /// wrap by itself, so text wider than its rectangle is simply clipped, with no
    /// ellipsis to show for it.
    pub fn hint(&self, top: i32) -> Rect {
        (
            self.label_left,
            top + self.field_height + self.hint_gap,
            self.inner_right() - self.label_left,
            self.hint_height,
        )
    }

    /// The output folder's label, field and browse button.
    pub fn output(&self) -> (Rect, Rect, Rect) {
        let top = self.content_top(self.file_top);
        let browse_x = self.inner_right() - self.small_button;
        let field_width = browse_x - self.field_left - self.label_gap;
        (
            self.label(top),
            self.field(top, field_width),
            (browse_x, top, self.small_button, self.field_height),
        )
    }

    /// The two destination check boxes, top first.
    pub fn checks(&self) -> (Rect, Rect) {
        let left = self.label_left;
        let width = self.inner_right() - left;
        let first = self.content_top(self.after_top);
        (
            (left, first, width, self.check_height),
            (left, first + self.check_step, width, self.check_height),
        )
    }

    /// The tray click button and its hint.
    pub fn tray(&self) -> (Rect, Rect) {
        let left = self.label_left;
        let width = self.inner_right() - left;
        let top = self.content_top(self.tray_top);
        (
            (left, top, width, self.row),
            (
                left,
                top + self.row + self.hint_gap,
                width,
                self.label_height,
            ),
        )
    }

    /// The Save and Cancel buttons.
    pub fn buttons(&self) -> (Rect, Rect) {
        let width = self.button_width;
        (
            (self.save_x, self.button_y, width, self.button_height),
            (self.cancel_x, self.button_y, width, self.button_height),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The DPIs a Windows display reports, from 100% to 300%. Every step is a
    /// scale Windows actually offers, so the rounding each one needs is covered.
    fn dpis() -> Vec<u32> {
        (96..=288).step_by(8).collect()
    }

    /// The widths of the longest label each button carries, at 96 DPI. "Cancel"
    /// and "Save" share the wider of the two; "Browse..." is measured on its own.
    /// They are constants rather than formulas because measuring a font is exactly
    /// what a pure layout test cannot do, and the arithmetic below does not depend
    /// on how they were obtained.
    const BROWSE_TEXT: i32 = 36;
    const PAIR_TEXT: i32 = 54;

    /// Build a layout the way the window does, from measured label widths.
    fn layout(dpi: u32) -> Layout {
        let factor = factor(dpi);
        Layout::new(
            dpi,
            button_width(BROWSE_TEXT, factor),
            button_width(PAIR_TEXT, factor),
        )
    }

    #[test]
    fn a_scale_of_one_reproduces_the_base_layout() {
        assert_eq!(scale(16, 1.0), 16);
        assert_eq!(scale(24, 1.0), 24);
        assert_eq!(factor(96), 1.0);
        assert_eq!(factor(144), 1.5);
        assert_eq!(factor(192), 2.0);
        let base = layout(BASE_DPI);
        assert_eq!(base.dpi, BASE_DPI);
        assert_eq!(base.pad, 16);
        assert_eq!(base.frame_width, base.client_width - base.pad * 2);
    }

    #[test]
    fn scaling_rounds_rather_than_truncates() {
        // Truncation drops a pixel from each of the columns independently, which
        // is what makes a scaled grid stop lining up with itself.
        assert_eq!(scale(24, 1.25), 30);
        assert_eq!(scale(10, 1.25), 13); // 12.5
        assert_eq!(scale(10, 1.75), 18); // 17.5
        assert_eq!(scale(7, 1.5), 11); // 10.5
        assert_eq!(scale(9, 1.5), 14); // 13.5
        // Nothing is lost when a value does not divide, and the sign survives.
        assert_eq!(scale(1, 1.25), 1);
        assert_eq!(scale(-24, 1.25), -30);
    }

    #[test]
    fn a_missing_dpi_falls_back_to_unscaled() {
        // Zero is what a failed query returns, and dividing by it would collapse
        // every control onto the origin.
        assert_eq!(factor(0), 1.0);
    }

    #[test]
    fn a_button_gets_wider_with_its_text_and_with_its_scale() {
        let narrow = button_width(40, 1.0);
        let wide = button_width(80, 1.0);
        assert!(wide > narrow);
        assert_eq!(wide - narrow, 40);
        // The margin grows with the label, so a scaled button is its scaled self.
        assert_eq!(button_width(40, 2.0), narrow * 2);
        // Even an empty label still gets room for its border.
        assert!(button_width(0, 1.0) >= 14);
    }

    #[test]
    fn the_button_row_ends_inside_the_window_at_every_scale() {
        for dpi in dpis() {
            let l = layout(dpi);
            let (save, cancel) = l.buttons();
            assert!(
                cancel.0 + cancel.2 <= l.frame_right,
                "at {dpi} dpi the cancel button runs past the right edge"
            );
            assert!(
                save.0 + save.2 + l.button_gap <= cancel.0,
                "at {dpi} dpi the save button overlaps the cancel button"
            );
            assert!(
                save.1 + save.3 + l.pad <= l.client_height,
                "at {dpi} dpi the button row falls below the client area"
            );
            assert!(
                save.0 >= l.pad,
                "at {dpi} dpi the button row runs past the left edge"
            );
        }
    }

    #[test]
    fn the_window_is_tall_enough_for_its_last_group() {
        for dpi in dpis() {
            let l = layout(dpi);
            let bottom = l.tray_top + l.tray_height;
            assert!(
                bottom < l.button_y,
                "at {dpi} dpi the tray group ends at {bottom} and the buttons start at {}",
                l.button_y
            );
            assert_eq!(
                l.button_y + l.button_height + l.pad,
                l.client_height,
                "at {dpi} dpi the client height does not follow from the button row"
            );
        }
    }

    #[test]
    fn every_column_stays_inside_its_frame() {
        for dpi in dpis() {
            let l = layout(dpi);
            let inner_right = l.inner_right();
            let (label, field, browse) = l.output();

            assert!(
                label.0 + label.2 + l.label_gap <= field.0,
                "at {dpi} dpi the label column runs into the field column"
            );
            assert!(
                browse.0 + browse.2 == inner_right,
                "at {dpi} dpi the browse button does not end at the frame's inner edge"
            );
            assert!(
                field.0 + field.2 + l.label_gap <= browse.0,
                "at {dpi} dpi the folder field runs into the browse button"
            );

            for (left, _, width, _) in [l.checks().0, l.checks().1, l.tray().0, l.tray().1] {
                assert_eq!(
                    left, l.label_left,
                    "at {dpi} dpi a full-width control leaves the column"
                );
                assert!(
                    left + width <= inner_right,
                    "at {dpi} dpi a full-width control leaves its frame"
                );
            }
        }
    }

    #[test]
    fn the_groups_do_not_overlap_and_keep_their_order() {
        for dpi in dpis() {
            let l = layout(dpi);
            let bottom = |top: i32, height: i32| top + height;
            assert!(
                bottom(l.shortcut_top, l.shortcut_height) <= l.after_top,
                "at {dpi} dpi the shortcut group overlaps the next one"
            );
            assert!(bottom(l.after_top, l.after_height) <= l.file_top);
            assert!(bottom(l.file_top, l.file_height) <= l.tray_top);
            assert!(
                l.shortcut_top >= l.pad,
                "at {dpi} dpi the first group starts above the margin"
            );
        }
    }

    #[test]
    fn the_controls_of_a_group_sit_below_its_caption() {
        for dpi in dpis() {
            let l = layout(dpi);
            let (_, field, _) = l.output();
            assert_eq!(field.1, l.content_top(l.file_top));
            assert_eq!(l.checks().0.1, l.content_top(l.after_top));
            assert_eq!(l.tray().0.1, l.content_top(l.tray_top));
            assert!(
                l.after_caption > 0,
                "at {dpi} dpi the frame caption has no room of its own"
            );
        }
    }

    #[test]
    fn a_hint_follows_its_field_without_a_gap_or_an_overlap() {
        for dpi in dpis() {
            let l = layout(dpi);
            let field = l.field(0, l.field_width);
            let hint = l.hint(0);
            assert_eq!(
                hint.1,
                field.1 + field.3 + l.hint_gap,
                "at {dpi} dpi the hint does not follow its field"
            );
            assert!(hint.1 > field.1 + field.3);
        }
    }

    #[test]
    fn the_window_grows_whole_with_the_scale() {
        let small = layout(BASE_DPI);
        let large = layout(BASE_DPI * 2);
        assert_eq!(large.client_width, small.client_width * 2);
        assert!(large.client_height > small.client_height);
        // The room below the groups scales with everything else, so no group
        // takes a larger share of the window as the display gets denser.
        assert_eq!(
            large.client_height - large.button_y,
            (small.client_height - small.button_y) * 2
        );
        assert_eq!(large.pad, small.pad * 2);
        assert_eq!(large.field_height, small.field_height * 2);
    }

    #[test]
    fn the_label_column_holds_its_longest_label() {
        // "Output folder" is the longest label. Right-aligned text wider than the
        // column grows leftwards, over whatever is beside it, so the column has
        // room for it at the base scale and the field still clears the gap.
        let l = layout(BASE_DPI);
        assert_eq!(l.label_width, 96);
        assert_eq!(l.label_left, l.frame_left + l.inset);
        assert!(l.field_left >= l.label_left + l.label_width + l.label_gap);
        assert!(l.field_width > l.label_width);
    }

    #[test]
    fn a_hint_never_reaches_the_frame_around_it() {
        // A static control does not wrap by itself: text wider than the rectangle
        // it was given is cut off at the edge, with no ellipsis to show for it.
        // A hint is given the whole inner width, which leaves the frame its own
        // margin on the right and lines up with the check boxes on the left.
        for dpi in dpis() {
            let l = layout(dpi);
            let hint = l.hint(0);
            assert_eq!(
                hint.0, l.label_left,
                "at {dpi} dpi the hint does not line up with the rows above it"
            );
            assert!(
                hint.0 + hint.2 <= l.inner_right(),
                "at {dpi} dpi the hint reaches the frame"
            );
            assert!(
                l.inner_right() - (hint.0 + hint.2) <= l.inset,
                "at {dpi} dpi the hint leaves more margin than the frame's own inset"
            );
        }
    }

    #[test]
    fn a_button_has_room_for_its_border() {
        // The margin is what the border and the focus rectangle live in. A button
        // sized to its text exactly clips the label's last character. The size is
        // rounded once, from the text and the margin together, so it can land a
        // pixel to either side of the two rounded separately.
        for dpi in dpis() {
            let l = layout(dpi);
            let (save, cancel) = l.buttons();
            let text = scale(PAIR_TEXT, factor(dpi));
            let margin = scale(BUTTON_TEXT_MARGIN, factor(dpi));
            assert!(
                save.2 + 1 >= text + margin,
                "at {dpi} dpi the save button has no room around its label"
            );
            assert_eq!(
                save.2, cancel.2,
                "at {dpi} dpi the two buttons are not the same width"
            );
            assert!(
                margin >= 18,
                "at {dpi} dpi the margin is too small to draw a focus rectangle in"
            );
        }
    }
}
