#!/usr/bin/env bash
#
# The written offer of amendment section 4.6, as a file.
#
# The package ships LGPL libraries — GTK, GLib, pango, cairo, libadwaita and the
# rest — as binaries inside /usr/lib/fermix-desktop. That is allowed on the
# condition that the corresponding source is offered, and an offer nobody can
# act on is not an offer. This produces the archive that discharges it: every
# locked tarball exactly as it was fetched, every patch applied to it, the lock
# file that pins it, the container that compiles it and the script that drives
# the whole thing. Someone with this archive and Docker can rebuild the runtime
# this package carries, without the network and without this repository.
#
# The tarballs come from the download cache when it has them and from the URLs
# in the lock file when it does not, and every one is checked against
# RUNTIME.lock.json either way. A source archive whose contents are not the
# sources that were built is worse than none, because it looks like compliance.
#
# Fetching is the default rather than an option, because of what this archive
# is. A release that could only build it from a cache would fail the first time
# that cache was evicted — which is the steady state of a lock file that
# deliberately does not change for months — and it would fail on the one asset
# that discharges a licence obligation. Everything needed is written down: a URL
# and a sha256 per component.
#
# Usage:
#   packaging/runtime/package_sources.sh <version> [--out <dir>] [--sources <dir>] [--no-fetch]
#
#   <version>    X.Y.Z or X.Y.Z+N, matching the package it accompanies
#   --out        where to write the archive; default packaging/out/runtime
#   --sources    the download cache; default ~/.cache/fermix-desktop-runtime/sources
#   --no-fetch   refuse the network; every tarball must already be cached
#
# The archive is deterministic: sorted entries, zero timestamps, numeric owners
# and a gzip header carrying no name or time, so the same inputs produce the
# same bytes and a release can be checked rather than trusted.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNTIME_DIR="$ROOT_DIR/packaging/runtime"
LOCK_FILE="$RUNTIME_DIR/RUNTIME.lock.json"
PATCH_DIR="$RUNTIME_DIR/patches"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.runtime"

VERSION=""
OUT_DIR="$ROOT_DIR/packaging/out/runtime"
SOURCE_CACHE="${FERMIX_RUNTIME_SOURCES:-${XDG_CACHE_HOME:-$HOME/.cache}/fermix-desktop-runtime/sources}"
STAGE=""
FETCH=1

fail() {
  echo "package_sources: $*" >&2
  exit 1
}

log() {
  echo "package_sources: $*" >&2
}

# The same fetch and the same digest rule build_runtime.sh uses, from the same
# file. Sourced after fail() and log(), which is what it calls.
# shellcheck source=packaging/runtime/fetch_source.sh
. "$RUNTIME_DIR/fetch_source.sh"

usage() {
  echo "usage: package_sources.sh <version> [--out <dir>] [--sources <dir>] [--no-fetch]" >&2
}

cleanup() {
  [ -z "$STAGE" ] || rm -rf "$STAGE"
}

parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --out) [ $# -ge 2 ] || fail "--out needs a directory"; OUT_DIR="$2"; shift 2 ;;
      --sources) [ $# -ge 2 ] || fail "--sources needs a directory"; SOURCE_CACHE="$2"; shift 2 ;;
      --no-fetch) FETCH=0; shift ;;
      -h|--help) usage; exit 0 ;;
      -*) usage; fail "unknown argument: $1" ;;
      *) [ -z "$VERSION" ] || fail "more than one version given"; VERSION="$1"; shift ;;
    esac
  done
  [ -n "$VERSION" ] || { usage; fail "no version given"; }
}

# The same rule build_packages.sh enforces, for the same reason: this archive is
# published beside the packages and has to sort with them.
check_version() {
  case "$VERSION" in
    *-*) fail "the version '$VERSION' carries a Debian revision, and this archive may not" ;;
    *:*) fail "the version '$VERSION' carries an rpm epoch, and this archive may not" ;;
  esac
  if ! grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(\+[0-9]+)?$' <<< "$VERSION"; then
    fail "the version '$VERSION' is neither X.Y.Z nor X.Y.Z+N"
  fi
}

