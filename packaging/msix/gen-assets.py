#!/usr/bin/env python3
"""Generate the MSIX logo assets (solid indigo tile with a teal center square).

Pure standard library (no Pillow). Run from this directory:
    python gen-assets.py
"""
import struct
import zlib
from pathlib import Path

BG = (33, 40, 61, 255)       # dark indigo
ACCENT = (90, 160, 200, 255)  # teal


def _png(width: int, height: int, pixels: bytes) -> bytes:
    def chunk(typ: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + typ
            + data
            + struct.pack(">I", zlib.crc32(typ + data) & 0xFFFFFFFF)
        )

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)  # 8-bit RGBA
    stride = width * 4
    raw = bytearray()
    for y in range(height):
        raw.append(0)  # filter: none
        raw += pixels[y * stride:(y + 1) * stride]
    idat = zlib.compress(bytes(raw), 9)
    return sig + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")


def make(size: int, path: Path) -> None:
    margin = max(1, size // 5)
    px = bytearray()
    for y in range(size):
        for x in range(size):
            inside = margin <= x < size - margin and margin <= y < size - margin
            px += bytes(ACCENT if inside else BG)
    path.write_bytes(_png(size, size, bytes(px)))


if __name__ == "__main__":
    out = Path(__file__).parent / "Assets"
    out.mkdir(exist_ok=True)
    make(44, out / "Square44x44Logo.png")
    make(150, out / "Square150x150Logo.png")
    make(50, out / "StoreLogo.png")
    print(f"wrote assets to {out}")
