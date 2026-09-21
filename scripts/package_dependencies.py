#!/usr/bin/env python3
"""Hold the declared relations equal to what the package's own ELFs need.

The question this asks is the opposite of the one it used to ask. The old
question was "do the declared toolkit relations cover the binary's needs", which
made sense when the toolkit came from the host. The package now carries its own
toolkit, so the question is the other way round (amendment section 8.2):

  * **Nothing reaches outside the package except to a declared host relation.**
    Every `NEEDED` entry of every ELF in the staged tree either names a file the
    package itself carries in the private prefix, or names a library on the host
    boundary whose relation the package declares in both families. A soname that
    is neither is a library the package will look for at exec and not find.
  * **Nothing is declared that nothing needs.** A relation with no `NEEDED`
    entry behind it makes a user's package manager install something for no
    reason, and it is how a list copied from a design document drifts away from
    the binaries. The two relations that are deliberately not about a `NEEDED`
    entry are named on the command line rather than exempted silently.
  * **The glibc floor is not below what the ELFs require.** Every ELF records,
    per symbol, the exact `GLIBC_x.y` it was bound to; the highest of those over
    the whole package is the real floor, and it has to be at or below the
    `libc6 (>= 2.34)` the package declares. This is the check that makes the
    refusal on openSUSE Leap a declared relation rather than a crash at exec.

`dpkg-shlibdeps` is deliberately not used and is not available: the build base
is AlmaLinux, which carries no dpkg, and the question above is about the ELFs
and the private prefix rather than about which Debian package ships a file on
the build host. Reading the ELFs directly is also the only way to ask the
question about an rpm at all.

**The engine's own executables are handled explicitly.** They are static, or
they are launched by the musl loader the package materialises under
`/var/lib/fermix/runtimes`, so they link nothing from the host, need no
relation, and set no glibc floor. They are recognised by what they are rather
than by where they sit, and an engine object that named a glibc interpreter
would be a refusal rather than a quiet exemption.

Every input is a file or a directory, so the whole check is reachable from
`scripts/build_packages_test.sh` with fixtures, on a host with no dpkg, no rpm
and no package built.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import elf_facts  # noqa: E402  (the path is set one line above)

ENGINE_LOADER_STORE = "/var/lib/fermix/runtimes/"
SEPARATOR = " :: "


class Refusal(RuntimeError):
    """Something the check will not let through, phrased for a person."""


def read(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        raise Refusal(f"cannot read {path}: {error}") from error


def host_relations(text: str) -> dict[str, tuple[str, str]]:
    """The host boundary: {soname: (deb relation, rpm capability)}."""
    found: dict[str, tuple[str, str]] = {}
    for line in text.splitlines():
        row = line.strip()
        if not row or row.startswith("#"):
            continue
        parts = [piece.strip() for piece in row.split(SEPARATOR)]
        if len(parts) != 3 or not all(parts):
            raise Refusal(f"cannot read the host relation row '{row}'")
        found[parts[0]] = (parts[1], parts[2])
    if not found:
        raise Refusal("the host relation map is empty, and it is the host boundary")
    return found


def declared(text: str) -> list[str]:
    """One relation per line or per comma, with rpmlib's own capabilities dropped."""
    relations = []
    for piece in text.replace("\n", ",").split(","):
        relation = piece.strip()
        if not relation or relation.startswith("rpmlib("):
            continue
        relations.append(relation)
    return relations


def parse_glibc_floor(relations: list[str], family: str) -> tuple[int, ...]:
    """The glibc floor the package declares, in whichever dialect it is written."""
    for relation in relations:
        if family == "deb" and relation.startswith("libc6 "):
            inside = relation.partition("(")[2].rstrip(")")
            operator, _, version = inside.partition(" ")
            if operator != ">=":
                raise Refusal(
                    f"the deb declares libc6 with '{operator}', and a floor is '>='"
                )
            return tuple(int(piece) for piece in version.split(".") if piece.isdigit())
        if family == "rpm" and relation.startswith("libc.so.6(GLIBC_"):
            version = relation[len("libc.so.6(GLIBC_") :].partition(")")[0]
            return tuple(int(piece) for piece in version.split(".") if piece.isdigit())
    raise Refusal(
        f"the {family} declares no glibc floor, and the floor is what makes the "
        "refusal on a host below it a package manager's refusal rather than a crash"
    )


