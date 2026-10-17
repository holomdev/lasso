"""What a PNG looks like where the icons and the drawing should be.

The captured screenshots have a file-open dialog behind the overlay, so reading them by
eye cannot separate the overlay's own pixels from what shows through. This reads the
overlay's back buffer instead, which is only ever the overlay's own composition.
"""

import pathlib
import sys

import numpy as np
from PIL import Image

files = sorted(pathlib.Path("target/overlay-repro").glob("*.png"))
if len(sys.argv) > 1:
    files = [pathlib.Path(p) for p in sys.argv[1:]]

# Where the strip is put for a selection at (500, 300): the metrics put it to the left of
# the region, standing off by 8px, and it is 34px wide and 357px tall.
STRIP = (458, 300, 492, 657)
# Every button's own rectangle, from the same walk the program does.
BUTTON, GAP, PADDING, GROUP_GAP = 26, 2, 4, 5
GROUP_STARTS = {5, 7, 9}


def button_rects():
    rects = []
    y = PADDING
    for index in range(12):
        if index > 0:
            y += GAP
        if index in GROUP_STARTS:
            y += GROUP_GAP
        rects.append((index, y, y + BUTTON))
        y += BUTTON
    return rects


def bright(pixels):
    """Pixels much lighter than the plate, which is 0x2A2A2A."""
    as_int = pixels.astype(int)
    return np.all(as_int > 120, axis=2)


for path in files:
    image = np.array(Image.open(path).convert("RGB"))
    height, width = image.shape[:2]
    print(f"=== {path.name} ({width}x{height}) ===")

    # The strip's own area, where the plate and the glyphs are.
    x0, y0, x1, y1 = STRIP
    if x1 <= width and y1 <= height:
        plate = image[y0:y1, x0:x1]
        print(f"  strip area: {bright(plate).sum()} bright pixels of {plate.shape[0]*plate.shape[1]}")
        for index, top, bottom in button_rects():
            patch = image[y0 + top : y0 + bottom, x0:x1]
            if patch.size == 0:
                continue
            count = bright(patch).sum()
            print(f"    button {index:2d} (y {top:3d}..{bottom:3d}): {count:4d} bright pixels")

    # The red rectangle that was drawn, at the region's (60,60)-(360,260).
    crop = image[300 + 40 : 300 + 280, 500 + 40 : 500 + 400]
    if crop.size:
        red = (
            (crop[:, :, 0] > 150) & (crop[:, :, 1] < 90) & (crop[:, :, 2] < 90)
        )
        print(f"  drawn rectangle: {red.sum()} red pixels")

    # The magnifier panel, wherever it ended up: a large block of pure white would mean its
    # content was never blitted.
    white = np.all(image.astype(int) > 245, axis=2)
    print(f"  near-pure white: {white.sum()} pixels")
    print()
