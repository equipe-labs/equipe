# Trim the bottom of a tall screenshot: keep everything down to the last row that differs from the page background, plus padding.
import sys
from PIL import Image
src, dst = sys.argv[1], sys.argv[2]
im = Image.open(src).convert('RGB'); w, h = im.size
bg = im.getpixel((2, 2))
px = im.load()
last = 0
for y in range(h - 1, -1, -1):
    if any(px[x, y] != bg for x in range(0, w, 3)):
        last = y; break
im.crop((0, 0, w, min(h, last + 56 * (w // 1400 or 1)))).save(dst)
