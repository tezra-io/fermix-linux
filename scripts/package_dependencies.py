#!/usr/bin/env python3
"""Check the packages' declared relations against what the binary actually needs.

nFPM runs no `dpkg-shlibdeps` and no rpm automatic-`Requires` generator: it
writes the relations its configuration lists and nothing else (M38 §12.1). So a
package that declares nothing installs cleanly on a host below the toolkit floor
and dies at exec with a dynamic-linker error, which is exactly what the floor
exists to prevent and what neither `desktop-file-validate` nor `appstreamcli`
can see.

This is the machine-derived half of that declaration. It takes five files that
`scripts/build_packages.sh` gathers from the built artifacts, and refuses when:

  * a shared library the binary names is one this application links directly and
    the package for it is not declared, in either family;
  * a Debian relation the binary derives names a package that is neither
    declared nor brought in by something that is;
  * a package is declared at a minimum version older than the one the binary
    needs;
  * the exact-version relation on the engine is missing or is not this version,
    in either family.

Every input is a file rather than a flag value, so the whole check is reachable
from `scripts/build_packages_test.sh` with fixtures, on a host with no dpkg, no
rpm and no packages built.

The boundary, stated rather than discovered: a relation covered *through*
another package's own dependencies is covered by name here, and its version is
whatever that package requires. The glibc floor in particular follows from the
build image, which is why the image is pinned, and the proof that the binary
runs on the toolkit floor is an install on a floor host (M38 §13.1 gate 25),
not a declaration.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# A Debian relation: a package name, optionally with one version constraint.
RELATION = re.compile(
    r"^(?P<name>[^\s(]+)"
    r"(?:\s*\(\s*(?P<operator>[<>=]+)\s*(?P<version>[^)\s]+)\s*\))?$"
)

# An rpm requirement line: a name, optionally with an operator and a version.
RPM_REQUIREMENT = re.compile(
    r"^(?P<name>[^\s]+)(?:\s+(?P<operator>[<>=]+)\s+(?P<version>\S+))?$"
)


class Refusal(RuntimeError):
    """Something the check will not let through, phrased for a person."""


def read(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        raise Refusal(f"cannot read {path}: {error}") from error


def deb_relations(text: str) -> dict[str, tuple[str | None, str | None]]:
    """Parse a Debian relation field into {name: (operator, version)}."""
    found: dict[str, tuple[str | None, str | None]] = {}
    for piece in text.replace("\n", ",").split(","):
        piece = piece.strip()
        if not piece:
            continue
        # dpkg-shlibdeps -O prints one line, `shlibs:Depends=<relations>`.
        if piece.startswith("shlibs:Depends="):
            piece = piece[len("shlibs:Depends=") :].strip()
            if not piece:
                continue
        match = RELATION.match(piece)
        if not match:
            raise Refusal(f"cannot read the Debian relation '{piece}'")
        found[match.group("name")] = (match.group("operator"), match.group("version"))
    return found


def rpm_requirements(text: str) -> dict[str, tuple[str | None, str | None]]:
    """Parse `rpm -qp --requires` output, dropping rpmlib's own capabilities."""
    found: dict[str, tuple[str | None, str | None]] = {}
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("rpmlib("):
            continue
        match = RPM_REQUIREMENT.match(line)
        if not match:
            raise Refusal(f"cannot read the rpm requirement '{line}'")
        found[match.group("name")] = (match.group("operator"), match.group("version"))
    return found


def _segments(version: str) -> list[tuple[int, str]]:
    """Split a version into the alternating runs dpkg compares."""
    return [
        (index % 2, part)
        for index, part in enumerate(re.split(r"(\d+)", version))
        if part != ""
    ]


def _order(character: str) -> int:
    # dpkg's own ordering for the non-digit runs: `~` sorts before everything,
    # then letters, then the rest by ASCII, above every letter.
    if character == "~":
        return -1
    if character.isalpha():
        return ord(character)
    return ord(character) + 256


def compare_versions(left: str, right: str) -> int:
    """dpkg's upstream-version ordering, as -1, 0 or 1.

    Epochs and Debian revisions never reach this: `build_packages.sh` refuses a
    version carrying `-` or `:` before it builds anything, and the toolkit
    relations are plain upstream versions.
    """
    left_parts, right_parts = _segments(left), _segments(right)
    for index in range(max(len(left_parts), len(right_parts))):
        left_part = left_parts[index] if index < len(left_parts) else (0, "")
        right_part = right_parts[index] if index < len(right_parts) else (0, "")

        if left_part[0] == 1 and right_part[0] == 1:
            difference = int(left_part[1] or 0) - int(right_part[1] or 0)
            if difference:
                return 1 if difference > 0 else -1
            continue

        one, two = left_part[1], right_part[1]
        for a, b in zip(one, two):
            if _order(a) != _order(b):
                return 1 if _order(a) > _order(b) else -1
        if len(one) != len(two):
            longer, sign = (one, 1) if len(one) > len(two) else (two, -1)
            extra = longer[min(len(one), len(two))]
            return -sign if extra == "~" else sign
    return 0


