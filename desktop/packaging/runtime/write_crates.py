#!/usr/bin/env python3
"""List the Rust crates one component compiled in, with their licence files.

Run after a Rust component is built, while its source tree exists. Asks cargo
which crates each package the lock file names compiles in, for the host target,
without build scripts and procedural macros, which run at build time and are
not linked. Then:

- copies each vendored crate's source into the tree runtime-crates.tar.gz is
  made from, for the source archive;
- copies each crate's licence files into the tree runtime-licenses.tar.gz is
  made from, under crates/<name>-<version>/;
- writes a fragment write_licenses.py folds into runtime-licenses.json.

Each crate's licence has to be an SPDX expression whose ids and exceptions are
on SPDX's lists, which the lock pins, and none deprecated there; Cargo's old
"/" between licences is read as OR.

A crate whose package carries no licence file needs an entry in the lock's
cargo.standard_license_texts naming the SPDX ids of its declared licence whose
standard texts stand for it; without one the crate is refused. write_licenses.py
copies those texts to standard/<id>.txt. README.md says why.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys

LICENCE_FILE = re.compile(
    r"(?i)^(licen[cs]e|copying|copyright|notice|unlicen[cs]e)([-_.].*)?$")
TREE_LINE = re.compile(r"^(\S+) v(\S+)(?: \((.+)\))?$")
SPDX_ID = re.compile(r"[A-Za-z0-9.+-]+")
# A licence or exception identifier as an SPDX expression writes one. Whether
# the identifier is on SPDX's list is not checked here.
SPDX_LICENCE = re.compile(r"LicenseRef-[A-Za-z0-9.-]+|[A-Za-z0-9][A-Za-z0-9.-]*\+?")
SPDX_OPERATORS = {"AND", "OR", "WITH"}
MAX_PARENTHESES = 16


def fail(message):
    raise SystemExit("write_crates: " + message)


def command(args, cwd=None):
    result = subprocess.run(args, cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        fail("%s failed: %s" % (" ".join(args), result.stderr.strip()))
    return result.stdout


def host_triple():
    for line in command(["rustc", "-vV"]).splitlines():
        if line.startswith("host: "):
            return line[len("host: "):]
    fail("rustc -vV names no host")


def target_commands(targets):
    """Every command line in meson's introspection of its targets."""
    for target in targets:
        yield from (source.get("compiler") or [] for source in target.get("target_sources", []))


def meson_packages(build_dir):
    """The packages meson's cargo targets build, from its introspection."""
    path = os.path.join(build_dir, "meson-info", "intro-targets.json")
    with open(path, encoding="utf-8") as handle:
        targets = json.load(handle)
    packages = set()
    for args in target_commands(targets):
        if any("cargo" in os.path.basename(arg) for arg in args):
            packages.update(value for flag, value in zip(args, args[1:]) if flag == "--packages")
    return packages


def spdx_term(tokens, at, depth):
    """Parses one licence, licence WITH exception, or parenthesised expression."""
    if depth > MAX_PARENTHESES or at >= len(tokens):
        raise ValueError("an expression ends early or nests too deep")
    if tokens[at] == "(":
        at = spdx_or(tokens, at + 1, depth + 1)
        if at >= len(tokens) or tokens[at] != ")":
            raise ValueError("an unclosed parenthesis")
        return at + 1
    if tokens[at] in SPDX_OPERATORS or not SPDX_LICENCE.fullmatch(tokens[at]):
        raise ValueError("%r where a licence belongs" % tokens[at])
    if at + 1 < len(tokens) and tokens[at + 1] == "WITH":
        if at + 2 >= len(tokens) or not SPDX_LICENCE.fullmatch(tokens[at + 2]):
            raise ValueError("WITH and no exception")
        return at + 3
    return at + 1


def spdx_and(tokens, at, depth):
    at = spdx_term(tokens, at, depth)
    while at < len(tokens) and tokens[at] == "AND":
        at = spdx_term(tokens, at + 1, depth)
    return at


def spdx_or(tokens, at, depth):
    at = spdx_and(tokens, at, depth)
    while at < len(tokens) and tokens[at] == "OR":
        at = spdx_and(tokens, at + 1, depth)
    return at


