"""Render a contact sheet of the Segoe MDL2 Assets glyphs, so the right ones are chosen by looking.

Guessing codepoints from memory is how an icon ends up being a printer instead of a pencil. This
writes the range as a labelled grid; the labels are the hex codepoints that go in the source.
"""

import pathlib

from PIL import Image, ImageDraw, ImageFont

FONT = pathlib.Path(r"C:\Windows\Fonts\segmdl2.ttf")
OUT = pathlib.Path("target/mdl2")

start, end = 0xE700, 0xE8FF
columns = 16
cell_w, cell_h = 64, 74
glyph_px = 30

font = ImageFont.truetype(str(FONT), glyph_px)
label = ImageFont.truetype(r"C:\Windows\Fonts\segoeui.ttf", 11)

rows = (end - start + 1 + columns - 1) // columns
image = Image.new("RGB", (columns * cell_w, rows * cell_h), "white")
draw = ImageDraw.Draw(image)

for index, code in enumerate(range(start, end + 1)):
    x = (index % columns) * cell_w
    y = (index // columns) * cell_h
    draw.rectangle((x, y, x + cell_w - 1, y + cell_h - 1), outline=(220, 220, 220))
    text = chr(code)
    try:
        box = draw.textbbox((0, 0), text, font=font)
        w, h = box[2] - box[0], box[3] - box[1]
        draw.text((x + (cell_w - w) / 2 - box[0], y + 8 - box[1]), text, font=font, fill="black")
    except Exception as error:  # a codepoint the font does not draw
        draw.text((x + 8, y + 12), "?", font=font, fill=(200, 0, 0))
        print(code, error)
    draw.text((x + 4, y + cell_h - 16), f"{code:04X}", font=label, fill=(90, 90, 90))

OUT.mkdir(parents=True, exist_ok=True)
image.save(OUT / "sheet.png")
print(f"{rows} filas, {columns} columnas, {image.width}x{image.height}")
