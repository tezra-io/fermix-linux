#!/usr/bin/env python3
"""Write the engine pin and the version from a published engine release.

`.github/workflows/engine-release.yml` receives a `repository_dispatch` from
`tezra-io/fermix`'s release workflow and has to turn its payload into four
edited files. That payload arrives over the network from another repository, so
every field in it is untrusted input until this script has agreed with it, and
nothing it carries is ever interpolated into a shell command.

Three rules decide everything here.

**Derive, then compare.** The certificate identity and both asset names are
computed from the tag and from the repository the checked-in pin already names,
and the payload's own values are then required to equal them. A payload that
names another repository's workflow, or an asset the engine release does not
publish, is refused rather than written down. The repository itself is read from
`engine/PIN.json` and never from the payload, so a dispatch cannot redirect this
repository at an engine somebody else built.

**Refuse a shape, not just a value.** The payload must carry exactly the keys
this script understands, and each artifact entry exactly `asset` and `sha256`. A
key nobody reads is a key somebody added expecting it to be read.

**Write all four files or none.** The pin, the crate's version, the lock file's
entry for the crate and the metainfo's release entry are one fact in four
places, and a run that wrote two of them would leave a tree that every gate
downstream refuses for the wrong reason. Everything is validated first, the new
contents are computed in memory, and the writes happen last.

Nothing here verifies the engine archives. That is `scripts/verify_engine.sh`,
which the workflow runs against the pin this script wrote, before the commit is
made.

Usage:
  engine_bump.py --payload <file> [--root <dir>] [--date YYYY-MM-DD] [--check]

  --payload  the `client_payload` object, as JSON, on disk
  --root     the repository root. Defaults to this script's parent
  --date     the date the metainfo release entry carries. Defaults to today UTC
  --check    validate and print, and write nothing

On success it prints one JSON object to stdout: the version, the tag, the branch
the bump belongs on and the commit subject the tagging job looks for.
"""

import argparse
import datetime
import json
import re
import sys
from pathlib import Path

TARGETS = ("linux_x86_64", "linux_aarch64")
ARTIFACT_FIELDS = ("asset", "sha256")
PAYLOAD_FIELDS = (
    "tag",
    "version",
    "source_commit",
    "certificate_identity",
    "artifacts",
)
WORKFLOW = ".github/workflows/release.yml"
TAG = re.compile(r"v(\d+\.\d+\.\d+)")
COMMIT = re.compile(r"[0-9a-f]{40}")
DIGEST = re.compile(r"[0-9a-f]{64}")
DATE = re.compile(r"\d{4}-\d{2}-\d{2}")


def refuse(sentence):
    sys.exit(f"engine_bump: {sentence}")


def read_json(path, what):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except OSError as error:
        refuse(f"cannot read {what} at {path}: {error}")
    except json.JSONDecodeError as error:
        refuse(f"{what} at {path} is not valid JSON: {error}")


def check_payload(payload, repository):
    """Every field of the dispatch, against what this repository derives itself."""
    if not isinstance(payload, dict):
        refuse("the dispatch payload is not a JSON object")
    if sorted(payload) != sorted(PAYLOAD_FIELDS):
        refuse(
            "the dispatch payload must carry exactly "
            + ", ".join(sorted(PAYLOAD_FIELDS))
            + f", and it carries {', '.join(sorted(payload)) or 'nothing'}"
        )

    tag = payload["tag"]
    if not isinstance(tag, str) or not TAG.fullmatch(tag):
        refuse(f"the dispatch names tag {tag!r}, which is not an engine release tag")
    if payload["version"] != tag[1:]:
        refuse(
            f"the dispatch names version {payload['version']!r}, and {tag} carries "
            f"'{tag[1:]}'"
        )
    commit = payload["source_commit"]
    if not isinstance(commit, str) or not COMMIT.fullmatch(commit):
        refuse(f"the dispatch names source commit {commit!r}, which is not 40 hex")

    identity = f"https://github.com/{repository}/{WORKFLOW}@refs/tags/{tag}"
    if payload["certificate_identity"] != identity:
        refuse(
            f"the dispatch names certificate identity "
            f"{payload['certificate_identity']!r}, and {tag} in {repository} signs "
            f"as '{identity}'"
        )
    return tag, payload["version"], commit, identity