def spdx_licence(declared):
    """The SPDX expression a crate declares, with Cargo's old "/" between
    licences read as OR, as Cargo defines it; None when it is not one."""
    expression = declared
    if "/" in declared:
        parts = [part.strip() for part in declared.split("/")]
        if not all(SPDX_LICENCE.fullmatch(part) for part in parts):
            return None
        expression = " OR ".join(parts)
    tokens = expression.replace("(", " ( ").replace(")", " ) ").split()
    try:
        end = spdx_or(tokens, 0, 0)
    except ValueError:
        return None
    return expression if end == len(tokens) else None


def spdx_list(licences_path, exceptions_path):
    """SPDX's licence and exception lists: each id, and whether it is deprecated."""
    with open(licences_path, encoding="utf-8") as handle:
        licences = json.load(handle)["licenses"]
    with open(exceptions_path, encoding="utf-8") as handle:
        exceptions = json.load(handle)["exceptions"]
    return ({entry["licenseId"]: entry["isDeprecatedLicenseId"] is True for entry in licences},
            {entry["licenseExceptionId"]: entry["isDeprecatedLicenseId"] is True
             for entry in exceptions})


def spdx_terms(expression):
    """Each licence and exception an SPDX expression names, and whether it is an
    exception, the term after WITH."""
    tokens = expression.replace("(", " ").replace(")", " ").split()
    return [(token, at > 0 and tokens[at - 1] == "WITH") for at, token in enumerate(tokens)
            if token not in SPDX_OPERATORS]


def unlisted(expression, listed):
    """What SPDX's lists do not take in an expression: an id they lack, or one
    they deprecate. A LicenseRef is the expression's own, and no list names it."""
    licences, exceptions = listed
    problems = []
    for token, is_exception in spdx_terms(expression):
        if token.startswith("LicenseRef-") and not is_exception:
            continue
        table, kind = (exceptions, "exception") if is_exception else (licences, "licence")
        spdx_id = token[:-1] if token.endswith("+") and not is_exception else token
        if spdx_id not in table:
            problems.append("%s is not on SPDX's %s list" % (token, kind))
        elif table[spdx_id]:
            problems.append("%s is deprecated" % token)
    return problems


def checksums(cargo_lock_text):
    found = {}
    for block in cargo_lock_text.split("[[package]]")[1:]:
        fields = dict(re.findall(r'^(\w+) = "([^"]*)"$', block, re.M))
        found[(fields.get("name"), fields.get("version"))] = fields.get("checksum")
    return found


def tree_crates(src, manifest, package, features, triple, listed):
    args = ["cargo", "tree", "--offline", "--locked", "--manifest-path", manifest,
            "-p", package, "-e", "normal,no-proc-macro", "--target", triple,
            "--prefix", "none", "--no-dedupe", "-f", "{p}|{l}"]
    if features:
        args += ["--features", ",".join(features)]
    crates = {}
    for line in command(args, cwd=src).splitlines():
        spec, _, licence = line.partition("|")
        match = TREE_LINE.match(spec.strip())
        if not match:
            fail("cargo tree printed a line it should not have: %r" % line)
        name, version, path = match.groups()
        if not licence.strip():
            fail("%s %s declares no licence" % (name, version))
        expression = spdx_licence(licence.strip())
        if expression is None:
            fail("%s %s declares %r, which is not an SPDX expression"
                 % (name, version, licence.strip()))
        problems = unlisted(expression, listed)
        if problems:
            fail("%s %s declares %r: %s" % (name, version, licence.strip(), "; ".join(problems)))
        crates[(name, version)] = (expression, path)
    return crates


def licence_files(directory):
    return sorted(name for name in os.listdir(directory)
                  if LICENCE_FILE.match(name)
                  and os.path.isfile(os.path.join(directory, name)))


def copy_licences(directory, names, licenses_tree, crate_dir):
    """Copies the named files of directory to crates/<crate_dir>/ of the licence tree."""
    destination = os.path.join(licenses_tree, "crates", crate_dir)
    if os.path.exists(destination):
        shutil.rmtree(destination)
    os.makedirs(destination)
    for name in names:
        shutil.copy2(os.path.join(directory, name), os.path.join(destination, name))
    return ["crates/%s/%s" % (crate_dir, name) for name in names]


def own_licences(src, name, version, path):
    """The directory holding a crate's licence files, and their names."""
    if path is None:
        vendored = os.path.join(src, "_crates", "%s-%s" % (name, version))
        if not os.path.isdir(vendored):
            fail("%s %s is compiled in and was not vendored at %s" % (name, version, vendored))
        return vendored, licence_files(vendored)
    if os.path.relpath(path, src).startswith(".."):
        fail("%s %s is a path crate outside the source tree: %s" % (name, version, path))
    # A workspace member without its own licence file is under the tree's.
    if licence_files(path):
        return path, licence_files(path)
    return src, licence_files(src)


