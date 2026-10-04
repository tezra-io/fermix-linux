#!/usr/bin/env python3
"""The engine pin: the one signed release of the Fermix engine that fermix-desktop carries.

desktop/engine/PIN.json names a release of tezra-io/fermix: its tag and version, the commit the tag
points to, the identity and issuer cosign must find in each package's certificate, and for amd64
and arm64 the asset name and sha256 of the deb and of the rpm. fetch_engine.sh downloads what it
names and verify_engine.py refuses anything that disagrees with it.

    engine_pin.py [--pin <file>] <tag>           write the pin of that engine release, through gh
    engine_pin.py [--pin <file>] --check         refuse unless the pin is complete and consistent
    engine_pin.py [--pin <file>] --get <field>   print one field of a checked pin

<field> is repository, tag, engine_version, source_commit, certificate_oidc_issuer,
certificate_identity, or <amd64|arm64>.<deb|rpm>.<asset|sha256>. --pin defaults to
desktop/engine/PIN.json.

A pin is complete or it is refused. A half-filled one reads as a pin at a glance and names an engine
nothing can verify, so every read checks the whole file and the writer only writes a pin it has
checked. What can be derived is checked against its source: the version and the certificate
identity against the tag, each asset name against the version. The repository and the issuer are
fixed, because a pin naming a fork would make the fork's signatures good.

The writer takes each digest from the release's own .sha256 sidecar, and refuses a sidecar that
disagrees with the digest GitHub computed over the uploaded asset. It refuses a draft, a prerelease,
and a release missing any of the eight packages or any of their .sha256, .sig and .pem sidecars.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

SCHEMA_VERSION = 3
REPOSITORY = "tezra-io/fermix"
ISSUER = "https://token.actions.githubusercontent.com"
SCALARS = (
    "repository",
    "tag",
    "engine_version",
    "source_commit",
    "certificate_oidc_issuer",
    "certificate_identity",
)
# dpkg's name for each architecture, and rpm's.
ARCHES = {"amd64": "x86_64", "arm64": "aarch64"}
FORMATS = ("deb", "rpm")
FIELDS = ("asset", "sha256")
PACKAGES = tuple((arch, fmt) for arch in ARCHES for fmt in FORMATS)
LEAVES = SCALARS + tuple(
    f"packages.{arch}.{fmt}.{field}" for arch, fmt in PACKAGES for field in FIELDS
)
SIDECARS = ("", ".sha256", ".sig", ".pem")
SIDECAR_FIELDS = 2
TAG = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+")
COMMIT = re.compile(r"[0-9a-f]{40}")
SHA256 = re.compile(r"[0-9a-f]{64}")
GITHUB_DIGEST = re.compile(r"sha256:[0-9a-f]{64}")
# An annotated tag names a tag object, which names the commit. Git allows tags of tags, so the walk
# is bounded rather than trusted to end.
MAX_TAG_HOPS = 4
# A gh call that has not answered by then has failed, rather than hanging the pin.
GH_TIMEOUT_SECONDS = 300
DEFAULT_PIN = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "engine", "PIN.json"
)
USAGE = "engine_pin.py [--pin <file>] <tag> | --check | --get <field>"


class Refusal(Exception):
    """A refusal; its message is the sentence printed after "engine_pin: "."""


class Parser(argparse.ArgumentParser):
    """argparse, refusing a malformed command line the way every other mistake here is refused."""

    def error(self, message):
        raise Refusal(f"usage: {USAGE} ({message})")


def asset_name(version, arch, fmt):
    if fmt == "deb":
        return f"fermix_{version}_{arch}.deb"
    # nFPM's rpm packager turns the engine's empty release into 1.
    return f"fermix-{version}-1.{ARCHES[arch]}.rpm"


def identity_of(tag):
    return f"https://github.com/{REPOSITORY}/.github/workflows/release.yml@refs/tags/{tag}"


def load_json(path, what):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except OSError as error:
        raise Refusal(f"cannot read {what} at {path}: {error.strerror}") from error
    except json.JSONDecodeError as error:
        raise Refusal(f"{what} at {path} is not valid JSON: {error}") from error


def child(node, key):
    return node.get(key) if isinstance(node, dict) else None


def leaf(pin, dotted):
    node = pin
    for key in dotted.split("."):
        node = child(node, key)
    return node


def unexpected_keys(pin):
    packages = child(pin, "packages")
    nodes = [
        ("", pin, (*SCALARS, "schema_version", "packages")),
        ("packages.", packages, tuple(ARCHES)),
    ]
    nodes += [(f"packages.{arch}.", child(packages, arch), FORMATS) for arch in ARCHES]
    nodes += [
        (f"packages.{arch}.{fmt}.", child(child(packages, arch), fmt), FIELDS)
        for arch, fmt in PACKAGES
    ]
    return [
        prefix + key
        for prefix, node, allowed in nodes
        if isinstance(node, dict)
        for key in node
        if key not in allowed
    ]


def check(pin, what):
    if not isinstance(pin, dict):
        raise Refusal(f"{what} is not a JSON object")
    if pin.get("schema_version") != SCHEMA_VERSION:
        raise Refusal(
            f"{what} declares schema_version {pin.get('schema_version')!r}, "
            f"and this reader understands {SCHEMA_VERSION}"
        )
    unexpected = unexpected_keys(pin)
    if unexpected:
        raise Refusal(f"{what} carries unexpected keys: {', '.join(unexpected)}")
    missing = [
        name for name in LEAVES if not isinstance(leaf(pin, name), str) or not leaf(pin, name)
    ]
    if missing:
        raise Refusal(
            f"{what} is half filled: {', '.join(missing)} must be filled in too. "
            "Write it whole with engine_pin.py <tag>"
        )
    check_scalars(pin, what)
    check_packages(pin, what)


def check_scalars(pin, what):
    tag = pin["tag"]
    if not TAG.fullmatch(tag):
        raise Refusal(f"{what} names the tag {tag}, which is not an engine release tag (vX.Y.Z)")
    if pin["repository"] != REPOSITORY:
        raise Refusal(
            f"{what} names {pin['repository']}, and the engine is released from {REPOSITORY}"
        )
    if pin["certificate_oidc_issuer"] != ISSUER:
        raise Refusal(
            f"{what} names the issuer {pin['certificate_oidc_issuer']}, "
            f"and the engine's is {ISSUER}"
        )
    if pin["engine_version"] != tag[1:]:
        raise Refusal(
            f"{what} has engine_version {pin['engine_version']}, and {tag} carries {tag[1:]}"
        )
    if not COMMIT.fullmatch(pin["source_commit"]):
        raise Refusal(
            f"{what} has source_commit {pin['source_commit']}, which is not a 40-character commit"
        )
    if pin["certificate_identity"] != identity_of(tag):
        raise Refusal(
            f"{what} has certificate_identity {pin['certificate_identity']}, "
            f"and {tag} is signed as {identity_of(tag)}"
        )


def check_packages(pin, what):
    for arch, fmt in PACKAGES:
        entry = pin["packages"][arch][fmt]
        wanted = asset_name(pin["engine_version"], arch, fmt)
        if entry["asset"] != wanted:
            raise Refusal(
                f"{what} names {entry['asset']} for {arch}.{fmt}, "
                f"and {pin['tag']} publishes {wanted}"
            )
        if not SHA256.fullmatch(entry["sha256"]):
            raise Refusal(
                f"{what} has {arch}.{fmt}.sha256 {entry['sha256']}, which is not a sha256 digest"
            )


def read_pin(path):
    pin = load_json(path, "the engine pin")
    check(pin, f"the engine pin at {path}")
    return pin


def get(pin, field):
    name = field if field in SCALARS else f"packages.{field}"
    if name not in LEAVES:
        raise Refusal(f"'{field}' is not a field of the engine pin")
    return leaf(pin, name)


def gh(arguments, failure):
    """Runs gh once and returns what it printed; its own complaint goes to stderr as it is."""
    try:
        result = subprocess.run(
            ["gh", *arguments],
            check=False,
            stdout=subprocess.PIPE,
            timeout=GH_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        raise Refusal(
            f"{failure}: gh did not answer within {GH_TIMEOUT_SECONDS} seconds"
        ) from error
    if result.returncode != 0:
        raise Refusal(failure)
    return result.stdout


def gh_json(path, failure):
    answer = gh(["api", path], failure)
    try:
        return json.loads(answer)
    except json.JSONDecodeError as error:
        raise Refusal(f"GitHub's answer to {path} is not valid JSON: {error}") from error


def release_sidecars(release, tag):
    """The names of the eight packages' .sha256 sidecars, once the release is whole."""
    if child(release, "tag_name") != tag:
        raise Refusal(f"GitHub answered for {child(release, 'tag_name')!r} when asked for {tag}")
    if release.get("draft") is not False:
        raise Refusal(f"{tag} is a draft release, and only a published release can be pinned")
    if release.get("prerelease") is not False:
        raise Refusal(f"{tag} is a prerelease, and only a release can be pinned")
    published = {child(asset, "name"): asset for asset in release.get("assets") or []}
    version = tag[1:]
    wanted = [asset_name(version, a, f) + suffix for a, f in PACKAGES for suffix in SIDECARS]
    missing = [name for name in wanted if name not in published]
    if missing:
        raise Refusal(f"the release {tag} lacks {', '.join(missing)}")
    for arch, fmt in PACKAGES:
        name = asset_name(version, arch, fmt)
        if not GITHUB_DIGEST.fullmatch(str(child(published[name], "digest"))):
            raise Refusal(
                f"GitHub records no sha256 digest for {name}, so its sidecar cannot be checked"
            )
    return [asset_name(version, a, f) + ".sha256" for a, f in PACKAGES]


