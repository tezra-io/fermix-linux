#!/usr/bin/env bash
#
# Regenerate the whole icon set from the vendored artwork masters.
#
# The icon the packages shipped until now was a placeholder: a brand-blue
# rounded square with a wordmark "F", drawn as an SVG because no real artwork
# had been wired up on this platform. The real Fermix icon is the mascot, and it
# exists only as PNG artwork authored for macOS, vendored here as
# packaging/icons/masters/ so this script never reaches into another repository.
#
# There are TWO copies of the icon in a build and they must not drift:
#
#   * packaging/icons/hicolor/  is installed to /usr/share/icons/hicolor, and is
#     what the desktop, the shell and the launcher resolve;
#   * App/Fermix/resources/icons/ is compiled into the gresource bundle and is
#     what the application itself resolves in-process, for its window and its
#     about dialog.
#
# Replacing only the first leaves the app drawing the placeholder at runtime
# while the launcher draws the mascot, which is the kind of split that reads as
# a caching bug for a week. This script writes both, and build_packages.sh
# refuses a build where the two disagree.
#
# The icon theme specification makes SVG support optional (M38 section 2.2), so
# the package carries rasters as well, and they are checked in so a reviewer can
# look at them and a release does not depend on whichever librsvg a build host
# happens to carry.
#
# Usage:
#   scripts/render_icons.sh              regenerate here, with python3 on PATH
#   scripts/render_icons.sh --container  regenerate inside the build container
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ICON_DIR="$ROOT_DIR/packaging/icons"
MASTER_DIR="$ICON_DIR/masters"
OUT_DIR="$ICON_DIR/hicolor"
RESOURCE_DIR="$ROOT_DIR/App/Fermix/resources/icons"
APP_ID="io.tezra.Fermix"
IMAGE="${FERMIX_BUILD_IMAGE:-fermix-linux-package-build}"

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
  # As the invoking user, so nothing lands in the tree owned by root.
  exec docker run --rm --init -u "$(id -u):$(id -g)" \
    -v "$ROOT_DIR:/workspace" -w /workspace \
    "$IMAGE" scripts/render_icons.sh
fi

[ "$#" -eq 0 ] || fail "unknown argument: $1"

command -v python3 >/dev/null 2>&1 ||
  fail "python3 is not installed; pass --container to regenerate in the build image"

for master in FermixAppIconMaster.png FermixMarkMaster.png; do
  [ -f "$MASTER_DIR/$master" ] || fail "no artwork master at $MASTER_DIR/$master"
done

echo "render_icons: the rasters, from the 1024 px application master"
python3 "$ICON_DIR/generate_icons.py" "$MASTER_DIR/FermixAppIconMaster.png" "$OUT_DIR"

echo "render_icons: the symbolic pair, traced from the mark master"
python3 "$ICON_DIR/trace_mark.py" "$MASTER_DIR/FermixMarkMaster.png" \
  "$OUT_DIR/symbolic/apps/$APP_ID-symbolic.svg"

echo "render_icons: the scalable icon, composed from both masters"
python3 "$ICON_DIR/compose_scalable.py" \
  "$MASTER_DIR/FermixAppIconMaster.png" "$MASTER_DIR/FermixMarkMaster.png" \
  "$OUT_DIR/scalable/apps/$APP_ID.svg"

# The gresource copies. Copied rather than re-derived, so the two can only ever
# be byte-identical and the gate that compares them cannot be satisfied by two
# renderings that merely look alike.
echo "render_icons: the copies the application compiles into its gresource"
install -d -m 0755 "$RESOURCE_DIR"
install -m 0644 "$OUT_DIR/scalable/apps/$APP_ID.svg" "$RESOURCE_DIR/$APP_ID.svg"
install -m 0644 "$OUT_DIR/symbolic/apps/$APP_ID-symbolic.svg" \
  "$RESOURCE_DIR/$APP_ID-symbolic.svg"
echo "  $APP_ID.svg and $APP_ID-symbolic.svg"

echo "render_icons: done; packaging/icons and App/Fermix/resources/icons now agree"
