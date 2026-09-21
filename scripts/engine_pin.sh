#!/usr/bin/env bash
#
# shellcheck disable=SC2034
# The constants in this file are read by the scripts that source it, which is
# the whole reason it exists; shellcheck cannot see across that boundary.
#
# The one owner of the engine pin: which engine release this desktop package
# carries.
#
# engine/PIN.json is the record, and three scripts need the same answers from it
# without any of them holding a second copy: fetch_engine.sh downloads the
# pinned archives, verify_engine.sh proves what arrived is what was pinned and
# unpacks it, and build_packages.sh asks whether there is a pin at all before it
# decides whether there is an engine to put in the package.
#
# Usage:  source "$(dirname "$0")/engine_pin.sh"
#         engine_pin_state <pin.json>                    -> pinned | unpinned
#         engine_pin_field <pin.json> <field>            -> repository,
#                            certificate_oidc_issuer, tag, engine_version,
#                            source_commit, certificate_identity. `version` is
#                            accepted as the older spelling of engine_version.
#         engine_pin_artifact_field <pin.json> <target> <asset|sha256>
#         engine_pin_architecture <target>               -> x86_64 | aarch64
#
# Schema 2. The pin names one signed archive per target,
# fermix_app_engine_linux_<arch>.tar.gz, rather than four distribution
# packages: the desktop package contains the engine rather than depending on it,
# so what is pinned is the engine build itself.
#
# Every reader refuses loudly rather than answering approximately: a pin that is
# neither fully filled nor fully empty, a tag that is not an engine release tag,
# a version that is not the tag's, a certificate identity that does not belong
# to the tag, an asset name the engine's release workflow does not publish, a
# target nobody builds for. A half-filled pin is the dangerous state, because it
# looks pinned to a glance and names an engine nothing can verify, so it is the
# one this file exists to stop.

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  echo "engine_pin.sh: must be sourced from bash" >&2
  exit 1
fi

# The two targets the engine's release workflow publishes app-engine archives
# for. Enumerated rather than globbed out of the pin, so a pin that grew a third
# key fails the reader instead of silently shipping whatever it found.
ENGINE_PIN_TARGETS=(linux_x86_64 linux_aarch64)

# The pin every caller means unless it is handed another one. Only the harnesses
# hand over another: they need a filled pin to prove the gates fire, and the
# checked-in record ships unpinned.
ENGINE_PIN_DEFAULT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/engine/PIN.json"

# The engine names its targets after the toolchain triple and the manifest names
# the machine alone. The two vocabularies have to be translated somewhere: here,
# once, as an exact-match allowlist, so a target nobody publishes is a refusal
# rather than a file name that happens to have no archive behind it.
engine_pin_architecture() {
  local target="${1:?engine_pin_architecture: <target> is required}"

  case "$target" in
    linux_x86_64) printf '%s\n' "x86_64" ;;
    linux_aarch64) printf '%s\n' "aarch64" ;;
    *)
      echo "engine_pin: no engine archive is published for '$target'" >&2
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

