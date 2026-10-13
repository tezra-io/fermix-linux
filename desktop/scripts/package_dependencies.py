#!/usr/bin/env python3
"""Every library the built fermix-desktop needs is inside it or declared, and glibc is not above
the declared floor (A§8.2).

    package_dependencies.py --root <staged tree> --map <host_relations.map>
                            --lock <RUNTIME.lock.json> --deb-control <control>
                            --rpm-requires <rpm -qp --requires output>

For every ELF object in the staged tree, each NEEDED entry and the program interpreter must be
either a library the private prefix carries, for an object inside the prefix, or a host library
whose row in the map names a relation the built deb declares in Depends and the built rpm declares
in its requirements. A program outside the prefix cannot use the private libraries, so all it needs
is the host's. The deb and the rpm must declare one glibc floor, and no object may need a GLIBC_
symbol version above it.

The declarations are read from what build_packages.sh takes out of the built packages, not from the
configuration they were built from, so this proves what a user's package manager is told.
"""

import argparse
import os
import re
import subprocess
import sys

PREFIX = "usr/lib/fermix-desktop"
# The engine's own executables are launched by a loader its postinstall puts here; no host package
# provides it.
ENGINE_LOADER_STORE = "/var/lib/fermix/runtimes/"
MAP_FIELDS = 3
DEB_FLOOR = re.compile(r"libc6 \(>= ([0-9.]+)\)")
RPM_FLOOR = re.compile(r"libc\.so\.6\(GLIBC_([0-9.]+)\)\(64bit\)")
NEEDED = re.compile(r"\(NEEDED\)\s+Shared library: \[(.+)\]")
INTERPRETER = re.compile(r"\[Requesting program interpreter: (.+)\]")
GLIBC_NEED = re.compile(r"Name: GLIBC_([0-9][0-9.]*)\b")
VERSION_SECTION = re.compile(r"\nVersion (?:definition|symbols) section")
ELF_MAGIC = b"\x7fELF"
READELF_TIMEOUT_SECONDS = 60
USAGE = (
    "package_dependencies.py --root <dir> --map <map> --lock <lock> --deb-control <file> "
    "--rpm-requires <file>"
)


class Refusal(Exception):
    """A refusal; its message is the sentence printed after "package_dependencies: "."""


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


def version_tuple(text):
    return tuple(int(part) for part in text.split("."))


def lock_floor(path):
    match = re.search(r'"glibc_floor"\s*:\s*"([0-9.]+)"', read_text(path))
    if match is None:
        raise Refusal(f"{path} has no glibc_floor")
    return match.group(1)


def read_map(path, floor):
    """soname -> (deb relation, rpm requirement), the map's library rows only."""
    rows = {}
    for number, line in enumerate(read_text(path).splitlines(), start=1):
        if not line.strip() or line.startswith("#"):
            continue
        fields = [field.strip().replace("{glibc_floor}", floor) for field in line.split(" :: ")]
        if len(fields) != MAP_FIELDS or not all(fields):
            raise Refusal(f"{path} line {number} is not <soname> :: <deb> :: <rpm>: {line}")
        if not fields[0].startswith("@"):
            rows[fields[0]] = (fields[1], fields[2])
    return rows


def deb_depends(path):
    """Each alternative of each Depends group, as written."""
    match = re.search(r"^Depends:(.*(?:\n[ \t].*)*)", read_text(path), re.MULTILINE)
    if match is None:
        raise Refusal(f"{path} has no Depends field")
    relations = set()
    for group in match.group(1).split(","):
        relations.update(" ".join(choice.split()) for choice in group.split("|"))
    return relations


def rpm_requires(path):
    return {line.strip() for line in read_text(path).splitlines() if line.strip()}


