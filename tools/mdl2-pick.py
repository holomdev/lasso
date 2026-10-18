"""Render one row of candidate glyphs, large, with the codepoint under each.

The contact sheet is for finding; this is for deciding. Nothing goes into the source until the
glyph has been looked at at the size it will be drawn.
"""

import pathlib

from PIL import Image, ImageDraw, ImageFont

FONT = r"C:\Windows\Fonts\segmdl2.ttf"
CANDIDATES = [
    ("Mover?", 0xE740),
    ("Mover?", 0xE741),
    ("Mover?", 0xE7C2),
    ("Flecha", 0xE741),
    ("Flecha", 0xE742),
    ("Cuadro", 0xE739),
    ("Cuadro", 0xE73A),
    ("Cuadro", 0xE7C3),
    ("Cuadro", 0xE8B3),
    ("Circulo", 0xE843),
    ("Circulo", 0xE7C3),
    ("Lapiz", 0xE70F),
    ("Undo", 0xE7A7),
    ("Redo", 0xE7A6),
    ("Guardar", 0xE74E),
    ("Copiar", 0xE8C8),
    ("Cancelar", 0xE711),
]

cell, glyph_px = 96, 56
font = ImageFont.truetype(FONT, glyph_px)
label = ImageFont.truetype(r"C:\Windows\Fonts\segoeui.ttf", 12)

image = Image.new("RGB", (cell * len(CANDIDATES), cell + 20), "white")
draw = ImageDraw.Draw(image)
for index, (name, code) in enumerate(CANDIDATES):
    x = index * cell
    draw.rectangle((x, 0, x + cell - 1, cell + 19), outline=(210, 210, 210))
    text = chr(code)
    box = draw.textbbox((0, 0), text, font=font)
    w, h = box[2] - box[0], box[3] - box[1]
    draw.text((x + (cell - w) / 2 - box[0], 6 - box[1]), text, font=font, fill="black")
    draw.text((x + 4, cell + 2), f"{name} {code:04X}", font=label, fill=(70, 70, 70))

pathlib.Path("target/mdl2").mkdir(parents=True, exist_ok=True)
image.save("target/mdl2/candidates.png")
print(f"{image.width}x{image.height}")