def check_artifacts(artifacts):
    """The two archives, named the way the engine release names them."""
    if not isinstance(artifacts, dict) or sorted(artifacts) != sorted(TARGETS):
        refuse("the dispatch must name exactly the targets " + ", ".join(TARGETS))
    for target in TARGETS:
        entry = artifacts[target]
        if not isinstance(entry, dict) or sorted(entry) != sorted(ARTIFACT_FIELDS):
            refuse(f"{target} must carry exactly " + " and ".join(ARTIFACT_FIELDS))
        wanted = f"fermix_app_engine_{target}.tar.gz"
        if entry["asset"] != wanted:
            refuse(
                f"{target} names asset {entry['asset']!r}, and the engine release "
                f"publishes '{wanted}'"
            )
        digest = entry["sha256"]
        if not isinstance(digest, str) or not DIGEST.fullmatch(digest):
            refuse(f"{target}'s sha256 {digest!r} is not a 64-character digest")
    return {
        target: {"asset": artifacts[target]["asset"], "sha256": artifacts[target]["sha256"]}
        for target in TARGETS
    }


def new_pin(pin, tag, version, commit, identity, artifacts):
    """The pinned pin, keeping every field the schema declares and its order."""
    written = dict(pin)
    written["tag"] = tag
    written["engine_version"] = version
    written["source_commit"] = commit
    written["certificate_identity"] = identity
    written["artifacts"] = artifacts
    # The note exists to tell a reader how to fill an unpinned pin in. A filled
    # pin is the record itself, and engine_pin.sh asks for the note only when
    # the pin is empty.
    written.pop("note", None)
    return json.dumps(written, indent=2) + "\n"


def replace_once(text, pattern, replacement, what):
    """Exactly one occurrence, or a refusal naming what was expected."""
    replaced, count = re.subn(pattern, replacement, text, count=2)
    if count != 1:
        refuse(f"{what} matched {count} times, and it has to match exactly once")
    return replaced


def new_cargo_toml(text, version):
    return replace_once(
        text,
        r'(?m)^version = "[^"]*"$',
        f'version = "{version}"',
        "the crate version in Cargo.toml",
    )


def new_cargo_lock(text, version):
    return replace_once(
        text,
        r'(?m)^(name = "fermix-desktop"\nversion = )"[^"]*"$',
        lambda match: f'{match.group(1)}"{version}"',
        "the crate entry in Cargo.lock",
    )


def new_metainfo(text, version, date):
    return replace_once(
        text,
        r'<release version="[^"]*" date="[^"]*"',
        f'<release version="{version}" date="{date}"',
        "the newest release entry in the metainfo",
    )


def parse_arguments(argv):
    parser = argparse.ArgumentParser(add_help=True)
    parser.add_argument("--payload", required=True)
    parser.add_argument("--root", default=None)
    parser.add_argument("--date", default=None)
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args(argv)
    if arguments.date is not None and not DATE.fullmatch(arguments.date):
        refuse(f"--date {arguments.date!r} is not YYYY-MM-DD")
    return arguments


def main(argv):
    arguments = parse_arguments(argv)
    root = Path(arguments.root) if arguments.root else Path(__file__).resolve().parents[1]
    date = arguments.date or datetime.datetime.now(datetime.timezone.utc).strftime(
        "%Y-%m-%d"
    )

    pin_path = root / "engine/PIN.json"
    cargo_path = root / "App/Fermix/Cargo.toml"
    lock_path = root / "App/Fermix/Cargo.lock"
    metainfo_path = root / "packaging/io.tezra.Fermix.metainfo.xml"

    pin = read_json(pin_path, "the engine pin")
    repository = pin.get("repository")
    if not isinstance(repository, str) or not repository:
        refuse("the checked-in engine pin names no repository to derive from")

    payload = read_json(arguments.payload, "the dispatch payload")
    tag, version, commit, identity = check_payload(payload, repository)
    artifacts = check_artifacts(payload["artifacts"])

    writes = {
        pin_path: new_pin(pin, tag, version, commit, identity, artifacts),
        cargo_path: new_cargo_toml(
            cargo_path.read_text(encoding="utf-8"), version
        ),
        lock_path: new_cargo_lock(lock_path.read_text(encoding="utf-8"), version),
        metainfo_path: new_metainfo(
            metainfo_path.read_text(encoding="utf-8"), version, date
        ),
    }
    if not arguments.check:
        for path, contents in writes.items():
            path.write_text(contents, encoding="utf-8")

    json.dump(
        {
            "tag": tag,
            "version": version,
            "branch": f"engine/{tag}",
            "subject": f"engine: pin {tag}",
        },
        sys.stdout,
    )
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