def sidecar_digest(path, name):
    try:
        with open(path, encoding="utf-8") as handle:
            text = handle.read()
    except (OSError, UnicodeDecodeError) as error:
        raise Refusal(f"cannot read {name}.sha256: {error}") from error
    fields = text.split()
    if len(fields) != SIDECAR_FIELDS or text.count("\n") != 1 or not text.endswith("\n"):
        raise Refusal(f"{name}.sha256 is not one '<sha256>  <name>' line")
    if fields[1] != name:
        raise Refusal(f"{name}.sha256 names {fields[1]}, not {name}")
    if not SHA256.fullmatch(fields[0]):
        raise Refusal(f"{name}.sha256 records {fields[0]}, which is not a sha256 digest")
    return fields[0]


def compose(release, sidecars, tag, commit):
    published = {child(asset, "name"): child(asset, "digest") for asset in release["assets"]}
    packages = {arch: {} for arch in ARCHES}
    for arch, fmt in PACKAGES:
        name = asset_name(tag[1:], arch, fmt)
        digest = sidecar_digest(os.path.join(sidecars, f"{name}.sha256"), name)
        if published[name] != f"sha256:{digest}":
            raise Refusal(
                f"{name}.sha256 records {digest}, "
                f"and GitHub computed {published[name]} over the upload"
            )
        packages[arch][fmt] = {"asset": name, "sha256": digest}
    pin = {
        "schema_version": SCHEMA_VERSION,
        "repository": REPOSITORY,
        "tag": tag,
        "engine_version": tag[1:],
        "source_commit": commit,
        "certificate_oidc_issuer": ISSUER,
        "certificate_identity": identity_of(tag),
        "packages": packages,
    }
    check(pin, f"the pin of {tag}")
    return pin


