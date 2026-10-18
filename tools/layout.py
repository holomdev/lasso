"""The strip's geometry, in one place, for the checks that have to click it.

Every check used to keep its own copy of these numbers. When the strip was redrawn bigger, each copy
was wrong in the same way and the checks failed on the layout rather than on the program — the same
mistake the program itself made once, with its palette rules written twice. These numbers mirror
`Metrics::BASE` and `button_top` mirrors the walk in `toolbar.rs`.
"""

# The card's colours: a light plate, a hairline border, dark ink.
PLATE = (0xF7, 0xF8, 0xFA)
EDGE = (0xDC, 0xDF, 0xE6)
INK = (0x37, 0x41, 0x51)
# A palette is drawn on white, not on the bar's plate.
POPUP = (0xFF, 0xFF, 0xFF)

BUTTON = 36
GAP = 4
PADDING = 6
GROUP_GAP = 10
STANDOFF = 10
RADIUS = 8
PANEL_RADIUS = 12
STRIP_WIDTH = BUTTON + PADDING * 2
# The program adds a gap between every pair of buttons and a group gap on top of that for
# each group that starts, so the two are summed, not substituted. Getting this wrong cropped
# the last button off every photograph of the strip.
STRIP_HEIGHT = 12 * BUTTON + 11 * GAP + 3 * GROUP_GAP + 2 * PADDING

# The buttons that start a group, by index: Colour, Undo, Save.
GROUPS = {5, 7, 9}
ORDER = [
    "move",
    "arrow",
    "rectangle",
    "ellipse",
    "pencil",
    "colour",
    "thickness",
    "undo",
    "redo",
    "save",
    "copy",
    "cancel",
]


def button_top(index):
    """The top of a button, from the same walk the toolbar does."""
    y = PADDING
    for i in range(index):
        y += GAP
        if i in GROUPS:
            y += GROUP_GAP
        y += BUTTON
    if index in GROUPS:
        y += GROUP_GAP - GAP
    return y


SCREEN = (1920, 1080)


def strip_x(region):
    """The card's left edge: it stands off the region, on the left when there is room."""
    left_of = region[0] - STANDOFF - STRIP_WIDTH
    if left_of >= 0:
        return left_of
    return region[0] + (region[2] - region[0]) + STANDOFF


def strip_top(region, screen=SCREEN):
    """The card's top, which is the region's own top unless that would push it off the screen."""
    return max(0, min(region[1], screen[1] - STRIP_HEIGHT))


def button_centre(index, region, screen=SCREEN):
    """A button's centre on the screen, for a region."""
    return (
        strip_x(region) + BUTTON // 2 + PADDING,
        strip_top(region, screen) + button_top(index) + BUTTON // 2,
    )


def palette_size(count):
    return (count * BUTTON + (count - 1) * GAP + 2 * PADDING, BUTTON + 2 * PADDING)


def palette_box(count, button_index, region, screen=SCREEN):
    """The palette's rectangle for a button, which opens on the region's side of the card.

    The card stands off the region by the same distance the palette stands off the card, so the
    palette's near edge is the region's edge — or the card's outer edge when the card is on the
    right.
    """
    width, height = palette_size(count)
    centre_y = button_centre(button_index, region, screen)[1]
    left = strip_x(region)
    if left < region[0]:
        x = region[0]
    else:
        x = left - STANDOFF - width
    return (x, centre_y - height // 2, width, height)


def cell_centre(index, count, button_index, region):
    """The centre of one palette cell."""
    left, top, _, _ = palette_box(count, button_index, region)
    return (
        left + PADDING + index * (BUTTON + GAP) + BUTTON // 2,
        top + PADDING + BUTTON // 2,
    )


def cell_box(index, count, button_index, region):
    left, top, _, _ = palette_box(count, button_index, region)
    x = left + PADDING + index * (BUTTON + GAP)
    y = top + PADDING
    return (x, y, BUTTON, BUTTON)


def strip_visible(screen, region):
    """Whether the card is on screen, which is how a check knows its drag took.

    Counted over the whole screen rather than inside the strip's band, so a check does not have to
    know where the strip ended up before it can tell whether there is one.
    """
    import numpy as np

    plate = np.all(np.abs(screen.astype(int) - np.array(PLATE)) <= 1, axis=2)
    # A card is thousands of plate pixels; a wallpaper has scattered ones.
    return int(plate.sum()) > 3000


def strip_ink(screen, region):
    """The icon pixels on the card, counted wherever the card is."""
    import numpy as np

    left = strip_x(region)
    band = screen[region[1]:region[1] + STRIP_HEIGHT, left:left + STRIP_WIDTH]
    ink = np.all(np.abs(band.astype(int) - np.array(INK)) <= 2, axis=2)
    return int(ink.sum())
