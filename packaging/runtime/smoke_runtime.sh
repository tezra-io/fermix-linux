#!/usr/bin/env bash
#
# Prove the private runtime runs on the oldest deb target it claims to support.
#
# Two containers. The first is the build image: it installs the dev variant of
# the runtime at its final path, compiles packaging/runtime/smoke/runtime_smoke.c
# against it with the same RUNPATH rule the application ELF will carry, and
# leaves a tree that is the shipped runtime plus that one binary. The second is a
# stock ubuntu:22.04 with nothing but the host libraries the package declares,
# plus Xvfb and a session bus, and it runs the binary.
#
# What that proves is the claim the whole amendment rests on: a window built
# against GTK 4.16 draws on a machine whose own GTK is 4.6, using nothing from
# the host but the declared libraries.
#
# Usage:
#   packaging/runtime/smoke_runtime.sh            run against packaging/out/runtime
#   packaging/runtime/smoke_runtime.sh --keep     leave the staged tree behind
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNTIME_DIR="$ROOT_DIR/packaging/runtime"
OUT_DIR="${FERMIX_RUNTIME_OUT:-$ROOT_DIR/packaging/out/runtime}"
BUILD_IMAGE="${FERMIX_RUNTIME_IMAGE:-fermix-desktop-runtime-build}"
SMOKE_IMAGE="${FERMIX_RUNTIME_SMOKE_IMAGE:-ubuntu:22.04}"
PREFIX="/usr/lib/fermix-desktop"

KEEP=0
WORK=""

fail() {
  echo "smoke_runtime: $*" >&2
  exit 1
}

log() {
  echo "smoke_runtime: $*"
}

cleanup() {
  if [ "$KEEP" = "0" ] && [ -n "$WORK" ]; then
    rm -rf "$WORK"
  fi
}

parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --keep) KEEP=1; shift ;;
      *) fail "unknown argument: $1" ;;
    esac
  done
}

target_arch() {
  case "$(uname -m)" in
    x86_64) echo "amd64" ;;
    aarch64|arm64) echo "arm64" ;;
    *) fail "unsupported architecture: $(uname -m)" ;;
  esac
}

require_inputs() {
  local arch="$1"
  command -v docker >/dev/null 2>&1 || fail "docker is not installed"
  [ -f "$OUT_DIR/runtime-$arch.tar" ] || fail "no shipped tree at $OUT_DIR/runtime-$arch.tar"
  [ -f "$OUT_DIR/runtime-dev-$arch.tar" ] || fail "no dev tree at $OUT_DIR/runtime-dev-$arch.tar"
  [ -f "$RUNTIME_DIR/smoke/runtime_smoke.c" ] || fail "no smoke program to compile"
  docker image inspect "$BUILD_IMAGE" >/dev/null 2>&1 \
    || fail "$BUILD_IMAGE is not built; run build_runtime.sh --container first"
}

# The compile happens in the build image because that is the image the
# application is compiled in too: same glibc, same compiler, same headers. The
# link line is the one amendment section 4.3 specifies for the application ELF,
# so what runs below is linked exactly as fermix-desktop will be.
compile_smoke() {
  local arch="$1"
  docker run --rm --init \
    -v "$RUNTIME_DIR:/runtime:ro" \
    -v "$OUT_DIR:/out:ro" \
    -v "$WORK:/work" \
    -w /work \
    "$BUILD_IMAGE" bash -euo pipefail -c "
      tar -xf /out/runtime-dev-$arch.tar -C /
      mkdir -p /work/tree
      tar -xf /out/runtime-$arch.tar -C /work/tree
      mkdir -p /work/tree$PREFIX/bin
      export PKG_CONFIG_PATH=$PREFIX/lib/pkgconfig
      gcc -O2 -Wall -Wextra -Werror -o /work/tree$PREFIX/bin/runtime-smoke /runtime/smoke/runtime_smoke.c \
        \$(pkg-config --cflags libadwaita-1 gdk-pixbuf-2.0) \
        \$(pkg-config --libs libadwaita-1 gdk-pixbuf-2.0) \
        -Wl,-rpath,'\$ORIGIN/../lib' -Wl,--enable-new-dtags
      readelf -d /work/tree$PREFIX/bin/runtime-smoke | grep -E 'RUNPATH|RPATH'
      # This container is root and /work is a host directory, so without this the
      # staged tree is root-owned and the cleanup below cannot remove it.
      chown -R $(id -u):$(id -g) /work
    "
  [ -x "$WORK/tree$PREFIX/bin/runtime-smoke" ] || fail "the smoke program did not compile"
  log "compiled runtime-smoke against the dev tree"
}

