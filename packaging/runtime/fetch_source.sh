#!/usr/bin/env bash
#
# One place a locked tarball is fetched and one place its digest is checked.
#
# Two scripts need this: build_runtime.sh, which compiles the components, and
# package_sources.sh, which publishes them as the source offer. They are the
# same operation and must not drift — a source archive assembled by a slightly
# different rule than the build used is a source archive that does not
# correspond to the binaries, which is the one thing the offer has to be.
#
# Usage:
#   source "$(dirname "$0")/fetch_source.sh"
#   fetch_source <label> <url> <destination> <sha256>
#
# The caller supplies fail() and log(). That is the seam: each script names
# itself in its own messages, and this file decides what is fetched, how often
# it is retried, and what makes a file acceptable.
#
# The destination is written only once the digest matches, so an interrupted or
# corrupted download can never be mistaken for a cached tarball by a later run.

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  echo "fetch_source.sh: must be sourced from bash" >&2
  exit 1
fi

# Bounded, and the cap is a refusal rather than a warning: a tarball that will
# not arrive three times is a run that must stop, not one that carries on with
# whatever happens to be on disk.
FETCH_MAX_ATTEMPTS="${FETCH_MAX_ATTEMPTS:-3}"
FETCH_RETRY_SECONDS="${FETCH_RETRY_SECONDS:-5}"
FETCH_MAX_SECONDS="${FETCH_MAX_SECONDS:-900}"

# curl is told which protocol it may use, rather than being left to follow a URL
# wherever it leads. Locked components are https and build_runtime_test.sh
# refuses a lock file that says otherwise; file:// is accepted here so that this
# function can be tested without a network, and it is the scheme of the URL that
# decides, never a flag a caller can pass.
fetch_source_protocol() {
  local url="$1"
  case "$url" in
    https://*) printf '=https' ;;
    file://*) printf '=file' ;;
    *) printf '' ;;
  esac
}

# Returns nothing on standard output: the result is the file at <destination>.
fetch_source() {
  local label="$1" url="$2" dest="$3" want="$4"
  local proto attempt=1 have

  proto="$(fetch_source_protocol "$url")"
  [ -n "$proto" ] || fail "$label: $url is not https"

  mkdir -p "$(dirname -- "$dest")"
  while [ "$attempt" -le "$FETCH_MAX_ATTEMPTS" ]; do
    if curl --proto "$proto" -fsSL --max-time "$FETCH_MAX_SECONDS" \
        "$url" -o "$dest.partial"; then
      have="$(sha256sum "$dest.partial" | cut -d' ' -f1)"
      # A server that answers with the wrong bytes is not a transient failure,
      # so it is not retried: asking the same question twice gets the same wrong
      # answer, and the interesting fact is that the URL no longer serves what
      # the lock file recorded.
      if [ "$have" != "$want" ]; then
        rm -f "$dest.partial"
        fail "$label: $url served sha256 $have, the lock file says $want"
      fi
      # Renamed, never re-fetched. Downloading a second time to the real name
      # would double every transfer and, worse, put bytes nobody checked at the
      # destination: the verified copy is the one that must land.
      mv "$dest.partial" "$dest"
      return 0
    fi
    rm -f "$dest.partial"
    log "$label: attempt $attempt of $FETCH_MAX_ATTEMPTS failed: $url"
    attempt=$((attempt + 1))
    if [ "$attempt" -le "$FETCH_MAX_ATTEMPTS" ]; then
      sleep "$FETCH_RETRY_SECONDS"
    fi
  done
  fail "$label: could not fetch $url in $FETCH_MAX_ATTEMPTS attempts"
}

# The cache is an optimisation and never a requirement. A file already there is
# checked exactly as a freshly fetched one is; a file whose digest is wrong is
# refused by name rather than quietly replaced, because it means something
# interfered with the cache and that is worth stopping for.
ensure_source() {
  local label="$1" url="$2" dest="$3" want="$4" have
  if [ ! -s "$dest" ]; then
    fetch_source "$label" "$url" "$dest" "$want"
    return 0
  fi
  have="$(sha256sum "$dest" | cut -d' ' -f1)"
  [ "$have" = "$want" ] \
    || fail "$label: $dest has sha256 $have, the lock file says $want"
}
