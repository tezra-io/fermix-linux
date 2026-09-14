#!/usr/bin/env bash
#
# Exercise the engine pin's readers and verify_engine.sh's refusals, offline.
#
# The pin exists to stop a release from shipping beside an engine nobody
# checked, and every one of its failure modes is quiet: a half-filled pin reads
# as a pin, a wrong digest reads as a download, an unsigned package reads as a
# package. So each refusal is driven here, against a fixture pin and fake assets
# in a throwaway directory, with a stub cosign that answers what the case needs.
#
# Nothing here reaches the network, the real pin's engine, or a real cosign.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/verify-engine-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

TAG="v0.11.0"
VERSION="0.11.0"
COMMIT="1111111111111111111111111111111111111111"
IDENTITY="https://github.com/tezra-io/fermix/.github/workflows/release.yml@refs/tags/$TAG"

fail() {
  echo "verify_engine_test: $*" >&2
  exit 1
}

digest_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

# A cosign that always says yes, and one that always says no. The signature
# check and the digest check are separate refusals, and a test that could not
# tell them apart would pass while one of them was broken.
stub_cosign() {
  local path="$WORK/cosign-$1"
  {
    printf '#!/bin/sh\n'
    printf 'exit %s\n' "$2"
  } > "$path"
  chmod +x "$path"
  printf '%s\n' "$path"
}

ACCEPTS="$(stub_cosign accepts 0)"
REFUSES="$(stub_cosign refuses 1)"

# One asset set: four packages, each with its three sidecars, in a directory of
# this test's own. The pin is then written to agree with them.
make_assets() {
  local dir="$1"
  mkdir -p "$dir"
  for asset in \
    "fermix_${VERSION}_amd64.deb" \
    "fermix_${VERSION}_arm64.deb" \
    "fermix-${VERSION}-1.x86_64.rpm" \
    "fermix-${VERSION}-1.aarch64.rpm"; do
    printf 'not really a package, but it hashes like one: %s\n' "$asset" > "$dir/$asset"
    digest_of "$dir/$asset" > "$dir/$asset.sha256"
    printf 'signature\n' > "$dir/$asset.sig"
    printf 'certificate\n' > "$dir/$asset.pem"
  done
}

# The pin that matches those assets. `--break <what>` writes one that does not.
write_pin() {
  local path="$1" assets="$2" broken="${3:-}"
  python3 - "$path" "$assets" "$broken" "$TAG" "$VERSION" "$COMMIT" "$IDENTITY" <<'PY'
import hashlib
import json
import sys

path, assets, broken, tag, version, commit, identity = sys.argv[1:8]


def digest(name):
    with open(f"{assets}/{name}", "rb") as handle:
        return hashlib.sha256(handle.read()).hexdigest()


names = {
    ("linux_x86_64", "deb"): f"fermix_{version}_amd64.deb",
    ("linux_aarch64", "deb"): f"fermix_{version}_arm64.deb",
    ("linux_x86_64", "rpm"): f"fermix-{version}-1.x86_64.rpm",
    ("linux_aarch64", "rpm"): f"fermix-{version}-1.aarch64.rpm",
}

packages = {
    target: {
        fmt: {"asset": names[(target, fmt)], "sha256": digest(names[(target, fmt)])}
        for fmt in ("deb", "rpm")
    }
    for target in ("linux_x86_64", "linux_aarch64")
}

pin = {
    "schema_version": 1,
    "repository": "tezra-io/fermix",
    "certificate_oidc_issuer": "https://token.actions.githubusercontent.com",
    "tag": tag,
    "source_commit": commit,
    "certificate_identity": identity,
    "packages": packages,
    "note": "a fixture pin, written by scripts/verify_engine_test.sh",
}

if broken == "half":
    pin["packages"]["linux_aarch64"]["rpm"]["sha256"] = None
elif broken == "tag":
    pin["tag"] = "0.11.0"
elif broken == "commit":
    pin["source_commit"] = "1111"
elif broken == "identity":
    pin["certificate_identity"] = identity.replace(tag, "v9.9.9")
elif broken == "asset":
    pin["packages"]["linux_x86_64"]["deb"]["asset"] = f"fermix_{version}_x86_64.deb"
elif broken == "digest":
    pin["packages"]["linux_x86_64"]["deb"]["sha256"] = "0" * 64
elif broken == "schema":
    pin["schema_version"] = 2
elif broken == "note":
    del pin["note"]
elif broken == "targets":
    pin["packages"]["linux_riscv64"] = pin["packages"]["linux_x86_64"]
elif broken == "unpinned":
    pin["tag"] = None
    pin["source_commit"] = None
    pin["certificate_identity"] = None
    for target in pin["packages"].values():
        for package in target.values():
            package["asset"] = None
            package["sha256"] = None

with open(path, "w", encoding="utf-8") as handle:
    json.dump(pin, handle, indent=2)
PY
}

