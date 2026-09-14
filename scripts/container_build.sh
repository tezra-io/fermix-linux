#!/usr/bin/env bash
#
# Build and gate the crate inside the build container.
#
# This is the run that matters: the development machine is a Mac and the product
# is a Linux application, so a build that has only ever happened on Homebrew's
# GTK has proven nothing about the toolkit floor. The same image runs here and
# in CI.
#
# Usage:
#   scripts/container_build.sh              build the image if needed, then gate
#   scripts/container_build.sh --rebuild    rebuild the image first
#   scripts/container_build.sh --shell      open a shell inside it instead
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${FERMIX_BUILD_IMAGE:-fermix-desktop-build}"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.build"
CACHE_VOLUME="${FERMIX_CARGO_CACHE:-fermix-desktop-cargo}"

REBUILD=0
SHELL_ONLY=0

fail() {
  echo "container_build: $*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --rebuild) REBUILD=1; shift ;;
    --shell) SHELL_ONLY=1; shift ;;
    *) fail "unknown argument: $1" ;;
  esac
done

command -v docker >/dev/null 2>&1 || fail "docker is not installed"
[ -f "$DOCKERFILE" ] || fail "no build container at $DOCKERFILE"

if [ "$REBUILD" = "1" ] || ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "container_build: building $IMAGE"
  docker build -f "$DOCKERFILE" -t "$IMAGE" "$ROOT_DIR/packaging/docker"
fi

# The cargo registry and the build directory live in a named volume, so a second
# run compiles the crate rather than the world. The repository is bind-mounted;
# nothing is copied into the image.
docker volume create "$CACHE_VOLUME" >/dev/null

run() {
  # `--init` gives the container a real init as process one. Without it the
  # display wrapper becomes process one itself, and a wrapper that is also the
  # reaper does not always get its own child's exit, which leaves the run
  # hanging after the tests have already passed.
  docker run --rm --init \
    -v "$ROOT_DIR:/workspace" \
    -v "$CACHE_VOLUME:/cache" \
    -e CARGO_HOME=/cache/cargo \
    -e CARGO_TARGET_DIR=/cache/target \
    -e FERMIX_DESKTOP_TEST_ROOT=/tmp/fermix-desktop-tests \
    -w /workspace \
    "$@"
}

if [ "$SHELL_ONLY" = "1" ]; then
  exec docker run --rm -it --init \
    -v "$ROOT_DIR:/workspace" \
    -v "$CACHE_VOLUME:/cache" \
    -e CARGO_HOME=/cache/cargo \
    -e CARGO_TARGET_DIR=/cache/target \
    -w /workspace \
    "$IMAGE" bash
fi

# `bash -c`, never `bash -lc`: a login shell re-reads /etc/profile, which resets
# PATH and loses the toolchain the image put on it.
echo "container_build: gates, inside $IMAGE"
run "$IMAGE" bash -c '
set -euo pipefail
cd App/Fermix

echo "== cargo fmt"
cargo fmt --check

echo "== cargo clippy"
cargo clippy --all-targets -- -D warnings

echo "== cargo test"
# Each test binary statically links the whole toolkit, and this crate has eight
# of them. Linking them all at once is what exhausts a container the linker is
# killed in, so the job count is bounded here rather than left to the core
# count.
cargo test --jobs 2

echo "== cargo test, with a display"
FERMIX_GTK_TESTS=1 xvfb-run -a cargo test --jobs 2 --test widgets -- --nocapture
'

echo "container_build: the crate builds and every gate passes on the toolkit floor"
