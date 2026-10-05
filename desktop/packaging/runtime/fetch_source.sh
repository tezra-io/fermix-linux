#!/usr/bin/env bash
#
# One place a locked tarball is fetched and one place its digest is checked.
#
# build_runtime.sh compiles the components and package_sources.sh publishes
# them as the source offer. Both fetch through here, so the archive that is
# offered is the archive that was built.
#
# Usage, from bash, after defining fail() and log():
#   . "$RUNTIME_DIR/fetch_source.sh"
#   ensure_source <label> <url> <destination> <sha256>
#
# The destination is written only once the digest matches, so an interrupted or
# corrupted download is never mistaken for a cached tarball.

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  echo "fetch_source.sh: must be sourced from bash" >&2
  exit 1
fi

# Bounded: a tarball that does not arrive in this many attempts stops the run.
FETCH_MAX_ATTEMPTS="${FETCH_MAX_ATTEMPTS:-3}"
FETCH_RETRY_SECONDS="${FETCH_RETRY_SECONDS:-5}"
FETCH_MAX_SECONDS="${FETCH_MAX_SECONDS:-900}"

# The URL's scheme decides what curl may speak, never a flag. file:// exists so
# the fetch can be tested offline; the lock file test refuses anything but https.
fetch_source_protocol() {
  case "$1" in
    https://*) printf '=https' ;;
    file://*) printf '=file' ;;
    *) printf '' ;;
  esac
}

fetch_source() {
  local label="$1" url="$2" dest="$3" want="$4"
  local proto attempt=1 have

  proto="$(fetch_source_protocol "$url")"
  [ -n "$proto" ] || fail "$label: $url is not https"

  mkdir -p "$(dirname -- "$dest")"
  while [ "$attempt" -le "$FETCH_MAX_ATTEMPTS" ]; do
    if curl --proto "$proto" -fsSL --max-time "$FETCH_MAX_SECONDS" "$url" -o "$dest.partial"; then
      have="$(sha256sum "$dest.partial" | cut -d' ' -f1)"
      # Wrong bytes are not retried: the server answered, and the answer is
      # that the URL no longer serves what the lock file recorded.
      if [ "$have" != "$want" ]; then
        rm -f "$dest.partial"
        fail "$label: $url served sha256 $have, the lock file says $want"
      fi
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

# A cached file is checked exactly as a fetched one is. One whose digest is
# wrong is refused by name rather than replaced, because something changed the
# cache.
ensure_source() {
  local label="$1" url="$2" dest="$3" want="$4" have
  if [ ! -s "$dest" ]; then
    fetch_source "$label" "$url" "$dest" "$want"
    return 0
  fi
  have="$(sha256sum "$dest" | cut -d' ' -f1)"
  [ "$have" = "$want" ] || fail "$label: $dest has sha256 $have, the lock file says $want"
}
