#!/usr/bin/env python3
"""The Rust crates the window's release build compiles, each with its licence and its licence
files' text, and whether it is linked into the window.

    window_crates.py --metadata <cargo metadata JSON> --root <package> --out <crates.json>

<cargo metadata JSON> is `cargo metadata --format-version 1 --locked --offline --filter-platform
<target triple>` of the desktop workspace, so a crate for another platform is already gone. From
the root package the walk follows normal and build dependencies, not dev dependencies, which only
tests compile. A crate is linked when a path of normal edges with no proc-macro on it reaches it;
the others run while the window is built, as a build script, a proc-macro, or what one of those
needs. Path packages are this repository's own code, under its own licence, and are walked through
but not listed.

Each crate's licence files are the ones its source carries at its top level (LICENSE*, LICENCE*,
COPYING*, COPYRIGHT*, NOTICE*, UNLICENSE*) and the file its license_file names, read from the cargo
registry the build used. A crate that ships none is listed with no files; the copyright file then
says its text is the licence's standard one. A licence written with the old "/" is read as OR, and
a crate with only a license_file has the licence LicenseRef-<name>-<version>.
"""

import argparse
import hashlib
import json
import os
import re
import sys

SCHEMA_VERSION = 1
LICENCE_FILE = re.compile(r"(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE|UNLICENSE)([-._].*)?", re.I)
USAGE = "window_crates.py --metadata <cargo metadata JSON> --root <package> --out <crates.json>"


class Refusal(Exception):
    """A refusal; its message is the sentence printed after "window_crates: "."""


class Parser(argparse.ArgumentParser):
    """argparse, refusing a malformed command line the way every other mistake here is refused."""

    def error(self, message):
        raise Refusal(f"usage: {USAGE} ({message})")


def read_text(path):
    try:
        with open(path, encoding="utf-8") as handle:
            return handle.read()
    except OSError as error:
        raise Refusal(f"cannot read {path}: {error.strerror}") from error
    except UnicodeDecodeError as error:
        raise Refusal(f"{path} is not UTF-8 text: {error}") from error


def read_metadata(path):
    try:
        metadata = json.loads(read_text(path))
    except ValueError as error:
        raise Refusal(f"{path} is not valid JSON: {error}") from error
    if not isinstance(metadata.get("packages"), list) or "resolve" not in metadata:
        raise Refusal(f"{path} is not cargo metadata with a resolved graph")
    return metadata


def is_proc_macro(package):
    return any("proc-macro" in target["kind"] for target in package["targets"])


def is_normal(dependency):
    return any(kind["kind"] is None for kind in dependency["dep_kinds"])


def is_built(dependency):
    return any(kind["kind"] in (None, "build") for kind in dependency["dep_kinds"])


def reach(packages, edges, root, follow):
    """The ids reached from <root> through the edges <follow> accepts."""
    reached, pending = set(), [root]
    # Each id is expanded once, so the walk ends after at most one step per package.
    while pending:
        current = pending.pop()
        if current in reached:
            continue
        reached.add(current)
        for dependency in edges.get(current, []):
            if follow(dependency, packages[dependency["pkg"]]):
                pending.append(dependency["pkg"])
    return reached


def compiled(metadata, root_name):
    """Every registry package the release build compiles, each with whether it is linked."""
    packages = {package["id"]: package for package in metadata["packages"]}
    edges = {node["id"]: node["deps"] for node in metadata["resolve"]["nodes"]}
    roots = [p["id"] for p in packages.values() if p["name"] == root_name and p["source"] is None]
    if len(roots) != 1:
        raise Refusal(f"the metadata has no package {root_name} of this workspace")
    built = reach(packages, edges, roots[0], lambda d, _: is_built(d))
    linked = reach(packages, edges, roots[0], lambda d, p: is_normal(d) and not is_proc_macro(p))
    return [(packages[i], i in linked) for i in sorted(built) if packages[i]["source"] is not None]


def licence(package):
    if package.get("license"):
        return " OR ".join(part.strip() for part in package["license"].split("/"))
    if package.get("license_file"):
        return f"LicenseRef-{package['name']}-{package['version']}"
    raise Refusal(f"{package['name']} {package['version']} declares no licence")


def licence_files(package):
    directory = os.path.dirname(package["manifest_path"])
    names = sorted(name for name in os.listdir(directory) if LICENCE_FILE.fullmatch(name))
    named = package.get("license_file")
    if named and named not in names:
        names.append(named)
    files = []
    for name in names:
        text = read_text(os.path.join(directory, name))
        files.append({"name": name, "sha256": hashlib.sha256(text.encode()).hexdigest(),
                      "text": text})  # fmt: skip
    return files


def record(package, is_linked):
    return {
        "name": package["name"],
        "version": package["version"],
        "linked": is_linked,
        "license": licence(package),
        "authors": package.get("authors") or [],
        "repository": package.get("repository"),
        "files": licence_files(package),
    }


def write(path, document):
    partial = f"{path}.partial"
    with open(partial, "w", encoding="utf-8") as handle:
        json.dump(document, handle, indent=2, sort_keys=True)
        handle.write("\n")
    os.replace(partial, path)


def parse_arguments(argv):
    parser = Parser(prog="window_crates.py", add_help=False)
    for name in ("metadata", "root", "out"):
        parser.add_argument(f"--{name}", required=True)
    return parser.parse_args(argv)


def main(argv):
    try:
        arguments = parse_arguments(argv)
        packages = compiled(read_metadata(arguments.metadata), arguments.root)
        crates = sorted((record(p, is_linked) for p, is_linked in packages),
                        key=lambda c: (c["name"], c["version"]))  # fmt: skip
        write(arguments.out, {"schema_version": SCHEMA_VERSION, "root": arguments.root,
                              "crates": crates})  # fmt: skip
    except Refusal as refusal:
        print(f"window_crates: {refusal}", file=sys.stderr)
        return 1
    linked = sum(1 for crate in crates if crate["linked"])
    print(f"window_crates: {len(crates)} crates in {arguments.root}'s release build, "
          f"{linked} of them linked into it, in {arguments.out}")  # fmt: skip
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