def declared_floor(deb, rpm):
    deb_floors = [m.group(1) for m in map(DEB_FLOOR.fullmatch, deb) if m]
    rpm_floors = [m.group(1) for m in map(RPM_FLOOR.fullmatch, rpm) if m]
    if len(deb_floors) != 1:
        raise Refusal("the deb declares no glibc floor, as libc6 (>= X.Y), or more than one")
    if len(rpm_floors) != 1:
        raise Refusal("the rpm declares no glibc floor, as libc.so.6(GLIBC_X.Y)(64bit), or more")
    if deb_floors[0] != rpm_floors[0]:
        raise Refusal(f"the deb declares glibc {deb_floors[0]} and the rpm {rpm_floors[0]}")
    return deb_floors[0]


def is_elf(path):
    with open(path, "rb") as handle:
        return handle.read(4) == ELF_MAGIC


def elf_objects(root):
    found = []
    for directory, names, files in os.walk(root):
        names.sort()
        for name in sorted(files):
            path = os.path.join(directory, name)
            if not os.path.islink(path) and is_elf(path):
                found.append(path)
    return found


def elf_facts(path):
    """(NEEDED entries and the interpreter's file name, GLIBC_ versions needed)."""
    result = subprocess.run(
        ["readelf", "-d", "-l", "-V", "-W", path],
        check=False,
        capture_output=True,
        text=True,
        timeout=READELF_TIMEOUT_SECONDS,
    )
    if result.returncode != 0:
        raise Refusal(f"readelf could not read {path}: {result.stderr.strip()}")
    needs = NEEDED.findall(result.stdout)
    for interpreter in INTERPRETER.findall(result.stdout):
        if not interpreter.startswith(ENGINE_LOADER_STORE):
            needs.append(os.path.basename(interpreter))
    # Only the version-needs section: a library that defines a version is not needing it.
    _, marker, needed = result.stdout.partition("Version needs section")
    needed = VERSION_SECTION.split(needed, maxsplit=1)[0]
    return needs, GLIBC_NEED.findall(needed) if marker else []


def check_need(shown, soname, rows, deb, rpm):
    if soname not in rows:
        raise Refusal(
            f"{shown} needs {soname}, which is not in the private prefix and host_relations.map "
            "has no row for"
        )
    deb_relation, rpm_requirement = rows[soname]
    if deb_relation not in deb:
        raise Refusal(f"{shown} needs {soname}, and the deb declares no '{deb_relation}'")
    if rpm_requirement not in rpm:
        raise Refusal(f"{shown} needs {soname}, and the rpm requires no '{rpm_requirement}'")


def check_object(root, path, rows, declared, floor):
    shown = "/" + os.path.relpath(path, root)
    private_lib = os.path.join(root, PREFIX, "lib")
    inside = shown.startswith(f"/{PREFIX}/")
    needs, glibc = elf_facts(path)
    for soname in needs:
        if inside and os.path.exists(os.path.join(private_lib, soname)):
            continue
        check_need(shown, soname, rows, *declared)
    for version in glibc:
        if version_tuple(version) > version_tuple(floor):
            raise Refusal(f"{shown} needs GLIBC_{version}, above the declared floor {floor}")
    return len(needs)


def check(arguments):
    root = arguments.root
    if not os.path.isdir(root):
        raise Refusal(f"no staged tree at {root}")
    deb = deb_depends(arguments.deb_control)
    rpm = rpm_requires(arguments.rpm_requires)
    floor = declared_floor(deb, rpm)
    rows = read_map(arguments.map, lock_floor(arguments.lock))
    objects = elf_objects(root)
    if not objects:
        raise Refusal(f"there is no ELF object in {root}")
    needs = sum(check_object(root, path, rows, (deb, rpm), floor) for path in objects)
    print(
        f"package_dependencies: {len(objects)} ELF objects, {needs} needs, each private or "
        f"declared in both families; nothing above glibc {floor}"
    )


def parse_arguments(argv):
    parser = Parser(prog="package_dependencies.py", add_help=False)
    for name in ("root", "map", "lock", "deb-control", "rpm-requires"):
        parser.add_argument(f"--{name}", required=True)
    return parser.parse_args(argv)


def main(argv):
    try:
        check(parse_arguments(argv))
    except Refusal as refusal:
        print(f"package_dependencies: {refusal}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
