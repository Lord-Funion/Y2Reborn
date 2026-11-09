"""Pictures of the Console example skin, drawn pixel by pixel (no libraries).

Run: python3 docs/skin-example/make_console_images.py docs/skin-example/Console/images
"""
import struct, sys, zlib
from pathlib import Path

OUT = Path(sys.argv[1])
DARK, MID, LIGHT, LIME, SHADE = (0x14, 0x1F, 0x12), (0x33, 0x47, 0x2C), (0xD8, 0xF5, 0xC8), (0xB6, 0xF2, 0x3A), (0x24, 0x33, 0x1F)

def png(name, w, h, pixel):
    rows = b"".join(b"\x00" + b"".join(bytes(pixel(x, y)) for x in range(w)) for y in range(h))
    chunk = lambda k, d: struct.pack(">I", len(d)) + k + d + struct.pack(">I", zlib.crc32(k + d) & 0xFFFFFFFF)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
    (OUT / name).write_bytes(data + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))

def rgba(c, a=255):
    return (*c, a)

def frame(x, y, w=24, h=24):
    """A window: 2 px dark outline, 2 px light bevel, then the dark-green inside."""
    edge = min(x, y, w - 1 - x, h - 1 - y)
    if edge < 2:
        return rgba(DARK)
    if edge < 4:
        return rgba(LIGHT if (x < 4 or y < 4) else MID)
    return rgba(SHADE)

def cursor(x, y, w=16, h=16):
    """The selected row: lime with a 2 px dark outline and square corners."""
    edge = min(x, y, w - 1 - x, h - 1 - y)
    return rgba(DARK) if edge < 2 else rgba(LIME)

def track(x, y):
    """A bar's empty part: a 2 x 2 checker inside a 2 px outline (slice 3 keeps the outline)."""
    edge = min(x, y, 11 - x, 11 - y)
    if edge < 2:
        return rgba(DARK)
    return rgba(MID if (x // 2 + y // 2) % 2 else SHADE)

def fill(x, y):
    """A bar's filled part: lime with a light top line."""
    edge = min(x, y, 11 - x, 11 - y)
    if edge < 2:
        return rgba(DARK)
    return rgba(LIGHT if y == 2 else LIME)

def screen(x, y):
    """The LCD background: every third row and column a shade darker."""
    return rgba(SHADE if x % 3 == 2 or y % 3 == 2 else (0x1E, 0x2B, 0x1A))

def logo(x, y):
    """A white cartridge on transparent ground; the skin tints it."""
    inside = 4 <= x < 28 and 2 <= y < 30 and not (x >= 22 and y < 8)
    label = 8 <= x < 24 and 10 <= y < 22
    return rgba((255, 255, 255), 255 if inside and not label else 0)

png("frame.png", 24, 24, frame)
png("cursor.png", 16, 16, cursor)
png("track.png", 12, 12, track)
png("fill.png", 12, 12, fill)
png("screen.png", 480, 360, screen)
png("logo.png", 32, 32, logo)
