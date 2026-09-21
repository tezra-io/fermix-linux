#!/usr/bin/env bash
#
# Name the container image the build image copies the private toolkit out of,
# and make sure it exists.
#
# The build image cannot carry the toolkit in its own Dockerfile, because the
# toolkit takes about an hour to compile and is rebuilt only when
# `packaging/runtime/RUNTIME.lock.json` changes (amendment section 4.2). So it
# arrives as an image, and there are two places that image can come from:
#
#   * **In CI**, the runtime workflow has already published
#     `ghcr.io/tezra-io/fermix-desktop-runtime:<cache key>-<arch>`, and every
#     build pulls it. `FERMIX_RUNTIME_IMAGE` names it.
#   * **On a developer's machine**, `packaging/runtime/build_runtime.sh
#     --container` has written `packaging/out/runtime/runtime-dev-<arch>.tar`,
#     and this script imports it under a local tag once. The dev tree rather
#     than the shipped one, because the crate is compiled against it and needs
#     the headers and the `.pc` files the shipped tree is pruned of.
#
# It prints the image reference and nothing else, so a caller can read it into
# a variable, and it refuses rather than printing a name that names nothing.
#
# Usage:
#   runtime_image.sh --arch <amd64|arm64>
#
# Environment:
#   FERMIX_RUNTIME_IMAGE   use this image and import nothing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNTIME_OUT="$ROOT_DIR/packaging/out/runtime"
ARCH=""

fail() {
  echo "runtime_image: $*" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --arch)
      [ "$#" -ge 2 ] || fail "--arch needs amd64 or arm64"
      ARCH="$2"
      shift 2
      ;;
    *) fail "unknown argument: $1" ;;
  esac
done

case "$ARCH" in
  amd64 | arm64) ;;
  *) fail "usage: runtime_image.sh --arch <amd64|arm64>" ;;
esac

if [ -n "${FERMIX_RUNTIME_IMAGE:-}" ]; then
  printf '%s\n' "$FERMIX_RUNTIME_IMAGE"
  exit 0
fi

command -v docker >/dev/null 2>&1 || fail "docker is not installed"

ARCHIVE="$RUNTIME_OUT/runtime-dev-$ARCH.tar"
KEY_FILE="$RUNTIME_OUT/cache-key"

[ -f "$ARCHIVE" ] ||
  fail "no private toolkit at $ARCHIVE; run packaging/runtime/build_runtime.sh --container, or set FERMIX_RUNTIME_IMAGE to a published one"

# The tag carries the cache key, so a lock file change produces a different tag
# and the build image is rebuilt against the new toolkit rather than silently
# reusing the old one. A tree built before cache-key existed is refused rather
# than tagged `unknown`, because an image whose contents nobody can name is the
# thing the key exists to stop.
[ -f "$KEY_FILE" ] ||
  fail "no cache key at $KEY_FILE; the toolkit tree does not say which lock file produced it"
KEY="$(tr -d '[:space:]' < "$KEY_FILE")"
[ -n "$KEY" ] || fail "the cache key at $KEY_FILE is empty"

IMAGE="fermix-desktop-runtime-dev:$KEY-$ARCH"
IDENTITY="usr/lib/fermix-desktop/share/fermix-desktop-runtime/identity.json"

# The cache key a tree is tagged with is a fact about the lock file, the
# Dockerfile and the patches. It is NOT a fact about the export, so two trees
# built from one lock file can differ and carry the same key: that is exactly
# how an image imported before a file was added kept a tag that said it had it.
# A tag is a label somebody can point at anything. So the tree is asked what it
# is, rather than the tag being believed.
image_cache_key() {
  local image="$1" container answer
  container="$(docker create "$image" /nonexistent 2>/dev/null)" || return 1
  answer="$(docker cp "$container:/$IDENTITY" - 2>/dev/null | tar -xO 2>/dev/null)" || answer=""
  docker rm "$container" >/dev/null 2>&1 || true
  [ -n "$answer" ] || return 1
  printf '%s' "$answer" |
    python3 -c 'import json,sys; print(json.load(sys.stdin).get("cache_key",""))' 2>/dev/null
}

# An image is reused only when it proves it is the tree on disk. The tar being
# newer is enough on its own: an export that changed without the key changing is
# the case this exists for.
needs_import() {
  docker image inspect "$IMAGE" >/dev/null 2>&1 || return 0

  local created archive_time
  created="$(docker image inspect --format '{{.Created}}' "$IMAGE" 2>/dev/null)" || return 0
  created="$(date -d "$created" +%s 2>/dev/null)" || return 0
  archive_time="$(stat -c %Y "$ARCHIVE" 2>/dev/null)" || return 0
  if [ "$archive_time" -gt "$created" ]; then
    echo "runtime_image: $(basename "$ARCHIVE") is newer than $IMAGE; re-importing" >&2
    return 0
  fi

  local stamped
  stamped="$(image_cache_key "$IMAGE")" || {
    echo "runtime_image: $IMAGE carries no $IDENTITY; re-importing" >&2
    return 0
  }
  if [ "$stamped" != "$KEY" ]; then
    echo "runtime_image: $IMAGE says it is $stamped and the lock file says $KEY; re-importing" >&2
    return 0
  fi
  return 1
}

if needs_import; then
  docker rmi -f "$IMAGE" >/dev/null 2>&1 || true
  echo "runtime_image: importing $(basename "$ARCHIVE") as $IMAGE" >&2
  docker import "$ARCHIVE" "$IMAGE" >/dev/null ||
    fail "docker could not import $ARCHIVE"
fi

# The check that makes the tag mean something, run whether the image was just
# imported or reused. A toolkit that does not say which lock file produced it,
# or says the wrong one, is refused here rather than compiled against for an
# hour and then shipped.
STAMPED="$(image_cache_key "$IMAGE")" ||
  fail "$IMAGE carries no $IDENTITY, so the tree does not say which lock file produced it; rebuild the runtime, or set FERMIX_RUNTIME_IMAGE to one that does"
[ -n "$STAMPED" ] ||
  fail "$IMAGE carries an identity file with no cache_key in it"
[ "$STAMPED" = "$KEY" ] ||
  fail "$IMAGE says it was built from lock file $STAMPED and this tree's lock file is $KEY"

printf '%s\n' "$IMAGE"
