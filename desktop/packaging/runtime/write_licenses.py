#!/usr/bin/env python3
"""Write runtime-licenses.json and the tree runtime-licenses.tar.gz is made from.

The tree holds the licence files of everything the runtime is built from:

- components/<name>-<version>/<path>: each lock component's, read from its
  locked tarball. These are the files at the top of its source tree whose names
  start with COPYING, LICENSE, LICENCE, COPYRIGHT or NOTICE, every file under a
  top-level LICENSES directory, and the files its extra_license_files names. A
  link holds the text of its target, which must be one of those files.
- crates/<name>-<version>/<file>: copied by write_crates.py while the crate's
  component was built. The fragments it wrote list them.
- standard/<SPDX id>.txt: the standard texts the fragments name for crates
  that publish no licence file, from the lock's license_text_source, SPDX's
  license-list-data.
- rust-std/<path>: the Rust standard library's, from the toolchain.

The index names every file in the tree, and the tree holds no other file. A
component, a crate or the standard library with no licence file is refused.
README.md documents the format.
"""

import argparse
import glob
import json
import os
import posixpath
import re
import shutil
import subprocess
import sys
import tarfile

ARCHIVE = "runtime-licenses.tar.gz"
TOP_LEVEL = re.compile(r"(?i)^(licen[cs]e|copying|copyright|notice)([-_.].*)?$")
REUSE_DIR = "LICENSES"
# The standard library's own licence, and Unicode-3.0 for core's Unicode
# tables, which every Rust object carries. COPYRIGHT-library.html lists every
# other notice, for every target.
STD_LICENSE = "(Apache-2.0 OR MIT) AND Unicode-3.0"
STD_FILES = ("COPYRIGHT-library.html", "licenses/Apache-2.0.txt", "licenses/MIT.txt",
             "licenses/Unicode-3.0.txt")


def fail(message):
    raise SystemExit("write_licenses: " + message)


def command(args):
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode != 0:
        fail("%s failed: %s" % (" ".join(args), result.stderr.strip()))
    return result.stdout


def inner_path(member_name, source_dir):
    """A tarball entry's path inside the component's source tree, or None."""
    path = posixpath.normpath(member_name)
    prefix = source_dir + "/"
    return path[len(prefix):] if path.startswith(prefix) else None


def is_licence(relative, named):
    parts = relative.split("/")
    if relative in named or (len(parts) > 1 and parts[0] == REUSE_DIR):
        return True
    return len(parts) == 1 and TOP_LEVEL.match(parts[0]) is not None


