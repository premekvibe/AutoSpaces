"""Regenerates assets/donate-qr.bmp (account QR, from donate-qr.png) and assets/donate-revolut.bmp
(Revolut QR, from donate-revolut-source.jpg, reduced to 448 px / 4 colours): 8-bit RLE-compressed
BMPs embedded in the exe as BITMAP resources. Needs Pillow: python -m pip install pillow
Usage: python tools/make-qr-bmp.py"""
import struct
import sys
from pathlib import Path

from PIL import Image

root = Path(__file__).resolve().parent.parent

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


def convert(src_name, dst_name, size=None, colors=256):
    src = Image.open(root / "assets" / src_name).convert("RGBA")
    bg = Image.new("RGBA", src.size, (255, 255, 255, 255))
    rgb = Image.alpha_composite(bg, src).convert("RGB")
    if size:
        rgb = rgb.resize((size, size), Image.Resampling.LANCZOS)
    w, h = rgb.size
    found = rgb.getcolors(1 << 24)
    if found and len(found) <= colors:
        pal = [c for _, c in found]  # exact: no quantisation at all
        index = {c: i for i, c in enumerate(pal)}
        pix = [index[p] for p in rgb.get_flattened_data()]
    else:
        q = rgb.quantize(colors, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE)
        flat = q.getpalette()[: 3 * colors]
        pal = [tuple(flat[i : i + 3]) for i in range(0, len(flat), 3)]
        pix = list(q.get_flattened_data())
    pal += [(0, 0, 0)] * (256 - len(pal))
    data = bytearray()
    for y in range(h - 1, -1, -1):  # bottom-up
        data += rle_row(pix[y * w : (y + 1) * w])
    data[-2:] = b"\x00\x01"  # end of bitmap
    off = 14 + 40 + 256 * 4
    hdr = b"BM" + struct.pack("<IHHI", off + len(data), 0, 0, off)
    dib = struct.pack("<IiiHHIIiiII", 40, w, h, 1, 8, 1, len(data), 2835, 2835, 256, 0)
    palb = b"".join(bytes((b, g, r, 0)) for r, g, b in pal)
    (root / "assets" / dst_name).write_bytes(hdr + dib + palb + data)
    print(f"{dst_name}: {w}x{h}, {off + len(data)} bytes", file=sys.stderr)


convert("donate-qr.png", "donate-qr.bmp")
convert("donate-revolut-source.jpg", "donate-revolut.bmp", size=448, colors=4)
