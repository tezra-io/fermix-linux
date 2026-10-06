#!/usr/bin/env python3
"""write_crates.py's own tests, run by build_runtime_test.sh.

A fixture Rust component stands in for librsvg: a Cargo.lock, two vendored
crates (one with licence files, one without, for which the lock names the
standard text of its licence), a workspace member whose licence is at the
source root, and the meson introspection that names the package meson builds.
cargo and rustc are stubs on PATH that answer only what write_crates.py asks,
so the test needs neither and touches no network.

Usage: build_runtime_test_crates.py <write_crates.py> <work dir>
"""

import json
import os
import shutil
import subprocess
import sys

CARGO_LOCK = """version = 4

[[package]]
name = "alpha"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "%s"

[[package]]
name = "beta"
version = "2.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "%s"

[[package]]
name = "member"
version = "0.1.0"
""" % ("a" * 64, "b" * 64)

# What `cargo tree -f '{p}|{l}' --prefix none` prints for the fixture.
TREE = """member v0.1.0 (%s/member)|LGPL-2.1-or-later
alpha v1.0.0|MIT OR Apache-2.0
beta v2.0.0|MPL-2.0
alpha v1.0.0|MIT OR Apache-2.0
"""

CARGO_STUB = """#!/bin/sh
# Answers `cargo tree` for the fixture, and records how it was asked.
echo "$*" >> "%s/cargo-calls"
[ "$1" = "tree" ] || exit 2
cat "%s/tree.txt"
"""

RUSTC_STUB = """#!/bin/sh
case "$*" in
  "-vV") printf 'rustc 9.9.9\\nhost: x86_64-unknown-linux-gnu\\nrelease: 9.9.9\\n' ;;
  *) exit 2 ;;
esac
"""
# beta's package carries no licence file; the standard MPL-2.0 text stands for it.
STANDARD = {"beta 2.0.0": ["MPL-2.0"]}


def fail(message):
    raise SystemExit("crates_test: " + message)


