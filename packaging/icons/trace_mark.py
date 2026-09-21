#!/usr/bin/env python3
"""Trace the vendored mark master into a single-colour SVG path.

The symbolic icon has to be real vector: it is recoloured by the icon theme to
whatever the current foreground is, so an embedded bitmap would ignore the
theme and show a black mark on a dark header. Nothing in the build container
traces bitmaps -- no potrace, no autotrace -- so the tracing is here.

The master is a template image: every pixel is black and the ALPHA carries the
shape. The mascot body is opaque, the snout is a transparent hole, and the two
nostrils are opaque islands inside that hole. Thresholding alpha therefore
yields the symbolic shape directly, and an even-odd fill renders the nesting
(body filled, snout hollow, nostrils filled) without any special casing.

The boundary is walked as unit edges between ink and non-ink pixels, which is
exact rather than approximate, and only then simplified and smoothed. Going the
other way -- smoothing first -- loses the corners that make the snout read.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_icons import read_png  # noqa: E402

INK_THRESHOLD = 128


def ink_mask(width, height, pixels):
    """True where the template is opaque enough to count as ink."""
    return [
        [pixels[(y * width + x) * 4 + 3] > INK_THRESHOLD for x in range(width)]
        for y in range(height)
    ]


def boundary_edges(mask, width, height):
    """Directed unit edges with ink on one consistent side."""

    def ink(x, y):
        return 0 <= x < width and 0 <= y < height and mask[y][x]

    edges = {}
    for y in range(height):
        for x in range(width):
            if not mask[y][x]:
                continue
            if not ink(x, y - 1):
                edges.setdefault((x, y), []).append((x + 1, y))
            if not ink(x + 1, y):
                edges.setdefault((x + 1, y), []).append((x + 1, y + 1))
            if not ink(x, y + 1):
                edges.setdefault((x + 1, y + 1), []).append((x, y + 1))
            if not ink(x - 1, y):
                edges.setdefault((x, y + 1), []).append((x, y))
    return edges


def closed_loops(edges):
    """Stitch the edge set into closed loops, consuming each edge once."""
    loops = []
    for start in list(edges):
        while edges.get(start):
            loop, point = [start], start
            while True:
                following = edges.get(point)
                if not following:
                    break
                nxt = following.pop()
                if not following:
                    edges.pop(point, None)
                if nxt == start:
                    break
                loop.append(nxt)
                point = nxt
            if len(loop) > 8:
                loops.append(loop)
    return loops


def simplify(points, epsilon):
    """Douglas-Peucker on a closed loop, keeping the shape's corners."""
    if len(points) < 3:
        return points

    def distance(p, a, b):
        (px, py), (ax, ay), (bx, by) = p, a, b
        dx, dy = bx - ax, by - ay
        if dx == 0 and dy == 0:
            return ((px - ax) ** 2 + (py - ay) ** 2) ** 0.5
        t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)))
        return ((px - ax - t * dx) ** 2 + (py - ay - t * dy) ** 2) ** 0.5

    def walk(chunk):
        if len(chunk) < 3:
            return chunk
        worst, index = 0.0, 0
        for i in range(1, len(chunk) - 1):
            d = distance(chunk[i], chunk[0], chunk[-1])
            if d > worst:
                worst, index = d, i
        if worst <= epsilon:
            return [chunk[0], chunk[-1]]
        return walk(chunk[: index + 1])[:-1] + walk(chunk[index:])

    return walk(points + [points[0]])[:-1]


def smooth_path(points, scale, offset):
    """Catmull-Rom through the simplified points, emitted as cubic beziers."""

    def place(p):
        return ((p[0] + offset[0]) * scale, (p[1] + offset[1]) * scale)

    pts = [place(p) for p in points]
    count = len(pts)
    if count < 3:
        return ""

    out = [f"M{pts[0][0]:.2f},{pts[0][1]:.2f}"]
    for i in range(count):
        p0, p1 = pts[(i - 1) % count], pts[i]
        p2, p3 = pts[(i + 1) % count], pts[(i + 2) % count]
        c1 = (p1[0] + (p2[0] - p0[0]) / 6.0, p1[1] + (p2[1] - p0[1]) / 6.0)
        c2 = (p2[0] - (p3[0] - p1[0]) / 6.0, p2[1] - (p3[1] - p1[1]) / 6.0)
        out.append(
            f"C{c1[0]:.2f},{c1[1]:.2f} {c2[0]:.2f},{c2[1]:.2f} {p2[0]:.2f},{p2[1]:.2f}"
        )
    return " ".join(out) + " Z"


def trace(master, canvas, epsilon, margin):
    """Return one SVG path data string for the whole mark."""
    width, height, pixels = read_png(master)
    mask = ink_mask(width, height, pixels)
    loops = closed_loops(boundary_edges(mask, width, height))
    if not loops:
        raise ValueError(f"{master} produced no contours; nothing was traced")

    usable = canvas - 2 * margin
    scale = usable / max(width, height)
    offset = (margin / scale - 0, margin / scale - 0)

    pieces = []
    for loop in sorted(loops, key=len, reverse=True):
        reduced = simplify(loop, epsilon)
        if len(reduced) >= 3:
            pieces.append(smooth_path(reduced, scale, offset))
    return pieces, len(loops)


def main(argv):
    if len(argv) != 3:
        print("usage: trace_mark.py <mark-master.png> <out.svg>", file=sys.stderr)
        return 2

    master, out = Path(argv[1]), Path(argv[2])
    if not master.is_file():
        print(f"no mark master at {master}", file=sys.stderr)
        return 1

    # No margin: the master already carries its own breathing room, and insetting
    # it again reframes the Linux mark relative to the macOS one. Measured
    # against the master's own alpha, margin 0 scores IoU 0.9936 while a 0.5 px
    # inset scores 0.8201 -- the whole of that gap was the inset, not the trace.
    pieces, loop_count = trace(master, canvas=16.0, epsilon=2.0, margin=0.0)
    print(f"traced {loop_count} contour(s) from {master.name}, kept {len(pieces)}")

    # ONE path holding every contour as a subpath. Even-odd cuts holes between
    # the SUBPATHS of a single path; four sibling <path> elements are four
    # separate fills that paint over one another, so the snout renders as a
    # blot rather than a hole. Measured: that mistake scored IoU 0.69 against
    # the master and looked like a plain blob.
    body = '<path d="' + " ".join(pieces) + '"/>'
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" '
        'viewBox="0 0 16 16" role="img" aria-label="Fermix">\n'
        "  <title>Fermix</title>\n"
        "  <!--\n"
        "    The symbolic pair of the application icon: the Fermix mascot mark,\n"
        "    traced from packaging/icons/masters/FermixMarkMaster.png by\n"
        "    packaging/icons/trace_mark.py. One fill, no ground and no brand\n"
        "    colour, so the icon theme recolours it for the current foreground.\n"
        "    Even-odd renders the nesting: body filled, snout hollow, nostrils\n"
        "    filled again inside it.\n"
        "  -->\n"
        '  <g fill="currentColor" fill-rule="evenodd">\n    '
        + body
        + "\n  </g>\n</svg>\n"
    )
    print(f"wrote {out} ({out.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
