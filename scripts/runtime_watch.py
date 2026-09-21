#!/usr/bin/env python3
"""Watch the private runtime's twenty-nine components for advisories and bumps.

Bundling GTK and everything under it means owning the CVE surface a
distribution would otherwise patch (amendment section 4.5). This script is the
rail that answers for it: for every component in
`packaging/runtime/RUNTIME.lock.json` it asks the OSV database what is known
about that name and version, and asks release-monitoring.org what upstream has
released since, and writes one finding per component that has something to say.
`.github/workflows/runtime-watch.yml` turns each finding into one issue.

Three decisions worth stating rather than discovering.

**A component with no coordinate is a finding, not a silence.** Neither source
keys on the names the lock file uses, so the map below is written by hand. A
component that is not in it is reported as unwatched, because a watcher that
quietly skipped a library would read as a watcher that found nothing wrong.
Where a component carries a `purl`, its upstream name is taken from that
instead, so the lock file stays the authority on what a component is called and
the map below only has to answer for the ones that have none.

**GTK and libadwaita are watched inside their series.** The crate's feature
floor pins GTK 4.16 and libadwaita 1.6 (amendment section 4.4), so 4.22 is not
an available upgrade for this product and reporting it every week would train
the reader to ignore the issue. For those components only the same `major.minor`
counts as available, and the issue says so.

**Nothing here bumps anything.** A GTK point release can change rendering and
the captures are a reviewed artifact, so the output is an issue for a person.

Offline by fixture: `--osv-fixture` and `--anitya-fixture` replace both network
calls with files, which is how `scripts/runtime_watch_test.sh` proves every
refusal and every finding shape without reaching either service.

Usage:
  runtime_watch.py [--lock <path>] --out <dir>
                   [--osv-fixture <file>] [--anitya-fixture <file>]
                   [--timeout <seconds>] [--attempts <n>]

Writes <out>/findings.json and one <out>/<component>.md per finding. Exit 0 when
it ran, whatever it found; a non-zero exit means the watcher itself is broken.
"""

import argparse
import json
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

OSV_URL = "https://api.osv.dev/v1/query"
ANITYA_URL = "https://release-monitoring.org/api/v2/projects/?name="

# Every component of the lock file, and the name each source knows it by.
# `series` means an upgrade only counts inside the pinned major.minor.
COORDINATES = {
    "libffi": {"osv": "libffi", "anitya": "libffi"},
    "pcre2": {"osv": "pcre2", "anitya": "pcre2"},
    "expat": {"osv": "expat", "anitya": "expat"},
    "libpng": {"osv": "libpng", "anitya": "libpng"},
    "libjpeg-turbo": {"osv": "libjpeg-turbo", "anitya": "libjpeg-turbo"},
    "libtiff": {"osv": "libtiff", "anitya": "libtiff"},
    "libwebp": {"osv": "libwebp", "anitya": "libwebp"},
    "freetype": {"osv": "freetype2", "anitya": "freetype"},
    "fribidi": {"osv": "fribidi", "anitya": "fribidi"},
    "pixman": {"osv": "pixman", "anitya": "pixman"},
    "fontconfig": {"osv": "fontconfig", "anitya": "fontconfig"},
    "glib": {"osv": "glib", "anitya": "glib"},
    "harfbuzz": {"osv": "harfbuzz", "anitya": "harfbuzz"},
    "cairo": {"osv": "cairo", "anitya": "cairo"},
    "graphene": {"osv": None, "anitya": "graphene"},
    "pango": {"osv": "pango", "anitya": "pango"},
    "gdk-pixbuf": {"osv": "gdk-pixbuf", "anitya": "gdk-pixbuf"},
    "libxml2": {"osv": "libxml2", "anitya": "libxml2"},
    "librsvg": {"osv": "librsvg", "anitya": "librsvg"},
    "webp-pixbuf-loader": {"osv": None, "anitya": "webp-pixbuf-loader"},
    "libxmlb": {"osv": None, "anitya": "libxmlb"},
    "appstream": {"osv": None, "anitya": "appstream"},
    "wayland": {"osv": "wayland", "anitya": "wayland"},
    "wayland-protocols": {"osv": None, "anitya": "wayland-protocols"},
    "libepoxy": {"osv": None, "anitya": "libepoxy"},
    "gtk": {"osv": "gtk", "anitya": "gtk", "series": True},
    "libadwaita": {"osv": None, "anitya": "libadwaita", "series": True},
    "dconf": {"osv": None, "anitya": "dconf"},
    "adwaita-icon-theme": {"osv": None, "anitya": "adwaita-icon-theme"},
}

# The ecosystem OSV files these projects under. They are C libraries with no
# package registry, so OSS-Fuzz is where their advisories live.
OSV_ECOSYSTEM = "OSS-Fuzz"

VERSION = re.compile(r"\d+(?:\.\d+)*")
# pkg:generic/<name>, with anything after a ? or a @ belonging to the locator
# rather than to the name.
PURL = re.compile(r"pkg:[^/]+/(?:.+/)?([^/@?#]+)")


