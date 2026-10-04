#!/usr/bin/env bash
#
# Downloads the engine packages desktop/engine/PIN.json pins, each with its .sha256, .sig and .pem
# sidecars, from the pinned release of tezra-io/fermix, through gh.
#
#   fetch_engine.sh [--pin <file>] <out dir> [<arch>]
#
# <out dir> is created if absent and must be empty, because a file left from an earlier run is a
# file nobody pinned. <arch> is amd64 or arm64 (x86_64 and aarch64 name the same); without it both
# are downloaded. On any failure the directory is left as empty as it was found.
#
# Nothing here is verified and nothing is unpacked: a downloaded file has no standing until
# verify_engine.py has checked it against the pin. Keeping the network here, and out of the verifier,
# is what lets every refusal of the verifier be tested offline.
set -euo pipefail

# A failed download is retried at most 3 times, a pause apart, so it is tried 4 times in all; then
# the fetch fails, naming the file. An attempt that stalls is cut off and counts as a failure.
MAX_RETRIES=3
RETRY_PAUSE_SECONDS=5
DOWNLOAD_TIMEOUT_SECONDS=900

here=$(cd "$(dirname "$0")" && pwd)

fail() {
  echo "fetch_engine: $*" >&2
  exit 1
}

usage() {
  fail "usage: fetch_engine.sh [--pin <file>] <out dir> [<amd64|arm64>]"
}

# dpkg's name for an architecture, which is what the pin is keyed by.
deb_arch() {
  case "$1" in
    amd64 | x86_64) echo amd64 ;;
    arm64 | aarch64) echo arm64 ;;
    *) fail "no engine package is built for the architecture '$1'; one is for amd64 and one for arm64" ;;
  esac
}

download() {
  local repository="$1" tag="$2" name="$3" dir="$4" attempt
  for attempt in $(seq 1 $((MAX_RETRIES + 1))); do
    if timeout "$DOWNLOAD_TIMEOUT_SECONDS" gh release download "$tag" --repo "$repository" \
      --pattern "$name" --dir "$dir" && [ -s "$dir/$name" ]; then
      return 0
    fi
    rm -f -- "$dir/$name"
    echo "fetch_engine: attempt $attempt of $((MAX_RETRIES + 1)) at $name failed" >&2
    [ "$attempt" -gt "$MAX_RETRIES" ] || sleep "$RETRY_PAUSE_SECONDS"
  done
  fail "$name did not download in $((MAX_RETRIES + 1)) attempts, one and $MAX_RETRIES retries, from $repository $tag"
}

# Downloads into a directory of its own inside <out dir> and moves the files out only when every
# one has arrived. A subshell, so that directory goes with it on every path.
fetch() (
  local out="$1" repository="$2" tag="$3" partial="" name suffix
  shift 3
  trap 'rm -rf -- ${partial:+"$partial"}' EXIT
  partial="$(mktemp -d "$out/.fetch.XXXXXX")"
  for name in "$@"; do
    for suffix in "" .sha256 .sig .pem; do
      download "$repository" "$tag" "$name$suffix" "$partial"
    done
  done
  mv -- "$partial"/* "$out"/
)

pin_args=()
positional=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --pin)
      [ "$#" -ge 2 ] || usage
      pin_args=(--pin "$2")
      shift 2
      ;;
    -*) usage ;;
    *)
      positional+=("$1")
      shift
      ;;
  esac
done
[ "${#positional[@]}" -ge 1 ] && [ "${#positional[@]}" -le 2 ] || usage
out="${positional[0]}"
arches=(amd64 arm64)
if [ "${#positional[@]}" -eq 2 ]; then
  arch="$(deb_arch "${positional[1]}")"
  arches=("$arch")
fi

"$here/engine_pin.py" "${pin_args[@]}" --check || fail "the engine pin is not one a fetch can trust"
mkdir -p -- "$out"
[ -z "$(ls -A -- "$out")" ] || fail "$out already holds files, and every file here must be one the pin names"
command -v gh > /dev/null || fail "the GitHub CLI is required to download the engine"

repository="$("$here/engine_pin.py" "${pin_args[@]}" --get repository)"
tag="$("$here/engine_pin.py" "${pin_args[@]}" --get tag)"
packages=()
for arch in "${arches[@]}"; do
  packages+=("$("$here/engine_pin.py" "${pin_args[@]}" --get "$arch.deb.asset")")
  packages+=("$("$here/engine_pin.py" "${pin_args[@]}" --get "$arch.rpm.asset")")
done

fetch "$out" "$repository" "$tag" "${packages[@]}"
echo "fetch_engine: ${packages[*]} and their sidecars from $repository $tag in $out"
