#!/usr/bin/env bash
#
# shellcheck disable=SC2034
# The constants in this file are read by the scripts that source it, which is
# the whole reason it exists; shellcheck cannot see across that boundary.
#
# The one owner of the engine pin: which engine release this desktop version is
# paired with.
#
# engine/PIN.json is the record, and three scripts need the same answers from it
# without any of them holding a second copy: fetch_engine.sh downloads the
# pinned packages, verify_engine.sh proves what arrived is what was pinned, and
# the release rail asks whether there is a pin at all before it decides whether
# one release page can carry both halves of an install.
#
# Usage:  source "$(dirname "$0")/engine_pin.sh"
#         engine_pin_state <pin.json>                   -> pinned | unpinned
#         engine_pin_field <pin.json> <field>           -> repository,
#                            certificate_oidc_issuer, tag, source_commit,
#                            certificate_identity, version
#         engine_pin_package_field <pin.json> <target> <deb|rpm> <asset|sha256>
#         engine_pin_architecture <target> <deb|rpm>    -> amd64 | arm64 |
#                                                          x86_64 | aarch64
#
# Every reader refuses loudly rather than answering approximately: a pin that is
# neither fully filled nor fully empty, a tag that is not an engine release tag,
# a certificate identity that does not belong to the tag, an asset name the
# engine's release workflow does not publish, a target nobody builds for. A
# half-filled pin is the dangerous state, because it looks pinned to a glance
# and names an engine nothing can verify, so it is the one this file exists to
# stop.

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  echo "engine_pin.sh: must be sourced from bash" >&2
  exit 1
fi

# The two targets the engine's release workflow publishes Linux packages for,
# and the two families each target carries. Enumerated rather than globbed out
# of the pin, so a pin that grew a third key fails the reader instead of
# silently shipping whatever it found.
ENGINE_PIN_TARGETS=(linux_x86_64 linux_aarch64)
ENGINE_PIN_FORMATS=(deb rpm)

# The pin every caller means unless it is handed another one. Only the harnesses
# hand over another: they need a filled pin to prove the gates fire, and the
# checked-in record ships unpinned.
ENGINE_PIN_DEFAULT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/engine/PIN.json"

# The engine release names its targets after the toolchain triple, dpkg names
# the same machines amd64 and arm64, and rpm names them x86_64 and aarch64. The
# three vocabularies have to be translated somewhere: here, once, as an
# exact-match allowlist, so a target nobody publishes is a refusal rather than a
# file name that happens to have no package behind it.
engine_pin_architecture() {
  local target="${1:?engine_pin_architecture: <target> is required}"
  local format="${2:?engine_pin_architecture: <deb|rpm> is required}"

  case "$target:$format" in
    linux_x86_64:deb) printf '%s\n' "amd64" ;;
    linux_aarch64:deb) printf '%s\n' "arm64" ;;
    linux_x86_64:rpm) printf '%s\n' "x86_64" ;;
    linux_aarch64:rpm) printf '%s\n' "aarch64" ;;
    *)
      echo "engine_pin: no $format package is published for '$target'" >&2
      return 1
      ;;
  esac
}

engine_pin_state() {
  engine_pin_read "${1:?engine_pin_state: <pin.json> is required}" state
}

engine_pin_field() {
  engine_pin_read "${1:?engine_pin_field: <pin.json> is required}" \
    "${2:?engine_pin_field: <field> is required}"
}

engine_pin_package_field() {
  engine_pin_read "${1:?engine_pin_package_field: <pin.json> is required}" \
    "package:${2:?engine_pin_package_field: <target> is required}:${3:?engine_pin_package_field: <format> is required}:${4:?engine_pin_package_field: <field> is required}"
}

