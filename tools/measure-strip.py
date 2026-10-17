"""Measure the strip in the screenshot that was reported.

The image came in at 1147x853, which is a scaled capture of the desktop, so the strip's
position has to be found rather than assumed. The strip's plate is a flat dark grey that
nothing else in the screenshot has, so it can be located by looking for it.
"""

import pathlib
import sys

import numpy as np
from PIL import Image

path = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "reported.png")
image = np.array(Image.open(path).convert("RGB"))
height, width = image.shape[:2]
print(f"{path.name}: {width}x{height}")

# The plate is 0x2A2A2A. Allow a couple of levels either way for the capture's own
# resampling.
plate = np.all(np.abs(image.astype(int) - 42) <= 3, axis=2)
ys, xs = np.nonzero(plate)
if len(ys) == 0:
    print("no plate-coloured pixels at all")
    raise SystemExit(0)

print(f"plate pixels: {len(ys)}")
print(f"  x {xs.min()}..{xs.max()}   y {ys.min()}..{ys.max()}")

# The strip is the column with by far the most plate pixels, so find it by column.
per_column = plate.sum(axis=0)
best = int(np.argmax(per_column))
left = best
while left > 0 and per_column[left - 1] > 20:
    left -= 1
right = best
while right + 1 < width and per_column[right + 1] > 20:
    right += 1
print(f"strip columns: {left}..{right} ({right - left + 1} wide)")

rows = plate[:, left : right + 1].sum(axis=1)
lit = np.nonzero(rows > (right - left) * 0.5)[0]
print(f"strip rows: {lit.min()}..{lit.max()}")

# Inside the strip, what colours are present? A white frame over it would show up as a
# large block of near-white; a missing glyph as an absence of bright pixels.
strip = image[lit.min() : lit.max() + 1, left : right + 1]
bright = np.all(strip.astype(int) > 120, axis=2)
print(f"bright ({'>'}120) pixels inside the strip: {bright.sum()} of {strip.shape[0]*strip.shape[1]}")

per_row = bright.sum(axis=1)
print("bright pixels per row band of 20:")
for start in range(0, len(per_row), 20):
    band = per_row[start : start + 20].sum()
    print(f"  rows {start:4d}..{start + 19:4d}: {band:5d}")

# The colours actually present in the strip, most common first.
flat = strip.reshape(-1, 3)
colours, counts = np.unique(flat, axis=0, return_counts=True)
order = np.argsort(-counts)
print("most common colours in the strip:")
for index in order[:8]:
    r, g, b = colours[index]
    print(f"  #{r:02X}{g:02X}{b:02X}: {counts[index]}")
