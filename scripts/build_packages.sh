#!/usr/bin/env bash
#
# Build the fermix-desktop deb and rpm from one tree, in the build container
# (M38 sections 2.2, 12.1).
#
# One run produces: the release binary with its build id compiled in, the
# manifest that records the same id, one staging tree with every file at its
# installed path, one nFPM configuration rendered from the checked-in template,
# and one package per family from that single configuration, so the two families
# cannot drift into two file lists.
#
# Four things this script refuses to assume:
#
#   * **A version is a version.** Neither package may carry a Debian revision or
#     an rpm epoch, because the exact-version relation on `fermix` has to mean
#     the same thing in both families, so a version containing `-` or `:` is
#     refused before anything is built. A prerelease tag therefore produces no
#     packages at all, which is a consequence worth stating rather than
#     discovering.
#   * **A build id is a fact about this build.** It is compiled into the binary
#     and written into the manifest the package installs from one value, so the
#     window's self-skew check compares an id with an id rather than with a
#     guess.
#   * **The declaration is only worth what a machine can check it against.**
#     nFPM generates no dependencies, so after the packages exist their declared
#     relations are checked against what the built binary actually needs.
#   * **The desktop files are only worth what the desktop can read.**
#     desktop-file-validate and appstreamcli run over the staged copies, not over
#     the sources, so what is checked is what is installed.
#
# Usage:
#   scripts/build_packages.sh <version> <arch> [--container]
#     <version>    X.Y.Z, no revision and no epoch
#     <arch>       amd64 or arm64, and it must be the machine this runs on:
#                  the binary links the host's GTK, so there is no cross build
#     --container  run the whole thing inside packaging/docker/Dockerfile.build
#
# Environment (build facts supplied by whatever is driving the build, never
# settings):
#   FERMIX_DESKTOP_BUILD_ID       the build id. Defaults to the source commit,
#                                 with -dirty appended for an uncommitted tree
#   FERMIX_DESKTOP_SOURCE_COMMIT  the commit being built. Defaults to git's answer
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_DIR="$ROOT_DIR/App/Fermix"
PACKAGING_DIR="$ROOT_DIR/packaging"
OUT_DIR="$PACKAGING_DIR/out"
STAGE_DIR="$OUT_DIR/stage"
TEMPLATE="$PACKAGING_DIR/nfpm-fermix-desktop.yaml.tmpl"
RENDERED="$OUT_DIR/nfpm-fermix-desktop.yaml"
IMAGE="${FERMIX_BUILD_IMAGE:-fermix-desktop-build}"
APP_ID="io.tezra.Fermix"

# The icon sizes the package installs, and the one place they are listed.
ICON_SIZES=(16 22 24 32 48 64 128 256)

# The shared libraries this application is allowed to link directly, with the
# package that carries each in both families. A NEEDED entry that is not in this
# table and not covered by a declared package's own dependencies fails the
# build: that is the whole shape of "a derived toolkit soname is not covered by
# the declared relations".
TOOLKIT_SONAMES=(
  "libgtk-4.so.1|libgtk-4-1|gtk4"
  "libadwaita-1.so.0|libadwaita-1-0|libadwaita"
)

fail() {
  echo "build_packages: $*" >&2
  exit 1
}

step() {
  echo
  echo "build_packages: == $*"
}

# ---- arguments -------------------------------------------------------------

CONTAINER=0
VERSION=""
ARCH=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --container)
      CONTAINER=1
      shift
      ;;
    -*) fail "unknown argument: $1" ;;
    *)
      if [ -z "$VERSION" ]; then
        VERSION="$1"
      elif [ -z "$ARCH" ]; then
        ARCH="$1"
      else
        fail "unexpected argument: $1"
      fi
      shift
      ;;
  esac
done

[ -n "$VERSION" ] || fail "usage: build_packages.sh <version> <arch> [--container]"
[ -n "$ARCH" ] || fail "usage: build_packages.sh <version> <arch> [--container]"

case "$VERSION" in
  *-*) fail "the version '$VERSION' carries a Debian revision, and neither package may: publish a packaging fix as the next version" ;;
  *:*) fail "the version '$VERSION' carries an rpm epoch, and neither package may: publish a packaging fix as the next version" ;;
esac
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
  fail "the version '$VERSION' is not X.Y.Z"

case "$ARCH" in
  amd64 | arm64) ;;
  *) fail "the architecture '$ARCH' is neither amd64 nor arm64" ;;
