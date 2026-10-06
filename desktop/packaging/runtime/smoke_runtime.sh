#!/usr/bin/env bash
#
# Prove the private runtime runs on the oldest deb target it supports.
#
# The build image compiles smoke/runtime_smoke.c against the dev tree, linked
# with the RUNPATH the application ELF carries. A stock ubuntu:22.04, GTK 4.6
# on the host, then gets the shipped tree, that binary and gst-inspect-1.0, and
# nothing from apt but the host libraries the boundary declares and a display.
# There the window is drawn under Xvfb twice, once with the cairo renderer and
# once with GL on Mesa's software driver, and every GStreamer element the voice
# call builds is looked up through the compiled-in plugin path.
#
# The GL pass is the one that reaches libepoxy's dlopen of the host GL
# libraries, which no NEEDED entry names.
#
# Usage: desktop/packaging/runtime/smoke_runtime.sh
#   Reads $FERMIX_RUNTIME_OUT (default ~/.cache/fermix-desktop-runtime/out) and
#   writes the screenshots to its smoke/ directory.
set -euo pipefail
shopt -s inherit_errexit

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNTIME_DIR="$ROOT_DIR/packaging/runtime"
MARKS_DIR="$ROOT_DIR/app/marks"
CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}/fermix-desktop-runtime"
OUT_DIR="${FERMIX_RUNTIME_OUT:-$CACHE_HOME/out}"
BUILD_IMAGE="${FERMIX_RUNTIME_IMAGE:-fermix-desktop-pkg-runtime-build}"
SMOKE_IMAGE="${FERMIX_RUNTIME_SMOKE_IMAGE:-ubuntu@sha256:b8b6ee6aa931ecd9d0d952abc34dc0e5f7c6a30c6bb71b079fe399fde0329c02}"
# Every element desktop/app/src/audio.rs makes, and level.
ELEMENTS="pulsesrc pulsesink webrtcdsp webrtcechoprobe level audioconvert audioresample capsfilter audiotestsrc fakesink appsrc appsink"

# What the EXIT trap removes. A trap runs after every function has returned.
WORK=""
CONTAINER=""

fail() {
  echo "smoke_runtime: $*" >&2
  exit 1
}

log() {
  echo "smoke_runtime: $*" >&2
}

cleanup() {
  [ -z "$CONTAINER" ] || docker rm -f "$CONTAINER" >/dev/null
  [ -z "$WORK" ] || rm -rf "$WORK"
}

parse_args() {
  case "${1:-}" in
    "") ;;
    -h|--help) echo "usage: smoke_runtime.sh" >&2; exit 0 ;;
    *) fail "unknown argument: $1" ;;
  esac
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
  [ -f "$RUNTIME_DIR/smoke/sample.gif" ] || fail "no GIF to decode"
  [ -d "$MARKS_DIR" ] || fail "no marks at $MARKS_DIR"
  docker image inspect "$BUILD_IMAGE" >/dev/null 2>&1 \
    || fail "$BUILD_IMAGE is not built; run build_runtime.sh --container first"
}

# Copies a file out of the container and holds it to the digest the container wrote.
copy_out_checked() {
  local path="$1" dest="$2"
  docker cp "$CONTAINER:$path" "$dest"
  docker cp "$CONTAINER:$path.sha256" "$dest.sha256"
  [ "$(cut -d' ' -f1 < "$dest.sha256")" = "$(sha256sum "$dest" | cut -d' ' -f1)" ] \
    || fail "$(basename -- "$dest") changed on its way out of the container"
}

# The link line is the one the application ELF uses: RUNPATH $ORIGIN/../lib
# from /usr/lib/fermix-desktop/bin, and nothing else.
compile_smoke() {
  local arch="$1"
  CONTAINER="fermix-desktop-pkg-runtime-smoke-compile-$$"
  docker create --name "$CONTAINER" "$BUILD_IMAGE" bash -euo pipefail -c '
    tar -xf /tmp/runtime-dev.tar -C /
    export PKG_CONFIG_PATH=/usr/lib/fermix-desktop/lib/pkgconfig:/usr/lib/fermix-desktop/share/pkgconfig
    gcc -O2 -Wall -Wextra -Werror -o /tmp/runtime-smoke /tmp/runtime_smoke.c \
      $(pkg-config --cflags --libs libadwaita-1) \
      -Wl,-rpath,"\$ORIGIN/../lib" -Wl,--enable-new-dtags
    readelf -d /tmp/runtime-smoke | grep -E "RUNPATH|RPATH"
    cp /usr/lib/fermix-desktop/bin/gst-inspect-1.0 /tmp/gst-inspect-1.0
    for f in /tmp/runtime-smoke /tmp/gst-inspect-1.0; do sha256sum "$f" > "$f.sha256"; done
  ' >/dev/null
  docker cp "$OUT_DIR/runtime-dev-$arch.tar" "$CONTAINER:/tmp/runtime-dev.tar"
  docker cp "$RUNTIME_DIR/smoke/runtime_smoke.c" "$CONTAINER:/tmp/runtime_smoke.c"
  docker start -a "$CONTAINER" >&2 || fail "the smoke program did not compile"
  copy_out_checked /tmp/runtime-smoke "$WORK/runtime-smoke"
  copy_out_checked /tmp/gst-inspect-1.0 "$WORK/gst-inspect-1.0"
  docker rm "$CONTAINER" >/dev/null
  CONTAINER=""
  log "compiled runtime-smoke against the dev tree"
}

