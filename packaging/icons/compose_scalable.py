#!/usr/bin/env python3
"""Compose the scalable application icon as real vector, not an embedded bitmap.

There is no vector source for the Fermix icon: the artwork is a 1024 px PNG
authored for macOS. The obvious fallback is an SVG that embeds that PNG as a
data URI, but that is a bitmap wearing an SVG extension -- it does not scale,
and it doubles the icon's bytes in every package.

Since packaging/icons/trace_mark.py already recovers the mascot as a faithful
path (measured IoU 0.9936 against the master's own alpha), the scalable icon is
composed from that path instead: the rounded ground as a <rect>, the mascot as
the traced outline filled in the master's own off-white. Even-odd leaves the
snout as a hole, so the ground shows through it exactly as in the artwork --
which means the snout is the ground colour by construction and cannot drift
from it.

Every constant below was measured from the master rather than guessed, and the
composition is checked against the generated 256 px raster before it ships.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_icons import read_png  # noqa: E402
from trace_mark import trace  # noqa: E402

CANVAS = 1024
GROUND_COLOUR = "#101014"
MASCOT_COLOUR = "#f4f5f7"


def measure(path, predicate):
    """Bounding box of the pixels a predicate accepts, as (x0, y0, w, h)."""
    width, height, pixels = read_png(path)

    def at(x, y):
        index = (y * width + x) * 4
        return pixels[index : index + 4]

    columns = [x for x in range(width) if any(predicate(at(x, y)) for y in range(height))]
    rows = [y for y in range(height) if any(predicate(at(x, y)) for x in range(width))]
    if not columns or not rows:
        raise ValueError(f"{path} has no pixels matching the predicate; nothing to measure")
    return columns[0], rows[0], columns[-1] - columns[0] + 1, rows[-1] - rows[0] + 1


def opaque(rgba):
    return rgba[3] > 128


def light(rgba):
    return rgba[3] > 128 and (rgba[0] * 299 + rgba[1] * 587 + rgba[2] * 114) // 1000 > 128


def main(argv):
    if len(argv) != 4:
        print("usage: compose_scalable.py <app-master> <mark-master> <out.svg>", file=sys.stderr)
        return 2

    app_master, mark_master, out = Path(argv[1]), Path(argv[2]), Path(argv[3])
    for candidate in (app_master, mark_master):
        if not candidate.is_file():
            print(f"no master at {candidate}", file=sys.stderr)
            return 1

    ground = measure(app_master, opaque)
    mascot = measure(app_master, light)
    print(f"ground bbox {ground}, mascot bbox {mascot}")

    # The rounded square: at its top row the straight edge begins one radius in.
    width, _, pixels = read_png(app_master)
    top_row = ground[1] + 1
    spans = [x for x in range(width) if pixels[(top_row * width + x) * 4 + 3] > 128]
    radius = spans[0] - ground[0]
    print(f"corner radius {radius} px on a {ground[2]} px square")

    # Trace the mark in its own pixel space (canvas == master width, so scale 1),
    # then place it by mapping its ink box onto the mascot box measured above.
    mark_width, _, _ = read_png(mark_master)
    pieces, _ = trace(mark_master, canvas=float(mark_width), epsilon=2.0, margin=0.0)
    ink = measure(mark_master, opaque)
    scale = mascot[2] / ink[2]
    offset_x = mascot[0] - ink[0] * scale
    offset_y = mascot[1] - ink[1] * scale
    print(f"mark placed at scale {scale:.5f}, offset ({offset_x:.1f}, {offset_y:.1f})")

    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{CANVAS}" height="{CANVAS}" '
        f'viewBox="0 0 {CANVAS} {CANVAS}" role="img" aria-label="Fermix">\n'
        "  <title>Fermix</title>\n"
        "  <!--\n"
        "    The application icon: the Fermix mascot on its rounded ground.\n"
        "    Composed by packaging/icons/compose_scalable.py from the vendored\n"
        "    masters in packaging/icons/masters: the ground as a rect, the\n"
        "    mascot as a path traced from FermixMarkMaster.png. There is no\n"
        "    vector source for this artwork; this is a trace of the 1024 px\n"
        "    master, not an embedded copy of it, so it scales properly.\n"
        "    Even-odd leaves the snout hollow and the ground shows through.\n"
        "  -->\n"
        f'  <rect x="{ground[0]}" y="{ground[1]}" width="{ground[2]}" height="{ground[3]}" '
        f'rx="{radius}" ry="{radius}" fill="{GROUND_COLOUR}"/>\n'
        f'  <g transform="translate({offset_x:.2f},{offset_y:.2f}) scale({scale:.5f})" '
        f'fill="{MASCOT_COLOUR}" fill-rule="evenodd">\n'
        '    <path d="' + " ".join(pieces) + '"/>\n'
        "  </g>\n"
        "</svg>\n"
    )
    print(f"wrote {out} ({out.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
