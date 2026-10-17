#!/usr/bin/env python3
"""Generate a small deterministic 24-bit gradient wallpaper BMP fixture."""

import struct
import sys


def make_wallpaper(width: int, height: int) -> bytes:
    """Create an aligned, bottom-up BI_RGB bitmap with a teal-blue gradient."""
    # NOTE: Contribution Area: keep the generated art deterministic and
    # dependency-free so clean builds need no external image assets.
    if width <= 0 or height <= 0:
        raise ValueError("wallpaper dimensions must be positive")

    bytes_per_pixel = 3
    row_stride = (width * bytes_per_pixel + 3) & ~3
    pixel_data = bytearray(row_stride * height)
    for y in range(height):
        for x in range(width):
            diagonal = (x * 255 // width + y * 255 // height) // 2
            glow = max(0, 96 - abs((x - width * 3 // 4) - (y - height // 3) * 2))
            red = 14 + diagonal // 8
            green = 36 + diagonal // 3 + glow // 2
            blue = 64 + diagonal // 2 + glow
            offset = (height - 1 - y) * row_stride + x * bytes_per_pixel
            pixel_data[offset : offset + 3] = bytes(
                (min(blue, 255), min(green, 255), min(red, 255))
            )

    pixel_offset = 14 + 40
    file_size = pixel_offset + len(pixel_data)
    file_header = struct.pack("<2sIHHI", b"BM", file_size, 0, 0, pixel_offset)
    info_header = struct.pack(
        "<IiiHHIIiiII",
        40,
        width,
        height,
        1,
        24,
        0,
        len(pixel_data),
        2835,
        2835,
        0,
        0,
    )
    return file_header + info_header + pixel_data


def main(output_path: str) -> None:
    """Write the generated wallpaper bytes to the requested build path."""
    # NOTE: Contribution Area: choose an original, generated palette here or
    # wire a properly licensed source image through a separate asset pipeline.
    with open(output_path, "wb") as output:
        output.write(make_wallpaper(320, 240))


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} OUTPUT")
    main(sys.argv[1])