def check(
    *,
    version: str,
    declared_deb: dict[str, tuple[str | None, str | None]],
    declared_rpm: dict[str, tuple[str | None, str | None]],
    derived_deb: dict[str, tuple[str | None, str | None]],
    sonames: list[str],
    closure: set[str],
    toolkit: dict[str, tuple[str, str]],
) -> list[str]:
    """Every way the declaration fails to cover the machine's own answer."""
    problems: list[str] = []

    # 1. Every library the binary names directly is either one this application
    #    is allowed to link, with its package declared in both families, or
    #    something a declared package brings with it.
    for soname in sonames:
        if soname not in toolkit:
            continue
        deb_package, rpm_package = toolkit[soname]
        if deb_package not in declared_deb:
            problems.append(
                f"the binary needs {soname} and the deb declares no {deb_package}"
            )
        if rpm_package not in declared_rpm:
            problems.append(
                f"the binary needs {soname} and the rpm declares no {rpm_package}"
            )

    # 2. Every Debian relation the binary derives is covered by name, and where
    #    its package is declared here, at a version that is not older.
    for name, (_, minimum) in sorted(derived_deb.items()):
        if name not in declared_deb and name not in closure:
            problems.append(
                f"the binary needs {name} and neither the declaration nor its closure carries it"
            )
            continue
        if name not in declared_deb or minimum is None:
            continue
        operator, declared_version = declared_deb[name]
        if declared_version is None:
            problems.append(
                f"{name} is declared with no version and the binary needs {minimum}"
            )
        elif operator not in (">=", "="):
            problems.append(
                f"{name} is declared with '{operator}', which cannot express a floor"
            )
        elif compare_versions(declared_version, minimum) < 0:
            problems.append(
                f"{name} is declared at {declared_version} and the binary needs {minimum}"
            )

    # 3. The exact-version relation on the engine, in both spellings, because it
    #    is what makes the two packages one release.
    for family, declared in (("deb", declared_deb), ("rpm", declared_rpm)):
        if "fermix" not in declared:
            problems.append(f"the {family} declares no relation on fermix")
            continue
        operator, declared_version = declared["fermix"]
        if operator != "=" or declared_version != version:
            problems.append(
                f"the {family} declares fermix {operator or ''} {declared_version or ''}".rstrip()
                + f", and this version is {version}"
            )

    return problems


def report(
    sonames: list[str],
    toolkit: dict[str, tuple[str, str]],
    declared_deb: dict[str, tuple[str | None, str | None]],
    declared_rpm: dict[str, tuple[str | None, str | None]],
    derived_deb: dict[str, tuple[str | None, str | None]],
) -> None:
    def render(relations):
        return ", ".join(
            f"{name} ({operator} {version})" if version else name
            for name, (operator, version) in sorted(relations.items())
        )

    print("  the binary names:")
    for soname in sonames:
        where = "linked directly" if soname in toolkit else "through a declared package"
        print(f"    {soname:<34}{where}")
    print(f"  deb declares:  {render(declared_deb)}")
    print(f"  rpm declares:  {render(declared_rpm)}")
    print(f"  deb derived:   {render(derived_deb)}")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, help="the version being built")
    parser.add_argument("--declared-deb", required=True, type=Path)
    parser.add_argument("--declared-rpm", required=True, type=Path)
    parser.add_argument("--derived-deb", required=True, type=Path)
    parser.add_argument("--sonames", required=True, type=Path)
    parser.add_argument("--closure", required=True, type=Path)
    parser.add_argument(
        "--toolkit",
        required=True,
        type=Path,
        help="one soname|deb package|rpm package row per library this links directly",
    )
    arguments = parser.parse_args(argv)

    try:
        declared_deb = deb_relations(read(arguments.declared_deb))
        declared_rpm = rpm_requirements(read(arguments.declared_rpm))
        derived_deb = deb_relations(read(arguments.derived_deb))
        sonames = sorted(
            {line.strip() for line in read(arguments.sonames).splitlines() if line.strip()}
        )
        closure = {
            line.strip() for line in read(arguments.closure).splitlines() if line.strip()
        }
        toolkit = {}
        for row in read(arguments.toolkit).splitlines():
            row = row.strip()
            if not row:
                continue
            parts = row.split("|")
            if len(parts) != 3:
                raise Refusal(f"cannot read the toolkit row '{row}'")
            toolkit[parts[0]] = (parts[1], parts[2])
    except Refusal as refusal:
        print(f"package_dependencies: {refusal}", file=sys.stderr)
        return 1

    if not sonames:
        print("package_dependencies: the binary names no shared library at all", file=sys.stderr)
        return 1
    if not derived_deb:
        print("package_dependencies: nothing was derived from the binary", file=sys.stderr)
        return 1

    report(sonames, toolkit, declared_deb, declared_rpm, derived_deb)

    problems = check(
        version=arguments.version,
        declared_deb=declared_deb,
        declared_rpm=declared_rpm,
        derived_deb=derived_deb,
        sonames=sonames,
        closure=closure,
        toolkit=toolkit,
    )
    for problem in problems:
        print(f"package_dependencies: {problem}", file=sys.stderr)
    if problems:
        return 1

    print("  every derived requirement is covered by a declared relation")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