def covers(declared_relations: set[str], wanted: str, family: str) -> bool:
    """Whether a declared relation satisfies the one a soname asks for.

    Two relations can say the same thing in different words, and exactly two
    shapes of that happen here. A Debian relation may carry a version
    constraint, so `libc6 (>= 2.34)` is the package `libc6`. And the glibc
    floor is written as a versioned symbol capability on the rpm side, so
    `libc.so.6(GLIBC_2.34)(64bit)` is what the package declares where an ELF
    needing `libc.so.6` asks for `libc.so.6()(64bit)`. Everything else matches
    by name, because everything else is a frozen SONAME.
    """
    if wanted in declared_relations:
        return True
    if family == "deb":
        names = {relation.split("(")[0].strip() for relation in declared_relations}
        return all(
            alternative.split("(")[0].strip() in names
            for alternative in wanted.split("|")
        )
    if wanted == "libc.so.6()(64bit)":
        return any(
            relation.startswith("libc.so.6(GLIBC_") for relation in declared_relations
        )
    return False


def is_engine_object(facts: dict) -> bool:
    """An engine executable: static, or launched by the engine's own loader."""
    interpreter = facts["interpreter"]
    if interpreter is not None:
        return interpreter.startswith(ENGINE_LOADER_STORE)
    return not facts["dynamic"] or not facts["needed"]


def survey(
    trees: list[Path], prefix: str
) -> tuple[dict[str, set[str]], list[str], tuple[int, ...] | None, int]:
    """What everything the package carries needs from outside the package.

    Every tree the package is built from is walked, not only the desktop stage.
    The engine's files are read straight out of the verified staging directory
    rather than copied into the stage, and an ELF the package installs is an ELF
    this has to account for wherever the build happens to be reading it from.

    Returns the host sonames with the objects that need each, the engine objects
    by path, the highest glibc any object requires, and how many objects the
    package carries in the private prefix.
    """
    private_roots = [
        os.path.abspath(os.path.join(str(tree), prefix.lstrip("/"))) for tree in trees
    ]
    carried = set()
    for root in private_roots:
        for directory, _, names in os.walk(root):
            carried.update(names)

    wanted: dict[str, set[str]] = {}
    engine: list[str] = []
    floor: tuple[int, ...] | None = None
    private = 0

    for tree in trees:
        for path, facts in elf_facts.walk(str(tree)):
            shown = os.path.relpath(path, str(tree))
            absolute = os.path.abspath(path)
            inside = any(absolute.startswith(root + os.sep) for root in private_roots)

            if not inside:
                if not is_engine_object(facts):
                    raise Refusal(
                        f"{shown} is an ELF outside the private prefix that is neither "
                        f"static nor launched by the engine's loader; it names "
                        f"{facts['interpreter']!r}"
                    )
                engine.append(shown)
                continue

            private += 1
            required = facts["glibc"]
            if required is not None:
                candidate = tuple(required)
                if floor is None or candidate > floor:
                    floor = candidate
            for soname in facts["needed"]:
                if soname in carried:
                    continue
                wanted.setdefault(soname, set()).add(shown)

    return wanted, engine, floor, private