esac

# ---- the record, before the machine ----------------------------------------
#
# Every check that reads a file rather than the host runs here, before the
# re-exec into the container, so a release that names the wrong version is
# refused in a second rather than after a build.

CRATE_VERSION="$(
  awk -F'"' '/^version = "/ { print $2; exit }' "$CRATE_DIR/Cargo.toml"
)"
[ "$CRATE_VERSION" = "$VERSION" ] ||
  fail "the crate is version $CRATE_VERSION and $VERSION was asked for; the tag, Cargo.toml and the metainfo name one version"

# The pin and this version are the same fact when there is a pin: the package
# declares `fermix (= <version>)`, so the engine release it is published beside
# has to be that version or the relation is unsatisfiable on the release page.
# shellcheck source=scripts/engine_pin.sh
source "$ROOT_DIR/scripts/engine_pin.sh"
PIN_STATE="$(engine_pin_state "$ROOT_DIR/engine/PIN.json")" || exit 1
if [ "$PIN_STATE" = "pinned" ]; then
  PINNED_VERSION="$(engine_pin_field "$ROOT_DIR/engine/PIN.json" version)"
  [ "$PINNED_VERSION" = "$VERSION" ] ||
    fail "engine/PIN.json pins engine $PINNED_VERSION and this is $VERSION; the exact-version relation makes them one version"
fi

if [ "$CONTAINER" = "1" ]; then
  command -v docker >/dev/null 2>&1 || fail "docker is not installed"
  [ -f "$PACKAGING_DIR/docker/Dockerfile.build" ] ||
    fail "no build container at packaging/docker/Dockerfile.build"
  if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
    echo "build_packages: building $IMAGE"
    docker build -f "$PACKAGING_DIR/docker/Dockerfile.build" -t "$IMAGE" \
      "$PACKAGING_DIR/docker"
  fi
  # The cargo cache lives in the same volume the gate run uses, so a release
  # build after a gate run compiles the crate rather than the world. A host
  # short of memory bounds the build's parallelism with CARGO_BUILD_JOBS; it is
  # passed through only when set, because cargo refuses an empty value.
  docker volume create "${FERMIX_CARGO_CACHE:-fermix-desktop-cargo}" >/dev/null
  exec docker run --rm --init \
    -v "$ROOT_DIR:/workspace" \
    -v "${FERMIX_CARGO_CACHE:-fermix-desktop-cargo}:/cache" \
    -e CARGO_HOME=/cache/cargo \
    -e CARGO_TARGET_DIR=/cache/target \
    -e CARGO_TERM_COLOR=always \
    -e FERMIX_DESKTOP_TEST_ROOT=/tmp/fermix-desktop-tests \
    -e "FERMIX_DESKTOP_BUILD_ID=${FERMIX_DESKTOP_BUILD_ID:-}" \
    -e "FERMIX_DESKTOP_SOURCE_COMMIT=${FERMIX_DESKTOP_SOURCE_COMMIT:-}" \
    ${CARGO_BUILD_JOBS:+-e "CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS"} \
    -w /workspace \
    "$IMAGE" scripts/build_packages.sh "$VERSION" "$ARCH"
fi

# ---- the host this must be ------------------------------------------------

[ "$(uname -s)" = "Linux" ] ||
  fail "the packages are built on Linux, in the build container; pass --container"

for tool in cargo nfpm dpkg-shlibdeps objdump rpm desktop-file-validate \
  appstreamcli python3 xvfb-run; do
  command -v "$tool" >/dev/null 2>&1 ||
    fail "$tool is not installed; this runs in the build container, so pass --container"
done

HOST_ARCH="$(dpkg --print-architecture)"
[ "$HOST_ARCH" = "$ARCH" ] ||
  fail "this is an $HOST_ARCH machine and $ARCH was asked for; the binary links the host's GTK, so each architecture is built on its own runner"

# ---- what this build is ----------------------------------------------------

SOURCE_COMMIT="${FERMIX_DESKTOP_SOURCE_COMMIT:-}"
if [ -z "$SOURCE_COMMIT" ]; then
  SOURCE_COMMIT="$(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || true)"
fi
[ -n "$SOURCE_COMMIT" ] ||
  fail "no source commit: this tree has no commit and FERMIX_DESKTOP_SOURCE_COMMIT names none"

DIRTY=""
if [ -n "$(git -C "$ROOT_DIR" status --porcelain 2>/dev/null || true)" ]; then
  DIRTY="-dirty"
