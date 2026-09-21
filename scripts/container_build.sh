#!/usr/bin/env bash
#
# Build and gate the crate inside the build container.
#
# This is the run that matters. The product carries its own GTK, and the image
# below is the only place that GTK exists before a package is built, so a build
# against whatever toolkit a development machine happens to have proves nothing
# about what ships. The same image runs here and in CI.
#
# The image is AlmaLinux 9 with the private toolkit copied in at the absolute
# path it will occupy on a user's machine, so the crate compiles against the
# libraries it will link. scripts/runtime_image.sh decides where that toolkit
# comes from: a published image in CI, a locally built tree on a developer's
# machine.
#
# Usage:
#   scripts/container_build.sh              build the image if needed, then gate
#   scripts/container_build.sh --rebuild    rebuild the image first
#   scripts/container_build.sh --shell      open a shell inside it instead
#   scripts/container_build.sh --arch A     build for amd64 or arm64 rather
#                                           than for this machine
#
# A binary in target/debug is the one case the design allows LD_LIBRARY_PATH:
# $ORIGIN/../lib does not resolve from there, so the run below exports it. That
# is a property of the container and not of the product, and
# scripts/check_private_runtime.sh asserts the installed ELF needs no such
# variable (amendment section 4.3).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${FERMIX_BUILD_IMAGE:-fermix-desktop-build}"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.build"
CACHE_VOLUME="${FERMIX_CARGO_CACHE:-fermix-desktop-cargo}"
PREFIX="/usr/lib/fermix-desktop"

REBUILD=0
SHELL_ONLY=0
ARCH=""

# shellcheck source=scripts/container_cache.sh
source "$ROOT_DIR/scripts/container_cache.sh"

fail() {
  echo "container_build: $*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --rebuild) REBUILD=1; shift ;;
    --shell) SHELL_ONLY=1; shift ;;
    --arch)
      [ "$#" -ge 2 ] || fail "--arch needs amd64 or arm64"
      ARCH="$2"
      shift 2
      ;;
    *) fail "unknown argument: $1" ;;
  esac
done

command -v docker >/dev/null 2>&1 || fail "docker is not installed"
[ -f "$DOCKERFILE" ] || fail "no build container at $DOCKERFILE"

# The architecture is this machine's: the image carries a toolkit compiled for
# one, and there is no cross build.
if [ -z "$ARCH" ]; then
  case "$(uname -m)" in
    x86_64) ARCH="amd64" ;;
    aarch64|arm64) ARCH="arm64" ;;
    *) fail "this build does not run on $(uname -m)" ;;
  esac
fi

if [ "$REBUILD" = "1" ] || ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  RUNTIME_IMAGE="$("$ROOT_DIR/scripts/runtime_image.sh" --arch "$ARCH")" ||
    fail "there is no private toolkit to build the image from"
  echo "container_build: building $IMAGE from $RUNTIME_IMAGE"
  docker build -f "$DOCKERFILE" -t "$IMAGE" \
    --build-arg "RUNTIME_IMAGE=$RUNTIME_IMAGE" \
    --build-arg "HOST_CAPABILITIES=$(host_capabilities "$ROOT_DIR/scripts/host_relations.map")" \
    --build-arg "RUNTIME_CACHE_KEY=$(tr -d '[:space:]' < "$ROOT_DIR/packaging/out/runtime/cache-key" 2>/dev/null || true)" \
    "$ROOT_DIR/packaging/docker"
fi

# The cargo registry and the build directory live in a named volume, so a second
# run compiles the crate rather than the world. The repository is bind-mounted;
# nothing is copied into the image, and the container runs as the person who
# invoked it so that nothing it writes into the tree is root-owned.
container_cache_prepare "$IMAGE" "$CACHE_VOLUME" ||
  fail "could not hand the cargo cache volume to $(id -u):$(id -g)"

mapfile -t USER_FLAGS < <(container_user_flags)

run() {
  # `--init` gives the container a real init as process one. Without it the
  # display wrapper becomes process one itself, and a wrapper that is also the
  # reaper does not always get its own child's exit, which leaves the run
  # hanging after the tests have already passed.
  docker run --rm --init \
    "${USER_FLAGS[@]}" \
    -v "$ROOT_DIR:/workspace" \
    -v "$CACHE_VOLUME:/cache" \
    -e HOME=/tmp \
    -e CARGO_HOME=/cache/cargo \
    -e CARGO_TARGET_DIR=/cache/target \
    -e FERMIX_DESKTOP_TEST_ROOT=/tmp/fermix-desktop-tests \
    -e "LD_LIBRARY_PATH=$PREFIX/lib" \
    -w /workspace \
    "$@"
}

if [ "$SHELL_ONLY" = "1" ]; then
  exec docker run --rm -it --init \
    "${USER_FLAGS[@]}" \
    -v "$ROOT_DIR:/workspace" \
    -v "$CACHE_VOLUME:/cache" \
    -e HOME=/tmp \
    -e CARGO_HOME=/cache/cargo \
    -e CARGO_TARGET_DIR=/cache/target \
    -e "LD_LIBRARY_PATH=$PREFIX/lib" \
    -w /workspace \
    "$IMAGE" bash
fi

# `bash -c`, never `bash -lc`: a login shell re-reads /etc/profile, which resets
# PATH and loses the toolchain the image put on it.
echo "container_build: gates, inside $IMAGE"
# shellcheck disable=SC2016  # this block is evaluated inside the container, not here
run "$IMAGE" bash -c '
set -euo pipefail
cd App/Fermix

echo "== cargo fmt"
cargo fmt --check

echo "== cargo clippy"
cargo clippy --all-targets -- -D warnings

echo "== cargo test"
# Each test binary links the whole toolkit, and this crate has eight of them.
# Linking them all at once is what exhausts a container the linker is killed
# in, so the job count is bounded here rather than left to the core count.
#
# --no-fail-fast because without it cargo stops at the first test binary that
# fails and never runs the rest, so one broken test hides every other failure
# in the run. A gate that reports one problem at a time costs a container start
# per problem.
source /workspace/scripts/crate_gates.sh
expected="$(crate_test_binary_count /workspace/App/Fermix)"
cargo test --no-fail-fast --jobs 2 2>&1 | tee /tmp/cargo-test.log
[ "${PIPESTATUS[0]}" = "0" ] || { echo "the crate tests failed" >&2; exit 1; }

# A zero exit from --no-fail-fast does not mean the suite ran. A test binary
# that dies at load prints no result line at all and the run carries on, so the
# results are counted against the binaries cargo built.
check_every_test_binary_reported /tmp/cargo-test.log "$expected"

echo "== cargo test, with a display"
FERMIX_GTK_TESTS=1 xvfb-run "${XVFB_ARGUMENTS[@]}" \
  cargo test --no-fail-fast --jobs 2 --test widgets -- --nocapture
'

echo "container_build: the crate builds and every gate passes against the toolkit that ships"