engine_pin_artifact_field() {
  engine_pin_read "${1:?engine_pin_artifact_field: <pin.json> is required}" \
    "artifact:${2:?engine_pin_artifact_field: <target> is required}:${3:?engine_pin_artifact_field: <field> is required}"
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
ARTIFACT_FIELDS = ("asset", "sha256")
ALWAYS_FILLED = ("repository", "certificate_oidc_issuer")
PINNED_ONLY = ("tag", "engine_version", "source_commit", "certificate_identity")
WORKFLOW = ".github/workflows/release.yml"


def refuse(sentence):
    sys.exit(f"engine_pin: {sentence}")


def expected_asset(target):
    # The engine's release workflow writes one archive per target, named after
    # the target itself. Derived rather than trusted: a pin that names an asset
    # the release does not publish is a download that fails at release time.
    return f"fermix_app_engine_{target}.tar.gz"


def load(path):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except OSError as error:
        refuse(f"cannot read the engine pin at {path}: {error}")
    except json.JSONDecodeError as error:
        refuse(f"the engine pin at {path} is not valid JSON: {error}")


def check_shape(pin, path):
    """Every key the schema declares is present and of the right kind."""
    if not isinstance(pin, dict):
        refuse(f"the engine pin at {path} is not a JSON object")
    if pin.get("schema_version") != 2:
        refuse(
            f"the engine pin declares schema_version {pin.get('schema_version')!r}, "
            "and this reader understands 2"
        )
    for field in ALWAYS_FILLED:
        if not isinstance(pin.get(field), str) or not pin[field]:
            refuse(f"the engine pin carries no {field}")
    for field in PINNED_ONLY:
        if field not in pin:
            refuse(f"the engine pin carries no {field} key at all")

    artifacts = pin.get("artifacts")
    if not isinstance(artifacts, dict) or sorted(artifacts) != sorted(TARGETS):
        refuse("the engine pin must carry exactly the targets " + ", ".join(TARGETS))
    for target in TARGETS:
        entry = artifacts[target]
        if not isinstance(entry, dict) or sorted(entry) != sorted(ARTIFACT_FIELDS):
            refuse(f"{target} must carry exactly " + " and ".join(ARTIFACT_FIELDS))
    return artifacts


def check_all_or_nothing(pin, artifacts):
    """Pinned or unpinned, with nothing in between.

    A pin with a tag and no digest, or a digest for one architecture only, names
    an engine that cannot be verified while reading as a pin to anyone who
    glances at it.
    """
    complete = list(PINNED_ONLY) + [
        f"{target}.{field}" for target in TARGETS for field in ARTIFACT_FIELDS
    ]
    filled = [field for field in PINNED_ONLY if pin[field] is not None]
    filled += [
        f"{target}.{field}"
        for target in TARGETS
        for field in ARTIFACT_FIELDS
        if artifacts[target][field] is not None
    ]
    if filled and sorted(filled) != sorted(complete):
        missing = ", ".join(field for field in complete if field not in filled)
        refuse(
            f"the engine pin is half filled: {missing} must be filled in too, "
            "or every pinned field must be null"
        )
    return bool(filled)


def check_pinned(pin, artifacts):
    """What a filled pin has to agree with: the tag, and itself."""
    tag = pin["tag"]
    if not isinstance(tag, str) or not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        refuse(f"the engine pin's tag {tag!r} is not an engine release tag (vX.Y.Z)")
    version = pin["engine_version"]
    if version != tag[1:]:
        refuse(
            f"the engine pin's engine_version is {version!r}, and {tag} carries '{tag[1:]}'"
        )
    commit = pin["source_commit"]
    if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{40}", commit):
        refuse(f"the engine pin's source_commit {commit!r} is not a 40-character commit")
    identity = f"https://github.com/{pin['repository']}/{WORKFLOW}@refs/tags/{tag}"
    if pin["certificate_identity"] != identity:
        refuse(
            f"the engine pin's certificate_identity is {pin['certificate_identity']!r}, "
            f"and {tag} in {pin['repository']} signs as '{identity}'"
        )
    for target in TARGETS:
        artifact = artifacts[target]
        wanted = expected_asset(target)
        if artifact["asset"] != wanted:
            refuse(
                f"{target} names asset {artifact['asset']!r}, "
                f"and the engine release publishes '{wanted}'"
            )
        digest = artifact["sha256"]
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            refuse(f"{target}'s sha256 {digest!r} is not a 64-character digest")


def check_unpinned(pin):
    # An unpinned pin has to say how to fill it in, because the note is the only
    # thing standing between a later reader and a half-filled pin. A filled one
    # needs no note: it is the record itself, and the release rail writes it.
    if not isinstance(pin.get("note"), str) or not pin["note"]:
        refuse("the engine pin is unpinned and carries no note saying how to fill it")


def answer(pin, artifacts, pinned, request):
    if request == "state":
        return "pinned" if pinned else "unpinned"
    if request in ALWAYS_FILLED:
        return pin[request]
    if not pinned:
        refuse(f"the engine pin is unpinned, so it names no {request}")
    # `version` is what the callers of schema 1 asked for, and it means the same
    # thing, so it keeps working rather than making every caller move at once.
    if request == "version":
        return pin["engine_version"]
    if request in PINNED_ONLY:
        return pin[request]
    if request.startswith("artifact:"):
        parts = request.split(":")
        if len(parts) != 3 or parts[1] not in TARGETS or parts[2] not in ARTIFACT_FIELDS:
            refuse(f"'{request}' is not an artifact field this pin carries")
        return artifacts[parts[1]][parts[2]]
    refuse(f"'{request}' is not a field of the engine pin")


pin = load(path)
artifacts = check_shape(pin, path)
pinned = check_all_or_nothing(pin, artifacts)
if pinned:
    check_pinned(pin, artifacts)
else:
    check_unpinned(pin)
print(answer(pin, artifacts, pinned, request))
PY
}