def write(path, text, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(text)
    os.chmod(path, mode)


def make_fixture(work, meson_packages):
    src = os.path.join(work, "src", "fixture-1.0")
    write(os.path.join(src, "Cargo.toml"), "[workspace]\n")
    write(os.path.join(src, "Cargo.lock"), CARGO_LOCK)
    write(os.path.join(src, "COPYING.LIB"), "the member's licence\n")
    write(os.path.join(src, "member", "Cargo.toml"), "[package]\n")
    write(os.path.join(src, "_crates", "alpha-1.0.0", "LICENSE-MIT"), "MIT\n")
    write(os.path.join(src, "_crates", "alpha-1.0.0", "LICENSE-APACHE"), "Apache\n")
    write(os.path.join(src, "_crates", "alpha-1.0.0", "src", "lib.rs"), "\n")
    write(os.path.join(src, "_crates", "beta-2.0.0", "src", "lib.rs"), "\n")
    write(os.path.join(src, "_crates", "unused-3.0.0", "LICENSE"), "not compiled in\n")
    targets = [{"name": p, "type": "custom", "target_sources": [{"compiler": [
        "/usr/bin/python3", src + "/meson/cargo_wrapper.py", "--command=build",
        "--packages", p, "--extension", "so"]}]} for p in meson_packages]
    intro = os.path.join(src, "_build", "meson-info", "intro-targets.json")
    write(intro, json.dumps(targets))
    bin_dir = os.path.join(work, "bin")
    write(os.path.join(work, "tree.txt"), TREE % src)
    write(os.path.join(bin_dir, "cargo"), CARGO_STUB % (work, work), 0o755)
    write(os.path.join(bin_dir, "rustc"), RUSTC_STUB, 0o755)
    return src, bin_dir


def run(script, work, bin_dir, *args):
    env = dict(os.environ, PATH=bin_dir + os.pathsep + os.environ["PATH"])
    return subprocess.run([sys.executable, script] + list(args), cwd=work, env=env,
                          capture_output=True, text=True, check=False)


def component(script, work, src, bin_dir, packages, standard=None):
    cargo = {"lock": "Cargo.lock", "packages": packages,
             "standard_license_texts": STANDARD if standard is None else standard}
    return run(script, work, bin_dir, "component", "--name", "fixture",
               "--source-dir", src, "--cargo", json.dumps(cargo),
               "--build-dir", os.path.join(src, "_build"),
               "--tree", os.path.join(work, "tree"),
               "--licenses-tree", os.path.join(work, "licenses"),
               "--out", os.path.join(work, "fragments", "fixture.json"))


EXPECTED = [
    {"component": "fixture", "name": "alpha", "version": "1.0.0",
     "license": "MIT OR Apache-2.0", "source": "crates.io", "checksum": "a" * 64,
     "license_files": ["crates/alpha-1.0.0/LICENSE-APACHE", "crates/alpha-1.0.0/LICENSE-MIT"]},
    {"component": "fixture", "name": "beta", "version": "2.0.0", "license": "MPL-2.0",
     "source": "crates.io", "checksum": "b" * 64, "license_files": ["standard/MPL-2.0.txt"]},
    {"component": "fixture", "name": "member", "version": "0.1.0",
     "license": "LGPL-2.1-or-later", "source": "path", "checksum": None,
     "license_files": ["crates/member-0.1.0/COPYING.LIB"]},
]


def files_under(root):
    return sorted(os.path.relpath(os.path.join(d, f), root)
                  for d, _, names in os.walk(root) for f in names)


def case_component(script, root):
    work = os.path.join(root, "good")
    src, bin_dir = make_fixture(work, ["member"])
    result = component(script, work, src, bin_dir, {"member": ["feature-a"]})
    if result.returncode != 0:
        fail("component refused the fixture: " + result.stderr)
    calls = open(os.path.join(work, "cargo-calls"), encoding="utf-8").read()
    for expected in ("-p member", "--features feature-a", "-e normal,no-proc-macro",
                     "--target x86_64-unknown-linux-gnu", "--offline", "--locked"):
        if expected not in calls:
            fail("cargo tree was not asked with %s: %s" % (expected, calls))
    with open(os.path.join(work, "fragments", "fixture.json"), encoding="utf-8") as handle:
        fragment = json.load(handle)
    if fragment != EXPECTED:
        fail("the fragment is not the crates compiled in:\n%s" % json.dumps(fragment, indent=2))
    licenses = os.path.join(work, "licenses")
    if files_under(licenses) != ["crates/alpha-1.0.0/LICENSE-APACHE",
                                 "crates/alpha-1.0.0/LICENSE-MIT",
                                 "crates/member-0.1.0/COPYING.LIB"]:
        fail("the licence tree is not the crates' own licence files: %s"
             % files_under(licenses))
    with open(os.path.join(licenses, "crates/member-0.1.0/COPYING.LIB"),
              encoding="utf-8") as handle:
        if handle.read() != "the member's licence\n":
            fail("the member's licence is not the source root's")
    tops = sorted(os.listdir(os.path.join(work, "tree")))
    vendored = sorted(os.listdir(os.path.join(work, "tree", "fixture-1.0", "_crates")))
    if tops != ["fixture-1.0"] or vendored != ["alpha-1.0.0", "beta-2.0.0"]:
        fail("the crate tree is not the compiled-in crates' sources: %s %s" % (tops, vendored))
    if not os.path.isfile(os.path.join(work, "tree", "fixture-1.0/_crates/beta-2.0.0/src/lib.rs")):
        fail("a compiled-in crate's source is not in the crate tree")
    print("  ok: the compiled-in crates, their sources, and their licence files or the"
          " standard text the lock names")


def refused(script, root, label, reason, meson=("member",), packages=None, standard=None,
            tree=None, vendored_gone=None):
    work = os.path.join(root, label)
    src, bin_dir = make_fixture(work, list(meson))
    if vendored_gone:
        shutil.rmtree(os.path.join(src, "_crates", vendored_gone))
    if tree:
        write(os.path.join(work, "tree.txt"), tree)
    result = component(script, work, src, bin_dir, packages or {"member": []}, standard)
    if result.returncode == 0 or reason not in result.stderr:
        fail("%s: not refused with '%s': %s" % (label, reason, result.stderr))


def case_refusals(script, root):
    refused(script, root, "mismatch", "another", meson=("member", "another"))
    refused(script, root, "missing", "beta 2.0.0 is compiled in and was not vendored",
            vendored_gone="beta-2.0.0")
    refused(script, root, "nolicence", "alpha 1.0.0 declares no licence", tree="alpha v1.0.0|\n")
    refused(script, root, "bare", "beta 2.0.0 ships no licence file", standard={})
    refused(script, root, "stale", "gamma 9.9.9",
            standard=dict(STANDARD, **{"gamma 9.9.9": ["MIT"]}))
    refused(script, root, "own", "alpha 1.0.0 ships its own",
            standard=dict(STANDARD, **{"alpha 1.0.0": ["MIT"]}))
    refused(script, root, "undeclared", "beta 2.0.0 declares MPL-2.0, which does not name MIT",
            standard={"beta 2.0.0": ["MIT"]})
    print("  refused: a package meson builds and the lock misses, an unvendored crate,"
          " a crate with no licence, no licence file and no standard text named, a standard"
          " text for a crate not compiled in, for one that ships its own, or of a licence"
          " it does not declare")


def main(argv):
    if len(argv) != 2:
        fail("usage: build_runtime_test_crates.py <write_crates.py> <work dir>")
    script, root = os.path.abspath(argv[0]), os.path.abspath(argv[1])
    os.makedirs(root, exist_ok=True)
    case_component(script, root)
    case_refusals(script, root)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