fi

BUILD_ID="${FERMIX_DESKTOP_BUILD_ID:-}"
if [ -z "$BUILD_ID" ]; then
  BUILD_ID="${SOURCE_COMMIT:0:12}$DIRTY"
fi

echo "build_packages: fermix-desktop $VERSION ($ARCH)"
echo "  build id      $BUILD_ID"
echo "  source commit $SOURCE_COMMIT"
echo "  engine pin    $PIN_STATE"

# ---- the gates -------------------------------------------------------------

step "the gates that are shell"
"$ROOT_DIR/scripts/check_app_identity.sh"
"$ROOT_DIR/scripts/verify_contract.sh"
"$ROOT_DIR/scripts/check_vendor_marks.sh"

step "format, lint, test"
(
  cd "$CRATE_DIR"
  cargo fmt --check
  cargo clippy --all-targets -- -D warnings
  # Each test binary statically links the whole toolkit, and linking them all at
  # once is what exhausts a container the linker is killed in.
  cargo test --jobs 2
  FERMIX_GTK_TESTS=1 xvfb-run -a cargo test --jobs 2 --test widgets
)

# ---- the binary ------------------------------------------------------------

step "the release binary"
(
  cd "$CRATE_DIR"
  FERMIX_DESKTOP_BUILD_ID="$BUILD_ID" cargo build --release --jobs 2 --bin fermix-desktop
)

BINARY="${CARGO_TARGET_DIR:-$CRATE_DIR/target}/release/fermix-desktop"
[ -x "$BINARY" ] || fail "the release build produced no binary at $BINARY"

# The binary says what it is, and both halves are checked. The build id in
# particular is the one the manifest below carries, and the window compares the
# two: a package that ships one and not the other is silently unknown rather
# than wrong, which is the failure this assertion exists to stop.
STAMPED="$("$BINARY" --version)" ||
  fail "the built binary cannot say what it is"
[ "$STAMPED" = "fermix-desktop $VERSION (build $BUILD_ID)" ] ||
  fail "the binary reports '$STAMPED', and this build is fermix-desktop $VERSION (build $BUILD_ID)"
echo "  the binary reports: $STAMPED"

# ---- the staging tree ------------------------------------------------------

step "the staging tree"
rm -rf "$STAGE_DIR"
mkdir -p \
  "$STAGE_DIR/usr/bin" \
  "$STAGE_DIR/usr/share/applications" \
  "$STAGE_DIR/usr/share/metainfo" \
  "$STAGE_DIR/usr/share/dbus-1/services" \
  "$STAGE_DIR/usr/lib/systemd/user" \
  "$STAGE_DIR/usr/share/fermix-desktop" \
  "$STAGE_DIR/usr/share/doc/fermix-desktop" \
  "$STAGE_DIR/usr/share/icons/hicolor/scalable/apps" \
  "$STAGE_DIR/usr/share/icons/hicolor/symbolic/apps"

install -m 0755 "$BINARY" "$STAGE_DIR/usr/bin/fermix-desktop"
install -m 0644 "$PACKAGING_DIR/$APP_ID.desktop" \
  "$STAGE_DIR/usr/share/applications/$APP_ID.desktop"
install -m 0644 "$PACKAGING_DIR/$APP_ID.metainfo.xml" \
  "$STAGE_DIR/usr/share/metainfo/$APP_ID.metainfo.xml"
install -m 0644 "$PACKAGING_DIR/dbus/$APP_ID.service" \
  "$STAGE_DIR/usr/share/dbus-1/services/$APP_ID.service"
install -m 0644 "$PACKAGING_DIR/systemd/app-$APP_ID.service" \
  "$STAGE_DIR/usr/lib/systemd/user/app-$APP_ID.service"
install -m 0644 "$PACKAGING_DIR/copyright" \
  "$STAGE_DIR/usr/share/doc/fermix-desktop/copyright"

