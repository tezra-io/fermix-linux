#!/usr/bin/env bash
#
# The written offer of amendment section 4.6, as a file.
#
# The package ships LGPL libraries as binaries under /usr/lib/fermix-desktop,
# which is allowed on condition that the corresponding source is offered. This
# writes the archive that offers it: every locked tarball as it was fetched,
# the Rust crates compiled into librsvg as the runtime build vendored them, the
# licence files the runtime build gathered, every patch, the lock file, the
# container definition and the scripts that drive the build. With it and Docker
# the runtime can be rebuilt without this repository.
#
# Each tarball comes from the download cache when it is there and from the lock
# file's URL when it is not, and is held to the locked sha256 either way.
# Fetching is the default because a release that could only build this archive
# from a cache would fail the first time the cache was evicted, on the one asset
# that discharges a licence obligation.
#
# Usage:
#   desktop/packaging/runtime/package_sources.sh <version> [--out <dir>] [--sources <dir>]
#                                                [--runtime <dir>] [--no-fetch]
#
#   <version>    X.Y.Z or X.Y.Z+N, matching the package it accompanies
#   --out        where to write the archive; default ~/.cache/fermix-desktop-runtime/out
#   --sources    the download cache; default ~/.cache/fermix-desktop-runtime/sources
#   --runtime    the runtime build's outputs, for runtime-crates.tar.gz,
#                runtime-licenses.tar.gz and runtime-licenses.json; default
#                $FERMIX_RUNTIME_OUT or ~/.cache/fermix-desktop-runtime/out
#   --no-fetch   refuse the network; every tarball must already be cached
#
# The archive is deterministic: sorted entries, zero timestamps, numeric owners
# and a gzip header with no name or time.
set -euo pipefail
shopt -s inherit_errexit

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNTIME_DIR="$ROOT_DIR/packaging/runtime"
LOCK_FILE="$RUNTIME_DIR/RUNTIME.lock.json"
PATCH_DIR="$RUNTIME_DIR/patches"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.runtime"
CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}/fermix-desktop-runtime"
# What build_runtime.sh needs beside itself to run from the archive.
BUILD_FILES="build_runtime.sh fetch_source.sh check_boundary.sh drop_unreachable.sh write_crates.py
write_licenses.py write_manifest.py compare_manifest.py"
# The runtime build's outputs this archive carries, each checked against the
# digest runtime-manifest.json records for it.
RUNTIME_FILES="runtime-crates.tar.gz runtime-licenses.tar.gz runtime-licenses.json"

VERSION=""
OUT_DIR="$CACHE_HOME/out"
SOURCE_CACHE="${FERMIX_RUNTIME_SOURCES:-$CACHE_HOME/sources}"
RUNTIME_OUT="${FERMIX_RUNTIME_OUT:-$CACHE_HOME/out}"
STAGE=""
FETCH=1

fail() {
  echo "package_sources: $*" >&2
  exit 1
}

log() {
  echo "package_sources: $*" >&2
}

# The same fetch and digest rule the build uses.
# shellcheck source=desktop/packaging/runtime/fetch_source.sh
. "$RUNTIME_DIR/fetch_source.sh"

usage() {
  echo "usage: package_sources.sh <version> [--out <dir>] [--sources <dir>] [--runtime <dir>] [--no-fetch]" >&2
}

cleanup() {
  [ -z "$STAGE" ] || rm -rf "$STAGE"
}

parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --out) [ $# -ge 2 ] || fail "--out needs a directory"; OUT_DIR="$2"; shift 2 ;;
      --sources) [ $# -ge 2 ] || fail "--sources needs a directory"; SOURCE_CACHE="$2"; shift 2 ;;
      --runtime) [ $# -ge 2 ] || fail "--runtime needs a directory"; RUNTIME_OUT="$2"; shift 2 ;;
      --no-fetch) FETCH=0; shift ;;
      -h|--help) usage; exit 0 ;;
      -*) usage; fail "unknown argument: $1" ;;
      *) [ -z "$VERSION" ] || fail "more than one version given"; VERSION="$1"; shift ;;
    esac
  done
  [ -n "$VERSION" ] || { usage; fail "no version given"; }
}

