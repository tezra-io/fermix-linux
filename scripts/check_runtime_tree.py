#!/usr/bin/env python3
"""The staged toolkit is the tree its manifest describes, file by file.

The runtime arrives as a tarball and a manifest, and until now the package
trusted the tarball. Three things happened on the way to a release that say it
should not.

A cache key names the lock file, the Dockerfile and the patches, so two exports
of the same key can differ; one did, by a file. A tarball's sha256 is not
reproducible either, because tar writes members in readdir order, so the digest
moves when nothing in the tree has. And a tag is a label somebody can point at
anything. None of those three can answer "is this the tree it claims to be".

The manifest can. It carries a sha256 for every regular file and a target for
every symlink, so the question becomes arithmetic: 1493 entries, each present,
each matching, and nothing in the prefix that the manifest does not list. The
last clause is the one that catches a file arriving rather than leaving.

The tarball's own digest is checked too, when the archive is named. That is not
the same question as the tree's, and slice 1 found the case that separates them:
gdbus-codegen's __pycache__ made the dev tarball non-reproducible while the tree
inside it was always correct, so every per-file check passed and the archive
still differed run to run. A tree check cannot see that; only the archive digest
can.

Usage:
  check_runtime_tree.py --stage <dir> --manifest <runtime-manifest.json>
                        [--archive <runtime-*.tar>] [--prefix /usr/lib/fermix-desktop]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path

# Enough that a wrong tree is obvious, few enough that the refusal stays
# readable. A tree that differs usually differs in one file or in all of them.
SHOWN = 5


class Refusal(RuntimeError):
    """A difference between the tree and its manifest, phrased for a person."""


def digest(path: Path) -> str:
    reader = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            reader.update(block)
    return reader.hexdigest()


def entries(manifest: Path) -> list[dict]:
    try:
        loaded = json.loads(manifest.read_text(encoding="utf-8"))
    except OSError as error:
        raise Refusal(f"cannot read {manifest}: {error}") from error
    except json.JSONDecodeError as error:
        raise Refusal(f"{manifest} is not valid JSON: {error}") from error

    files = loaded.get("files")
    if not files:
        raise Refusal(f"{manifest} lists no files, and it is what the tree is checked against")
    return files


def check_archive(archive: Path, manifest: Path) -> list[str]:
    """The tarball is the one the manifest names, by its own digest."""
    try:
        loaded = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise Refusal(f"cannot read {manifest}: {error}") from error

    archives = loaded.get("archives")
    if not archives:
        # An older manifest does not carry them, and refusing would make this
        # gate a version requirement on somebody else's file rather than a check.
        return []
    recorded = archives.get(archive.name)
    if not recorded:
        return [
            f"{manifest.name} records digests for {', '.join(sorted(archives))} "
            f"and not for {archive.name}"
        ]
    found = digest(archive)
    if found != recorded:
        return [
            f"{archive.name} hashes to {found[:16]} and {manifest.name} says "
            f"{recorded[:16]}; the archive is not the one this manifest describes"
        ]
    return []


def check(stage: Path, manifest: Path, prefix: str) -> list[str]:
    problems: list[str] = []
    listed: set[str] = set()

    for entry in entries(manifest):
        path = entry.get("path")
        if not path:
            problems.append(f"{manifest.name} carries an entry with no path")
            continue
        listed.add(path)
        on_disk = stage / path.lstrip("/")

        if entry.get("kind") == "link":
            if not on_disk.is_symlink():
                problems.append(f"{path} is a symlink in the manifest and is not one in the tree")
                continue
            target = os.readlink(on_disk)
            if target != entry.get("target"):
                problems.append(
                    f"{path} points at {target} and the manifest says {entry.get('target')}"
                )
            continue

        if on_disk.is_symlink() or not on_disk.is_file():
            problems.append(f"{path} is in the manifest and not in the staged tree")
            continue
        found = digest(on_disk)
        if found != entry.get("sha256"):
            problems.append(
                f"{path} hashes to {found[:16]} and the manifest says "
                f"{str(entry.get('sha256'))[:16]}"
            )

    # Nothing in the prefix that the manifest does not list. A file that arrives
    # is as much a difference as a file that goes missing, and it is the one a
    # per-entry loop cannot see.
    root = stage / prefix.lstrip("/")
    if not root.is_dir():
        problems.append(f"the staged tree carries no private prefix at {root}")
        return problems

    for directory, _, names in os.walk(root):
        for name in names:
            found = Path(directory) / name
            path = "/" + str(found.relative_to(stage))
            if path not in listed:
                problems.append(f"{path} is in the staged tree and not in {manifest.name}")

    return problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument(
        "--archive",
        type=Path,
        help="the tarball the stage was unpacked from, checked against the "
        "manifest's own record of its digest",
    )
    parser.add_argument("--prefix", default="/usr/lib/fermix-desktop")
    arguments = parser.parse_args(argv)

    try:
        if not arguments.stage.is_dir():
            raise Refusal(f"no staged tree at {arguments.stage}")
        problems = check(arguments.stage, arguments.manifest, arguments.prefix)
        if arguments.archive is not None:
            if not arguments.archive.is_file():
                raise Refusal(f"no archive at {arguments.archive}")
            problems += check_archive(arguments.archive, arguments.manifest)
        total = len(entries(arguments.manifest))
    except Refusal as refusal:
        print(f"check_runtime_tree: {refusal}", file=sys.stderr)
        return 1

    for problem in problems[:SHOWN]:
        print(f"check_runtime_tree: {problem}", file=sys.stderr)
    if len(problems) > SHOWN:
        print(
            f"check_runtime_tree: and {len(problems) - SHOWN} more differences",
            file=sys.stderr,
        )
    if problems:
        return 1

    print(f"  {total} entries, every one the tree its manifest describes")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
