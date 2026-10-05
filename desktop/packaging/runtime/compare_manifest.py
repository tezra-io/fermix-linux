#!/usr/bin/env python3
"""Compare a rebuilt runtime manifest against the recorded one.

`build_runtime.sh --verify` rebuilds the whole runtime and asks this whether the
result is the same tree. A difference is reported file by file, with a cap on how
many are printed, because "the rebuild differs" tells a reader nothing about
whether one object carried a timestamp or the whole tree moved.
"""

import argparse
import json
import sys

MAX_REPORTED = 20


def load(path):
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def index(manifest):
    return {entry["path"]: entry for entry in manifest["files"]}


def describe(entry):
    if entry["kind"] == "link":
        return "-> %s" % entry["target"]
    return entry["sha256"]


def compare(reference, fresh):
    problems = []
    if reference["lock"] != fresh["lock"]:
        problems.append("the lock file recorded in the manifest is not the one rebuilt")
    if reference["architecture"] != fresh["architecture"]:
        problems.append(
            "architecture %s was rebuilt as %s"
            % (reference["architecture"], fresh["architecture"])
        )

    # The tarballs, not only the tree inside them. Two archives of an identical
    # tree can differ in entry order or metadata, and it is the archive's digest
    # that gets published, so a rebuild that reproduces the tree but not the tar
    # is a rebuild whose published digest verifies for nobody.
    old_archives = reference.get("archives", {})
    new_archives = fresh.get("archives", {})
    for name in sorted(set(old_archives) | set(new_archives)):
        if name not in old_archives:
            problems.append("new archive in the rebuild: %s" % name)
        elif name not in new_archives:
            problems.append("missing from the rebuild: %s" % name)
        elif old_archives[name] != new_archives[name]:
            problems.append(
                "archive differs: %s (%s -> %s)"
                % (name, old_archives[name], new_archives[name])
            )

    old, new = index(reference), index(fresh)
    for path in sorted(set(old) - set(new)):
        problems.append("missing from the rebuild: %s" % path)
    for path in sorted(set(new) - set(old)):
        problems.append("new in the rebuild: %s" % path)
    for path in sorted(set(old) & set(new)):
        if describe(old[path]) != describe(new[path]):
            problems.append(
                "differs: %s (%s -> %s)"
                % (path, describe(old[path]), describe(new[path]))
            )
    return problems


def main(argv):
    parser = argparse.ArgumentParser()
    parser.add_argument("reference")
    parser.add_argument("fresh")
    args = parser.parse_args(argv)

    problems = compare(load(args.reference), load(args.fresh))
    if not problems:
        print("compare_manifest: the rebuild is identical")
        return 0

    for line in problems[:MAX_REPORTED]:
        print("compare_manifest: %s" % line, file=sys.stderr)
    if len(problems) > MAX_REPORTED:
        print(
            "compare_manifest: and %d more" % (len(problems) - MAX_REPORTED),
            file=sys.stderr,
        )
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
