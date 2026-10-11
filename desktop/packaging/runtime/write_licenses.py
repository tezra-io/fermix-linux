#!/usr/bin/env python3
"""Write runtime-licenses.json and the tree runtime-licenses.tar.gz is made from.

The tree holds the licence files of everything the runtime is built from:

- components/<name>-<version>/<path>: each lock component's, read from its
  locked tarball. These are the files at the top of its source tree whose names
  start with COPYING, LICENSE, LICENCE, COPYRIGHT or NOTICE, every file under a
  top-level LICENSES directory, and the files its extra_license_files names. A
  link holds the text of its target, which must be one of those files.
- components/<name>-<version>/<path>.notice: a notice inside a source file,
  the lines its license_excerpts entry names, held to that entry's sha256. A
  file with two notices has two entries, and the notice holds both in order.
- crates/<name>-<version>/<file>: copied by write_crates.py while the crate's
  component was built. The fragments it wrote list them.
- standard/<SPDX id>.txt: the standard texts the fragments name for crates
  that publish no licence file, and those a component's standard_license_texts
  names for a licence no file of its own holds, each a file of SPDX's
  license-list-data that the lock's license_text_sources pins.
- rust-std/<path>: the Rust standard library's, from the toolchain.

The index names every file in the tree, and the tree holds no other file. A
component, a crate or the standard library with no licence file is refused.
Its license_refs maps each LicenseRef any expression names to the one file in
the tree that holds its text, from the components' license_refs; a LicenseRef
mapped nowhere, mapped twice or not named is refused. README.md documents the
format.
"""

import argparse
import glob
import hashlib
import json
import os
import posixpath
import re
import shutil
import subprocess
import sys
import tarfile
from collections import namedtuple

ARCHIVE = "runtime-licenses.tar.gz"
TOP_LEVEL = re.compile(r"(?i)^(licen[cs]e|copying|copyright|notice)([-_.].*)?$")
REUSE_DIR = "LICENSES"
SPDX_ID = re.compile(r"[A-Za-z0-9.+-]+")
LICENSE_REF = re.compile(r"LicenseRef-[A-Za-z0-9.-]+")
# A component's extra_license_files, and the paths its license_excerpts read.
Named = namedtuple("Named", "files excerpts")
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


def wanted_members(tar, source_dir, wanted):
    """The entries of a component's tarball whose paths wanted takes."""
    for member in tar:
        relative = inner_path(member.name, source_dir)
        if relative is not None and not member.isdir() and wanted(relative):
            yield relative, member


def read_member(tar, member, relative, found, label):
    """Sorts one entry into the links, or reads a regular file's bytes."""
    if member.issym() or member.islnk():
        found["links"][relative] = (member.linkname, link_target(member, found["source_dir"]))
        return
    if not member.isfile():
        fail("%s: %s in its tarball is not a regular file" % (label, relative))
    found["data"][relative] = tar.extractfile(member).read()


def read_component(component, sources, named, excerpted):
    """The licence files and excerpted sources of a component's tarball: their
    bytes by path, and the links among them with their targets."""
    label = "%s %s" % (component["name"], component["version"])
    found = {"source_dir": component["source_dir"], "data": {}, "links": {}}
    path = os.path.join(sources, component["archive"])
    # Stream mode reads each tarball once, front to back.
    with tarfile.open(path, "r|*") as tar:
        for relative, member in wanted_members(
                tar, component["source_dir"],
                lambda relative: is_licence(relative, named) or relative in excerpted):
            read_member(tar, member, relative, found, label)
    return found


def excerpt_notice(label, excerpt, data):
    """The lines an excerpt names, held to the digest the lock records."""
    if data is None:
        fail("%s: its tarball has no %s" % (label, excerpt["path"]))
    first, last = excerpt["lines"]
    lines = data.splitlines(keepends=True)
    if not 1 <= first <= last <= len(lines):
        fail("%s: %s has %d lines, not %d" % (label, excerpt["path"], len(lines), last))
    notice = b"".join(lines[first - 1:last])
    digest = hashlib.sha256(notice).hexdigest()
    if digest != excerpt["sha256"]:
        fail("%s: lines %d-%d of %s have sha256 %s, and the lock says %s"
             % (label, first, last, excerpt["path"], digest, excerpt["sha256"]))
    return notice


def licence_files(label, found, named):
    """The licence files' bytes by path, a link holding its target's text when
    that target is a licence file read too."""
    files = {path: data for path, data in found["data"].items() if path not in named.excerpts}
    for relative, (linkname, target) in sorted(found["links"].items()):
        if target not in files:
            fail("%s: %s links to %s, which is not a licence file in its source tree"
                 % (label, relative, target or linkname))
        files[relative] = files[target]
    return files


def component_standard(label, component):
    """The standard texts a component's licence needs beyond its own files."""
    ids = component.get("standard_license_texts", [])
    named = set(SPDX_ID.findall(component["license"])) - {"AND", "OR", "WITH"}
    unnamed = [spdx_id for spdx_id in ids if spdx_id not in named]
    if unnamed:
        fail("%s names the standard text of %s, which its licence does not name"
             % (label, ", ".join(unnamed)))
    return ["standard/%s.txt" % spdx_id for spdx_id in ids]


