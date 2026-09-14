#!/usr/bin/env bash
#
# Render the packaged icon set from the application's own source SVG.
#
# The icon theme specification makes SVG support optional, so a compliant
# implementation may ignore the scalable icon entirely (M38 section 2.2). The
# package therefore carries eight rasters as well, and they are checked in so a
# reviewer can look at them and a release does not depend on whichever librsvg a
# build host happens to carry.
#
# One source: App/Fermix/resources/icons/. This script is how the checked-in set
# is regenerated; scripts/build_packages.sh stages what is checked in and
# refuses a set that is incomplete or a raster whose pixels disagree with the
# directory it sits in.
#
# Usage:
#   scripts/render_icons.sh              render here, with rsvg-convert on PATH
#   scripts/render_icons.sh --container  render inside the build container
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_DIR="$ROOT_DIR/App/Fermix/resources/icons"
OUT_DIR="$ROOT_DIR/packaging/icons/hicolor"
APP_ID="io.tezra.Fermix"
SIZES=(16 22 24 32 48 64 128 256)
IMAGE="${FERMIX_BUILD_IMAGE:-fermix-desktop-build}"

fail() {
  echo "render_icons: $*" >&2
  exit 1
}

if [ "${1:-}" = "--container" ]; then
  shift
  [ "$#" -eq 0 ] || fail "unknown argument: $1"
  command -v docker >/dev/null 2>&1 || fail "docker is not installed"
  docker image inspect "$IMAGE" >/dev/null 2>&1 ||
    fail "the build image $IMAGE does not exist yet; run scripts/container_build.sh first"
  exec docker run --rm --init \
    -v "$ROOT_DIR:/workspace" -w /workspace \
    "$IMAGE" scripts/render_icons.sh
fi

[ "$#" -eq 0 ] || fail "unknown argument: $1"

command -v rsvg-convert >/dev/null 2>&1 ||
  fail "rsvg-convert is not installed; pass --container to render in the build image"

[ -f "$SOURCE_DIR/$APP_ID.svg" ] || fail "no source icon at $SOURCE_DIR/$APP_ID.svg"
[ -f "$SOURCE_DIR/$APP_ID-symbolic.svg" ] ||
  fail "no symbolic icon at $SOURCE_DIR/$APP_ID-symbolic.svg"

for size in "${SIZES[@]}"; do
  directory="$OUT_DIR/${size}x${size}/apps"
  mkdir -p "$directory"
  rsvg-convert --width "$size" --height "$size" --keep-aspect-ratio \
    --format png --output "$directory/$APP_ID.png" \
    "$SOURCE_DIR/$APP_ID.svg" ||
    fail "rsvg-convert could not render $size"
  echo "  ${size}x${size}/apps/$APP_ID.png"
done

mkdir -p "$OUT_DIR/scalable/apps" "$OUT_DIR/symbolic/apps"
cp "$SOURCE_DIR/$APP_ID.svg" "$OUT_DIR/scalable/apps/$APP_ID.svg"
cp "$SOURCE_DIR/$APP_ID-symbolic.svg" "$OUT_DIR/symbolic/apps/$APP_ID-symbolic.svg"
echo "  scalable/apps/$APP_ID.svg"
echo "  symbolic/apps/$APP_ID-symbolic.svg"

echo "render_icons: ${#SIZES[@]} rasters, the scalable icon and its symbolic pair"
