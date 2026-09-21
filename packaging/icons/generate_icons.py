#!/usr/bin/env python3
"""Regenerate the hicolor PNG sizes from the vendored 1024 px master.

Why this exists at all: the icon the packages shipped was a placeholder -- a
brand-blue rounded square with a wordmark "F" -- and the real Fermix icon is the
mascot, authored for macOS and vendored beside this script. The sizes are
produced from that master rather than drawn again, so the two platforms cannot
drift into two different icons.

Why it depends on nothing: the build container carries python3 and no image
toolkit at all -- no ImageMagick, no Pillow, no rsvg-convert outside the private
runtime, no potrace. Rather than add a build dependency to draw eight PNGs, the
PNG codec and the resampler are here, in the standard library, which also means
this runs identically on a developer's host and inside the container.

Resampling is Lanczos-3 in PREMULTIPLIED alpha. The master is a dark glyph on a
fully transparent ground; resampling straight RGBA averages the colour of
transparent pixels into the edge and leaves a dark halo that is plainly visible
at 16 px. Premultiplying first is the whole difference between a clean edge and
a dirty one, so it is not an optimisation to be removed.
"""

import struct
import sys
import zlib
from pathlib import Path

PNG_MAGIC = b"\x89PNG\r\n\x1a\n"
BYTES_PER_PIXEL = 4
SIZES = (16, 22, 24, 32, 48, 64, 128, 256, 512)


def read_png(path):
    """Decode an 8-bit RGBA PNG into (width, height, bytearray of RGBA)."""
    data = path.read_bytes()
    if data[:8] != PNG_MAGIC:
        raise ValueError(f"{path} is not a PNG")

    header, idat, offset = None, bytearray(), 8
    while offset < len(data):
        (length,) = struct.unpack(">I", data[offset : offset + 4])
        kind = data[offset + 4 : offset + 8]
        body = data[offset + 8 : offset + 8 + length]
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            break
        offset += 12 + length

    if header is None:
        raise ValueError(f"{path} carries no IHDR")
    width, height, depth, colour, compression, filt, interlace = header
    if (depth, colour, interlace) != (8, 6, 0):
        raise ValueError(
            f"{path} is depth {depth} colour-type {colour} interlace {interlace}; "
            "this decoder reads only 8-bit RGBA, non-interlaced, and refuses "
            "rather than producing a plausible wrong image"
        )
    if (compression, filt) != (0, 0):
        raise ValueError(f"{path} uses an unknown compression or filter method")

    return width, height, unfilter(zlib.decompress(bytes(idat)), width, height)


def unfilter(raw, width, height):
    """Reverse the per-scanline PNG filters, returning packed RGBA."""
    stride = width * BYTES_PER_PIXEL
    out = bytearray(stride * height)
    previous = bytearray(stride)

    for row in range(height):
        start = row * (stride + 1)
        method = raw[start]
        line = bytearray(raw[start + 1 : start + 1 + stride])
        if method > 4:
            raise ValueError(f"row {row} uses filter {method}, which does not exist")

        for index in range(stride):
            left = line[index - BYTES_PER_PIXEL] if index >= BYTES_PER_PIXEL else 0
            up = previous[index]
            upleft = previous[index - BYTES_PER_PIXEL] if index >= BYTES_PER_PIXEL else 0
            line[index] = (line[index] + predictor(method, left, up, upleft)) & 0xFF

        out[row * stride : (row + 1) * stride] = line
        previous = line

    return out


def predictor(method, left, up, upleft):
    """The five PNG filter predictors, by number."""
    if method == 0:
        return 0
    if method == 1:
        return left
    if method == 2:
        return up
    if method == 3:
        return (left + up) // 2

    # Paeth: pick whichever neighbour the linear estimate is closest to.
    estimate = left + up - upleft
    distances = (abs(estimate - left), abs(estimate - up), abs(estimate - upleft))
    return (left, up, upleft)[distances.index(min(distances))]