def component_entry(component, sources, tree):
    name, version = component["name"], component["version"]
    label = "%s %s" % (name, version)
    named = Named(component.get("extra_license_files", []),
                  {e["path"] for e in component.get("license_excerpts", [])})
    for path in named.files + sorted(named.excerpts):
        if path.startswith("/") or ".." in path.split("/"):
            fail("%s: the file %s is not inside its source tree" % (label, path))
    found = read_component(component, sources, named.files, named.excerpts)
    linked = sorted(named.excerpts & set(found["links"]))
    if linked:
        fail("%s: %s is a link, and an excerpt reads a regular file" % (label, ", ".join(linked)))
    files = licence_files(label, found, named)
    missing = [path for path in named.files if path not in files]
    if missing:
        fail("%s: its tarball has no %s" % (label, ", ".join(missing)))
    # The excerpts of one file make one notice, in the lock's order.
    for excerpt in component.get("license_excerpts", []):
        notice = excerpt_notice(label, excerpt, found["data"].get(excerpt["path"]))
        key = excerpt["path"] + ".notice"
        files[key] = files.get(key, b"") + notice
    if not files:
        fail("%s has no licence file in %s" % (label, component["archive"]))
    standard = component_standard(label, component)
    top = "components/%s-%s" % (name, version)
    for relative, data in files.items():
        write_bytes(os.path.join(tree, top, relative), data)
    return {"name": name, "version": version, "license": component["license"],
            "build_only": component.get("build_only", False) is True,
            "license_files": sorted(["%s/%s" % (top, path) for path in files] + standard)}


def component_refs(component, entry):
    """A component's LicenseRefs, each mapped to one of its licence files."""
    label = "%s %s" % (entry["name"], entry["version"])
    refs = component.get("license_refs", {})
    named = sorted(set(LICENSE_REF.findall(entry["license"])))
    if sorted(refs) != named:
        fail("%s: its licence names %s, and license_refs maps %s"
             % (label, ", ".join(named) or "no LicenseRef", ", ".join(sorted(refs)) or "none"))
    top = "components/%s-%s/" % (entry["name"], entry["version"])
    mapped = {ref: top + path for ref, path in refs.items()}
    for ref, path in sorted(mapped.items()):
        if path not in entry["license_files"]:
            fail("%s: %s maps to %s, which is not one of its licence files" % (label, ref, path))
    return mapped


def licence_refs(pairs, crates):
    """Every LicenseRef an expression names, mapped to exactly one licence file.
    Only a component's license_refs maps one, so a crate that names one fails."""
    refs = {}
    for component, entry in pairs:
        for ref, path in component_refs(component, entry).items():
            if ref in refs:
                fail("%s is mapped twice: %s and %s" % (ref, refs[ref], path))
            refs[ref] = path
    for entry in crates:
        named = sorted(set(LICENSE_REF.findall(entry["license"])))
        if named:
            fail("%s %s names %s, which only a component's license_refs can map"
                 % (entry["name"], entry["version"], ", ".join(named)))
    return dict(sorted(refs.items()))


def crate_entries(fragments):
    entries = []
    for path in sorted(glob.glob(os.path.join(fragments, "*.json"))):
        with open(path, encoding="utf-8") as handle:
            entries.extend(json.load(handle))
    for entry in entries:
        if not entry["license_files"]:
            fail("%s %s has no licence file" % (entry["name"], entry["version"]))
    return sorted(entries, key=lambda e: (e["component"], e["name"], e["version"]))


def write_standard_texts(lock, sources, tree, entries):
    """Copies the standard texts the entries name, each a file the lock pins."""
    wanted = sorted({path for entry in entries for path in entry["license_files"]
                     if path.startswith("standard/")})
    pinned = {"standard/%s.txt" % source["spdx_id"]: source["file"]
              for source in lock.get("license_text_sources", [])}
    unpinned = [path for path in wanted if path not in pinned]
    if unpinned:
        fail("the index names %s, and the lock pins no text for them" % ", ".join(unpinned))
    for path in wanted:
        os.makedirs(os.path.dirname(os.path.join(tree, path)), exist_ok=True)
        shutil.copyfile(os.path.join(sources, pinned[path]), os.path.join(tree, path))


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
        if os.path.exists(os.path.join(args.tree, written_here)):
            shutil.rmtree(os.path.join(args.tree, written_here))
    pairs = [(c, component_entry(c, args.sources, args.tree)) for c in lock["components"]]
    components = sorted((entry for _, entry in pairs), key=lambda e: e["name"])
    crates = crate_entries(args.crates)
    refs = licence_refs(pairs, crates)
    write_standard_texts(lock, args.sources, args.tree, components + crates)
    # The standard library is compiled in only with a Rust crate.
    rust_std = std_entry(args.tree) if crates else None
    check_tree(args.tree, components + crates + ([rust_std] if rust_std else []))
    index = {"schema_version": 1, "archive": ARCHIVE, "components": components,
             "crates": crates, "rust_std": rust_std, "license_refs": refs}
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(index, handle, indent=2)
        handle.write("\n")
    print("write_licenses: %d components, %d crates%s"
          % (len(components), len(crates), " and std" if rust_std else ""), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