require_inputs() {
  command -v jq >/dev/null 2>&1 || fail "jq is not installed"
  command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is not installed"
  [ -f "$LOCK_FILE" ] || fail "no lock file at $LOCK_FILE"
  [ -f "$DOCKERFILE" ] || fail "no runtime container at $DOCKERFILE"
  [ -f "$RUNTIME_DIR/build_runtime.sh" ] || fail "no build script to include"
  local needed
  for needed in fetch_source.sh write_manifest.py compare_manifest.py; do
    [ -f "$RUNTIME_DIR/$needed" ] || fail "no $needed to include"
  done
  [ -d "$PATCH_DIR" ] || fail "no patch directory at $PATCH_DIR"
  if [ "$FETCH" = "1" ]; then
    command -v curl >/dev/null 2>&1 || fail "curl is not installed"
  else
    [ -d "$SOURCE_CACHE" ] \
      || fail "--no-fetch, and there is no download cache at $SOURCE_CACHE"
  fi
}

# Every tarball, checked against the lock file rather than assumed. A cache is a
# directory anyone can write to, so its contents are evidence only once they
# have been verified. ensure_source does both halves — take it from the cache or
# fetch it, then hold it to the recorded digest — and it is the same function
# the build itself uses.
copy_verified_sources() {
  local dest="$1" name archive url want fetched=0
  mkdir -p "$dest"
  while read -r name; do
    [ -n "$name" ] || continue
    archive="$(jq -r --arg n "$name" '.components[] | select(.name == $n) | .archive' "$LOCK_FILE")"
    url="$(jq -r --arg n "$name" '.components[] | select(.name == $n) | .url' "$LOCK_FILE")"
    want="$(jq -r --arg n "$name" '.components[] | select(.name == $n) | .sha256' "$LOCK_FILE")"
    if [ ! -s "$SOURCE_CACHE/$archive" ]; then
      [ "$FETCH" = "1" ] \
        || fail "$name: $archive is not cached and --no-fetch was given"
      fetched=$((fetched + 1))
    fi
    ensure_source "$name" "$url" "$SOURCE_CACHE/$archive" "$want"
    cp -p "$SOURCE_CACHE/$archive" "$dest/$archive"
  done < <(jq -r '.components[].name' "$LOCK_FILE")
  [ "$fetched" -eq 0 ] || log "fetched $fetched tarball(s) the cache did not have"
}

# The build inputs keep the repository's own layout — packaging/runtime and
# packaging/docker — rather than being flattened next to the sources. That is
# not tidiness: build_runtime.sh finds its lock file, its patches, its Dockerfile
# and the library it sources by paths relative to itself, so a flattened archive
# ships a build script that cannot run. The first version of this archive was
# flat, and unpacking it and running the documented command is what proved it.
copy_build_inputs() {
  local dest="$1" runtime="$1/packaging/runtime" docker_dir="$1/packaging/docker"
  mkdir -p "$runtime/patches" "$docker_dir"
  cp "$LOCK_FILE" "$runtime/RUNTIME.lock.json"
  cp "$DOCKERFILE" "$docker_dir/Dockerfile.runtime"
  cp "$RUNTIME_DIR/build_runtime.sh" "$runtime/build_runtime.sh"
  # build_runtime.sh sources this, so an archive without it is an archive that
  # cannot rebuild what it documents.
  cp "$RUNTIME_DIR/fetch_source.sh" "$runtime/fetch_source.sh"
  # And these two: the build writes runtime-manifest.json as its last step, and
  # --verify compares against it. Leaving them out produced an archive that
  # compiled all 29 components, passed every check, and then died on a missing
  # python file. That is how this line came to exist, and it is the reason the
  # archive is tested by running it rather than by reading its file list.
  cp "$RUNTIME_DIR/write_manifest.py" "$runtime/write_manifest.py"
  cp "$RUNTIME_DIR/compare_manifest.py" "$runtime/compare_manifest.py"
  # cp of a glob that matches nothing would fail the script; patches are
  # normally empty and that is not an error.
  find "$PATCH_DIR" -maxdepth 1 -type f -name '*.patch' -exec cp {} "$runtime/patches/" \;
  find "$PATCH_DIR" -maxdepth 1 -type f -name 'README.md' -exec cp {} "$runtime/patches/" \;
  [ -d "$dest" ] || fail "the staging directory vanished"
}