def check(
    *,
    wanted: dict[str, set[str]],
    relations: dict[str, tuple[str, str]],
    declared_deb: list[str],
    declared_rpm: list[str],
    not_a_library: dict[str, list[str]],
    floor_required: tuple[int, ...] | None,
) -> list[str]:
    problems: list[str] = []

    deb_set = set(declared_deb)
    rpm_set = set(declared_rpm)
    needed_deb: set[str] = set()
    needed_rpm: set[str] = set()

    # 1. Everything the ELFs reach for is on the boundary, and declared.
    for soname in sorted(wanted):
        if soname not in relations:
            objects = ", ".join(sorted(wanted[soname])[:3])
            problems.append(
                f"{soname} is needed by {objects} and is neither carried in the "
                "private prefix nor a row in the host relation map"
            )
            continue
        deb_relation, rpm_relation = relations[soname]
        needed_deb.add(deb_relation)
        needed_rpm.add(rpm_relation)
        if not covers(deb_set, deb_relation, "deb"):
            problems.append(f"{soname} is needed and the deb declares no {deb_relation}")
        if not covers(rpm_set, rpm_relation, "rpm"):
            problems.append(f"{soname} is needed and the rpm declares no {rpm_relation}")

    # 2. Nothing is declared that nothing needs.
    for family, declared_set, needed_set in (
        ("deb", deb_set, needed_deb),
        ("rpm", rpm_set, needed_rpm),
    ):
        for relation in sorted(declared_set - needed_set):
            if relation in not_a_library[family]:
                continue
            if relation.startswith("libc6 (") or relation.startswith("libc.so.6(GLIBC_"):
                continue
            problems.append(
                f"the {family} declares {relation} and no ELF in the package needs it"
            )

    # 3. The floor the ELFs require is not above the floor the package declares.
    for family, relations_list in (("deb", declared_deb), ("rpm", declared_rpm)):
        try:
            declared_floor = parse_glibc_floor(relations_list, family)
        except Refusal as refusal:
            problems.append(str(refusal))
            continue
        if floor_required is not None and floor_required > declared_floor:
            problems.append(
                f"an ELF in the package requires GLIBC_"
                f"{'.'.join(str(piece) for piece in floor_required)} and the {family} "
                f"declares a floor of "
                f"{'.'.join(str(piece) for piece in declared_floor)}"
            )

    return problems


def check_engine_alternative(
    version: str, package: dict[str, dict[str, list[str]]]
) -> list[str]:
    """The three relations that replace the old exact-version dependency."""
    problems: list[str] = []
    for family, expected in (("deb", f"fermix (= {version})"), ("rpm", f"fermix = {version}")):
        if expected not in package[family]["provides"]:
            problems.append(
                f"the {family} does not provide '{expected}', so anything that "
                "depends on the engine by name is unsatisfied by this package"
            )
        if "fermix" not in package[family]["conflicts"]:
            problems.append(
                f"the {family} does not conflict with fermix, so the engine-only "
                "package and this one are not mutually exclusive"
            )
    if "fermix" not in package["deb"]["replaces"]:
        problems.append(
            "the deb does not replace fermix, so dpkg cannot hand /usr/bin/fermix "
            "from one package to the other in one transaction"
        )
    if package["rpm"]["obsoletes"]:
        problems.append(
            "the rpm obsoletes "
            + ", ".join(package["rpm"]["obsoletes"])
            + ", which would convert a headless install into a desktop one on the "
            "next upgrade"
        )
    return problems


def load_package_relations(path: Path) -> dict[str, dict[str, list[str]]]:
    """What the built packages declare, as `build_packages.sh` read it back."""
    try:
        loaded = json.loads(read(path))
    except json.JSONDecodeError as error:
        raise Refusal(f"{path} is not valid JSON: {error}") from error
    for family in ("deb", "rpm"):
        if family not in loaded:
            raise Refusal(f"{path} carries no {family} relations")
        for field in ("depends", "provides", "conflicts", "replaces", "obsoletes"):
            loaded[family].setdefault(field, [])
    return loaded


# A library named as a plain string inside an ELF that does not link it. That is
# what a dlopen looks like from the outside, and it is the one dependency an ELF
# header cannot tell you about.
SONAME_STRING = re.compile(rb"(?<![A-Za-z0-9_./+-])lib[A-Za-z0-9_+-]+\.so\.[0-9]+(?![A-Za-z0-9_.])")


def carried_in_prefix(trees: list[Path], prefix: str) -> set[str]:
    """Every file name the private prefix can answer a name with."""
    names: set[str] = set()
    for tree in trees:
        root = os.path.abspath(os.path.join(str(tree), prefix.lstrip("/")))
        for _, _, files in os.walk(root):
            names.update(files)
    return names