# ubuntu:22.04 is the oldest deb target: glibc 2.35, GTK 4.6 on the host.
#
# The apt list is the Debian spelling of RUNTIME.lock.json's host_libraries, one
# package per SONAME, and it is written out in full rather than derived because
# what it has to be checkable against is the package's declared Depends. An
# omission here is not a test-harness detail: the first run of this smoke died on
# libxcb-render.so.0, which is on the host list and was missing from this list,
# and that is precisely the bug the smoke exists to find before a user does.
#
# Four additions a base image lacks and a desktop always has: xvfb for a display,
# dbus-x11 for a session bus, fontconfig-config for /etc/fonts, which the private
# fontconfig reads, and one font for it to find. dconf-service and
# gsettings-desktop-schemas are the two the private dconf module talks to.
#
# It runs twice. The cairo pass proves the toolkit draws with nothing from the
# host but the declared libraries. The GL pass proves the other half, and it
# exists because its absence is how a missing host library reached a package:
# with GSK_RENDERER=cairo the process never reaches libepoxy's dlopen at all, so
# libGL, libEGL and libGLESv2 could all have been missing and this smoke would
# still have passed. The GL pass installs Mesa's software driver — the smoke
# image's business, not the package's relations, since a user's machine has a
# real driver — and the program refuses to pass if GTK quietly fell back to
# cairo, because a silent fallback would turn the point of the run into a green
# tick.
run_smoke() {
  local renderer="$1" expect_gl="$2" extra_packages="$3"
  log "pass: the $renderer renderer"
  docker run --rm --init \
    -v "$WORK/tree:/tree:ro" \
    -e "SMOKE_RENDERER=$renderer" \
    -e "SMOKE_EXPECT_GL=$expect_gl" \
    -e "SMOKE_EXTRA_PACKAGES=$extra_packages" \
    "$SMOKE_IMAGE" bash -euo pipefail -c "
      export DEBIAN_FRONTEND=noninteractive
      apt-get update -qq
      apt-get install -y --no-install-recommends -qq \
        libgl1 libegl1 libgbm1 libdrm2 \
        libx11-6 libx11-xcb1 libxcb1 libxcb-render0 libxcb-shm0 \
        libxext6 libxi6 libxcursor1 libxdamage1 libxfixes3 libxrandr2 \
        libxinerama1 libxrender1 libxkbcommon0 \
        libdbus-1-3 zlib1g libyaml-0-2 libcurl4 libzstd1 liblzma5 libgcc-s1 \
        dconf-service gsettings-desktop-schemas \
        fontconfig-config fonts-dejavu-core \
        xvfb xauth dbus-x11 \$SMOKE_EXTRA_PACKAGES >/dev/null
      cp -a /tree/usr /
      echo '--- what the smoke binary needs'
      ldd $PREFIX/bin/runtime-smoke | sort
      echo '--- running'
      export GSETTINGS_SCHEMA_DIR=$PREFIX/share/glib-2.0/schemas
      export GSK_RENDERER=\"\$SMOKE_RENDERER\"
      # Mesa's software rasteriser, so the GL pass has a driver to find on a
      # machine with no GPU. Harmless in the cairo pass, which never looks.
      export LIBGL_ALWAYS_SOFTWARE=1
      export GALLIUM_DRIVER=llvmpipe
      # The smoke makes every GTK warning fatal, and a bare container has no
      # accessibility bus, so GTK's own \"set GTK_A11Y to none\" is taken rather
      # than the fatality relaxed. This is a property of the container, not of
      # the runtime: a desktop session provides org.a11y.Bus, and the
      # application must NOT set this.
      export GTK_A11Y=none
      export HOME=/root
      dbus-run-session -- xvfb-run -a $PREFIX/bin/runtime-smoke
    "
}

main() {
  local arch
  parse_args "$@"
  arch="$(target_arch)"
  require_inputs "$arch"

  WORK="$(mktemp -d "${TMPDIR:-/tmp}/fermix-runtime-smoke.XXXXXX")"
  trap cleanup EXIT

  compile_smoke "$arch"
  # Cairo first: if the toolkit cannot draw at all, a GL failure would be the
  # less interesting of the two answers.
  run_smoke cairo "" ""
  run_smoke gl 1 "libgl1-mesa-dri libglx-mesa0"
  log "the private runtime draws a libadwaita window on $SMOKE_IMAGE, with cairo and with GL"
}

main "$@"