# The icon set is checked in, rendered from the application's own source SVG by
# scripts/render_icons.sh. It is verified here rather than re-rendered, so the
# bytes a reviewer looked at are the bytes that ship and a release does not
# depend on whichever librsvg a build host happens to carry.
for size in "${ICON_SIZES[@]}"; do
  source_png="$PACKAGING_DIR/icons/hicolor/${size}x${size}/apps/$APP_ID.png"
  [ -f "$source_png" ] ||
    fail "the icon set has no ${size}x${size} raster; run scripts/render_icons.sh"
  read -r width height < <(python3 - "$source_png" <<'PY'
import struct
import sys

with open(sys.argv[1], "rb") as handle:
    header = handle.read(24)
if header[:8] != b"\x89PNG\r\n\x1a\n":
    sys.exit("not a PNG")
print(*struct.unpack(">II", header[16:24]))
PY
  )
  [ "$width" = "$size" ] && [ "$height" = "$size" ] ||
    fail "the ${size}x${size} icon is ${width}x${height} pixels; run scripts/render_icons.sh"
  mkdir -p "$STAGE_DIR/usr/share/icons/hicolor/${size}x${size}/apps"
  install -m 0644 "$source_png" \
    "$STAGE_DIR/usr/share/icons/hicolor/${size}x${size}/apps/$APP_ID.png"
done
install -m 0644 "$PACKAGING_DIR/icons/hicolor/scalable/apps/$APP_ID.svg" \
  "$STAGE_DIR/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
install -m 0644 "$PACKAGING_DIR/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg" \
  "$STAGE_DIR/usr/share/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"
echo "  ${#ICON_SIZES[@]} rasters, the scalable icon and its symbolic pair"

# The manifest the window reads to find out whether it is older than the
# application on disk. Its build id is the one compiled into the binary above.
python3 - "$OUT_DIR/build.json" "$VERSION" "$BUILD_ID" "$SOURCE_COMMIT" <<'PY'
import json
import os
import sys

path, version, build_id, source_commit = sys.argv[1:5]
os.makedirs(os.path.dirname(path), exist_ok=True)
with open(path, "w", encoding="utf-8") as handle:
    json.dump(
        {
            "schema_version": 1,
            "product_version": version,
            "build_id": build_id,
            "source_commit": source_commit,
        },
        handle,
        indent=2,
    )
    handle.write("\n")
PY
install -m 0644 "$OUT_DIR/build.json" "$STAGE_DIR/usr/share/fermix-desktop/build.json"
echo "  build.json"

# ---- what the desktop reads ------------------------------------------------

step "desktop-file-validate and appstreamcli, over the staged copies"
desktop-file-validate "$STAGE_DIR/usr/share/applications/$APP_ID.desktop"
echo "  the desktop entry validates"

# `--no-net` because a release must not depend on a third party answering: the
# validator would otherwise fetch the screenshot images and the OARS vocabulary.
#
# Straight through, with nothing accepted: an error or a warning fails the build.
# The version of appstreamcli this container pins reports one thing about this
# file and it is a pedantic hint, which this run does not ask for and which is
# inherent to the settled application identity: `cid-contains-uppercase-letter`,
# because `io.tezra.Fermix` carries a capital F, which is the convention every
# GNOME application follows. The absent screenshots are deliberate until the
# reference captures carry a reviewer's decision, and this validator does not
# report them at all. If a later version reports either, the build fails loudly
# and a person decides, which is better than a rail that quietly accepts a list.
APPSTREAM_REPORT="$OUT_DIR/appstreamcli.txt"
appstreamcli validate --no-net --explain \
  "$STAGE_DIR/usr/share/metainfo/$APP_ID.metainfo.xml" 2>&1 | tee "$APPSTREAM_REPORT"
echo "  the metainfo validates"

# ---- the packages ----------------------------------------------------------

step "the nFPM configuration"
[ -f "$TEMPLATE" ] || fail "no template at $TEMPLATE"
python3 - "$TEMPLATE" "$RENDERED" \
  "ARCH=$ARCH" \
  "VERSION=$VERSION" \
  "STAGE=$STAGE_DIR" \
  "POSTINSTALL=$PACKAGING_DIR/scripts/postinstall.sh" \
  "POSTREMOVE=$PACKAGING_DIR/scripts/postremove.sh" <<'PY'
import re
import sys

template, out, *pairs = sys.argv[1:]
values = dict(pair.split("=", 1) for pair in pairs)

with open(template, encoding="utf-8") as handle:
    body = handle.read()

missing = set()


def fill(match):
    name = match.group(1)
    if name not in values:
        missing.add(name)
        return match.group(0)
    return values[name]


body = re.sub(r"\{\{([A-Z_]+)\}\}", fill, body)
if missing:
    sys.exit(
        "build_packages: the template carries placeholders nothing fills: "
        + ", ".join(sorted(missing))
    )