def dlopen_candidates(
    trees: list[Path], prefix: str, carried: set[str]
) -> dict[str, set[str]]:
    """Every soname the package names but does not link, by the objects naming it.

    This exists because of a real failure rather than a hypothetical one. GTK
    reaches OpenGL through libepoxy, which links none of the GL libraries and
    loads whichever it needs by name, so a boundary read out of NEEDED entries
    listed none of them, every gate passed, and the window aborted on the first
    machine that lacked one. No header records a dlopen; the name is only a
    string in the file.

    So the strings are read. A name that is carried in the private prefix is not
    a host dependency, and a name the object also links is already accounted
    for; what is left is every library this package might ask a host for without
    ever saying so. Each one then has to be a decision somebody made, which is
    what the caller checks.
    """
    private_roots = [
        os.path.abspath(os.path.join(str(tree), prefix.lstrip("/"))) for tree in trees
    ]
    found: dict[str, set[str]] = {}

    for tree in trees:
        for path, facts in elf_facts.walk(str(tree)):
            absolute = os.path.abspath(path)
            if not any(absolute.startswith(root + os.sep) for root in private_roots):
                continue
            shown = os.path.relpath(path, str(tree))
            linked = set(facts["needed"])
            try:
                with open(path, "rb") as handle:
                    body = handle.read()
            except OSError as error:
                raise Refusal(f"cannot read {shown}: {error}") from error
            for match in set(SONAME_STRING.findall(body)):
                soname = match.decode("ascii")
                if soname in carried or soname in linked:
                    continue
                found.setdefault(soname, set()).add(shown)

    return found


def check_every_dlopen_is_a_decision(
    candidates: dict[str, set[str]],
    relations: dict[str, tuple[str, str]],
    declared_deb: set[str],
    declared_rpm: set[str],
    considered: list[str],
) -> list[str]:
    """Nothing the package can load by name is left to luck.

    A candidate is fine two ways: the package declares a relation for it, so a
    machine that installs the package has it; or somebody looked at it and wrote
    it down as deliberately not declared, with the reason beside the name in
    build_packages.sh. What is refused is the third case, the one that shipped:
    a library this package can ask for that nobody has decided about.
    """
    problems = []
    for soname in sorted(candidates):
        if soname in considered:
            continue
        relation = relations.get(soname)
        if relation is not None and (
            covers(declared_deb, relation[0], "deb")
            or covers(declared_rpm, relation[1], "rpm")
        ):
            continue
        objects = ", ".join(sorted(candidates[soname])[:3])
        problems.append(
            f"{soname} is named as a string by {objects} and is neither linked, "
            "declared, nor on the list of names somebody decided not to declare; "
            "a library this package can load by name is a dependency no ELF header "
            "records"
        )
    return problems


