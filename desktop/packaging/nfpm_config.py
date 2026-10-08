#!/usr/bin/env python3
"""Render fermix-desktop's nFPM configuration from the template and the staged tree.

    nfpm_config.py --template <tmpl> --map <host_relations.map> --lock <RUNTIME.lock.json>
                   --engine-relations <stage>/relations.json --root <staged tree>
                   --version <version> --arch <amd64|arm64> --mtime <YYYY-MM-DDTHH:MM:SSZ>
                   --postinstall <file> --postremove <file> --out <file>

The relations: every host library of the lock through its row in the map, every @ row of the map,
then the engine package's own declared relations, each once, in that order. The map and the lock
must name the same libraries: a host library with no row is a dependency nobody has named for
users' package managers, and a row the lock does not list is a dependency nothing needs.

The contents: one entry for every file and symbolic link under <root>, with its mode, and one for
every directory under the two the package owns. A link is given 0777, which the deb keeps; nFPM
writes every rpm link with mode 0 whatever it is given. The package's own documents, under
/usr/share/doc/fermix-desktop, and its manual page are listed twice: a plain file for the deb,
where nFPM would drop a doc or license entry, and %doc for the rpm. The copyright file is the
rpm's %license instead, at /usr/share/licenses/fermix-desktop/copyright, where Fedora keeps
licences, in a directory the rpm alone lists. The engine's documents stay plain files, as the
fermix rpm lists them. The lintian overrides
are listed for the deb alone. The tree has to hold the engine and the window, and nothing a
package cannot carry: no special file, no absolute symbolic link, nothing writable by group or
others, no setuid or setgid bit.
"""

import argparse
import json
import os
import re
import stat
import sys

ARCHES = ("amd64", "arm64")
PLACEHOLDER = re.compile(r"{{([A-Z_]+)}}")
PLACEHOLDERS = {
    "VERSION",
    "ARCH",
    "MTIME",
    "DEB_DEPENDS",
    "RPM_DEPENDS",
    "POSTINSTALL",
    "POSTREMOVE",
    "CONTENTS",
}
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(\+[0-9A-Za-z.]+)?")
MTIME = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z")
MAP_FIELDS = 3
# The directories the package owns, so the only ones it lists. Every other directory it installs
# into belongs to the host's filesystem package or to the engine's layout, which lists none.
OWNED = ("usr/lib/fermix-desktop", "usr/share/doc/fermix-desktop")
OWN_DOCS = ("usr/share/doc/fermix-desktop/", "usr/share/man/man1/fermix-desktop.1.gz")
DEB_ONLY = ("usr/share/lintian/overrides/fermix-desktop",)
RPM_LICENSES = "usr/share/licenses/fermix-desktop"
LINK_MODE = 0o777
REQUIRED = ("usr/bin/fermix", "usr/lib/fermix-desktop/bin/fermix-desktop")
UNSAFE_BITS = stat.S_ISUID | stat.S_ISGID | stat.S_ISVTX | stat.S_IWGRP | stat.S_IWOTH
USAGE = (
    "nfpm_config.py --template <tmpl> --map <map> --lock <lock> --engine-relations <json> "
    "--root <dir> --version <v> --arch <arch> --mtime <time> --postinstall <file> "
    "--postremove <file> --out <file>"
)


class Refusal(Exception):
    """A refusal; its message is the sentence printed after "nfpm_config: "."""


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


def read_json(path):
    try:
        return json.loads(read_text(path))
    except ValueError as error:
        raise Refusal(f"{path} is not valid JSON: {error}") from error


def check_version(version):
    for mark in ("-", ":"):
        if mark in version:
            raise Refusal(
                f"the version {version} has a '{mark}': no Debian revision and no rpm epoch"
            )
    if not VERSION.fullmatch(version):
        raise Refusal(f"{version} is not a package version: X.Y.Z, or X.Y.Z+<suffix>")


def check_arguments(arguments):
    check_version(arguments.version)
    if arguments.arch not in ARCHES:
        raise Refusal(f"fermix-desktop is built for amd64 and arm64, not {arguments.arch}")
    if not MTIME.fullmatch(arguments.mtime):
        raise Refusal(f"{arguments.mtime} is not a UTC time as YYYY-MM-DDTHH:MM:SSZ")


def read_map(path, floor):
    """The map's rows in order: (soname or @what, deb relation, rpm requirement)."""
    rows = []
    for number, line in enumerate(read_text(path).splitlines(), start=1):
        if not line.strip() or line.startswith("#"):
            continue
        fields = [field.strip() for field in line.split(" :: ")]
        if len(fields) != MAP_FIELDS or not all(fields):
            raise Refusal(f"{path} line {number} is not <soname> :: <deb> :: <rpm>: {line}")
        rows.append(tuple(field.replace("{glibc_floor}", floor) for field in fields))
    return rows


def lock_facts(path):
    lock = read_json(path)
    floor = lock.get("glibc_floor")
    hosts = lock.get("host_libraries")
    if not isinstance(floor, str) or not isinstance(hosts, list) or not hosts:
        raise Refusal(f"{path} has no glibc_floor and host_libraries to declare")
    return floor, hosts


def once(items):
    return list(dict.fromkeys(items))


def host_relations(rows, hosts):
    named = {row[0] for row in rows if not row[0].startswith("@")}
    for soname in hosts:
        if soname not in named:
            raise Refusal(f"host_libraries names {soname}, which host_relations.map has no row for")
    unused = sorted(named - set(hosts))
    if unused:
        raise Refusal(
            f"host_relations.map has a row for {unused[0]}, which the lock's host_libraries "
            "does not name"
        )
    return once(row[1] for row in rows), once(row[2] for row in rows)


