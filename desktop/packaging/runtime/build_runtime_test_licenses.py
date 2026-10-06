#!/usr/bin/env python3
"""write_licenses.py's own tests, run by build_runtime_test.sh.

A fixture lock file names three components, each in a tarball of another
compression:

- one, like GLib: a REUSE LICENSES directory, and a COPYING that is a link into it;
- two, like AppStream: entries that start with ./, and like FreeType, a licence
  text in a file the lock names;
- three: built from only, with a LICENSE that is a link to its NOTICE.

A licence text source stands for SPDX's license-list-data, a crate fragment for
what write_crates.py wrote, and rustc is a stub on PATH that answers the two
questions write_licenses.py asks.

Usage: build_runtime_test_licenses.py <write_licenses.py> <work dir>
"""

import io
import json
import os
import subprocess
import sys
import tarfile

RUSTC_STUB = """#!/bin/sh
case "$*" in
  "-vV") printf 'rustc 9.9.9\\nhost: x86_64-unknown-linux-gnu\\nrelease: 9.9.9\\n' ;;
  "--print sysroot") echo "%s/sysroot" ;;
  *) exit 2 ;;
esac
"""

COMPONENTS = [
    {"name": "one", "version": "1.0", "archive": "one-1.0.tar.xz", "source_dir": "one-1.0",
     "license": "MIT AND Apache-2.0"},
    {"name": "two", "version": "2.0", "archive": "two-2.0.tar.gz", "source_dir": "two-2.0",
     "license": "FTL OR GPL-2.0-or-later", "extra_license_files": ["docs/FTL.TXT"]},
    {"name": "three", "version": "3.0", "archive": "three-3.0.tar.bz2",
     "source_dir": "three-3.0", "license": "MIT", "build_only": True},
]
TEXT_SOURCE = {"name": "spdx", "version": "1.0", "archive": "spdx-1.0.tar.gz",
               "source_dir": "spdx-1.0", "text_dir": "text"}

MEMBERS = {
    "one": {"one-1.0/LICENSES/MIT.txt": "MIT\n", "one-1.0/LICENSES/Apache-2.0.txt": "Apache\n",
            "one-1.0/LICENSES/sub/deep.txt": "deep\n", "one-1.0/README": "not a licence\n",
            "one-1.0/src/COPYING": "not at the top\n"},
    "two": {"./two-2.0/LICENSE.TXT": "see docs/FTL.TXT\n", "./two-2.0/docs/FTL.TXT": "FTL\n",
            "./two-2.0/docs/GPLv2.TXT": "not named\n"},
    "three": {"three-3.0/Copyright": "three's notice\n", "three-3.0/NOTICE": "notice\n"},
    "spdx": {"spdx-1.0/text/MIT.txt": "standard MIT\n",
             "spdx-1.0/text/Apache-2.0.txt": "standard Apache\n",
             "spdx-1.0/text/MPL-2.0.txt": "standard MPL\n", "spdx-1.0/README.md": "spdx\n"},
}
# Entries that are not regular files: (name, type, link target).
SPECIALS = {"one": [("one-1.0/COPYING", tarfile.SYMTYPE, "LICENSES/MIT.txt")],
            "three": [("three-3.0/LICENSE", tarfile.SYMTYPE, "NOTICE")]}


def crate(name, version, licence, files, source="crates.io"):
    return {"component": "rusty", "name": name, "version": version, "license": licence,
            "source": source, "checksum": "c" * 64 if source == "crates.io" else None,
            "license_files": files}


CRATES = [
    crate("zeta", "1.0.0", "MIT", ["crates/zeta-1.0.0/LICENSE"]),
    crate("alpha", "2.0.0", "MIT/Apache-2.0", ["standard/Apache-2.0.txt", "standard/MIT.txt"]),
    crate("beta", "1.0.0", "MPL-2.0", ["standard/MPL-2.0.txt"]),
    crate("member", "0.1.0", "LGPL-2.1-or-later", ["crates/member-0.1.0/COPYING"], "path"),
]


def fail(message):
    raise SystemExit("licenses_test: " + message)


