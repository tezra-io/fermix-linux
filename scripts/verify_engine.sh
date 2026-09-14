#!/usr/bin/env bash
#
# Prove that the downloaded engine packages are the engine the pin names.
#
# This is the gate between "four files with the right names arrived" and "these
# are the packages we decided to publish beside our own". Two things have to
# agree for each of them, and each disagreement is its own sentence:
#
#   1. the file's sha256 is the digest engine/PIN.json records. The .sha256
#      sidecar beside the package is NOT what is checked: it is written by the
#      same release that wrote the package, so a re-cut or replaced release
#      carries a sidecar that agrees with itself and with nothing we decided.
#      The pin is the only authority here.
#   2. cosign verifies the detached signature against the pinned certificate
#      identity and OIDC issuer, so the package provably came out of the engine
#      repository's release workflow at that tag.
#
# No network, no token, no `gh`: everything it needs is the pin and the files
# scripts/fetch_engine.sh already put on disk. That is what makes every refusal
# above provable offline in scripts/verify_engine_test.sh.
#
# Usage: verify_engine.sh <pin.json> <download-dir> [--cosign <binary>]
#   --cosign  the verifier to use. Defaults to cosign on PATH.
set -euo pipefail

USAGE="usage: verify_engine.sh <pin.json> <download-dir> [--cosign <binary>]"

PIN="${1:?$USAGE}"
DOWNLOAD_DIR="${2:?$USAGE}"
shift 2

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/engine_pin.sh
source "$ROOT_DIR/scripts/engine_pin.sh"

COSIGN_BIN=""

fail() {
  echo "verify_engine: $*" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --cosign)
      COSIGN_BIN="${2:?--cosign needs a binary path}"
      shift 2
      ;;
    *) fail "unknown argument '$1' ($USAGE)" ;;
  esac
done

resolve_cosign() {
  if [ -n "$COSIGN_BIN" ]; then
    [ -x "$COSIGN_BIN" ] || fail "the cosign binary is not executable: $COSIGN_BIN"
    return 0
  fi
  COSIGN_BIN="$(command -v cosign)" ||
    fail "cosign is not on PATH, so a pinned package's signature cannot be checked; pass --cosign <binary>"
}

# One digest, computed the same way on every host this runs on. macOS ships
# shasum and no sha256sum; a Debian container ships sha256sum and no shasum.
digest_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

verify_package() {
  local target="$1" format="$2"
  local asset path expected actual

  asset="$(engine_pin_package_field "$PIN" "$target" "$format" asset)"
  path="$DOWNLOAD_DIR/$asset"
  [ -f "$path" ] || fail "the pinned package $asset is not in $DOWNLOAD_DIR"
  [ -f "$path.sig" ] || fail "the pinned package $asset has no $asset.sig beside it"
  [ -f "$path.pem" ] || fail "the pinned package $asset has no $asset.pem beside it"

  expected="$(engine_pin_package_field "$PIN" "$target" "$format" sha256)"
  actual="$(digest_of "$path")"
  [ "$actual" = "$expected" ] ||
    fail "$asset hashes to $actual, and the engine pin records $expected"

  "$COSIGN_BIN" verify-blob \
    --certificate "$path.pem" \
    --signature "$path.sig" \
    --certificate-identity "$IDENTITY" \
    --certificate-oidc-issuer "$ISSUER" \
    "$path" >/dev/null 2>&1 ||
    fail "$asset carries no signature from $IDENTITY issued by $ISSUER"

  echo "verify_engine: $asset verified ($target, $format, $TAG, ${COMMIT:0:12})"
}

STATE="$(engine_pin_state "$PIN")" || exit 1
[ "$STATE" = "pinned" ] ||
  fail "$PIN is unpinned, so there is no engine release to verify against"

TAG="$(engine_pin_field "$PIN" tag)"
COMMIT="$(engine_pin_field "$PIN" source_commit)"
IDENTITY="$(engine_pin_field "$PIN" certificate_identity)"
ISSUER="$(engine_pin_field "$PIN" certificate_oidc_issuer)"

[ -d "$DOWNLOAD_DIR" ] || fail "no download directory at $DOWNLOAD_DIR"

resolve_cosign

for target in "${ENGINE_PIN_TARGETS[@]}"; do
  for format in "${ENGINE_PIN_FORMATS[@]}"; do
    verify_package "$target" "$format"
  done
done

echo "verify_engine: ok"