write_readme() {
  local dest="$1" count
  count="$(jq -r '.components | length' "$LOCK_FILE")"
  cat > "$dest/README" <<EOF
The sources of the private toolkit runtime in fermix-desktop $VERSION.

This archive is the corresponding source for the LGPL libraries the
fermix-desktop package installs under /usr/lib/fermix-desktop. It contains all
$count component tarballs exactly as they were fetched, every patch applied to
them, the lock file that pins each one by version and sha256, the container
definition that compiles them and the script that drives the build.

  sources/                              every component tarball, named as the
                                        lock file names it
  packaging/runtime/RUNTIME.lock.json   version, URL, sha256, licence, build
                                        system and options, per component
  packaging/runtime/build_runtime.sh    the build itself
  packaging/runtime/fetch_source.sh     the fetch and digest check it uses
  packaging/runtime/write_manifest.py   writes runtime-manifest.json
  packaging/runtime/compare_manifest.py what --verify compares with
  packaging/runtime/patches/            every patch, applied by name from the
                                        lock file
  packaging/docker/Dockerfile.runtime   the toolchain image the build runs in

The packaging/ layout is the one the build script expects; it finds everything
else by paths relative to itself, so keep it as it is.

To rebuild, point the build at the tarballs that are already here and run it
from the top of this archive:

  FERMIX_RUNTIME_SOURCES="\$PWD/sources" packaging/runtime/build_runtime.sh --container

All $count component tarballs are here, so none of the sources is downloaded,
and the build refuses any file whose sha256 is not the one the lock file
records. The result is /usr/lib/fermix-desktop, exported as a tarball under
packaging/out/.

What the build does still take from the network is the toolchain image that
compiles those sources: Dockerfile.runtime starts from a digest-pinned
AlmaLinux 9 and installs a compiler, meson, ninja, patchelf, a Rust toolchain
and sassc. Those are build tools rather than parts of the runtime, and none of
them is linked into what the package ships — but a machine with no network
cannot build the image, and saying otherwise would be a promise this archive
cannot keep.

Each component's licence is recorded in RUNTIME.lock.json, and the installed
package carries the same information at
/usr/share/doc/fermix-desktop/runtime-manifest.json.
EOF
}

# Deterministic: sorted names, zeroed timestamps, numeric ownership, and gzip
# with -n so the compressed stream carries neither the file name nor the time.
# Two runs from the same inputs produce the same bytes, which is what makes a
# published source archive checkable rather than merely present.
write_archive() {
  local stage="$1" top="$2" out="$3"
  tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    --format=gnu -C "$stage" -cf - "$top" | gzip -n > "$out"
}

main() {
  local top out
  parse_args "$@"
  check_version
  require_inputs

  top="fermix_desktop_runtime_sources_$VERSION"
  out="$OUT_DIR/$top.tar.gz"
  mkdir -p "$OUT_DIR"

  STAGE="$(mktemp -d "${TMPDIR:-/tmp}/fermix-runtime-sources.XXXXXX")"
  trap cleanup EXIT

  copy_verified_sources "$STAGE/$top/sources"
  copy_build_inputs "$STAGE/$top"
  write_readme "$STAGE/$top"
  write_archive "$STAGE" "$top" "$out"

  log "$out"
  log "$(du -h "$out" | cut -f1), $(find "$STAGE/$top" -type f | wc -l) files, sha256 $(sha256sum "$out" | cut -c1-16)"
}

main "$@"
