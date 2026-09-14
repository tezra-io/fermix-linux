#!/usr/bin/env bash
#
# Download the pinned engine release's Linux packages.
#
# Deliberately thin, and deliberately separate from scripts/verify_engine.sh:
# this half is the one that needs the network and a GitHub token, and the half
# that decides whether what arrived may be shipped needs neither. Split that
# way, every refusal that matters is provable offline against fixtures
# (scripts/verify_engine_test.sh) rather than only during a release.
#
# Nothing here is verified. A file in the download directory has no standing
# until verify_engine.sh has checked it against engine/PIN.json.
#
# Usage: fetch_engine.sh <pin.json> <download-dir>
#   <pin.json>      the engine pin, normally engine/PIN.json
#   <download-dir>  created if absent, required to be empty: a stale asset left
#                   over from an earlier run is an asset nobody pinned
#
# Downloads, per target and per family, the package and its .sha256, .sig and
# .pem sidecars. The .sha256 is the release's own record and travels with the
# package for the release log; the digest that decides anything is the one in
# the pin.
set -euo pipefail

USAGE="usage: fetch_engine.sh <pin.json> <download-dir>"

PIN="${1:?$USAGE}"
DOWNLOAD_DIR="${2:?$USAGE}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/engine_pin.sh
source "$ROOT_DIR/scripts/engine_pin.sh"

fail() {
  echo "fetch_engine: $*" >&2
  exit 1
}

STATE="$(engine_pin_state "$PIN")" || exit 1
[ "$STATE" = "pinned" ] ||
  fail "$PIN is unpinned, so there is no engine release to download"

REPOSITORY="$(engine_pin_field "$PIN" repository)"
TAG="$(engine_pin_field "$PIN" tag)"

command -v gh >/dev/null 2>&1 ||
  fail "the GitHub CLI is required to download $REPOSITORY $TAG"

mkdir -p "$DOWNLOAD_DIR"
[ -z "$(find "$DOWNLOAD_DIR" -mindepth 1 -print -quit)" ] ||
  fail "the download directory already holds files: $DOWNLOAD_DIR"

# One request per file, each naming the asset exactly. `--pattern` takes a glob,
# and a glob over the release's assets would download whatever a future release
# happens to add under a matching name.
for target in "${ENGINE_PIN_TARGETS[@]}"; do
  for format in "${ENGINE_PIN_FORMATS[@]}"; do
    asset="$(engine_pin_package_field "$PIN" "$target" "$format" asset)"
    for suffix in "" ".sha256" ".sig" ".pem"; do
      gh release download --repo "$REPOSITORY" "$TAG" \
        --pattern "$asset$suffix" --dir "$DOWNLOAD_DIR" ||
        fail "cannot download $asset$suffix from $REPOSITORY $TAG"
      [ -f "$DOWNLOAD_DIR/$asset$suffix" ] ||
        fail "$REPOSITORY $TAG published no $asset$suffix"
    done
    echo "fetch_engine: $asset with its .sha256, .sig and .pem from $REPOSITORY $TAG"
  done
done
