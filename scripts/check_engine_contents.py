#!/usr/bin/env python3
"""Every file the engine archive authors landed in both packages, at its mode.

Amendment section 3.2: "the engine's installed layout is byte-identical in both
packages". The engine artifact's own `nfpm-contents.yaml` is the single author
of that layout, `scripts/splice_engine_contents.py` puts it into the rendered
configuration, and this is the proof that it survived the whole way to the built
packages.

Splicing already refuses an entry whose source is missing or whose destination
is not the engine's, so this is not the same check twice. What it adds is the
other end of the pipe: the deb and the rpm are read back, and every destination
the archive named has to be there, in both, at the mode the archive gave it. The
failures that only show up here are the ones the packager introduces rather than
the ones the archive carries: an entry silently dropped, a mode reshaped by the
umask, a file that landed in one family and not the other.

The loader payload is matched by shape rather than by name: it is
`libc-musl-<digest>.so` and the digest changes with every engine build.

Usage:
  check_engine_contents.py --engine-contents <yaml> --rendered <config>
                           --deb <path> --rpm <path>
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

PAYLOAD_DIRECTORY = "/usr/lib/fermix/runtime-payload"
PAYLOAD_PATTERN = re.compile(r"^libc-musl-[0-9a-f]{64}\.so$")


class Refusal(RuntimeError):
    """A difference the build will not let through, phrased for a person."""


def read(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        raise Refusal(f"cannot read {path}: {error}") from error


def wanted(text: str) -> dict[str, str]:
    """The archive's own answer: {destination: mode}."""
    found: dict[str, str] = {}
    destination: str | None = None
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("- ") or line == "-":
            destination = None
            line = line[2:].strip()
        if line.startswith("dst:"):
            destination = line[len("dst:") :].strip().strip('"').rstrip("/")
            if destination in found:
                raise Refusal(f"the engine archive installs two things at {destination}")
            found[destination] = ""
        elif line.startswith("mode:") and destination is not None:
            found[destination] = line[len("mode:") :].strip().strip('"')
    if not found:
        raise Refusal("the engine archive's contents block installs nothing at all")
    return found


def normalise_mode(mode: str) -> str:
    """`0755`, `755` and `0o755` are one mode. An unset mode stays unset."""
    if not mode:
        return ""
    digits = mode.lower().removeprefix("0o").lstrip("0") or "0"
    return digits.rjust(3, "0")


def mode_from_permissions(permissions: str) -> str:
    """`-rwxr-xr-x` as `755`. dpkg-deb prints the mode no other way."""
    bits = permissions[1:10]
    if len(bits) != 9:
        raise Refusal(f"cannot read the permissions {permissions!r} out of the deb")
    value = 0
    for index, character in enumerate(bits):
        if character != "-":
            value |= 1 << (8 - index)
    return normalise_mode(f"{value:o}")


def deb_contents(path: Path) -> dict[str, str]:
    """{destination: mode} out of the built deb, directories left out."""
    result = subprocess.run(
        ["dpkg-deb", "--contents", str(path)], capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise Refusal(f"dpkg-deb could not read {path.name}: {result.stderr.strip()}")

    found: dict[str, str] = {}
    for line in result.stdout.splitlines():
        fields = line.split(None, 5)
        if len(fields) < 6:
            continue
        permissions, name = fields[0], fields[5]
        if permissions.startswith("d"):
            continue
        name = name.split(" -> ", 1)[0]
        if not name.startswith("./"):
            continue
        found[name[1:].rstrip("/")] = mode_from_permissions(permissions)
    return found


def rpm_contents(path: Path) -> dict[str, str]:
    """The same, out of the built rpm, which states a mode directly."""
    result = subprocess.run(
        ["rpm", "-qp", "--queryformat", "[%{FILEMODES:octal} %{FILENAMES}\n]", str(path)],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise Refusal(f"rpm could not read {path.name}: {result.stderr.strip()}")

    found: dict[str, str] = {}
    for line in result.stdout.splitlines():
        mode, _, name = line.strip().partition(" ")
        if not name or not mode.isdigit():
            continue
        # rpm's octal file mode carries the file type in its top bits, and only
        # the permission bits are being compared.
        found[name.rstrip("/")] = normalise_mode(f"{int(mode, 8) & 0o7777:04o}")
    return found


def resolve_payload(destination: str, present: dict[str, str]) -> str | None:
    """The payload's real name in a package, since its digest is per build."""
    parent, _, name = destination.rpartition("/")
    if parent != PAYLOAD_DIRECTORY:
        return destination if destination in present else None
    if not PAYLOAD_PATTERN.match(name):
        raise Refusal(
            f"the engine archive carries {destination}, and a loader payload is named "
            "libc-musl-<64 hex>.so"
        )
    candidates = [
        path
        for path in present
        if path.startswith(PAYLOAD_DIRECTORY + "/")
        and PAYLOAD_PATTERN.match(path.rpartition("/")[2])
    ]
    return candidates[0] if len(candidates) == 1 else None


def compare(expected: dict[str, str], present: dict[str, str], family: str) -> list[str]:
    problems = []
    for destination, mode in sorted(expected.items()):
        landed = resolve_payload(destination, present)
        if landed is None:
            problems.append(
                f"the engine archive installs {destination} and the {family} does not carry it"
            )
            continue
        if mode and present[landed] != normalise_mode(mode):
            problems.append(
                f"{landed} is mode {present[landed]} in the {family} and the engine "
                f"archive gives it {normalise_mode(mode)}"
            )
    return problems


def check_rendered(rendered: str, expected: dict[str, str]) -> list[str]:
    """The configuration the packages were built from names every entry once."""
    destinations = [
        line.strip()[len("dst: ") :].strip('"').rstrip("/")
        for line in rendered.splitlines()
        if line.strip().startswith("dst: ")
    ]
    problems = []
    for destination in sorted(set(destinations)):
        if destinations.count(destination) > 1:
            problems.append(
                f"the rendered configuration installs {destination} more than once"
            )
    for destination in sorted(expected):
        if destination not in destinations:
            problems.append(
                f"the engine archive installs {destination} and the rendered "
                "configuration does not; the splice dropped it"
            )
    return problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine-contents", required=True, type=Path)
    parser.add_argument("--rendered", required=True, type=Path)
    parser.add_argument("--deb", required=True, type=Path)
    parser.add_argument("--rpm", required=True, type=Path)
    arguments = parser.parse_args(argv)

    try:
        expected = wanted(read(arguments.engine_contents))
        problems = check_rendered(read(arguments.rendered), expected)
        problems += compare(expected, deb_contents(arguments.deb), "deb")
        problems += compare(expected, rpm_contents(arguments.rpm), "rpm")
    except Refusal as refusal:
        print(f"check_engine_contents: {refusal}", file=sys.stderr)
        return 1

    for problem in problems:
        print(f"check_engine_contents: {problem}", file=sys.stderr)
    if problems:
        return 1

    print(
        f"  {len(expected)} engine files, every one in both packages at the mode the "
        "engine archive gives it"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
