#!/usr/bin/env bash
#
# Download the pinned engine release's app-engine archives.
#
# Deliberately thin, and deliberately separate from scripts/verify_engine.sh:
# this half is the one that needs the network and a GitHub token, and the half
# that decides whether what arrived may be shipped needs neither. Split that
# way, every refusal that matters is provable offline against fixtures
# (scripts/verify_engine_test.sh) rather than only during a release.
#
# Nothing here is verified. A file in the download directory has no standing
# until verify_engine.sh has checked it against engine/PIN.json, and nothing
# here unpacks anything.
#
# Usage: fetch_engine.sh <pin.json> <download-dir> [--target <target>]...
#   <pin.json>      the engine pin, normally engine/PIN.json
#   <download-dir>  created if absent, required to be empty: a stale asset left
#                   over from an earlier run is an asset nobody pinned
#   --target        linux_x86_64 or linux_aarch64, repeatable. Defaults to both.
#                   A build for one architecture has no use for the other's
#                   archive, and each archive is the whole engine
#
# Downloads, per target, the archive and its .sha256, .sig and .pem sidecars.
# The .sha256 is the release's own record and travels with the archive for the
# release log; the digest that decides anything is the one in the pin.
set -euo pipefail

USAGE="usage: fetch_engine.sh <pin.json> <download-dir> [--target <target>]..."

# Bounded, because a release rail that retries forever is a release rail that
# hangs rather than fails. Four attempts over roughly half a minute covers a
# GitHub API blip and nothing longer; the cap behaviour is a refusal naming the
# asset, and the release job is the retry surface above that.
MAX_ATTEMPTS=4
RETRY_DELAY_SECONDS=5

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/engine_pin.sh
source "$ROOT_DIR/scripts/engine_pin.sh"

fail() {
  echo "fetch_engine: $*" >&2
  exit 1
}

PIN=""
DOWNLOAD_DIR=""
TARGETS=()

parse_arguments() {
  local positional=()
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --target)
        [ "$#" -ge 2 ] || fail "--target needs a target ($USAGE)"
        engine_pin_architecture "$2" >/dev/null || exit 1
        TARGETS+=("$2")
        shift 2
        ;;
      --*) fail "unknown argument '$1' ($USAGE)" ;;
      *)
        positional+=("$1")
        shift
        ;;
    esac
  done

  [ "${#positional[@]}" -eq 2 ] || fail "$USAGE"
  PIN="${positional[0]}"
  DOWNLOAD_DIR="${positional[1]}"
  [ "${#TARGETS[@]}" -gt 0 ] || TARGETS=("${ENGINE_PIN_TARGETS[@]}")
}

prepare_directory() {
  local dir="$1"
  mkdir -p "$dir"
  [ -z "$(find "$dir" -mindepth 1 -print -quit)" ] ||
    fail "the download directory already holds files: $dir"
}

# One request per file, each naming the asset exactly. `--pattern` takes a glob,
# and a glob over the release's assets would download whatever a future release
# happens to add under a matching name.
download_file() {
  local name="$1" attempt=1
  while [ "$attempt" -le "$MAX_ATTEMPTS" ]; do
    if gh release download --repo "$REPOSITORY" "$TAG" \
      --pattern "$name" --dir "$DOWNLOAD_DIR" >/dev/null 2>&1 &&
      [ -f "$DOWNLOAD_DIR/$name" ]; then
      return 0
    fi
    rm -f "$DOWNLOAD_DIR/$name"
    echo "fetch_engine: attempt $attempt of $MAX_ATTEMPTS for $name did not arrive" >&2
    attempt=$((attempt + 1))
    [ "$attempt" -le "$MAX_ATTEMPTS" ] && sleep "$RETRY_DELAY_SECONDS"
  done
  fail "$REPOSITORY $TAG published no $name, or it could not be downloaded in $MAX_ATTEMPTS attempts"
}

download_target() {
  local target="$1" asset suffix
  asset="$(engine_pin_artifact_field "$PIN" "$target" asset)" || exit 1
  for suffix in "" ".sha256" ".sig" ".pem"; do
    download_file "$asset$suffix"
  done
  echo "fetch_engine: $asset with its .sha256, .sig and .pem from $REPOSITORY $TAG"
}

parse_arguments "$@"

STATE="$(engine_pin_state "$PIN")" || exit 1
[ "$STATE" = "pinned" ] ||
  fail "$PIN is unpinned, so there is no engine release to download"

REPOSITORY="$(engine_pin_field "$PIN" repository)"
TAG="$(engine_pin_field "$PIN" tag)"

command -v gh >/dev/null 2>&1 ||
  fail "the GitHub CLI is required to download $REPOSITORY $TAG"

prepare_directory "$DOWNLOAD_DIR"

for target in "${TARGETS[@]}"; do
  download_target "$target"
done
