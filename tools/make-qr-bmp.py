"""Regenerates assets/donate-qr.bmp (8-bit RLE-compressed BMP, embedded in the exe as a
BITMAP resource) from assets/donate-qr.png. Needs Pillow: python -m pip install pillow
Usage: python tools/make-qr-bmp.py"""
import struct
import sys
from pathlib import Path

from PIL import Image

root = Path(__file__).resolve().parent.parent
src = Image.open(root / "assets" / "donate-qr.png").convert("RGBA")
bg = Image.new("RGBA", src.size, (255, 255, 255, 255))
rgb = Image.alpha_composite(bg, src).convert("RGB")
w, h = rgb.size
colors = rgb.getcolors(1 << 24)
if len(colors) <= 256:
    pal = [c for _, c in colors]  # exact: no quantisation at all
    index = {c: i for i, c in enumerate(pal)}
    pix = [index[p] for p in rgb.getdata()]
else:
    q = rgb.quantize(256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE)
    flat = q.getpalette()[: 3 * 256]
    pal = [tuple(flat[i : i + 3]) for i in range(0, len(flat), 3)]
    pix = list(q.getdata())
    # the QR modules must stay pure black and white
    assert (0, 0, 0) in pal and (255, 255, 255) in pal, "palette lost black/white"
pal += [(0, 0, 0)] * (256 - len(pal))


def rle_row(row):
    out = bytearray()
    i = 0
    n = len(row)
    while i < n:
        j = i
        while j < n and row[j] == row[i] and j - i < 255:
            j += 1
        out += bytes((j - i, row[i]))  # encoded run (also used for runs of 1-2 pixels)
        i = j
    out += b"\x00\x00"  # end of line
    return out


data = bytearray()
for y in range(h - 1, -1, -1):  # bottom-up
    data += rle_row(pix[y * w : (y + 1) * w])
data[-2:] = b"\x00\x01"  # end of bitmap
off = 14 + 40 + 256 * 4
hdr = b"BM" + struct.pack("<IHHI", off + len(data), 0, 0, off)
dib = struct.pack("<IiiHHIIiiII", 40, w, h, 1, 8, 1, len(data), 2835, 2835, 256, 0)
palb = b"".join(bytes((b, g, r, 0)) for r, g, b in pal)
(root / "assets" / "donate-qr.bmp").write_bytes(hdr + dib + palb + data)
print(f"donate-qr.bmp: {w}x{h}, {len(pal)} palette entries, {off + len(data)} bytes", file=sys.stderr)