def git_object(answer):
    kind = child(child(answer, "object"), "type")
    sha = child(child(answer, "object"), "sha")
    if kind not in ("tag", "commit") or not COMMIT.fullmatch(str(sha)):
        raise Refusal(
            f"GitHub's answer names a {kind!r} {sha!r}, and a tag names a tag object or a commit"
        )
    return kind, sha


def tag_commit(tag):
    """Follows the tag to its commit through GitHub, one tag object at a time."""
    kind, sha = git_object(
        gh_json(
            f"repos/{REPOSITORY}/git/ref/tags/{tag}",
            f"GitHub cannot resolve the tag {tag} of {REPOSITORY}",
        )
    )
    for _ in range(MAX_TAG_HOPS):
        if kind == "commit":
            return sha
        kind, sha = git_object(
            gh_json(
                f"repos/{REPOSITORY}/git/tags/{sha}",
                f"GitHub cannot read the tag object {sha} of {tag}",
            )
        )
    if kind == "commit":
        return sha
    raise Refusal(f"the tag {tag} does not reach a commit within {MAX_TAG_HOPS} tag objects")


def install(pin, path):
    """Replaces the pin in one rename, so a reader never sees half a file."""
    handle, staged = tempfile.mkstemp(
        prefix=".PIN.json.", dir=os.path.dirname(os.path.abspath(path))
    )
    try:
        with os.fdopen(handle, "w", encoding="utf-8") as out:
            out.write(json.dumps(pin, indent=2) + "\n")
        os.chmod(staged, 0o644)
        os.replace(staged, path)
    finally:
        if os.path.exists(staged):
            os.remove(staged)


def write_pin(path, tag):
    """Reads the release and its tag from GitHub, and writes the pin only once it checks whole."""
    if not TAG.fullmatch(tag):
        raise Refusal(f"{tag} is not an engine release tag (vX.Y.Z)")
    if shutil.which("gh") is None:
        raise Refusal(f"the GitHub CLI is required to read the release {tag}")
    if not os.path.isdir(os.path.dirname(os.path.abspath(path))):
        raise Refusal(f"there is no directory for the pin at {path}")
    release = gh_json(
        f"repos/{REPOSITORY}/releases/tags/{tag}", f"GitHub has no release {tag} in {REPOSITORY}"
    )
    sidecars = release_sidecars(release, tag)
    with tempfile.TemporaryDirectory() as work:
        for name in sidecars:
            gh(
                ["release", "download", tag,
                 "--repo", REPOSITORY, "--pattern", name, "--dir", work],
                f"cannot download {name} from {REPOSITORY} {tag}",
            )  # fmt: skip
        commit = tag_commit(tag)
        pin = compose(release, work, tag, commit)
    install(pin, path)
    print(f"engine_pin: {path} pins {REPOSITORY} {tag} at {commit[:12]}")


def parse_arguments(argv):
    parser = Parser(prog="engine_pin.py", add_help=False)
    parser.add_argument("--pin", default=DEFAULT_PIN)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--get", metavar="FIELD")
    parser.add_argument("tag", nargs="?")
    arguments = parser.parse_intermixed_args(argv)
    if [arguments.check, arguments.get is not None, arguments.tag is not None].count(True) != 1:
        raise Refusal(f"usage: {USAGE}")
    return arguments


def dispatch(arguments):
    if arguments.check:
        read_pin(arguments.pin)
    elif arguments.get is not None:
        print(get(read_pin(arguments.pin), arguments.get))
    else:
        write_pin(arguments.pin, arguments.tag)


def main(argv):
    try:
        dispatch(parse_arguments(argv))
    except Refusal as refusal:
        print(f"engine_pin: {refusal}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
