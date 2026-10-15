"""Generate the application icon.

The icon is a selection frame with corner grips, the same shape the overlay
draws, so the tray icon says what the program does. It is generated rather than
hand-drawn so every size inside the .ico stays consistent with the others.

Run from the project root:

    python tools/make_icon.py
"""

from pathlib import Path

from PIL import Image, ImageDraw

# The master is rendered large and anti-aliased by supersampling, then Pillow
# downsamples it into every size the .ico carries.
MASTER = 256
SUPERSAMPLE = 4

BACKGROUND = (26, 32, 44, 255)
FRAME = (206, 232, 255, 255)
GRIP = (72, 196, 235, 255)

ICON_SIZES = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]


def draw(size: int) -> Image.Image:
    """Render the icon at `size` pixels.

    Small sizes get a different design on purpose. At 16 px — which is how the
    notification area draws the icon at 100% scaling — a frame *and* four grips
    are more detail than the pixels can carry, and the two features blur into
    one blob. There the frame is drawn bolder instead, and the grips are left
    out.
    """
    scale = size * SUPERSAMPLE
    detailed = size >= 24

    image = Image.new("RGBA", (scale, scale), (0, 0, 0, 0))
    pen = ImageDraw.Draw(image)

    # A rounded plate keeps the glyph readable on light and dark taskbars alike.
    inset = scale * (0.02 if detailed else 0.0)
    pen.rounded_rectangle(
        [inset, inset, scale - inset, scale - inset],
        radius=scale * (0.22 if detailed else 0.19),
        fill=BACKGROUND,
    )

    # The selection frame, larger and bolder when there are few pixels to use.
    margin = scale * (0.245 if detailed else 0.20)
    stroke = max(1, round(scale * (0.055 if detailed else 0.085)))
    pen.rectangle(
        [margin, margin, scale - margin, scale - margin],
        outline=FRAME,
        width=stroke,
    )

    if detailed:
        # Corner grips: the same affordance the overlay offers for resizing.
        grip = scale * 0.105
        half = grip / 2
        for x in (margin, scale - margin):
            for y in (margin, scale - margin):
                pen.rounded_rectangle(
                    [x - half, y - half, x + half, y + half],
                    radius=grip * 0.32,
                    fill=GRIP,
                )

    return image.resize((size, size), Image.LANCZOS)


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    assets = root / "assets"
    assets.mkdir(exist_ok=True)

    icon = assets / "lasso.ico"
    draw(MASTER).save(icon, sizes=ICON_SIZES)
    print(f"wrote {icon} ({icon.stat().st_size} bytes)")

    # A magnified sheet of the small sizes, to judge them by eye rather than by
    # hoping they downsample well.
    zoom = 6
    pad = 12
    sizes = [16, 24, 32, 48]
    width = sum(size * zoom + pad for size in sizes) + pad
    height = max(sizes) * zoom + 2 * pad
    sheet = Image.new("RGBA", (width, height), (122, 122, 122, 255))
    x = pad
    for size in sizes:
        tile = draw(size).resize((size * zoom, size * zoom), Image.NEAREST)
        sheet.alpha_composite(tile, (x, pad))
        x += size * zoom + pad

    preview = root / "target" / "test-output" / "icon-preview.png"
    preview.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(preview)
    print(f"wrote {preview} (16, 24, 32 and 48 px, magnified {zoom}x)")


if __name__ == "__main__":
    main()