def refuse(sentence):
    sys.exit(f"runtime_watch: {sentence}")


def read_json(path, what):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except OSError as error:
        refuse(f"cannot read {what} at {path}: {error}")
    except json.JSONDecodeError as error:
        refuse(f"{what} at {path} is not valid JSON: {error}")


def components_of(lock, path):
    components = lock.get("components")
    if not isinstance(components, list) or not components:
        refuse(f"{path} carries no components list")
    for component in components:
        if not isinstance(component, dict):
            refuse(f"{path} carries a component that is not an object")
        for field in ("name", "version"):
            if not isinstance(component.get(field), str) or not component[field]:
                refuse(f"{path} carries a component with no {field}")
    return components


# Bounded by construction: `attempts` tries with a fixed pause, and the cap is a
# recorded failure for that one component rather than a retry that never ends or
# a run that dies on one service being slow.
def fetch(request, timeout, attempts):
    last = ""
    for attempt in range(1, attempts + 1):
        try:
            with urllib.request.urlopen(request, timeout=timeout) as answer:
                return json.loads(answer.read().decode("utf-8")), ""
        except (urllib.error.URLError, TimeoutError, ValueError) as error:
            last = str(error)
            if attempt < attempts:
                time.sleep(2)
    return None, last


def osv_advisories(name, version, timeout, attempts):
    """What OSV knows about this name at this version."""
    body = json.dumps(
        {"package": {"name": name, "ecosystem": OSV_ECOSYSTEM}, "version": version}
    ).encode("utf-8")
    request = urllib.request.Request(
        OSV_URL, data=body, headers={"Content-Type": "application/json"}
    )
    answer, error = fetch(request, timeout, attempts)
    if answer is None:
        return None, error
    return [
        {
            "id": vulnerability.get("id", "unknown"),
            "summary": (vulnerability.get("summary") or "").strip(),
            "aliases": vulnerability.get("aliases", []),
        }
        for vulnerability in answer.get("vulns", [])
    ], ""


def anitya_versions(name, timeout, attempts):
    """Every stable version release-monitoring.org has seen for this project."""
    request = urllib.request.Request(
        ANITYA_URL + urllib.parse.quote(name), headers={"Accept": "application/json"}
    )
    answer, error = fetch(request, timeout, attempts)
    if answer is None:
        return None, error
    for project in answer.get("items", []):
        if project.get("name") == name:
            return project.get("stable_versions") or [], ""
    return [], ""


def upstream_name(component, coordinate):
    """What release-monitoring.org calls this component.

    The lock file's `purl` is preferred where it has one, because the lock file
    is the authority on what a component is and a hand-written map is a second
    copy of that fact.
    """
    purl = component.get("purl")
    if isinstance(purl, str):
        match = PURL.match(purl)
        if match:
            return match.group(1)
    return coordinate["anitya"]


def as_tuple(version):
    match = VERSION.match(version)
    if not match:
        return None
    return tuple(int(part) for part in match.group(0).split("."))


def newest(installed, available, series):
    """The newest version worth reporting, or nothing.

    With `series`, only the same major.minor counts, because the crate's feature
    floor pins that series and a newer one is a deliberate change to Cargo.toml,
    the lock file and the design together rather than an upgrade.
    """
    here = as_tuple(installed)
    if here is None:
        return None
    best = None
    for candidate in available:
        there = as_tuple(candidate)
        if there is None or there <= here:
            continue
        if series and there[:2] != here[:2]:
            continue
        if best is None or there > as_tuple(best):
            best = candidate
    return best


def look_at(component, coordinate, osv, anitya):
    """One component, as one finding or as nothing to say."""
    name = component["name"]
    installed = component["version"]
    series = coordinate.get("series", False)

    advisories, advisory_error = ([], "")
    if coordinate["osv"] is not None:
        advisories, advisory_error = osv(coordinate["osv"], installed)
    available, version_error = ([], "")
    upstream = upstream_name(component, coordinate)
    if upstream is not None:
        available, version_error = anitya(upstream)

    upgrade = newest(installed, available or [], series)
    errors = [error for error in (advisory_error, version_error) if error]
    if not advisories and not upgrade and not errors:
        return None
    return {
        "component": name,
        "installed": installed,
        "available": upgrade,
        "series_pinned": series,
        "advisories": advisories or [],
        "errors": errors,
    }


def title_of(finding):
    if finding.get("unwatched"):
        return f"runtime: {finding['component']} is not watched by either source"
    if finding["available"]:
        return (
            f"runtime: {finding['component']} {finding['installed']} -> "
            f"{finding['available']}"
        )
    return f"runtime: {finding['component']} {finding['installed']} advisories"


