"""Check that what was drawn reached the saved capture, and stayed in the region.

Written as a file rather than a shell one-liner: PowerShell strips the quotes out of
inline Python, which is how the first attempt turned "result.png" into a syntax error.
"""

import pathlib
import numpy as np
from PIL import Image

out = pathlib.Path("target/overlay-check")
saved = np.array(Image.open(out / "result.png").convert("RGB"))
print(f"result.png: {saved.shape[1]}x{saved.shape[0]}")

# The rectangle was asked for at the region's (40,40)-(300,200), so inside a 500x400
# region it is well away from every edge.
crop = saved[30:210, 30:310]
red = (crop[:, :, 0] > 150) & (crop[:, :, 1] < 90) & (crop[:, :, 2] < 90)
print(f"red pixels in the drawn area: {red.sum()}")

ys, xs = np.nonzero(red)
if len(ys):
    print(f"  spans x {xs.min() + 30}..{xs.max() + 30}, y {ys.min() + 30}..{ys.max() + 30}")

# Nothing outside the region may carry that red: the stroke escaping would mean the
# clamp failed, and the region is what was cropped so anything outside it is a bug.
height, width = saved.shape[:2]
outside = red_outside = (
    (saved[:, :, 0] > 150) & (saved[:, :, 1] < 90) & (saved[:, :, 2] < 90)
)
outside = red_outside.copy()
outside[:400, :500] = False
print(f"red pixels outside the region: {outside.sum()}")

# The strip must not have been baked into the capture. Its plate is a flat dark grey that
# is unlikely to appear by chance in a terminal window, so a large patch of it would mean
# the strip was composed into the saved pixels.
plate = np.all(np.abs(saved.astype(int) - np.array([42, 42, 42])) <= 1, axis=2)
print(f"strip-plate pixels in the capture: {plate.sum()} (expected 0)")
