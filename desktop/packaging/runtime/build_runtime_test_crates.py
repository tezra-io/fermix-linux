#!/usr/bin/env python3
"""write_crates.py's own tests, run by build_runtime_test.sh.

A fixture Rust component stands in for librsvg: a Cargo.lock, two vendored
crates (one with licence files, one without, for which the lock names the
standard text of its licence), a workspace member whose licence is at the
source root, and the meson introspection that names the package meson builds.
cargo and rustc are stubs on PATH that answer only what write_crates.py asks,
so the test needs neither and touches no network. A few entries stand for
SPDX's licence and exception lists, in their format.

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

# What `cargo tree -f '{p}|{l}' --prefix none` prints for the fixture. alpha
# declares its licence the way futf 0.1.5 does, in Cargo's old "/" form.
TREE = """member v0.1.0 (%s/member)|LGPL-2.1-or-later
alpha v1.0.0|MIT / Apache-2.0
beta v2.0.0|MPL-2.0
alpha v1.0.0|MIT / Apache-2.0
"""

# The licences the runtime's crates declare in the old form, and what each
# becomes; then expressions that are not SPDX at all.
OLD_FORMS = {
    "MIT / Apache-2.0": "MIT OR Apache-2.0",       # futf
    "Apache-2.0/MIT": "Apache-2.0 OR MIT",         # fxhash
    "MIT/Apache-2.0": "MIT OR Apache-2.0",         # language-tags, mac, tendril, ...
}
VALID = ["MIT", "(Apache-2.0 OR MIT) AND BSD-3-Clause", "0BSD OR MIT OR Apache-2.0",
         "Apache-2.0 WITH LLVM-exception", "LicenseRef-gbsearcharray AND MIT", "GPL-2.0+"]
NOT_SPDX = ["", "MIT or Apache-2.0", "MIT AND", "(MIT", "MIT)", "MIT OR OR Apache-2.0",
            "MIT/Apache-2.0 AND Zlib", "MIT/", "Apache 2.0", "WITH MIT", "MIT WITH",
            "(" * 40 + "MIT" + ")" * 40]

# SPDX's lists, cut to what the cases name, with one deprecated id of each kind.
SPDX_LICENCES = {"licenseListVersion": "1.0", "licenses": [
    {"licenseId": spdx_id, "isDeprecatedLicenseId": spdx_id == "GPL-2.0"}
    for spdx_id in ("MIT", "Apache-2.0", "MPL-2.0", "LGPL-2.1-or-later", "BSD-3-Clause",
                    "GPL-2.0", "GPL-2.0-only")]}
SPDX_EXCEPTIONS = {"licenseListVersion": "1.0", "exceptions": [
    {"licenseExceptionId": "LLVM-exception", "isDeprecatedLicenseId": False},
    {"licenseExceptionId": "Nokia-Qt-exception-1.1", "isDeprecatedLicenseId": True}]}
# Expressions the lists take, and what they do not, with the reason.
LISTED = ["MIT OR Apache-2.0", "(MIT OR Apache-2.0) AND BSD-3-Clause",
          "Apache-2.0 WITH LLVM-exception", "LicenseRef-gbsearcharray AND MIT",
          "GPL-2.0-only+"]
UNLISTED = {
    "Foo-1.0 OR MIT": ["Foo-1.0 is not on SPDX's licence list"],
    "GPL-2.0": ["GPL-2.0 is deprecated"],
    "GPL-2.0+": ["GPL-2.0+ is deprecated"],
    "MIT WITH Bar-exception": ["Bar-exception is not on SPDX's exception list"],
    "LGPL-2.1-or-later WITH Nokia-Qt-exception-1.1": ["Nokia-Qt-exception-1.1 is deprecated"],
    "MIT WITH LicenseRef-x": ["LicenseRef-x is not on SPDX's exception list"],
    "LLVM-exception": ["LLVM-exception is not on SPDX's licence list"],
}

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
    write(os.path.join(work, "licenses.json"), json.dumps(SPDX_LICENCES))
    write(os.path.join(work, "exceptions.json"), json.dumps(SPDX_EXCEPTIONS))
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
               "--spdx-licences", os.path.join(work, "licenses.json"),
               "--spdx-exceptions", os.path.join(work, "exceptions.json"),
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
    refused(script, root, "notspdx", "alpha 1.0.0 declares 'MIT or Apache-2.0', which is not",
            tree="alpha v1.0.0|MIT or Apache-2.0\n")
    refused(script, root, "unlisted",
            "alpha 1.0.0 declares 'Foo-1.0/MIT': Foo-1.0 is not on SPDX's licence list",
            tree="alpha v1.0.0|Foo-1.0/MIT\n")
    refused(script, root, "deprecated", "alpha 1.0.0 declares 'GPL-2.0': GPL-2.0 is deprecated",
            tree="alpha v1.0.0|GPL-2.0\n")
    refused(script, root, "bare", "beta 2.0.0 ships no licence file", standard={})
    refused(script, root, "stale", "gamma 9.9.9",
            standard=dict(STANDARD, **{"gamma 9.9.9": ["MIT"]}))
    refused(script, root, "own", "alpha 1.0.0 ships its own",
            standard=dict(STANDARD, **{"alpha 1.0.0": ["MIT"]}))
    refused(script, root, "undeclared", "beta 2.0.0 declares MPL-2.0, which does not name MIT",
            standard={"beta 2.0.0": ["MIT"]})
    print("  refused: a package meson builds and the lock misses, an unvendored crate,"
          " a crate with no licence, a licence off SPDX's list or deprecated there,"
          " no licence file and no standard text named, a standard"
          " text for a crate not compiled in, for one that ships its own, or of a licence"
          " it does not declare")


def case_expressions(script):
    """The normalisation and the check, called directly."""
    sys.path.insert(0, os.path.dirname(script))
    import write_crates  # noqa: E402 - the module under test, found beside the script
    for declared, expected in OLD_FORMS.items():
        if write_crates.spdx_licence(declared) != expected:
            fail("%r did not become %r" % (declared, expected))
    for expression in VALID:
        if write_crates.spdx_licence(expression) != expression:
            fail("the SPDX expression %r was not kept as it is" % expression)
    for expression in NOT_SPDX:
        if write_crates.spdx_licence(expression) is not None:
            fail("%r was taken for an SPDX expression" % expression)
    print("  ok: Cargo's old \"/\" read as OR, SPDX kept as it is, anything else refused")


def case_lists(script, root):
    """Ids held to SPDX's lists, read from files in their format."""
    sys.path.insert(0, os.path.dirname(script))
    import write_crates  # noqa: E402 - the module under test, found beside the script
    work = os.path.join(root, "lists")
    make_fixture(work, ["member"])
    listed = write_crates.spdx_list(os.path.join(work, "licenses.json"),
                                    os.path.join(work, "exceptions.json"))
    for expression in LISTED:
        if write_crates.unlisted(expression, listed):
            fail("%r was refused: %s" % (expression, write_crates.unlisted(expression, listed)))
    for expression, problems in UNLISTED.items():
        if write_crates.unlisted(expression, listed) != problems:
            fail("%r gave %s, not %s"
                 % (expression, write_crates.unlisted(expression, listed), problems))
    print("  ok: ids and exceptions on SPDX's lists pass; one missing or deprecated is named")


def main(argv):
    if len(argv) != 2:
        fail("usage: build_runtime_test_crates.py <write_crates.py> <work dir>")
    script, root = os.path.abspath(argv[0]), os.path.abspath(argv[1])
    os.makedirs(root, exist_ok=True)
    case_component(script, root)
    case_refusals(script, root)
    case_expressions(script)
    case_lists(script, root)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
