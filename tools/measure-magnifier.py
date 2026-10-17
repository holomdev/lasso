"""What the magnifier panel holds in each canvas dump.

The panel is drawn as a black outline, a white frame, and then the magnified capture over
the inside. A solid white panel therefore means the last step did not happen; the tell is
the colour at the panel's centre.
"""

import pathlib
import sys

import numpy as np
from PIL import Image

files = sorted(pathlib.Path("target/overlay-idle/canvas").glob("*.png"))
if len(sys.argv) > 1:
    files = [pathlib.Path(p) for p in sys.argv[1:]]

for path in files:
    image = np.array(Image.open(path).convert("RGB"))
    height, width = image.shape[:2]
    print(f"=== {path.name} ({width}x{height}) ===")

    # Find the panel by its white frame: a rectangle of near-white pixels far from the
    # strip and the hints. The strip's plate is dark, so it cannot be confused with this.
    white = np.all(image.astype(int) >= 250, axis=2)
    # Rows and columns with long white runs, which a panel border has and a hint does not.
    row_runs = white.sum(axis=1)
    col_runs = white.sum(axis=0)
    rows = np.nonzero(row_runs > 60)[0]
    cols = np.nonzero(col_runs > 40)[0]
    if len(rows) == 0 or len(cols) == 0:
        print("  no panel frame found")
        counts, values = np.unique(
            image.reshape(-1, 3), axis=0, return_counts=True
        )
        order = np.argsort(-counts)
        print("  most common colours:")
        for index in order[:5]:
            r, g, b = values[index]
            print(f"    #{r:02X}{g:02X}{b:02X}: {counts[index]}")
        continue

    top, bottom = rows.min(), rows.max()
    left, right = cols.min(), cols.max()
    print(f"  panel frame: x {left}..{right}, y {top}..{bottom}")

    # The middle of the panel, which should be the magnified capture.
    centre = image[(top + bottom) // 2, (left + right) // 2]
    print(f"  centre pixel: #{centre[0]:02X}{centre[1]:02X}{centre[2]:02X}")

    inside = image[top + 3 : bottom - 2, left + 3 : right - 2]
    pure_white = np.all(inside.astype(int) >= 250, axis=2).sum()
    print(f"  inside: {pure_white} of {inside.shape[0]*inside.shape[1]} pixels are pure white")

    counts, values = np.unique(inside.reshape(-1, 3), axis=0, return_counts=True)
    order = np.argsort(-counts)
    print("  most common colours inside:")
    for index in order[:5]:
        r, g, b = values[index]
        print(f"    #{r:02X}{g:02X}{b:02X}: {counts[index]}")
    print()
