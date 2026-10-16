#!/usr/bin/env python3
# /// script
# requires-python = ">=3.9"
# dependencies = ["Pillow==11.3.0"]
# ///
"""Rebuild the original daemon artwork: uv run scripts/generate-brand.py."""
from pathlib import Path
import subprocess
import tempfile
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "assets/brand"
OUT.mkdir(parents=True, exist_ok=True)
ACID = "#D7FF3F"
CARBON = "#10120D"
# A 100-unit grid, with broad negative spaces that survive small rendering.
BRACKET = [(22, 6), (38, 6), (38, 15), (31, 15), (31, 46), (22, 46)]
EYE = [(38, 24), (47, 30), (47, 37), (42, 40), (37, 36)]
CLAW = [(24, 50), (34, 52), (30, 64), (35, 70), (28, 76),
        (32, 89), (23, 92), (13, 89), (6, 81), (4, 71), (7, 61), (14, 54)]
def mirror(points):
    return [(100 - x, y) for x, y in points]
SHAPES = [BRACKET, mirror(BRACKET), EYE, mirror(EYE), CLAW, mirror(CLAW)]

def svg_polygons(shapes, viewbox, path):
    data = " ".join("M" + " L".join(f"{x},{y}" for x,y in p) + " Z" for p in shapes)
    body = f'<path d="{data}"/>'
    path.write_text(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{viewbox}" fill="#000000">{body}</svg>\n')

svg_polygons(SHAPES, '0 0 100 100', OUT / 'daemon.svg')
# The wordmark is original lettering on a seven-row pixel grid.
letters = [
    ['00000','11110','11011','11011','11110','11000','11000'],
    ['11','00','11','11','11','11','11'],
    ['00000','00000','11110','11011','11011','11011','11011'],
    ['00000','00000','01111','11000','11000','11000','01111'],
    ['11000','11000','11110','11011','11011','11011','11011'],
]
x = 0
pixels = []
for letter in letters:
    for y, row in enumerate(letter):
        for col, bit in enumerate(row):
            if bit == '1':
                pixels.append([(x+col,y),(x+col+1,y),(x+col+1,y+1),(x+col,y+1)])
    x += len(letter[0]) + 1
svg_polygons(pixels, f'0 0 {x-1} 7', OUT / 'wordmark.svg')

def render(size, shapes=SHAPES, app=False):
    scale = 4
    image = Image.new('RGBA', (size*scale, size*scale))
    draw = ImageDraw.Draw(image)
    if app:
        draw.rounded_rectangle((size*.06*scale,size*.06*scale,size*.94*scale,size*.94*scale), radius=size*.14*scale, fill=ACID)
    for polygon in shapes:
        points = [((.14+x*.0072)*size*scale, (.14+y*.0072)*size*scale) if app
                  else (x/100*size*scale, y/100*size*scale) for x,y in polygon]
        draw.polygon(points, fill=CARBON if app else '#000000')
    return image.resize((size,size), Image.Resampling.LANCZOS)

render(1024, app=True).save(OUT / 'app-icon.png')
# A monochrome, optically enlarged pair of claws for the status item.
menu_shapes = [[((x-4)/92*100, (y-50)/42*100) for x,y in p] for p in [CLAW, mirror(CLAW)]]
svg_polygons(menu_shapes, '0 0 100 100', OUT / 'menu-bar.svg')
menu = render(96, menu_shapes).resize((48, 32), Image.Resampling.LANCZOS)
menu.save(OUT / 'menu-bar.png')
with tempfile.TemporaryDirectory() as temporary:
    iconset = Path(temporary) / 'Pinch.iconset'
    iconset.mkdir()
    for size in (16,32,128,256,512):
        for multiplier, suffix in ((1,''),(2,'@2x')):
            render(size*multiplier, app=True).save(iconset / f'icon_{size}x{size}{suffix}.png')
    subprocess.run(['iconutil','-c','icns',str(iconset),'-o',str(OUT / 'Pinch.icns')],check=True)
