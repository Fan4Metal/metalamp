r"""
Write images/icon.png (for the READMEs) from the icon the program draws:
runs `metalamp.exe --export-icon` and turns the 256-pixel layer of the
.ico into a PNG. No external dependencies.

Run from any folder after a build: python tools/icon_png.py [path\to\metalamp.exe]
"""

import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "images" / "icon.png"


def largest_layer(ico: bytes) -> tuple[int, bytes]:
    """The side and the top-down RGBA pixels of the largest 32-bit BMP layer."""
    count = struct.unpack_from("<H", ico, 4)[0]
    entries = []
    for i in range(count):
        w, _h, _c, _r, _p, _bpp, size, offset = struct.unpack_from("<BBBBHHII", ico, 6 + 16 * i)
        entries.append((w or 256, size, offset))
    side, _size, offset = max(entries)
    header = 40
    bgra = ico[offset + header : offset + header + side * side * 4]
    rows = [bgra[y * side * 4 : (y + 1) * side * 4] for y in range(side)][::-1]
    rgba = bytearray()
    for row in rows:
        for x in range(side):
            b, g, r, a = row[4 * x : 4 * x + 4]
            rgba += bytes((r, g, b, a))
    return side, bytes(rgba)


def png(side: int, rgba: bytes) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    raw = b"".join(b"\0" + rgba[y * side * 4 : (y + 1) * side * 4] for y in range(side))
    header = struct.pack(">IIBBBBB", side, side, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def main() -> int:
    exe = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target" / "release" / "metalamp.exe"
    if not exe.is_file():
        exe = ROOT / "target" / "debug" / "metalamp.exe"
    with tempfile.TemporaryDirectory() as tmp:
        ico = Path(tmp) / "app.ico"
        subprocess.run([str(exe), "--export-icon", str(ico)], check=True)
        side, rgba = largest_layer(ico.read_bytes())
    OUT.parent.mkdir(exist_ok=True)
    OUT.write_bytes(png(side, rgba))
    print(f"{OUT} ({side}x{side})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
