"""Is the strip still drawn while a stroke is in progress?

The strip is drawn beside the selection at a known place, so its plate and its icon ink can
be counted in the same band of each screenshot. Mid-drag and after the drag are the same
strip in the same place, so the counts have to match: if the mid-drag one has the plate but
no ink, the strip was cleared and not redrawn — which is what "it disappears while I paint"
is.

Counted as exact colours rather than "something is there": the plate alone proves nothing,
because the plate is what survives when only the glyphs are lost.
"""

import pathlib
import numpy as np
from PIL import Image

OUT = pathlib.Path("target/overlay-midrag")

# The strip's band, in screen coordinates. Wider and taller than the strip itself so a
# misplaced or clipped one still lands inside the window being counted.
STRIP = (548, 236, 604, 624)

INK = (224, 224, 224)  # BUTTON_INK
PLATE = (42, 42, 42)  # the strip's background


def count(image, colour, tolerance=2):
    band = image[STRIP[1] : STRIP[3], STRIP[0] : STRIP[2]]
    return int(np.all(np.abs(band.astype(int) - np.array(colour)) <= tolerance, axis=2).sum())


print(f"{'shot':<22} {'plate':>8} {'ink':>8}")
print("-" * 40)
results = {}
for path in sorted(OUT.glob("*.png")):
    if path.name == "result.png":
        continue
    image = np.array(Image.open(path).convert("RGB"))
    plate, ink = count(image, PLATE), count(image, INK)
    results[path.stem] = (plate, ink)
    print(f"{path.stem:<22} {plate:>8} {ink:>8}")

print()
failures = []
for name, (plate, ink) in results.items():
    if name.startswith("1-mid-"):
        tool = name[len("1-mid-") :]
        after = results.get(f"2-after-{tool}")
        if after is None:
            continue
        if plate == 0:
            failures.append(f"{tool}: no strip at all while drawing")
        elif ink == 0:
            failures.append(f"{tool}: plate drawn but no icons while drawing")
        elif after[1] > 0 and ink < after[1] * 0.5:
            failures.append(
                f"{tool}: {ink} ink pixels while drawing against {after[1]} after it"
            )

if failures:
    print("FAILED")
    for line in failures:
        print("  " + line)
else:
    print("every tool kept its strip and its icons while the stroke was in progress")