def check_the_two_boundary_lists_agree(map_path: Path, lock_path: Path) -> list[str]:
    """The host boundary is one list, spelled in two files, and they must match.

    `scripts/host_relations.map` is the authority: it names every soname the
    package may take from the host and what that soname is called in each
    family. `packaging/runtime/RUNTIME.lock.json`'s `host_libraries` is the
    runtime build's copy of the same boundary, and it is what the toolkit's own
    build enforces on every private ELF.

    Neither file can prove the other complete on its own, which is exactly how
    libGLESv2.so.2 was missing from both until a window aborted: the runtime's
    check proves nothing UNDECLARED is linked, and a linked-only view cannot see
    a library that is dlopened. So the one property a machine can hold is that
    the two lists are the same list, and a soname added to either without the
    other is refused here, by name, at build time.
    """
    problems: list[str] = []
    try:
        lock = json.loads(read(lock_path))
    except json.JSONDecodeError as error:
        raise Refusal(f"{lock_path} is not valid JSON: {error}") from error

    declared_in_lock = set(lock.get("host_libraries") or ())
    if not declared_in_lock:
        raise Refusal(f"{lock_path} carries no host_libraries list, and it is the host boundary")

    in_map = set(host_relations(read(map_path)))

    for soname in sorted(declared_in_lock - in_map):
        problems.append(
            f"{soname} is on the runtime lock's host boundary and has no row in "
            f"{map_path.name}, so the package cannot say what to require for it "
            "in either family"
        )
    for soname in sorted(in_map - declared_in_lock):
        problems.append(
            f"{soname} has a row in {map_path.name} and is not on the runtime "
            f"lock's host boundary in {lock_path.name}; one of the two files is "
            "behind the other"
        )
    return problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, help="the version being built")
    parser.add_argument("--stage", required=True, type=Path)
    parser.add_argument(
        "--engine-tree",
        action="append",
        default=[],
        type=Path,
        help="another tree the package is built from, such as the verified engine's",
    )
    parser.add_argument("--relations", required=True, type=Path, help="the built packages' own relations, as JSON")
    parser.add_argument("--host-map", required=True, type=Path)
    parser.add_argument(
        "--lock",
        type=Path,
        help="the runtime lock file, whose host_libraries must be the same "
        "boundary as the host map's rows",
    )
    parser.add_argument(
        "--prefix", default="/usr/lib/fermix-desktop", help="the private prefix"
    )
    parser.add_argument(
        "--not-a-library-deb",
        action="append",
        default=[],
        help="a deb relation declared for a reason other than a NEEDED entry",
    )
    parser.add_argument(
        "--dlopen-considered",
        action="append",
        default=[],
        help="a soname this package names but deliberately does not declare, "
        "with the reason recorded beside it in build_packages.sh",
    )
    parser.add_argument(
        "--not-a-library-rpm",
        action="append",
        default=[],
        help="an rpm relation declared for a reason other than a NEEDED entry",
    )
    arguments = parser.parse_args(argv)

    try:
        relations = host_relations(read(arguments.host_map))
        package = load_package_relations(arguments.relations)
        trees = [arguments.stage, *arguments.engine_tree]
        for tree in trees:
            if not tree.is_dir():
                raise Refusal(f"no tree at {tree}")
        wanted, engine, floor, private = survey(trees, arguments.prefix)
    except Refusal as refusal:
        print(f"package_dependencies: {refusal}", file=sys.stderr)
        return 1

    if not private:
        print(
            "package_dependencies: the staged tree carries no ELF in the private "
            f"prefix at {arguments.prefix}",
            file=sys.stderr,
        )
        return 1

    print(f"  {private} objects in the private prefix, {len(engine)} of the engine's own")
    print(f"  they reach outside the package for: {', '.join(sorted(wanted)) or 'nothing'}")
    print(
        "  the highest glibc any of them requires: "
        + (".".join(str(piece) for piece in floor) if floor else "none")
    )

    problems = check(
        wanted=wanted,
        relations=relations,
        declared_deb=declared("\n".join(package["deb"]["depends"])),
        declared_rpm=declared("\n".join(package["rpm"]["depends"])),
        not_a_library={
            "deb": arguments.not_a_library_deb,
            "rpm": arguments.not_a_library_rpm,
        },
        floor_required=floor,
    )
    problems += check_engine_alternative(arguments.version, package)

    try:
        carried_names = carried_in_prefix(trees, arguments.prefix)
        candidates = dlopen_candidates(trees, arguments.prefix, carried_names)
    except Refusal as refusal:
        print(f"package_dependencies: {refusal}", file=sys.stderr)
        return 1
    if candidates:
        print(
            "  named but not linked, so loadable by name: "
            + ", ".join(sorted(candidates))
        )
    problems += check_every_dlopen_is_a_decision(
        candidates,
        relations,
        set(declared("\n".join(package["deb"]["depends"]))),
        set(declared("\n".join(package["rpm"]["depends"]))),
        arguments.dlopen_considered,
    )
    if arguments.lock is not None:
        try:
            problems += check_the_two_boundary_lists_agree(arguments.host_map, arguments.lock)
        except Refusal as refusal:
            problems.append(str(refusal))

    for problem in problems:
        print(f"package_dependencies: {problem}", file=sys.stderr)
    if problems:
        return 1

    print("  every relation the packages declare is one the ELFs justify, and back")
    if arguments.lock is not None:
        print("  the host map and the runtime lock describe the same boundary")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
