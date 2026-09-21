#!/usr/bin/env python3
"""Splice the engine archive's own contents block into the rendered nFPM config.

Amendment section 5.1. The engine artifact carries `nfpm-contents.yaml`, which
is the `contents:` block of the headless `fermix` package's own nFPM
configuration with its staging prefix already resolved. That block is the single
author of where the engine's files go and what mode each one carries, and this
is what puts it into the desktop package's configuration without anyone
restating it.

Restating those paths in `packaging/nfpm-fermix-desktop.yaml.tmpl` was the
alternative and is refused. An engine release that adds a packaged file, one
more shell completion say, would then turn the automated version bump red until
a person hand-edited a template here, and one package exists precisely to remove
that stitching. With the splice, the engine's file list has exactly one author
and a new engine file needs no edit in this repository at all.

Three things are checked rather than trusted, because the archive is an input
from another repository and this is where it becomes part of what ships:

  * **Every `src:` resolves inside the verified staging directory.** The
    archive's paths are relative (`tree/usr/bin/fermix`), and they are rewritten
    against the directory `scripts/verify_engine.sh` unpacked into. A `src:`
    that escapes that directory, or names a file that is not there, is a
    refusal.
  * **Every `dst:` is under a root the engine owns.** `/usr/bin/fermix`,
    `/usr/lib/fermix/`, the vendor unit, `/usr/share/fermix/`,
    `/usr/share/doc/fermix/`, the three completions and the man page. An archive
    that tried to install `/usr/lib/fermix-desktop/lib/libgtk-4.so.1` would
    otherwise overwrite the window's own toolkit, and the near-namesakes are one
    word apart, so the test is written with its separator.
  * **No `dst:` collides with one the template already claims.** Two authors and
    one package: nFPM would carry the file twice or take whichever entry it read
    last, and neither is a thing to find out about afterwards.

Usage:
  splice_engine_contents.py --rendered <in> --out <out>
                            --engine-contents <yaml> --staging <dir>
                            [--marker <text>]
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

# The comment line in the template that this replaces. Agreed with slice 5,
# which asserts it appears exactly once, and a comment rather than a placeholder
# because a placeholder would have to be filled with a value nothing else in the
# render step has.
MARKER = "# >>> engine contents, spliced from the verified archive"

# Every root the engine owns, as amendment 3.2 lays the installed layout out. A
# trailing slash means "and everything under it"; anything else is exact. The
# separator is not decoration: every one of these has a desktop-owned namesake
# one word longer, `/usr/lib/fermix` against `/usr/lib/fermix-desktop` and
# `/usr/share/doc/fermix` against `/usr/share/doc/fermix-desktop`, and a prefix
# test written without it would hand the window's own files to the engine.
ENGINE_OWNED = (
    "/usr/bin/fermix",
    "/usr/lib/fermix/",
    "/usr/lib/systemd/user/fermix.service",
    "/usr/share/fermix/",
    "/usr/share/doc/fermix/",
    "/usr/share/bash-completion/completions/fermix",
    "/usr/share/zsh/site-functions/_fermix",
    "/usr/share/fish/vendor_completions.d/fermix.fish",
    "/usr/share/man/man1/fermix.1.gz",
)


class Refusal(RuntimeError):
    """Something the build will not splice in, phrased for a person."""


def engine_owned(destination: str) -> bool:
    for owned in ENGINE_OWNED:
        if owned.endswith("/"):
            if destination.startswith(owned):
                return True
        elif destination == owned:
            return True
    return False


def read(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        raise Refusal(f"cannot read {path}: {error}") from error


def parse_entries(text: str) -> list[dict]:
    """The archive's contents block, as a list of {src, dst, mode, type}.

    A reader for the shape the engine's packager emits rather than a YAML
    parser, because the build container is not guaranteed to carry PyYAML and
    because anything this reader does not recognise has to be a refusal rather
    than a skipped line: a silently dropped entry is a file missing from the
    package that nothing downstream would notice.
    """
    entries: list[dict] = []
    seen_contents = False
    current: dict | None = None

    for number, raw in enumerate(text.splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line == "contents:":
            seen_contents = True
            continue
        if not seen_contents:
            raise Refusal(
                f"line {number} of the engine's contents block comes before "
                "`contents:`, and the block is the whole file"
            )
        if line.startswith("- ") or line == "-":
            current = {}
            entries.append(current)
            line = line[2:].strip()
            if not line:
                continue
        if current is None:
            raise Refusal(f"line {number} of the engine's contents block is outside any entry")
        key, separator, value = line.partition(":")
        if not separator:
            raise Refusal(f"cannot read line {number} of the engine's contents block: {raw!r}")
        value = value.strip().strip('"')
        if key in ("src", "dst", "type", "packager"):
            current[key] = value
        elif key == "mode":
            current["mode"] = value
        elif key in ("file_info", "owner", "group", "mtime"):
            continue
        else:
            raise Refusal(
                f"the engine's contents block carries the key {key!r} at line {number}, "
                "which this splice does not know how to carry across"
            )

    if not entries:
        raise Refusal("the engine's contents block installs nothing at all")
    return entries


def resolve(entry: dict, staging: Path, number: int) -> Path:
    """One entry's `src:`, as a real file inside the verified staging tree."""
    source = entry.get("src")
    if not source:
        raise Refusal(f"engine contents entry {number} carries no src")
    if source.startswith("/"):
        raise Refusal(
            f"engine contents entry {number} names the absolute source {source!r}; "
            "the archive's paths are relative to its own root"
        )
    resolved = (staging / source).resolve()
    root = staging.resolve()
    if resolved != root and root not in resolved.parents:
        raise Refusal(
            f"engine contents entry {number} names {source!r}, which resolves outside "
            f"the verified staging directory"
        )
    if not resolved.is_file():
        raise Refusal(
            f"the engine archive's contents name {source!r} and there is no file there; "
            "the archive and its own contents block disagree"
        )
    return resolved