def engine_relations(path):
    relations = read_json(path)
    deb = relations.get("deb")
    rpm = relations.get("rpm")
    if not isinstance(deb, dict) or not isinstance(deb.get("depends"), list):
        raise Refusal(f"{path} has no deb relations")
    if not isinstance(rpm, dict) or not isinstance(rpm.get("requires"), list):
        raise Refusal(f"{path} has no rpm relations: the engine stage has no rpm to build one from")
    return deb["depends"], rpm["requires"]


def tree_paths(root):
    paths = []
    for directory, names, files in os.walk(root):
        names.sort()
        for name in names + sorted(files):
            paths.append(os.path.relpath(os.path.join(directory, name), root))
    return sorted(paths)


def owned(relative):
    return any(relative == top or relative.startswith(top + "/") for top in OWNED)


def entry(root, relative):
    path = os.path.join(root, relative)
    info = os.lstat(path)
    if stat.S_ISLNK(info.st_mode):
        target = os.readlink(path)
        if os.path.isabs(target):
            raise Refusal(f"/{relative} is an absolute symbolic link, to {target}")
        link_info = {"mode": LINK_MODE}
        return {"src": target, "dst": f"/{relative}", "type": "symlink", "file_info": link_info}
    if stat.S_ISDIR(info.st_mode) and not owned(relative):
        return None
    mode = stat.S_IMODE(info.st_mode)
    if mode & UNSAFE_BITS:
        raise Refusal(f"/{relative} has mode {mode:04o}: writable by others, or setuid or setgid")
    if stat.S_ISDIR(info.st_mode):
        return {"dst": f"/{relative}", "type": "dir", "file_info": {"mode": mode}}
    if not stat.S_ISREG(info.st_mode):
        raise Refusal(f"/{relative} is neither a file, a directory nor a symbolic link")
    return {"src": path, "dst": f"/{relative}", "type": "file", "file_info": {"mode": mode}}


def per_packager(item):
    """The package's own document as the deb's plain file and the rpm's %license or %doc."""
    relative = item["dst"][1:]
    if item["type"] != "file":
        return [item]
    if relative in DEB_ONLY:
        return [{**item, "packager": "deb"}]
    if not relative.startswith(OWN_DOCS):
        return [item]
    if os.path.basename(relative) == "copyright":
        licence = f"/{RPM_LICENSES}/copyright"
        return [{**item, "packager": "deb"},
                {**item, "dst": licence, "packager": "rpm", "type": "license"}]
    return [{**item, "packager": "deb"}, {**item, "packager": "rpm", "type": "doc"}]


def contents(root):
    for relative in REQUIRED:
        if not os.path.isfile(os.path.join(root, relative)):
            raise Refusal(f"the staged tree {root} has no /{relative}")
    entries = (entry(root, relative) for relative in tree_paths(root))
    listed = [split for item in entries if item is not None for split in per_packager(item)]
    licences = {"dst": f"/{RPM_LICENSES}", "type": "dir", "packager": "rpm",
                "file_info": {"mode": 0o755}}
    return listed + [licences]


def render(template, values):
    found = set(PLACEHOLDER.findall(template))
    unknown = sorted(found - PLACEHOLDERS)
    if unknown:
        raise Refusal(f"the template has the placeholder {{{{{unknown[0]}}}}}, which nothing fills")
    missing = sorted(PLACEHOLDERS - found)
    if missing:
        raise Refusal(f"the template lacks the placeholder {{{{{missing[0]}}}}}")
    return PLACEHOLDER.sub(lambda match: values[match.group(1)], template)


def configuration(arguments):
    check_arguments(arguments)
    floor, hosts = lock_facts(arguments.lock)
    deb, rpm = host_relations(read_map(arguments.map, floor), hosts)
    engine_deb, engine_rpm = engine_relations(arguments.engine_relations)
    entries = contents(os.path.abspath(arguments.root))
    values = {
        "VERSION": arguments.version,
        "ARCH": arguments.arch,
        "MTIME": arguments.mtime,
        "DEB_DEPENDS": json.dumps(once(deb + engine_deb)),
        "RPM_DEPENDS": json.dumps(once(rpm + engine_rpm)),
        "POSTINSTALL": arguments.postinstall,
        "POSTREMOVE": arguments.postremove,
        "CONTENTS": "\n".join(f"  - {json.dumps(item, sort_keys=True)}" for item in entries),
    }
    return render(read_text(arguments.template), values), len(entries)


def write(path, text):
    partial = f"{path}.partial"
    with open(partial, "w", encoding="utf-8") as handle:
        handle.write(text)
    os.replace(partial, path)


def parse_arguments(argv):
    parser = Parser(prog="nfpm_config.py", add_help=False)
    for name in ("template", "map", "lock", "engine-relations", "root", "version", "arch",
                 "mtime", "postinstall", "postremove", "out"):  # fmt: skip
        parser.add_argument(f"--{name}", required=True)
    return parser.parse_args(argv)


def main(argv):
    try:
        arguments = parse_arguments(argv)
        text, count = configuration(arguments)
        write(arguments.out, text)
    except Refusal as refusal:
        print(f"nfpm_config: {refusal}", file=sys.stderr)
        return 1
    print(f"nfpm_config: {arguments.out}, fermix-desktop {arguments.version}, {count} entries")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
