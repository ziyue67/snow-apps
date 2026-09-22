#!/usr/bin/env python3
"""Split the packaged Windows icon into a Freedesktop hicolor theme ladder.

Every frame of resources/app-icon.ico is a standalone PNG stream, so the
conversion only has to walk the ICONDIR entries and copy each payload into
share/icons/hicolor/<size>x<size>/apps/<name>.png. Staying on the standard
library keeps the Linux package build free of an image toolchain dependency.

Usage: extract-app-icons.py <icon.ico> <hicolor-dir> <icon-name>
"""

from __future__ import annotations

import struct
import sys
from pathlib import Path

ICONDIR_ENTRY = struct.Struct("<BBBBHHII")
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


def read_frames(path: Path):
    data = path.read_bytes()
    if len(data) < 6:
        raise SystemExit(f"{path}: file is too short to be an icon")
    reserved, image_type, count = struct.unpack_from("<HHH", data, 0)
    if reserved != 0 or image_type != 1:
        raise SystemExit(f"{path}: not a Windows icon file")
    for index in range(count):
        offset = 6 + index * ICONDIR_ENTRY.size
        width, height, _colors, _reserved, _planes, _bpp, size, start = ICONDIR_ENTRY.unpack_from(
            data, offset
        )
        # A zero dimension encodes 256 pixels in the ICO container.
        width = width or 256
        height = height or 256
        payload = data[start : start + size]
        if payload[:8] != PNG_SIGNATURE:
            raise SystemExit(f"{path}: frame {index} is not PNG encoded")
        yield width, height, payload


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        raise SystemExit(__doc__)
    source = Path(argv[1])
    destination = Path(argv[2])
    name = argv[3]

    written = 0
    for width, height, payload in read_frames(source):
        target = destination / f"{width}x{height}" / "apps"
        target.mkdir(parents=True, exist_ok=True)
        (target / f"{name}.png").write_bytes(payload)
        written += 1

    if written == 0:
        raise SystemExit(f"{source}: no icon frames found")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