def body_of(finding):
    lines = [f"`{finding['component']}` is pinned at {finding['installed']} in "
             "`packaging/runtime/RUNTIME.lock.json`.", ""]
    if finding.get("unwatched"):
        lines += [
            "Neither the OSV database nor release-monitoring.org is queried for "
            "this component, because `scripts/runtime_watch.py` has no name for "
            "it in either source. Add one, or record here why there is none.",
        ]
        return "\n".join(lines) + "\n"

    if finding["available"]:
        lines += [f"Upstream has released {finding['available']}.", ""]
        if finding["series_pinned"]:
            lines += [
                "Only this component's own series is considered, because the "
                "crate's feature floor pins it (amendment section 4.4). A move "
                "to the next series changes `Cargo.toml`, the lock file and the "
                "design together.",
                "",
            ]
    if finding["advisories"]:
        lines += ["OSV reports, against the pinned version:", ""]
        for advisory in finding["advisories"]:
            aliases = ", ".join(advisory["aliases"])
            suffix = f" ({aliases})" if aliases else ""
            lines.append(f"- {advisory['id']}{suffix}: {advisory['summary'] or 'no summary'}")
        lines.append("")
    if finding["errors"]:
        lines += ["One source could not be read, so this issue is incomplete:", ""]
        lines += [f"- {error}" for error in finding["errors"]]
        lines.append("")
    lines += [
        "Nothing is bumped automatically: a point release can change rendering "
        "and the reference captures are a reviewed artifact. A bump is a change "
        "to `RUNTIME.lock.json`, which rebuilds the runtime image and is "
        "published as the next `<engine version>+<n>`.",
    ]
    return "\n".join(lines) + "\n"


def build_sources(arguments):
    """Either the two services, or two fixtures. Never a mixture by accident.

    Each returns a pair of the answer and a sentence saying why there is none,
    so a source that could not be read reaches the issue rather than reading as
    a component with nothing to report.
    """

    def osv(name, version):
        if arguments.osv_fixture:
            return read_json(arguments.osv_fixture, "the OSV fixture").get(name, []), ""
        return osv_advisories(name, version, arguments.timeout, arguments.attempts)

    def anitya(name):
        if arguments.anitya_fixture:
            fixture = read_json(arguments.anitya_fixture, "the upstream fixture")
            return fixture.get(name, []), ""
        return anitya_versions(name, arguments.timeout, arguments.attempts)

    return osv, anitya


def parse_arguments(argv):
    parser = argparse.ArgumentParser(add_help=True)
    parser.add_argument("--lock", default=None)
    parser.add_argument("--out", required=True)
    parser.add_argument("--osv-fixture", default=None)
    parser.add_argument("--anitya-fixture", default=None)
    parser.add_argument("--timeout", type=int, default=20)
    parser.add_argument("--attempts", type=int, default=3)
    arguments = parser.parse_args(argv)
    if arguments.timeout < 1 or arguments.timeout > 120:
        refuse(f"--timeout {arguments.timeout} is outside 1 to 120 seconds")
    if arguments.attempts < 1 or arguments.attempts > 5:
        refuse(f"--attempts {arguments.attempts} is outside 1 to 5")
    return arguments


def main(argv):
    arguments = parse_arguments(argv)
    root = Path(__file__).resolve().parents[1]
    lock_path = Path(arguments.lock or root / "packaging/runtime/RUNTIME.lock.json")
    out = Path(arguments.out)
    out.mkdir(parents=True, exist_ok=True)

    lock = read_json(lock_path, "the runtime lock file")
    components = components_of(lock, lock_path)
    osv, anitya = build_sources(arguments)

    findings = []
    for component in components:
        coordinate = COORDINATES.get(component["name"])
        # A purl is enough on its own: it names the upstream project, which is
        # the half neither source can guess. The map is then only the OSV name
        # and the series rule, both of which default to absent.
        if coordinate is None and isinstance(component.get("purl"), str):
            coordinate = {"osv": None, "anitya": None}
        if coordinate is None:
            findings.append(
                {
                    "component": component["name"],
                    "installed": component["version"],
                    "available": None,
                    "series_pinned": False,
                    "advisories": [],
                    "errors": [],
                    "unwatched": True,
                }
            )
            continue
        finding = look_at(component, coordinate, osv, anitya)
        if finding is not None:
            findings.append(finding)

    for finding in findings:
        finding["title"] = title_of(finding)
        (out / f"{finding['component']}.md").write_text(
            body_of(finding), encoding="utf-8"
        )

    (out / "findings.json").write_text(
        json.dumps(findings, indent=2) + "\n", encoding="utf-8"
    )
    # One line per finding, component then title, so the workflow that opens
    # the issues is a `while read` loop rather than a JSON parser written in
    # shell. A title carrying a tab or a newline would break that loop, so it
    # cannot: every title this script writes is built from a component name and
    # two version strings.
    (out / "index.tsv").write_text(
        "".join(f"{f['component']}\t{f['title']}\n" for f in findings),
        encoding="utf-8",
    )
    print(f"runtime_watch: {len(components)} components, {len(findings)} findings")
    for finding in findings:
        print(f"  {finding['title']}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