def check_destination(entry: dict, claimed: set[str], number: int) -> str:
    destination = entry.get("dst")
    if not destination:
        raise Refusal(f"engine contents entry {number} carries no dst")
    if not destination.startswith("/"):
        raise Refusal(f"engine contents entry {number} names the relative destination {destination!r}")
    if not engine_owned(destination):
        raise Refusal(
            f"the engine archive installs {destination}, which is not a path the engine "
            "owns; this package would be overwriting the window's own files"
        )
    if destination.rstrip("/") in claimed:
        raise Refusal(
            f"the engine archive installs {destination} and this package already claims it; "
            "two authors and one package is a file carried twice or taken from whichever "
            "entry nFPM read last"
        )
    return destination


def render_entry(entry: dict, source: Path, destination: str) -> str:
    lines = [f"  - src: {source}", f"    dst: {destination}"]
    if entry.get("type"):
        lines.append(f"    type: {entry['type']}")
    if entry.get("mode"):
        lines.append("    file_info:")
        lines.append(f"      mode: {entry['mode']}")
    return "\n".join(lines) + "\n"


def claimed_destinations(rendered: str) -> set[str]:
    return {
        line.strip()[len("dst: ") :].strip('"').rstrip("/")
        for line in rendered.splitlines()
        if line.strip().startswith("dst: ")
    }


def splice(rendered: str, entries: list[dict], staging: Path, marker: str) -> str:
    occurrences = [
        index for index, line in enumerate(rendered.splitlines()) if line.strip() == marker
    ]
    if len(occurrences) != 1:
        raise Refusal(
            f"the rendered configuration carries the splice marker {len(occurrences)} times "
            "and it must carry it exactly once"
        )

    claimed = claimed_destinations(rendered)
    block = []
    for number, entry in enumerate(entries, start=1):
        source = resolve(entry, staging, number)
        destination = check_destination(entry, claimed, number)
        claimed.add(destination.rstrip("/"))
        block.append(render_entry(entry, source, destination))

    lines = rendered.splitlines(keepends=True)
    return "".join(
        lines[: occurrences[0]] + ["\n".join(block)] + lines[occurrences[0] + 1 :]
    )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rendered", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--engine-contents", required=True, type=Path)
    parser.add_argument("--staging", required=True, type=Path)
    parser.add_argument("--marker", default=MARKER)
    arguments = parser.parse_args(argv)

    try:
        if not arguments.staging.is_dir():
            raise Refusal(f"no verified staging directory at {arguments.staging}")
        entries = parse_entries(read(arguments.engine_contents))
        spliced = splice(
            read(arguments.rendered), entries, arguments.staging, arguments.marker
        )
    except Refusal as refusal:
        print(f"splice_engine_contents: {refusal}", file=sys.stderr)
        return 1

    try:
        arguments.out.write_text(spliced, encoding="utf-8")
    except OSError as error:
        print(f"splice_engine_contents: cannot write {arguments.out}: {error}", file=sys.stderr)
        return 1

    print(f"  {len(entries)} engine entries, authored by the archive and spliced in")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
