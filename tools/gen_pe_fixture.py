#!/usr/bin/env python3
import struct
import sys
from pathlib import Path


def put_u16(image: bytearray, offset: int, value: int) -> None:
    struct.pack_into("<H", image, offset, value)


def put_u32(image: bytearray, offset: int, value: int) -> None:
    struct.pack_into("<I", image, offset, value)


def build_fixture() -> bytes:
    image = bytearray(0x600)
    put_u16(image, 0, 0x5A4D)
    put_u32(image, 0x3C, 0x80)
    put_u32(image, 0x80, 0x00004550)

    coff = 0x84
    put_u16(image, coff, 0x014C)
    put_u16(image, coff + 2, 2)
    put_u16(image, coff + 18, 0x0102)
    put_u16(image, coff + 16, 224)

    optional = coff + 20
    put_u16(image, optional, 0x010B)
    put_u32(image, optional + 16, 0x1000)
    put_u32(image, optional + 28, 0x00400000)
    put_u32(image, optional + 32, 0x1000)
    put_u32(image, optional + 36, 0x200)
    put_u32(image, optional + 56, 0x3000)
    put_u32(image, optional + 60, 0x200)
    put_u16(image, optional + 68, 3)

    sections = optional + 224
    image[sections:sections + 8] = b".text\0\0\0"
    put_u32(image, sections + 8, 0x10)
    put_u32(image, sections + 12, 0x1000)
    put_u32(image, sections + 16, 0x200)
    put_u32(image, sections + 20, 0x200)
    put_u32(image, sections + 36, 0x60000020)
    text = b"HORIZON PE TEST"
    image[0x200:0x200 + len(text)] = text

    data = sections + 40
    image[data:data + 8] = b".data\0\0\0"
    put_u32(image, data + 8, 0x20)
    put_u32(image, data + 12, 0x2000)
    put_u32(image, data + 16, 0x200)
    put_u32(image, data + 20, 0x400)
    put_u32(image, data + 36, 0xC0000040)
    image[0x400:0x40c] = b"parser-test\0"
    return bytes(image)


if __name__ == "__main__":
    output = Path(sys.argv[1])
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(build_fixture())