def write_png(path, width, height, pixels):
    """Encode packed 8-bit RGBA as a PNG."""
    stride = width * BYTES_PER_PIXEL
    raw = bytearray()
    for row in range(height):
        raw.append(0)  # filter "none": these are small and clarity beats bytes
        raw += pixels[row * stride : (row + 1) * stride]

    def chunk(kind, body):
        return (
            struct.pack(">I", len(body))
            + kind
            + body
            + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
        )

    path.write_bytes(
        PNG_MAGIC
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def lanczos(x, lobes=3):
    """The Lanczos-3 kernel, which is what makes the small sizes legible."""
    import math

    if x == 0.0:
        return 1.0
    if abs(x) >= lobes:
        return 0.0
    px = math.pi * x
    return (lobes * math.sin(px) * math.sin(px / lobes)) / (px * px)


def contributions(source_length, target_length, lobes=3):
    """For each output position, the input indices and weights that form it."""
    if target_length > source_length:
        raise ValueError(
            "this resampler only reduces; enlarging a 1024 master would invent "
            "detail that is not in the artwork"
        )

    scale = source_length / target_length
    support = lobes * scale
    rows = []

    for target in range(target_length):
        centre = (target + 0.5) * scale - 0.5
        first = max(0, int(centre - support + 0.5))
        last = min(source_length - 1, int(centre + support + 0.5))
        weights = [(i, lanczos((i - centre) / scale, lobes)) for i in range(first, last + 1)]
        total = sum(weight for _, weight in weights)
        if total == 0:
            raise ValueError(f"output {target} of {target_length} gathered no weight")
        rows.append([(i, weight / total) for i, weight in weights])

    return rows


def resize(width, height, pixels, target):
    """Lanczos-reduce packed RGBA to target x target, in premultiplied alpha."""
    premultiplied = premultiply(width, height, pixels)
    horizontal = contributions(width, target)
    vertical = contributions(height, target)

    # Horizontal pass into a float buffer of (target x height) pixels.
    middle = [0.0] * (target * height * BYTES_PER_PIXEL)
    for row in range(height):
        base = row * width * BYTES_PER_PIXEL
        for column, taps in enumerate(horizontal):
            out = (row * target + column) * BYTES_PER_PIXEL
            for channel in range(BYTES_PER_PIXEL):
                middle[out + channel] = sum(
                    premultiplied[base + i * BYTES_PER_PIXEL + channel] * weight
                    for i, weight in taps
                )

    # Vertical pass into the final 8-bit buffer.
    out_pixels = bytearray(target * target * BYTES_PER_PIXEL)
    for row, taps in enumerate(vertical):
        for column in range(target):
            out = (row * target + column) * BYTES_PER_PIXEL
            for channel in range(BYTES_PER_PIXEL):
                value = sum(
                    middle[(i * target + column) * BYTES_PER_PIXEL + channel] * weight
                    for i, weight in taps
                )
                out_pixels[out + channel] = clamp(value)

    return unpremultiply(target, out_pixels)


def premultiply(width, height, pixels):
    """Scale colour by alpha so transparent pixels carry no colour weight."""
    out = [0.0] * (width * height * BYTES_PER_PIXEL)
    for index in range(0, len(pixels), BYTES_PER_PIXEL):
        alpha = pixels[index + 3] / 255.0
        out[index] = pixels[index] * alpha
        out[index + 1] = pixels[index + 1] * alpha
        out[index + 2] = pixels[index + 2] * alpha
        out[index + 3] = float(pixels[index + 3])
    return out


def unpremultiply(size, pixels):
    """Undo the premultiply, leaving straight RGBA for the encoder."""
    for index in range(0, len(pixels), BYTES_PER_PIXEL):
        alpha = pixels[index + 3]
        if alpha == 0:
            pixels[index] = pixels[index + 1] = pixels[index + 2] = 0
            continue
        scale = 255.0 / alpha
        pixels[index] = clamp(pixels[index] * scale)
        pixels[index + 1] = clamp(pixels[index + 1] * scale)
        pixels[index + 2] = clamp(pixels[index + 2] * scale)
    return pixels


def clamp(value):
    return 0 if value < 0 else 255 if value > 255 else int(value + 0.5)


def main(argv):
    if len(argv) != 3:
        print("usage: generate_icons.py <master.png> <hicolor-dir>", file=sys.stderr)
        return 2

    master, hicolor = Path(argv[1]), Path(argv[2])
    if not master.is_file():
        print(f"no master at {master}", file=sys.stderr)
        return 1

    width, height, pixels = read_png(master)
    if width != height:
        print(f"{master} is {width}x{height}; the icon master must be square", file=sys.stderr)
        return 1
    print(f"master {master.name}: {width}x{height}")

    for size in SIZES:
        target = hicolor / f"{size}x{size}" / "apps" / "io.tezra.Fermix.png"
        target.parent.mkdir(parents=True, exist_ok=True)
        write_png(target, size, size, resize(width, height, pixels, size))
        print(f"  {size:>4}px -> {target.relative_to(hicolor)}  {target.stat().st_size} bytes")

    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