# The apt list is the Debian spelling of RUNTIME.lock.json's host_libraries,
# written out so it can be read against the package's declared dependencies.
# The rest is what a desktop has and a base image lacks: a display, a session
# bus, /etc/fonts and a font, and the dconf service the private module talks to.
# shellcheck disable=SC2016 # expanded inside the container
SMOKE_SCRIPT='
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y --no-install-recommends -qq \
    libgl1 libegl1 libgbm1 libdrm2 libgles2 \
    libx11-6 libx11-xcb1 libxcb1 libxcb-render0 libxcb-shm0 \
    libxext6 libxi6 libxcursor1 libxdamage1 libxfixes3 libxrandr2 \
    libxinerama1 libxrender1 libxkbcommon0 \
    libdbus-1-3 zlib1g libyaml-0-2 libcurl4 libzstd1 liblzma5 libgcc-s1 libstdc++6 libpulse0 \
    dconf-service gsettings-desktop-schemas fontconfig-config fonts-dejavu-core \
    xvfb xauth dbus-x11 $SMOKE_EXTRA_PACKAGES >/dev/null
  tar -xf /tmp/runtime.tar -C /
  install -D -m 0755 -t /usr/lib/fermix-desktop/bin /tmp/runtime-smoke /tmp/gst-inspect-1.0
  echo "--- the libraries the window resolves outside the prefix"
  ldd /usr/lib/fermix-desktop/bin/runtime-smoke | grep -v /usr/lib/fermix-desktop/ | sort
  export HOME=/root GSETTINGS_SCHEMA_DIR=/usr/lib/fermix-desktop/share/glib-2.0/schemas
  export GSK_RENDERER="$SMOKE_RENDERER" LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
  # A bare container has no accessibility bus, and the smoke makes warnings
  # fatal. A desktop session has one; the application must not set this.
  export GTK_A11Y=none
  export SMOKE_MARKS=/tmp/marks SMOKE_SCREENSHOT=/tmp/screenshot.png
  export SMOKE_PNG=/tmp/marks/channels/slack-color.png SMOKE_WEBP=/tmp/marks/channels/whatsapp-color.webp
  export SMOKE_GIF=/tmp/sample.gif
  dbus-run-session -- xvfb-run -a -s "-screen 0 1280x800x24" /usr/lib/fermix-desktop/bin/runtime-smoke
  sha256sum /tmp/screenshot.png > /tmp/screenshot.png.sha256
  unset GST_PLUGIN_SYSTEM_PATH GST_PLUGIN_SYSTEM_PATH_1_0 GST_PLUGIN_PATH GST_PLUGIN_PATH_1_0
  for element in $SMOKE_ELEMENTS; do
    /usr/lib/fermix-desktop/bin/gst-inspect-1.0 "$element" > /tmp/inspect.txt
    plugin="$(awk "/^  Filename/ { print \$2; exit }" /tmp/inspect.txt)"
    case "$plugin" in
      /usr/lib/fermix-desktop/lib/gstreamer-1.0/*) echo "gst-inspect-1.0 $element: $plugin" ;;
      *) echo "gst-inspect-1.0 $element: loaded from \"$plugin\", not the prefix" >&2; exit 1 ;;
    esac
  done
'

run_pass() {
  local arch="$1" renderer="$2" expect_gl="$3" extra="$4"
  log "pass: the $renderer renderer on $SMOKE_IMAGE"
  CONTAINER="fermix-desktop-pkg-runtime-smoke-$renderer-$$"
  docker create --name "$CONTAINER" --init \
    -e "SMOKE_RENDERER=$renderer" -e "SMOKE_EXPECT_GL=$expect_gl" \
    -e "SMOKE_EXTRA_PACKAGES=$extra" -e "SMOKE_ELEMENTS=$ELEMENTS" \
    "$SMOKE_IMAGE" bash -euo pipefail -c "$SMOKE_SCRIPT" >/dev/null
  docker cp "$OUT_DIR/runtime-$arch.tar" "$CONTAINER:/tmp/runtime.tar"
  docker cp "$WORK/runtime-smoke" "$CONTAINER:/tmp/runtime-smoke"
  docker cp "$WORK/gst-inspect-1.0" "$CONTAINER:/tmp/gst-inspect-1.0"
  docker cp "$MARKS_DIR" "$CONTAINER:/tmp/marks"
  docker cp "$RUNTIME_DIR/smoke/sample.gif" "$CONTAINER:/tmp/sample.gif"
  docker start -a "$CONTAINER" >&2 || fail "the $renderer pass failed"
  mkdir -p "$OUT_DIR/smoke"
  copy_out_checked /tmp/screenshot.png "$OUT_DIR/smoke/smoke-$renderer.png"
  docker rm "$CONTAINER" >/dev/null
  CONTAINER=""
  log "the $renderer pass drew the window: $OUT_DIR/smoke/smoke-$renderer.png"
}

main() {
  local arch
  parse_args "$@"
  arch="$(target_arch)"
  require_inputs "$arch"
  WORK="$(mktemp -d "${TMPDIR:-/tmp}/fermix-runtime-smoke.XXXXXX")"
  trap cleanup EXIT
  compile_smoke "$arch"
  # Cairo first: if the toolkit cannot draw at all, GL is the less useful answer.
  run_pass "$arch" cairo "" ""
  run_pass "$arch" gl 1 "libgl1-mesa-dri libglx-mesa0 libegl-mesa0"
  log "the runtime draws a libadwaita window on $SMOKE_IMAGE with cairo and with GL"
}

main "$@"