def write(path, text, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(text)
    os.chmod(path, mode)


def write_tarball(path, members, specials=()):
    mode = {"xz": "w:xz", "gz": "w:gz", "bz2": "w:bz2"}[path.rsplit(".", 1)[1]]
    with tarfile.open(path, mode) as tar:
        for name, text in sorted(members.items()):
            data = text.encode("utf-8")
            info = tarfile.TarInfo(name)
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
        for name, kind, target in specials:
            info = tarfile.TarInfo(name)
            info.type = kind
            info.linkname = target
            tar.addfile(info)


def make_fixture(work, components=None, members=None, crates=None, specials=None,
                 text_source=True):
    components = COMPONENTS if components is None else components
    members = MEMBERS if members is None else members
    crates = CRATES if crates is None else crates
    specials = SPECIALS if specials is None else specials
    sources = os.path.join(work, "sources")
    os.makedirs(sources, exist_ok=True)
    for entry in components + [TEXT_SOURCE]:
        write_tarball(os.path.join(sources, entry["archive"]), members[entry["name"]],
                      specials.get(entry["name"], ()))
    lock = {"components": components}
    if text_source:
        lock["license_text_source"] = TEXT_SOURCE
    write(os.path.join(work, "lock.json"), json.dumps(lock))
    tree = os.path.join(work, "tree")
    write(os.path.join(tree, "crates", "zeta-1.0.0", "LICENSE"), "zeta's licence\n")
    write(os.path.join(tree, "crates", "member-0.1.0", "COPYING"), "the member's\n")
    fragments = os.path.join(work, "fragments")
    os.makedirs(fragments, exist_ok=True)
    if crates:
        write(os.path.join(fragments, "rusty.json"), json.dumps(crates))
    bin_dir = os.path.join(work, "bin")
    write(os.path.join(bin_dir, "rustc"), RUSTC_STUB % work, 0o755)
    doc = os.path.join(work, "sysroot", "share", "doc", "rust")
    write(os.path.join(doc, "COPYRIGHT-library.html"), "<html>std</html>\n")
    for name in ("Apache-2.0", "MIT", "Unicode-3.0", "GPL-3.0-or-later"):
        write(os.path.join(doc, "licenses", name + ".txt"), name + "\n")
    return bin_dir


def run(script, work, bin_dir):
    env = dict(os.environ, PATH=bin_dir + os.pathsep + os.environ["PATH"])
    return subprocess.run(
        [sys.executable, script, "--lock", os.path.join(work, "lock.json"),
         "--sources", os.path.join(work, "sources"),
         "--crates", os.path.join(work, "fragments"),
         "--tree", os.path.join(work, "tree"),
         "--out", os.path.join(work, "runtime-licenses.json")],
        cwd=work, env=env, capture_output=True, text=True, check=False)


def read_tree(tree):
    found = {}
    for directory, _, files in os.walk(tree):
        for name in files:
            path = os.path.join(directory, name)
            with open(path, encoding="utf-8") as handle:
                found[os.path.relpath(path, tree)] = handle.read()
    return found


EXPECTED = {
    "schema_version": 1,
    "archive": "runtime-licenses.tar.gz",
    "components": [
        {"name": "one", "version": "1.0", "license": "MIT AND Apache-2.0", "build_only": False,
         "license_files": ["components/one-1.0/COPYING",
                           "components/one-1.0/LICENSES/Apache-2.0.txt",
                           "components/one-1.0/LICENSES/MIT.txt",
                           "components/one-1.0/LICENSES/sub/deep.txt"]},
        {"name": "three", "version": "3.0", "license": "MIT", "build_only": True,
         "license_files": ["components/three-3.0/Copyright", "components/three-3.0/LICENSE",
                           "components/three-3.0/NOTICE"]},
        {"name": "two", "version": "2.0", "license": "FTL OR GPL-2.0-or-later",
         "build_only": False,
         "license_files": ["components/two-2.0/LICENSE.TXT", "components/two-2.0/docs/FTL.TXT"]},
    ],
    "crates": [CRATES[1], CRATES[2], CRATES[3], CRATES[0]],
    "rust_std": {"version": "9.9.9", "license": "(Apache-2.0 OR MIT) AND Unicode-3.0",
                 "license_files": ["rust-std/COPYRIGHT-library.html",
                                   "rust-std/licenses/Apache-2.0.txt",
                                   "rust-std/licenses/MIT.txt",
                                   "rust-std/licenses/Unicode-3.0.txt"]},
}


def case_index_and_tree(script, root):
    work = os.path.join(root, "good")
    bin_dir = make_fixture(work)
    for stale in ("components/gone-0.1/COPYING", "standard/GPL-3.0-only.txt"):
        write(os.path.join(work, "tree", stale), "a stale run\n")
    result = run(script, work, bin_dir)
    if result.returncode != 0:
        fail("write_licenses.py refused the fixture: " + result.stderr)
    with open(os.path.join(work, "runtime-licenses.json"), encoding="utf-8") as handle:
        index = json.load(handle)
    if index != EXPECTED:
        fail("the index is not the expected one:\n%s" % json.dumps(index, indent=2))
    tree = read_tree(os.path.join(work, "tree"))
    named = sorted({p for entry in index["components"] + index["crates"] + [index["rust_std"]]
                    for p in entry["license_files"]})
    if sorted(tree) != named:
        fail("the tree is not exactly the files the index names: %s" % sorted(tree))
    expected_bytes = {"components/two-2.0/docs/FTL.TXT": "FTL\n",
                      "components/one-1.0/COPYING": "MIT\n",
                      "components/three-3.0/LICENSE": "notice\n",
                      "standard/MPL-2.0.txt": "standard MPL\n"}
    for path, text in expected_bytes.items():
        if tree[path] != text:
            fail("%s does not hold its source's text" % path)
    print("  ok: components from their tarballs, ./ entries, links, a whole LICENSES"
          " directory, named files, build-only, crates, standard texts and std indexed")


def case_no_rust(script, root):
    work = os.path.join(root, "no-rust")
    bin_dir = make_fixture(work, crates=[], text_source=False)
    os.remove(os.path.join(work, "tree", "crates", "zeta-1.0.0", "LICENSE"))
    os.remove(os.path.join(work, "tree", "crates", "member-0.1.0", "COPYING"))
    result = run(script, work, bin_dir)
    if result.returncode != 0:
        fail("write_licenses.py refused a runtime with no Rust: " + result.stderr)
    with open(os.path.join(work, "runtime-licenses.json"), encoding="utf-8") as handle:
        index = json.load(handle)
    if index["crates"] != [] or index["rust_std"] is not None:
        fail("a runtime with no Rust component lists crates or std")
    print("  ok: no Rust component, no crates, no std and no text source needed")


def refused(script, root, label, reason, stale=None, **fixture):
    work = os.path.join(root, label)
    bin_dir = make_fixture(work, **fixture)
    if stale:
        write(os.path.join(work, "tree", stale), "left by another run\n")
    result = run(script, work, bin_dir)
    if result.returncode == 0 or reason not in result.stderr:
        fail("%s: not refused with '%s': %s" % (label, reason, result.stderr))


def with_extra(files):
    return [COMPONENTS[0], dict(COMPONENTS[1], extra_license_files=files), COMPONENTS[2]]


def case_component_refusals(script, root):
    bare = dict(MEMBERS, three={"three-3.0/README": "no licence here\n"})
    refused(script, root, "bare-component", "three 3.0 has no licence file", members=bare,
            specials={"one": SPECIALS["one"]})
    refused(script, root, "missing-named", "docs/NONE.TXT",
            components=with_extra(["docs/NONE.TXT"]))
    refused(script, root, "escaping-named", "../etc/x", components=with_extra(["../etc/x"]))
    refused(script, root, "linked-elsewhere", "docs/LINK.TXT links to docs/GPLv2.TXT",
            components=with_extra(["docs/LINK.TXT"]), specials=dict(SPECIALS, two=[
                ("./two-2.0/docs/LINK.TXT", tarfile.SYMTYPE, "GPLv2.TXT")]))
    refused(script, root, "linked-out", "LICENSE links to ../outside", specials=dict(
        SPECIALS, three=[("three-3.0/LICENSE", tarfile.SYMTYPE, "../outside")]))
    refused(script, root, "fifo", "COPYING.fifo in its tarball is not a regular file",
            specials=dict(SPECIALS, one=SPECIALS["one"] + [
                ("one-1.0/COPYING.fifo", tarfile.FIFOTYPE, "")]))
    print("  refused: a component with no licence file, a named file missing or outside"
          " the tree, a link to a file not taken or outside the tree, a special file")


def case_crate_refusals(script, root):
    refused(script, root, "bare-crate", "zeta 1.0.0 has no licence file",
            crates=[dict(CRATES[0], license_files=[])] + CRATES[1:])
    refused(script, root, "absent-file", "rust-std/licenses/GPL.txt",
            crates=CRATES[:3] + [dict(CRATES[3], license_files=["rust-std/licenses/GPL.txt"])])
    refused(script, root, "stale-file", "crates/old-0.1/LICENSE", stale="crates/old-0.1/LICENSE")
    refused(script, root, "unknown-standard", "WTFPL",
            crates=[CRATES[0], CRATES[1], dict(CRATES[2], license_files=["standard/WTFPL.txt"]),
                    CRATES[3]])
    refused(script, root, "no-text-source", "no license_text_source", text_source=False)
    print("  refused: a crate with no licence file, a listed file not in the tree, a file no"
          " entry names, a standard text the source lacks or no source for one")


def main(argv):
    if len(argv) != 2:
        fail("usage: build_runtime_test_licenses.py <write_licenses.py> <work dir>")
    script, root = os.path.abspath(argv[0]), os.path.abspath(argv[1])
    os.makedirs(root, exist_ok=True)
    case_index_and_tree(script, root)
    case_no_rust(script, root)
    case_component_refusals(script, root)
    case_crate_refusals(script, root)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