expect_refusal() {
  local what="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    fail "$what was accepted"
  fi
  echo "  refused: $what"
}

echo "verify_engine_test: the scripts parse"
for script in engine_pin.sh fetch_engine.sh verify_engine.sh; do
  bash -n "$ROOT_DIR/scripts/$script" || fail "scripts/$script does not parse"
done
echo "  ok: shell syntax"

echo "verify_engine_test: the pin this repository ships"
STATE="$(bash -c "source '$ROOT_DIR/scripts/engine_pin.sh'; engine_pin_state '$ROOT_DIR/engine/PIN.json'")" ||
  fail "the checked-in pin does not read"
[ "$STATE" = "unpinned" ] ||
  fail "the checked-in pin reports $STATE, and this slice ships it unpinned"
echo "  ok: unpinned, and it says so"

expect_refusal "verifying against an unpinned pin" \
  bash "$ROOT_DIR/scripts/verify_engine.sh" "$ROOT_DIR/engine/PIN.json" "$WORK" --cosign "$ACCEPTS"
expect_refusal "downloading against an unpinned pin" \
  bash "$ROOT_DIR/scripts/fetch_engine.sh" "$ROOT_DIR/engine/PIN.json" "$WORK/download"

echo "verify_engine_test: a filled pin and the assets it names"
ASSETS="$WORK/assets"
make_assets "$ASSETS"
write_pin "$WORK/pin.json" "$ASSETS"

bash "$ROOT_DIR/scripts/verify_engine.sh" "$WORK/pin.json" "$ASSETS" --cosign "$ACCEPTS" >/dev/null ||
  fail "a pin that agrees with its assets was refused"
echo "  accepted: four packages whose digests and signatures agree with the pin"

echo "verify_engine_test: refusals in the record"
for broken in half tag commit identity asset digest schema note targets; do
  write_pin "$WORK/broken-$broken.json" "$ASSETS" "$broken"
  expect_refusal "a pin whose $broken is wrong" \
    bash "$ROOT_DIR/scripts/verify_engine.sh" "$WORK/broken-$broken.json" "$ASSETS" --cosign "$ACCEPTS"
done

echo "verify_engine_test: refusals in what arrived"

TAMPERED="$WORK/tampered"
cp -R "$ASSETS" "$TAMPERED"
printf 'one more byte\n' >> "$TAMPERED/fermix_${VERSION}_arm64.deb"
expect_refusal "a package whose bytes changed after it was pinned" \
  bash "$ROOT_DIR/scripts/verify_engine.sh" "$WORK/pin.json" "$TAMPERED" --cosign "$ACCEPTS"

# The sidecar the release wrote agrees with the tampered file, and the pin does
# not. This is the case the comment in verify_engine.sh is about.
digest_of "$TAMPERED/fermix_${VERSION}_arm64.deb" > "$TAMPERED/fermix_${VERSION}_arm64.deb.sha256"
expect_refusal "a package whose own sidecar agrees with it and the pin does not" \
  bash "$ROOT_DIR/scripts/verify_engine.sh" "$WORK/pin.json" "$TAMPERED" --cosign "$ACCEPTS"

UNSIGNED="$WORK/unsigned"
cp -R "$ASSETS" "$UNSIGNED"
rm "$UNSIGNED/fermix-${VERSION}-1.aarch64.rpm.sig"
expect_refusal "a package with no signature beside it" \
  bash "$ROOT_DIR/scripts/verify_engine.sh" "$WORK/pin.json" "$UNSIGNED" --cosign "$ACCEPTS"

expect_refusal "a package whose signature does not verify" \
  bash "$ROOT_DIR/scripts/verify_engine.sh" "$WORK/pin.json" "$ASSETS" --cosign "$REFUSES"

INCOMPLETE="$WORK/incomplete"
mkdir -p "$INCOMPLETE"
cp "$ASSETS/fermix_${VERSION}_amd64.deb"* "$INCOMPLETE/"
expect_refusal "a download that carries one family and not the other" \
  bash "$ROOT_DIR/scripts/verify_engine.sh" "$WORK/pin.json" "$INCOMPLETE" --cosign "$ACCEPTS"

expect_refusal "a verifier that is not executable" \
  bash "$ROOT_DIR/scripts/verify_engine.sh" "$WORK/pin.json" "$ASSETS" --cosign "$WORK/no-such-cosign"

expect_refusal "an unknown argument" \
  bash "$ROOT_DIR/scripts/verify_engine.sh" "$WORK/pin.json" "$ASSETS" --nonsense

echo "verify_engine_test: downloading refuses before it reaches the network"
mkdir -p "$WORK/occupied"
touch "$WORK/occupied/left-over-from-an-earlier-run"
expect_refusal "a download directory that already holds files" \
  bash "$ROOT_DIR/scripts/fetch_engine.sh" "$WORK/pin.json" "$WORK/occupied"

echo "verify_engine_test: every refusal fired"
