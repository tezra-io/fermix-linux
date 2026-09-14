#!/usr/bin/env python3
"""Vendor mark provenance: the offline gate.

The record and the bytes are vendored from the macOS door, which is where a
mark is retrieved from its vendor and where the record is written. This door
proves the copy is complete and true of the bytes on disk, and that every mark
it ships is reachable from the resource bundle the application draws from.

Two records, so neither can be edited alone:

  PROVENANCE.json  one record per vendor, and the hash of the roster below
  ROSTER.json      the vendored snapshot of fermix's provider, channel and
                   plugin sets

Completeness is checked against ROSTER.json and runs everywhere. Two upstream
cross-checks sit on top of it:

  providers and channels  against the vendored management fixtures in this
                          repository, so they run offline and always
  plugins                 against fermix's own catalog, which needs a checkout
                          and is therefore opt-in through --fermix-repo

A skipped check names itself, so it can never be mistaken for the completeness
gate it is not.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

SCHEMA_VERSION = 2
ROSTER_SCHEMA_VERSION = 2
ROSTER_FILE = "ROSTER.json"
PROVENANCE_FILE = "PROVENANCE.json"
# The two records, which describe the tree rather than living in it.
RECORD_FILES = (PROVENANCE_FILE, ROSTER_FILE)
KINDS = ("provider", "channel", "plugin", "feature", "meeting_platform", "oauth_client")
TREATMENTS = ("vendor_mark", "vendor_text_with_symbol")
ASSET_ROLES = ("color", "light", "dark", "monochrome")
# The plate a shipped mark is drawn on. The application reads this field rather
# than guessing from the pixels: a mark that carries its own ground fills the
# slot, and one drawn on transparency is inset in it.
PLATES = ("neutral", "bleed")

# The first bytes of each format a mark may ship in, so a record cannot claim a
# format the file does not have. A hash pins which bytes ship, never what they
# are.
MAGIC = {
    "png": ((b"\x89PNG\r\n\x1a\n", 0),),
    "webp": ((b"RIFF", 0), (b"WEBP", 8)),
    "svg": ((b"<svg", None), (b"<?xml", 0)),
}
ORIGINS = ("vendor", "catalog", "first_party")
FERMIX_REPOSITORY = "https://github.com/tezra-io/fermix"
FALLBACK_REASONS = ("unretrievable_official_asset", "no_published_brand_kit")

REQUIRED_TEXT_FIELDS = (
    "key",
    "kind",
    "display_name",
    "accessibility_label",
    "treatment",
    "origin",
    "source_url",
    "source_retrieved_on",
    "usage_terms",
    "permitted_treatment",
    "dark_mode_policy",
)

DATE = re.compile(r"^\d{4}-\d{2}-\d{2}$")

# Copy rules that hold for every shipped resource, not only for the catalogue.
FORBIDDEN_COPY = ("—", "!", "please wait", "FermixPet", "TODO", "TBD")

# Where the application's own copies of these two sets come from, so the roster
# is compared against something this repository actually renders rather than
# against a count the record declares about itself.
FIXTURES = Path("App/Fermix/contracts/management/fixtures/success.jsonl")
GRESOURCE = Path("App/Fermix/resources/fermix.gresource.xml")
MARKS_DIR = Path("App/Fermix/resources/VendorMarks")
# The prefix the bundle serves marks under. The application composes a resource
# path from it and the recorded relative path, so the two have to agree.
RESOURCE_PREFIX = "/io/tezra/Fermix/marks"


class Failure(Exception):
    """A gate failure with an operator-readable sentence."""


def sha256_of(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load(marks_dir: Path) -> dict:
    path = marks_dir / PROVENANCE_FILE
    if not path.is_file():
        raise Failure(f"provenance record is missing at {path}")
    try:
        record = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise Failure(f"provenance record is not valid JSON: {error}") from error
    if record.get("schema_version") != SCHEMA_VERSION:
        raise Failure(
            f"provenance schema {record.get('schema_version')!r} is not supported "
            f"(this gate reads {SCHEMA_VERSION})"
        )
    return record


def check_copy(marks_dir: Path) -> None:
    text = (marks_dir / PROVENANCE_FILE).read_text(encoding="utf-8")
    for banned in FORBIDDEN_COPY:
        if banned in text:
            raise Failure(f"provenance record carries forbidden copy {banned!r}")


def check_mark_fields(mark: dict) -> None:
    key = mark.get("key", "<unnamed>")
    for field in REQUIRED_TEXT_FIELDS:
        value = mark.get(field)
        if not isinstance(value, str) or not value.strip():
            raise Failure(f"{key}: field {field} is missing or empty")
    if mark["kind"] not in KINDS:
        raise Failure(f"{key}: kind {mark['kind']!r} is not one of {KINDS}")
    if mark["origin"] not in ORIGINS:
        raise Failure(f"{key}: origin {mark['origin']!r} is not one of {ORIGINS}")
    if mark["origin"] != "vendor" and not mark["source_url"].startswith(FERMIX_REPOSITORY):
        raise Failure(
            f"{key}: origin {mark['origin']!r} but source_url {mark['source_url']!r} "
            f"is not under {FERMIX_REPOSITORY}"
        )
    if mark["treatment"] not in TREATMENTS:
        raise Failure(f"{key}: treatment {mark['treatment']!r} is not one of {TREATMENTS}")
    if mark["treatment"] == "vendor_mark" and mark.get("plate") not in PLATES:
        raise Failure(f"{key}: plate {mark.get('plate')!r} is not one of {PLATES}")
    if mark["treatment"] != "vendor_mark" and "plate" in mark:
        raise Failure(f"{key}: renders as vendor text and must not record a plate")
    if not DATE.match(mark["source_retrieved_on"]):
        raise Failure(
            f"{key}: source_retrieved_on {mark['source_retrieved_on']!r} is not YYYY-MM-DD"
        )
    if not mark["source_url"].startswith("https://"):
        raise Failure(f"{key}: source_url is not an https URL")


def check_assets(mark: dict, marks_dir: Path, claimed: set[str]) -> None:
    key = mark["key"]
    assets = mark.get("assets") or []
    if mark["treatment"] == "vendor_text_with_symbol":
        if assets:
            raise Failure(
                f"{key}: declares the text treatment but ships {len(assets)} asset(s); "
                "a vendor without a retrievable mark ships no file"
            )
        return
    if not assets:
        raise Failure(f"{key}: declares a vendor mark but ships no asset")

    seen_roles: set[str] = set()
    for asset in assets:
        role = asset.get("role")
        if role not in ASSET_ROLES:
            raise Failure(f"{key}: asset role {role!r} is not one of {ASSET_ROLES}")
        if role in seen_roles:
            raise Failure(f"{key}: asset role {role!r} is declared twice")
        seen_roles.add(role)

        relative = asset.get("path")
        if not isinstance(relative, str) or not relative:
            raise Failure(f"{key}: asset {role} has no path")
        path = marks_dir / relative
        if not path.is_file():
            raise Failure(f"{key}: asset {role} is recorded at {relative} but no such file exists")
        payload = path.read_bytes()
        actual = sha256_of(payload)
        if actual != asset.get("sha256"):
            raise Failure(
                f"{key}: asset {relative} hashes to {actual} "
                f"but the record pins {asset.get('sha256')}"
            )
        check_magic(key, relative, payload)
        url = asset.get("asset_url", "")
        if not isinstance(url, str) or not url.startswith("https://"):
            raise Failure(f"{key}: asset {role} has no https asset_url")
        if not DATE.match(asset.get("retrieved_on", "")):
            raise Failure(f"{key}: asset {role} has no YYYY-MM-DD retrieved_on")
        claimed.add(relative)


def check_magic(key: str, relative: str, payload: bytes) -> None:
    """The file's bytes are the format its name claims.

    The loader picks its decoder from the file's own content type, and a name
    that lies about its format is how a mark comes to draw on one host and not
    on another.
    """
    extension = relative.rsplit(".", 1)[-1].lower()
    signatures = MAGIC.get(extension)
    if signatures is None:
        raise Failure(
            f"{key}: asset {relative} has extension {extension!r}, which is not a mark format"
        )
    for marker, offset in signatures:
        if offset is None:
            if payload[:512].find(marker) >= 0:
                return
        elif payload[offset:offset + len(marker)] == marker:
            return
    raise Failure(
        f"{key}: asset {relative} is named {extension} but its bytes are not "
        f"{extension}; name the file for the format it actually is"
    )


def check_fallback(mark: dict) -> None:
    """A vendor with no retrievable mark records why, and renders as text.

    The symbol name in the record is the macOS door's, because the record is
    vendored from there byte for byte. This door's neutral icon per kind is the
    application's own table, and `mark.rs` carries the test that every kind has
    one, so what is checked here is that the reason and the detail are recorded.
    """
    key = mark["key"]
    fallback = mark.get("fallback")
    if mark["treatment"] == "vendor_mark":
        if fallback is not None:
            raise Failure(f"{key}: ships a vendor mark and must not also declare a fallback")
        return
    if not isinstance(fallback, dict):
        raise Failure(f"{key}: declares the text treatment but records no fallback")
    if fallback.get("reason") not in FALLBACK_REASONS:
        raise Failure(
            f"{key}: fallback reason {fallback.get('reason')!r} is not one of {FALLBACK_REASONS}"
        )
    detail = fallback.get("detail")
    if not isinstance(detail, str) or not detail.strip():
        raise Failure(f"{key}: fallback records no detail explaining why the mark is absent")


def check_no_orphans(marks_dir: Path, claimed: set[str]) -> None:
    on_disk = {
        str(path.relative_to(marks_dir))
        for path in marks_dir.rglob("*")
        if path.is_file() and path.name not in RECORD_FILES
    }
    undeclared = sorted(on_disk - claimed)
    if undeclared:
        raise Failure(
            "these files sit in VendorMarks but no record claims them: " + ", ".join(undeclared)
        )
    missing = sorted(claimed - on_disk)
    if missing:
        raise Failure("these recorded assets are absent: " + ", ".join(missing))


def check_bundle(root: Path, claimed: set[str]) -> None:
    """Every recorded mark is served by the resource bundle.

    A file that ships on disk and not in the bundle is a row that draws the
    text treatment on every host, which no record declared.
    """
    path = root / GRESOURCE
    if not path.is_file():
        raise Failure(f"the resource bundle is missing at {path}")

    served = served_marks(path.read_text(encoding="utf-8"))
    missing = sorted(claimed - served)
    if missing:
        raise Failure(
            f"these recorded marks are not served under {RESOURCE_PREFIX}: " + ", ".join(missing)
        )
    extra = sorted(served - claimed)
    if extra:
        raise Failure(
            f"the bundle serves marks no record claims under {RESOURCE_PREFIX}: " + ", ".join(extra)
        )


def served_marks(bundle: str) -> set[str]:
    """The aliases the bundle serves under the marks prefix.

    Read with a scanner rather than an XML parser: the bundle is this
    repository's own generated file, the entries are one shape, and a build
    gate that runs inside a container with the standard library alone should
    carry no parser of its own.
    """
    opening = f'<gresource prefix="{RESOURCE_PREFIX}">'
    start = bundle.find(opening)
    if start < 0:
        raise Failure(f"the resource bundle serves nothing under {RESOURCE_PREFIX}")
    end = bundle.find("</gresource>", start)
    if end < 0:
        raise Failure(f"the {RESOURCE_PREFIX} group in the resource bundle is not closed")

    group = bundle[start + len(opening):end]
    return set(re.findall(r'alias="([^"]+)"', group))


def load_roster(marks_dir: Path, record: dict) -> dict:
    """The vendored roster, proven to be the bytes PROVENANCE.json pinned."""
    path = marks_dir / ROSTER_FILE
    if not path.is_file():
        raise Failure(f"roster snapshot is missing at {path}")

    payload = path.read_bytes()
    pinned = (record.get("roster_source") or {}).get("roster_sha256")
    actual = sha256_of(payload)
    if actual != pinned:
        raise Failure(
            f"{ROSTER_FILE} hashes to {actual} but {PROVENANCE_FILE} pins {pinned}; "
            "re-vendor the roster rather than editing one record alone"
        )

    try:
        roster = json.loads(payload.decode("utf-8"))
    except json.JSONDecodeError as error:
        raise Failure(f"{ROSTER_FILE} is not valid JSON: {error}") from error
    if roster.get("schema_version") != ROSTER_SCHEMA_VERSION:
        raise Failure(
            f"roster schema {roster.get('schema_version')!r} is not supported "
            f"(this gate reads {ROSTER_SCHEMA_VERSION})"
        )
    for kind in KINDS:
        keys = roster.get(f"{kind}s")
        if not isinstance(keys, list) or not keys:
            raise Failure(f"{ROSTER_FILE} carries no {kind} roster")
    return roster


def check_roster(record: dict, roster: dict) -> None:
    """Every vendor the roster names has a record, and no record invents one."""
    source = record.get("roster_source") or {}
    marks = record["marks"]
    for kind in KINDS:
        keys = sorted(m["key"] for m in marks if m["kind"] == kind)
        expected = sorted(roster[f"{kind}s"])
        if keys != expected:
            missing = sorted(set(expected) - set(keys))
            extra = sorted(set(keys) - set(expected))
            raise Failure(
                f"{kind} roster drifted from {ROSTER_FILE}: "
                f"missing {missing}, unrecorded {extra}"
            )
        declared = source.get(f"{kind}_count")
        if len(keys) != declared:
            raise Failure(
                f"{len(keys)} {kind} records against a declared count of {declared}"
            )


def published_sections(root: Path) -> list[str]:
    """The section inventory the vendored fixtures publish."""
    path = root / FIXTURES
    if not path.is_file():
        raise Failure(f"the vendored fixtures are missing at {path}")

    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        record = json.loads(line)
        if record.get("method") != "settings.sections":
            continue
        return [section["id"] for section in record["response"]["result"]["sections"]]
    raise Failure(f"{path} publishes no settings.sections golden")


def check_fixture_rosters(root: Path, roster: dict) -> None:
    """Providers and channels, against what this repository actually renders.

    The rows the Providers and Channels panes draw come from the section
    inventory, so a vendor the daemon publishes with no mark recorded here is a
    row that would draw its text name, and a mark for a vendor the daemon has
    dropped is a file nothing can reach. Both are offline facts of this
    repository, so this check has no skip.
    """
    sections = published_sections(root)

    for kind, prefix in (("provider", "providers."), ("channel", "channels.")):
        published = sorted(
            section[len(prefix):] for section in sections if section.startswith(prefix)
        )
        recorded = sorted(roster[f"{kind}s"])
        if published != recorded:
            missing = sorted(set(published) - set(recorded))
            extra = sorted(set(recorded) - set(published))
            raise Failure(
                f"{kind} roster disagrees with the vendored fixtures: "
                f"the daemon publishes {published}, {ROSTER_FILE} carries {recorded} "
                f"(missing {missing}, unrecorded {extra})"
            )


def check_plugin_union(roster: dict, fermix_repo: Path | None) -> None:
    """The plugin roster, against the union of the two upstream sets.

    `index.json` is the catalog a machine installs FROM; `catalog.json` names
    the plugins the engine ships INSIDE itself, which `Registry.list` unions
    into every `plugins.list` answer. Reading only the first is how the plugins
    every install already has came to draw the text treatment.
    """
    if fermix_repo is None:
        print(
            "check_vendor_marks: plugin-union check skipped, no fermix checkout given "
            "(pass --fermix-repo <path> to run it). Completeness was checked against "
            f"{ROSTER_FILE}, and providers and channels against the vendored fixtures."
        )
        return

    source = roster["source"]["plugins"]
    index_path = fermix_repo / source["path"]
    bundled_path = fermix_repo / source["bundled"]["path"]
    for path in (index_path, bundled_path):
        if not path.is_file():
            raise Failure(f"the plugin catalog is missing at {path}")

    published = {
        entry["name"]
        for entry in json.loads(index_path.read_text(encoding="utf-8")).get("plugins", [])
    }
    bundled = set(json.loads(bundled_path.read_text(encoding="utf-8")).get("plugins", []))

    upstream = sorted(published | bundled)
    recorded = sorted(roster["plugins"])
    if upstream != recorded:
        missing = sorted(set(upstream) - set(recorded))
        extra = sorted(set(recorded) - set(upstream))
        raise Failure(
            "plugin roster drifted: fermix publishes and bundles "
            f"{upstream} and {ROSTER_FILE} carries {recorded} "
            f"(missing {missing}, unrecorded {extra})"
        )


def run_check(root: Path, fermix_repo: str | None) -> None:
    marks_dir = root / MARKS_DIR
    if not marks_dir.is_dir():
        raise Failure(f"no vendor marks at {marks_dir}")

    record = load(marks_dir)
    check_copy(marks_dir)
    marks = record.get("marks")
    if not isinstance(marks, list) or not marks:
        raise Failure("provenance record carries no marks")

    # Identity is (kind, key), not key alone: Discord and Slack are each both a
    # channel and a plugin, and a label is only ever spoken beside its own kind
    # of row.
    seen: set[tuple[str, str]] = set()
    labels: set[tuple[str, str]] = set()
    claimed: set[str] = set()
    for mark in marks:
        check_mark_fields(mark)
        identity = (mark["kind"], mark["key"])
        if identity in seen:
            raise Failure(f"{mark['kind']} {mark['key']}: recorded twice")
        seen.add(identity)
        label = (mark["kind"], mark["accessibility_label"])
        if label in labels:
            raise Failure(
                f"{mark['kind']} {mark['key']}: accessibility label "
                f"{mark['accessibility_label']!r} is not unique among {mark['kind']} marks"
            )
        labels.add(label)
        check_assets(mark, marks_dir, claimed)
        check_fallback(mark)

    check_no_orphans(marks_dir, claimed)
    check_bundle(root, claimed)
    roster = load_roster(marks_dir, record)
    check_roster(record, roster)
    check_fixture_rosters(root, roster)
    check_plugin_union(roster, resolve_repo(fermix_repo))

    shipped = sum(1 for m in marks if m["treatment"] == "vendor_mark")
    print(
        f"check_vendor_marks: {len(marks)} marks recorded, {shipped} ship a vendor asset, "
        f"{len(marks) - shipped} render as vendor text with a neutral symbol"
    )


def resolve_repo(override: str | None) -> Path | None:
    if not override:
        return None
    path = Path(override).expanduser().resolve()
    if not path.is_dir():
        raise Failure(f"--fermix-repo points at {path}, which is not a directory")
    return path


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        default=str(Path(__file__).resolve().parents[1]),
        help="the repository root (default: the one holding this script)",
    )
    parser.add_argument(
        "--fermix-repo",
        default=None,
        help="a fermix checkout, which turns on the plugin-union check",
    )
    arguments = parser.parse_args(argv)

    try:
        run_check(Path(arguments.root).resolve(), arguments.fermix_repo)
    except Failure as failure:
        print(f"check_vendor_marks: {failure}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