# The archive is published beside the packages and sorts with them.
check_version() {
  case "$VERSION" in
    *-*) fail "the version '$VERSION' carries a Debian revision, and this archive may not" ;;
    *:*) fail "the version '$VERSION' carries an rpm epoch, and this archive may not" ;;
  esac
  grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(\+[0-9]+)?$' <<< "$VERSION" \
    || fail "the version '$VERSION' is neither X.Y.Z nor X.Y.Z+N"
}

require_inputs() {
  local tool needed
  for tool in jq sha256sum tar gzip; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed"
  done
  [ -f "$LOCK_FILE" ] || fail "no lock file at $LOCK_FILE"
  [ -f "$DOCKERFILE" ] || fail "no runtime container at $DOCKERFILE"
  [ -d "$PATCH_DIR" ] || fail "no patch directory at $PATCH_DIR"
  for needed in $BUILD_FILES; do
    [ -f "$RUNTIME_DIR/$needed" ] || fail "no $needed to include"
  done
  if [ "$FETCH" = "1" ]; then
    command -v curl >/dev/null 2>&1 || fail "curl is not installed"
  else
    [ -d "$SOURCE_CACHE" ] || fail "--no-fetch, and there is no download cache at $SOURCE_CACHE"
  fi
  require_runtime_outputs
}

# The crates and licence files are those of a runtime built from this lock
# file, as its manifest records them.
require_runtime_outputs() {
  local manifest="$RUNTIME_OUT/runtime-manifest.json" name recorded
  [ -f "$manifest" ] || fail "no runtime manifest at $manifest; build the runtime first"
  jq -e --slurpfile lock "$LOCK_FILE" '.lock == $lock[0]' "$manifest" >/dev/null \
    || fail "the runtime at $RUNTIME_OUT was built from another lock file"
  for name in $RUNTIME_FILES; do
    recorded="$(jq -r --arg n "$name" '.archives[$n] // empty' "$manifest")"
    [ -n "$recorded" ] || fail "the runtime manifest records no $name"
    [ -f "$RUNTIME_OUT/$name" ] || fail "no $name at $RUNTIME_OUT"
    [ "$(sha256sum < "$RUNTIME_OUT/$name" | cut -d' ' -f1)" = "$recorded" ] \
      || fail "$RUNTIME_OUT/$name is not the one the runtime manifest records"
  done
}

copy_runtime_outputs() {
  local dest="$1"
  cp -p "$RUNTIME_OUT/runtime-crates.tar.gz" "$dest/sources/runtime-crates.tar.gz"
  cp -p "$RUNTIME_OUT/runtime-licenses.tar.gz" "$dest/runtime-licenses.tar.gz"
  cp -p "$RUNTIME_OUT/runtime-licenses.json" "$dest/runtime-licenses.json"
}