def standard_texts(key, licence, ids):
    """The standard texts of the licences a crate declares, as the lock names them."""
    declared = set(SPDX_ID.findall(licence)) - {"OR", "AND", "WITH"}
    for spdx_id in ids:
        if spdx_id not in declared:
            fail("%s declares %s, which does not name %s" % (key, licence, spdx_id))
    return sorted("standard/%s.txt" % spdx_id for spdx_id in ids)


def licence_files_for(src, licenses_tree, crate, standard):
    """A crate's licence files in the licence tree, or the standard texts for them."""
    name, version, licence, path = crate
    key = "%s %s" % (name, version)
    directory, names = own_licences(src, name, version, path)
    if names and key in standard:
        fail("%s ships its own licence files; drop it from cargo.standard_license_texts" % key)
    if names:
        return copy_licences(directory, names, licenses_tree, "%s-%s" % (name, version))
    if key not in standard:
        fail("%s ships no licence file, and cargo.standard_license_texts names no text for it"
             % key)
    return standard_texts(key, licence, standard[key])


def copy_sources(src, tree, crates):
    """The vendored crates compiled in, as the source archive carries them."""
    top = os.path.basename(src)
    if os.path.exists(os.path.join(tree, top)):
        shutil.rmtree(os.path.join(tree, top))
    for name, version, _, path in crates:
        if path is None:
            relative = os.path.join("_crates", "%s-%s" % (name, version))
            shutil.copytree(os.path.join(src, relative), os.path.join(tree, top, relative),
                            symlinks=True)


def run_component(args):
    src = os.path.abspath(args.source_dir)
    cargo = json.loads(args.cargo)
    lock_path = os.path.join(src, cargo["lock"])
    manifest = os.path.join(os.path.dirname(lock_path), "Cargo.toml")
    built = meson_packages(args.build_dir)
    declared = set(cargo["packages"])
    if built != declared:
        fail("meson builds the Rust packages %s; the lock file names %s"
             % (sorted(built), sorted(declared)))
    with open(lock_path, encoding="utf-8") as handle:
        sums = checksums(handle.read())
    triple = host_triple()
    listed = spdx_list(args.spdx_licences, args.spdx_exceptions)
    found = {}
    for package, features in sorted(cargo["packages"].items()):
        found.update(tree_crates(src, manifest, package, features, triple, listed))
    crates = [(n, v, licence, path) for (n, v), (licence, path) in sorted(found.items())]
    standard = cargo.get("standard_license_texts", {})
    stale = set(standard) - {"%s %s" % (n, v) for n, v, _, _ in crates}
    if stale:
        fail("cargo.standard_license_texts names %s, which %s does not compile in"
             % (", ".join(sorted(stale)), args.name))
    entries = []
    for name, version, licence, path in crates:
        files = licence_files_for(src, args.licenses_tree, (name, version, licence, path),
                                  standard)
        entries.append({"component": args.name, "name": name, "version": version,
                        "license": licence, "source": "crates.io" if path is None else "path",
                        "checksum": sums.get((name, version)) if path is None else None,
                        "license_files": files})
    copy_sources(src, args.tree, crates)
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(entries, handle, indent=2)
        handle.write("\n")
    print("write_crates: %s compiles in %d crates; standard texts stand for the licence"
          " files of %s" % (args.name, len(entries), ", ".join(sorted(standard)) or "none"),
          file=sys.stderr)
    return 0


def main(argv):
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    one = commands.add_parser("component")
    one.add_argument("--name", required=True)
    one.add_argument("--source-dir", required=True)
    one.add_argument("--cargo", required=True,
                     help="the lock entry's cargo object, as JSON")
    one.add_argument("--build-dir", required=True)
    one.add_argument("--tree", required=True, help="the crate source tree")
    one.add_argument("--licenses-tree", required=True, help="the licence file tree")
    one.add_argument("--spdx-licences", required=True, help="SPDX's json/licenses.json")
    one.add_argument("--spdx-exceptions", required=True, help="SPDX's json/exceptions.json")
    one.add_argument("--out", required=True, help="the fragment to write")
    args = parser.parse_args(argv)
    return run_component(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