def write_bytes(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as handle:
        handle.write(data)


def link_target(member, source_dir):
    """Where a link in the tarball points, inside the source tree, or None."""
    if member.issym():
        return inner_path(posixpath.join(posixpath.dirname(member.name), member.linkname),
                          source_dir)
    return inner_path(member.linkname, source_dir)


def read_licence_members(component, sources, destination, named):
    """Writes the licence files of a component's tarball under destination and
    returns their paths, and the links among them with their targets."""
    label = "%s %s" % (component["name"], component["version"])
    files, links = set(), {}
    # Stream mode reads each tarball once, front to back.
    with tarfile.open(os.path.join(sources, component["archive"]), "r|*") as tar:
        for member in tar:
            relative = inner_path(member.name, component["source_dir"])
            if relative is None or member.isdir() or not is_licence(relative, named):
                continue
            if member.issym() or member.islnk():
                links[relative] = (member.linkname, link_target(member, component["source_dir"]))
                continue
            if not member.isfile():
                fail("%s: %s in its tarball is not a regular file" % (label, relative))
            write_bytes(os.path.join(destination, relative), tar.extractfile(member).read())
            files.add(relative)
    return files, links


def component_entry(component, sources, tree):
    name, version = component["name"], component["version"]
    label = "%s %s" % (name, version)
    named = component.get("extra_license_files", [])
    for path in named:
        if path.startswith("/") or ".." in path.split("/"):
            fail("%s: the licence file %s is not inside its source tree" % (label, path))
    top = "components/%s-%s" % (name, version)
    files, links = read_licence_members(component, sources, os.path.join(tree, top), named)
    # A link holds its target's text, when that is a licence file read above.
    for relative, (linkname, target) in sorted(links.items()):
        if target not in files:
            fail("%s: %s links to %s, which is not a licence file in its source tree"
                 % (label, relative, target or linkname))
        shutil.copyfile(os.path.join(tree, top, target), os.path.join(tree, top, relative))
        files.add(relative)
    missing = [path for path in named if path not in files]
    if missing:
        fail("%s: its tarball has no %s" % (label, ", ".join(missing)))
    if not files:
        fail("%s has no licence file in %s" % (label, component["archive"]))
    return {"name": name, "version": version, "license": component["license"],
            "build_only": component.get("build_only", False) is True,
            "license_files": sorted("%s/%s" % (top, path) for path in files)}


def crate_entries(fragments):
    entries = []
    for path in sorted(glob.glob(os.path.join(fragments, "*.json"))):
        with open(path, encoding="utf-8") as handle:
            entries.extend(json.load(handle))
    for entry in entries:
        if not entry["license_files"]:
            fail("%s %s has no licence file" % (entry["name"], entry["version"]))
    return sorted(entries, key=lambda e: (e["component"], e["name"], e["version"]))


def write_standard_texts(lock, sources, tree, crates):
    """Copies the standard texts the crates name from the licence text source."""
    wanted = {path for entry in crates for path in entry["license_files"]
              if path.startswith("standard/")}
    if not wanted:
        return
    source = lock.get("license_text_source")
    if not source:
        fail("crates name %s, and the lock has no license_text_source"
             % ", ".join(sorted(wanted)))
    prefix = "%s/%s/" % (source["source_dir"], source["text_dir"])
    with tarfile.open(os.path.join(sources, source["archive"]), "r|*") as tar:
        for member in tar:
            name = posixpath.normpath(member.name)
            path = "standard/" + name[len(prefix):] if name.startswith(prefix) else None
            if path in wanted and member.isfile():
                write_bytes(os.path.join(tree, path), tar.extractfile(member).read())
                wanted.discard(path)
    if wanted:
        fail("%s %s has no %s" % (source["name"], source["version"], ", ".join(sorted(wanted))))


def std_entry(tree):
    release = ""
    for line in command(["rustc", "-vV"]).splitlines():
        if line.startswith("release: "):
            release = line[len("release: "):]
    if not release:
        fail("rustc -vV names no release")
    doc = os.path.join(command(["rustc", "--print", "sysroot"]).strip(), "share", "doc", "rust")
    for relative in STD_FILES:
        destination = os.path.join(tree, "rust-std", relative)
        os.makedirs(os.path.dirname(destination), exist_ok=True)
        shutil.copyfile(os.path.join(doc, relative), destination)
    return {"version": release, "license": STD_LICENSE,
            "license_files": sorted("rust-std/" + relative for relative in STD_FILES)}


def check_tree(tree, entries):
    """The tree holds exactly the files the index names."""
    named = {path for entry in entries for path in entry["license_files"]}
    for path in sorted(named):
        if not os.path.isfile(os.path.join(tree, path)):
            fail("%s is listed and is not in the archive tree" % path)
    present = {os.path.relpath(os.path.join(directory, name), tree)
               for directory, _, names in os.walk(tree) for name in names}
    extra = sorted(present - named)
    if extra:
        fail("the archive tree holds files no entry names: %s" % ", ".join(extra))


def main(argv):
    parser = argparse.ArgumentParser()
    parser.add_argument("--lock", required=True)
    parser.add_argument("--sources", required=True, help="the locked tarballs")
    parser.add_argument("--crates", required=True, help="write_crates.py's fragments")
    parser.add_argument("--tree", required=True, help="the licence file tree")
    parser.add_argument("--out", required=True)
    args = parser.parse_args(argv)
    with open(args.lock, encoding="utf-8") as handle:
        lock = json.load(handle)
    for written_here in ("components", "standard", "rust-std"):
        shutil.rmtree(os.path.join(args.tree, written_here), ignore_errors=True)
    components = sorted((component_entry(c, args.sources, args.tree)
                         for c in lock["components"]), key=lambda e: e["name"])
    crates = crate_entries(args.crates)
    write_standard_texts(lock, args.sources, args.tree, crates)
    # The standard library is compiled in only with a Rust crate.
    rust_std = std_entry(args.tree) if crates else None
    check_tree(args.tree, components + crates + ([rust_std] if rust_std else []))
    index = {"schema_version": 1, "archive": ARCHIVE, "components": components,
             "crates": crates, "rust_std": rust_std}
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(index, handle, indent=2)
        handle.write("\n")
    print("write_licenses: %d components, %d crates%s"
          % (len(components), len(crates), " and std" if rust_std else ""), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
