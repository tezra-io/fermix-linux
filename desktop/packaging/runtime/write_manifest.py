#!/usr/bin/env python3
"""Write runtime-manifest.json: the lock file plus the sha256 of every installed file.

This is what ships at /usr/share/doc/fermix-desktop/runtime-manifest.json, and it
is what makes the licensing obligation of amendment section 4.6 answerable from
an installed machine: every component, its version, its source URL, its digest,
and the digest of every file it put on disk.
"""

import argparse
import hashlib
import json
import os
import sys

CHUNK = 1024 * 1024


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as handle:
        while True:
            block = handle.read(CHUNK)
            if not block:
                break
            h.update(block)
    return h.hexdigest()


def walk_tree(tree, prefix):
    """Every regular file and every symbolic link under the tree, sorted.

    A link is recorded by its target rather than by a digest: a link has no
    contents, and what matters about libfoo.so.1 is what it points at.
    """
    entries = []
    for root, dirs, names in os.walk(tree):
        dirs.sort()
        for name in sorted(names):
            full = os.path.join(root, name)
            installed = os.path.join(prefix, os.path.relpath(full, tree))
            if os.path.islink(full):
                entries.append(
                    {"path": installed, "kind": "link", "target": os.readlink(full)}
                )
            else:
                entries.append(
                    {
                        "path": installed,
                        "kind": "file",
                        "mode": os.stat(full).st_mode & 0o7777,
                        "sha256": digest(full),
                    }
                )
    entries.sort(key=lambda entry: entry["path"])
    return entries


def main(argv):
    parser = argparse.ArgumentParser()
    parser.add_argument("--lock", required=True)
    parser.add_argument("--tree", required=True)
    parser.add_argument("--prefix", required=True)
    parser.add_argument("--arch", required=True)
    parser.add_argument("--out", required=True)
    # The exported tarballs, as NAME=SHA256. Recorded here so --verify checks
    # the bytes a consumer actually downloads and not only the tree they unpack
    # to: a tar can differ from an identical tree through entry order or
    # metadata, and a published digest that verifies for nobody is worse than
    # no published digest at all.
    parser.add_argument("--archive", action="append", default=[],
                        metavar="NAME=SHA256")
    args = parser.parse_args(argv)

    archives = {}
    for pair in args.archive:
        name, _, digest = pair.partition("=")
        if not name or len(digest) != 64:
            parser.error("--archive wants NAME=SHA256, got %r" % pair)
        archives[name] = digest

    if not os.path.isdir(args.tree):
        parser.error("no staged tree at %s" % args.tree)

    with open(args.lock, encoding="utf-8") as handle:
        lock = json.load(handle)

    files = walk_tree(args.tree, args.prefix)
    if not files:
        parser.error("the staged tree at %s is empty" % args.tree)

    manifest = {
        "schema_version": 1,
        "architecture": args.arch,
        "prefix": args.prefix,
        "lock": lock,
        "archives": dict(sorted(archives.items())),
        "files": files,
    }
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2, sort_keys=False)
        handle.write("\n")
    print("write_manifest: %d files recorded for %s" % (len(files), args.arch))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