# A cache is a directory anyone can write to, so its contents count only once
# ensure_source has held them to the lock file. The standard licence texts and
# SPDX's lists are build inputs like the components, so they are copied too.
copy_verified_sources() {
  local dest="$1" name archive url sha256 fetched=0
  mkdir -p "$dest"
  while IFS=$'\t' read -r name archive url sha256; do
    if [ ! -s "$SOURCE_CACHE/$archive" ]; then
      [ "$FETCH" = "1" ] || fail "$name: $archive is not cached and --no-fetch was given"
      fetched=$((fetched + 1))
    fi
    ensure_source "$name" "$url" "$SOURCE_CACHE/$archive" "$sha256"
    cp -p "$SOURCE_CACHE/$archive" "$dest/$archive"
  done < <(jq -r '(.components[] | [.name, .archive, .url, .sha256]),
                  (.license_text_sources[] | ["SPDX text " + .spdx_id, .file, .url, .sha256]),
                  (.license_list | to_entries[] | ["SPDX list " + .key, .value.file, .value.url,
                                                   .value.sha256])
                  | @tsv' "$LOCK_FILE")
  [ "$fetched" -eq 0 ] || log "fetched $fetched tarball(s) the cache did not have"
}

# The repository's layout is kept, because build_runtime.sh finds its lock
# file, patches, Dockerfile and helpers by paths relative to itself.
copy_build_inputs() {
  local dest="$1" runtime="$1/packaging/runtime" needed
  mkdir -p "$runtime/patches" "$dest/packaging/docker"
  cp "$LOCK_FILE" "$runtime/RUNTIME.lock.json"
  cp "$DOCKERFILE" "$dest/packaging/docker/Dockerfile.runtime"
  for needed in $BUILD_FILES; do
    cp "$RUNTIME_DIR/$needed" "$runtime/$needed"
  done
  find "$PATCH_DIR" -maxdepth 1 -type f \( -name '*.patch' -o -name 'README.md' \) \
    -exec cp {} "$runtime/patches/" \;
}

write_readme() {
  local dest="$1" count
  count="$(jq -r '.components | length' "$LOCK_FILE")"
  cat > "$dest/README" <<EOF
The sources of the private toolkit runtime in fermix-desktop $VERSION.

This archive is the corresponding source for the libraries the fermix-desktop
package installs under /usr/lib/fermix-desktop: all $count component tarballs as
they were fetched, the Rust crates compiled into librsvg, the licence files of
all of them, every patch applied, the lock file that pins each component by
version and sha256, the container definition that compiles them and the
scripts that drive the build.

  sources/                              every component tarball; from SPDX's
                                        license-list-data, the standard texts
                                        of licences no file of a component or
                                        crate holds, and the licence and
                                        exception lists every id is held to
  sources/runtime-crates.tar.gz         the source of every Rust crate compiled
                                        in, as cargo vendored it
  runtime-licenses.tar.gz               the licence files of every component,
                                        every crate and the Rust standard library
  runtime-licenses.json                 the index of those: name, version,
                                        licence and licence files of each
  packaging/runtime/RUNTIME.lock.json   version, URL, sha256, licence, build
                                        system and options, per component
  packaging/runtime/build_runtime.sh    the build
  packaging/runtime/patches/            every patch, applied by name
  packaging/docker/Dockerfile.runtime   the toolchain image the build runs in

Keep the packaging/ layout: the build script finds everything by paths
relative to itself. To rebuild from the top of this archive:

  FERMIX_RUNTIME_SOURCES="\$PWD/sources" packaging/runtime/build_runtime.sh --container

No component tarball is downloaded, and the build refuses any whose sha256 is
not the locked one. The Rust crates librsvg compiles are the exception: cargo
fetches them from crates.io at the versions and sha256 digests of the
Cargo.lock inside librsvg's tarball, and refuses any that differs;
runtime-crates.tar.gz holds the same bytes for those compiled in. The
toolchain image uses the network too: Dockerfile.runtime starts from a
digest-pinned AlmaLinux 9 and installs a compiler, Meson, Ninja, patchelf,
sassc, Rust and cargo-c. Of those, only Rust's standard library is compiled
into what the package ships, inside librsvg.

Each component's, crate's and the standard library's licence is in
runtime-licenses.json, and the installed package carries the lock file at
/usr/share/doc/fermix-desktop/runtime-manifest.json.
EOF
}

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
  copy_runtime_outputs "$STAGE/$top"
  copy_build_inputs "$STAGE/$top"
  write_readme "$STAGE/$top"
  write_archive "$STAGE" "$top" "$out"
  log "$out"
  log "$(du -h "$out" | cut -f1), $(find "$STAGE/$top" -type f | wc -l) files, sha256 $(sha256sum "$out" | cut -d' ' -f1)"
}

main "$@"