# The whole pin is validated on every read, not only on the first one.
#
# Reading one field at a time means each read is its own process, and a
# validator that ran only for `state` would let a caller that asks for a tag
# straight away walk past every consistency check. Validating each time costs a
# python start and makes every answer mean the same thing.
engine_pin_read() {
  python3 - "$1" "$2" <<'PY'
import json
import re
import sys

path, request = sys.argv[1], sys.argv[2]

TARGETS = ("linux_x86_64", "linux_aarch64")
FORMATS = ("deb", "rpm")
PACKAGE_FIELDS = ("asset", "sha256")
ALWAYS_FILLED = ("repository", "certificate_oidc_issuer")
PINNED_ONLY = ("tag", "source_commit", "certificate_identity")
WORKFLOW = ".github/workflows/release.yml"

# dpkg and rpm name the same machine differently, and the engine's own release
# workflow writes both file names, so the expected asset name is derived rather
# than trusted: a pin that names an asset the release does not publish is a
# download that fails at release time.
ARCHITECTURES = {
    ("linux_x86_64", "deb"): "amd64",
    ("linux_aarch64", "deb"): "arm64",
    ("linux_x86_64", "rpm"): "x86_64",
    ("linux_aarch64", "rpm"): "aarch64",
}


def refuse(sentence):
    sys.exit(f"engine_pin: {sentence}")


def expected_asset(target, fmt, version):
    architecture = ARCHITECTURES[(target, fmt)]
    if fmt == "deb":
        return f"fermix_{version}_{architecture}.deb"
    # nFPM's rpm packager defaults an empty release to 1, which is what the
    # engine package ships, so the release number is part of the file name.
    return f"fermix-{version}-1.{architecture}.rpm"


try:
    with open(path, encoding="utf-8") as handle:
        pin = json.load(handle)
except OSError as error:
    refuse(f"cannot read the engine pin at {path}: {error}")
except json.JSONDecodeError as error:
    refuse(f"the engine pin at {path} is not valid JSON: {error}")

if not isinstance(pin, dict):
    refuse(f"the engine pin at {path} is not a JSON object")
if pin.get("schema_version") != 1:
    refuse(
        f"the engine pin declares schema_version {pin.get('schema_version')!r}, "
        "and this reader understands 1"
    )
for field in ALWAYS_FILLED:
    if not isinstance(pin.get(field), str) or not pin[field]:
        refuse(f"the engine pin carries no {field}")
if not isinstance(pin.get("note"), str) or not pin["note"]:
    refuse("the engine pin carries no note saying how to fill it")

packages = pin.get("packages")
if not isinstance(packages, dict) or sorted(packages) != sorted(TARGETS):
    refuse("the engine pin must carry exactly the targets " + ", ".join(TARGETS))
for target in TARGETS:
    entry = packages[target]
    if not isinstance(entry, dict) or sorted(entry) != sorted(FORMATS):
        refuse(f"target {target} must carry exactly " + " and ".join(FORMATS))
    for fmt in FORMATS:
        package = entry[fmt]
        if not isinstance(package, dict) or sorted(package) != sorted(PACKAGE_FIELDS):
            refuse(
                f"{target}'s {fmt} must carry exactly " + " and ".join(PACKAGE_FIELDS)
            )

# Pinned or unpinned, with nothing in between. A pin with a tag and no digest,
# or a digest for one architecture only, names an engine that cannot be verified
# while reading as a pin to anyone who glances at it.
complete = list(PINNED_ONLY) + [
    f"{target}.{fmt}.{field}"
    for target in TARGETS
    for fmt in FORMATS
    for field in PACKAGE_FIELDS
]
filled = [field for field in PINNED_ONLY if pin[field] is not None]
filled += [
    f"{target}.{fmt}.{field}"
    for target in TARGETS
    for fmt in FORMATS
    for field in PACKAGE_FIELDS
    if packages[target][fmt][field] is not None
]
if filled and sorted(filled) != sorted(complete):
    missing = ", ".join(field for field in complete if field not in filled)
    refuse(
        f"the engine pin is half filled: {missing} must be filled in too, "
        "or every pinned field must be null"
    )

pinned = bool(filled)

if pinned:
    tag = pin["tag"]
    if not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        refuse(f"the engine pin's tag '{tag}' is not an engine release tag (vX.Y.Z)")
    version = tag[1:]
    commit = pin["source_commit"]
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        refuse(f"the engine pin's source_commit '{commit}' is not a 40-character commit")
    identity = f"https://github.com/{pin['repository']}/{WORKFLOW}@refs/tags/{tag}"
    if pin["certificate_identity"] != identity:
        refuse(
            f"the engine pin's certificate_identity is '{pin['certificate_identity']}', "
            f"and {tag} in {pin['repository']} signs as '{identity}'"
        )
    for target in TARGETS:
        for fmt in FORMATS:
            package = packages[target][fmt]
            wanted = expected_asset(target, fmt, version)
            if package["asset"] != wanted:
                refuse(
                    f"{target}'s {fmt} names asset '{package['asset']}', "
                    f"and the engine release publishes '{wanted}'"
                )
            digest = package["sha256"]
            if not re.fullmatch(r"[0-9a-f]{64}", digest):
                refuse(
                    f"{target}'s {fmt} sha256 '{digest}' is not a 64-character digest"
                )

if request == "state":
    print("pinned" if pinned else "unpinned")
    sys.exit(0)
if request in ALWAYS_FILLED:
    print(pin[request])
    sys.exit(0)
if not pinned:
    refuse(f"the engine pin is unpinned, so it names no {request}")
if request == "version":
    # The engine version a tag carries is the tag without its v, and it is also
    # this desktop package's own version, because both packages of a version are
    # published together and each pins the other exactly.
    print(pin["tag"][1:])
    sys.exit(0)
if request in PINNED_ONLY:
    print(pin[request])
    sys.exit(0)
if request.startswith("package:"):
    parts = request.split(":")
    if (
        len(parts) != 4
        or parts[1] not in TARGETS
        or parts[2] not in FORMATS
        or parts[3] not in PACKAGE_FIELDS
    ):
        refuse(f"'{request}' is not a package field this pin carries")
    print(packages[parts[1]][parts[2]][parts[3]])
    sys.exit(0)
refuse(f"'{request}' is not a field of the engine pin")
PY
}
