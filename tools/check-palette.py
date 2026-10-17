"""Did hovering open the palette, and did a click change the colour?

Two different measurements, in two different places: the palette's band for the swatches, and
the colour button's own square for the colour in force. Measuring both in the same place was
wrong the first time round — the button was asked about in the palette's band, where its
colour is not, and the answer looked like a failure.

The palette's band overlaps bright wallpaper, so "no white pixels" proves nothing. What it
cannot contain is pure red, yellow, green and blue at once: four saturated primaries side by
side is the palette and nothing else.
"""

import pathlib
import numpy as np
from PIL import Image

OUT = pathlib.Path("target/overlay-palette")
PALETTE = (600, 393, 746, 427)  # the band the palette occupies
BUTTON = (562, 399, 588, 425)  # the colour button's own square

# As the buffer holds them: to_colorref is 0x00BBGGRR.
COLOURS = {
    "red": (255, 0, 0),
    "yellow": (255, 255, 0),
    "green": (0, 255, 0),
    "blue": (0, 0, 255),
    "white": (255, 255, 255),
}
# The four that cannot occur together by accident.
SATURATED = ("red", "yellow", "green", "blue")


def count(image, colour, box, tolerance=6):
    band = image[box[1] : box[3], box[0] : box[2]]
    return int(np.all(np.abs(band.astype(int) - np.array(colour)) <= tolerance, axis=2).sum())


shots = {}
for path in sorted(OUT.glob("*.png")):
    if path.name == "result.png":
        continue
    image = np.array(Image.open(path).convert("RGB"))
    shots[path.stem] = {
        "palette": {name: count(image, colour, PALETTE) for name, colour in COLOURS.items()},
        "button": {name: count(image, colour, BUTTON) for name, colour in COLOURS.items()},
    }

print(f"{'shot':<18} {'palette: r/y/g/b':<20} button")
print("-" * 58)
for name, shot in shots.items():
    swatches = "/".join(str(shot["palette"][c]) for c in SATURATED)
    held = max(shot["button"], key=lambda c: shot["button"][c])
    print(f"{name:<18} {swatches:<20} {held} ({shot['button'][held]} px)")


def palette_open(shot):
    """Every saturated colour on screen at once: the palette and nothing else."""
    return all(shots[shot]["palette"][c] >= 100 for c in SATURATED)


def held(shot):
    """The colour the button is showing, if it is showing one clearly."""
    counts = shots[shot]["button"]
    name = max(counts, key=lambda c: counts[c])
    return name if counts[name] >= 100 else None


failures = []
if palette_open("0-no-hover"):
    failures.append("the palette is on screen with the pointer away from the colour button")
if not palette_open("1-hover"):
    failures.append("hovering the colour button does not open the palette")
if not palette_open("2-on-swatch"):
    failures.append("the palette closes when the pointer moves onto a swatch")
if held("1-hover") != "red":
    failures.append(f"before choosing, the button should show red, and shows {held('1-hover')}")
if held("3-picked-blue") != "blue":
    failures.append(f"after clicking blue the button should be blue, and is {held('3-picked-blue')}")
if palette_open("4-closed"):
    failures.append("the palette stays on screen after the pointer leaves it")
if held("4-closed") != "blue":
    failures.append(f"the chosen colour is lost when the palette closes: {held('4-closed')}")

print()
print(
    "FAILED"
    if failures
    else "the palette opens on hover, offers every colour, and the button keeps the chosen one"
)
for line in failures:
    print("  " + line)