if "{{" in body:
    sys.exit("build_packages: the rendered configuration still carries a placeholder")

with open(out, "w", encoding="utf-8") as handle:
    handle.write(body)
PY
echo "  $RENDERED"

step "nfpm, one configuration, two families"
rm -f "$OUT_DIR"/*.deb "$OUT_DIR"/*.rpm
nfpm package --config "$RENDERED" --packager deb --target "$OUT_DIR"
nfpm package --config "$RENDERED" --packager rpm --target "$OUT_DIR"

DEB="$OUT_DIR/fermix-desktop_${VERSION}_${ARCH}.deb"
case "$ARCH" in
  amd64) RPM_ARCH="x86_64" ;;
  arm64) RPM_ARCH="aarch64" ;;
esac
RPM="$OUT_DIR/fermix-desktop-${VERSION}-1.${RPM_ARCH}.rpm"

[ -f "$DEB" ] || fail "nfpm produced no $DEB"
[ -f "$RPM" ] || fail "nfpm produced no $RPM"
echo "  $(basename "$DEB")"
echo "  $(basename "$RPM")"

# ---- declared against derived ----------------------------------------------

step "the declared relations against what the binary needs"

DEPENDENCY_DIR="$OUT_DIR/dependencies"
rm -rf "$DEPENDENCY_DIR"
mkdir -p "$DEPENDENCY_DIR"

dpkg-deb --field "$DEB" Depends > "$DEPENDENCY_DIR/declared-deb.txt"
rpm -qp --requires "$RPM" 2>/dev/null > "$DEPENDENCY_DIR/declared-rpm.txt"

# dpkg-shlibdeps answers "which Debian packages, at which minimum versions, does
# this ELF need", which is exactly the question the deb's Depends field is an
# answer to. It reads debian/control for the package it is computing for, so it
# is given one. `--ignore-missing-info` keeps it from refusing over a library
# whose package ships no symbols file, which is a gap in that library's
# packaging rather than a fact about this binary.
SHLIBDEPS_DIR="$DEPENDENCY_DIR/shlibdeps"
mkdir -p "$SHLIBDEPS_DIR/debian"
{
  echo "Source: fermix-desktop"
  echo ""
  echo "Package: fermix-desktop"
  echo "Architecture: any"
} > "$SHLIBDEPS_DIR/debian/control"
(
  cd "$SHLIBDEPS_DIR" &&
    dpkg-shlibdeps -O --ignore-missing-info "$STAGE_DIR/usr/bin/fermix-desktop"
) > "$DEPENDENCY_DIR/derived-deb.txt"

objdump -p "$STAGE_DIR/usr/bin/fermix-desktop" |
  awk '/NEEDED/ { print $2 }' | sort > "$DEPENDENCY_DIR/sonames.txt"

# The recursive dependency closure of the packages the deb declares, minus the
# engine, which is this repository's other half rather than a library. A derived
# package inside the closure is covered: installing what is declared installs it.
DECLARED_PACKAGES="$(tr ',' '\n' < "$DEPENDENCY_DIR/declared-deb.txt" |
  awk '{ print $1 }' | grep -v '^$' | grep -v '^fermix$' | sort -u)"
# shellcheck disable=SC2086
apt-cache depends --recurse --no-recommends --no-suggests \
  --no-conflicts --no-breaks --no-replaces --no-enhances $DECLARED_PACKAGES |
  grep -E '^[a-zA-Z0-9]' | sort -u > "$DEPENDENCY_DIR/closure.txt"

printf '%s\n' "${TOOLKIT_SONAMES[@]}" > "$DEPENDENCY_DIR/toolkit.txt"

python3 "$ROOT_DIR/scripts/package_dependencies.py" \
  --version "$VERSION" \
  --declared-deb "$DEPENDENCY_DIR/declared-deb.txt" \
  --declared-rpm "$DEPENDENCY_DIR/declared-rpm.txt" \
  --derived-deb "$DEPENDENCY_DIR/derived-deb.txt" \
  --sonames "$DEPENDENCY_DIR/sonames.txt" \
  --closure "$DEPENDENCY_DIR/closure.txt" \
  --toolkit "$DEPENDENCY_DIR/toolkit.txt"

# ---- what came out ---------------------------------------------------------

step "the packages"
ls -l "$DEB" "$RPM"
echo
echo "build_packages: fermix-desktop $VERSION for $ARCH, both families, from one configuration"
