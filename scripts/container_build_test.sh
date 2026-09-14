#!/usr/bin/env bash
#
# Exercise container_build.sh's refusals without building anything.
#
# The gate it wraps takes minutes; its argument handling and its preconditions
# take milliseconds, and those are the parts that break silently. Docker itself
# is never invoked here: the script is driven with a stand-in on PATH.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/container_build.sh"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/container-build-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fail() {
  echo "container_build_test: $*" >&2
  exit 1
}

echo "container_build_test: the script parses"
bash -n "$SCRIPT" || fail "the script does not parse"
echo "  ok: shell syntax"

echo "container_build_test: refusals"

if "$SCRIPT" --unknown-argument >/dev/null 2>&1; then
  fail "an unknown argument was accepted"
fi
echo "  refused: an unknown argument"

# A PATH with no docker on it at all.
mkdir -p "$WORK/empty-bin"
if PATH="$WORK/empty-bin" "$SCRIPT" >/dev/null 2>&1; then
  fail "a host without docker was accepted"
fi
echo "  refused: a host with no container runtime"

# A repository whose build container is missing.
mkdir -p "$WORK/no-dockerfile/scripts"
cp "$SCRIPT" "$WORK/no-dockerfile/scripts/"
if bash "$WORK/no-dockerfile/scripts/container_build.sh" >/dev/null 2>&1; then
  fail "a repository with no build container was accepted"
fi
echo "  refused: a repository with no build container"

echo "container_build_test: the container it declares"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.build"
[ -f "$DOCKERFILE" ] || fail "no build container at $DOCKERFILE"

# xauth is listed beside xvfb deliberately: xvfb-run refuses without it, and
# the refusal reads as a broken test rather than a missing package. The two
# image loaders are listed for the same reason: without them a vendor mark in
# SVG or WebP takes its declared no-mark treatment, so the captures a reviewer
# looks at would show the neutral symbol on most of the product's rows.
# `rpm` is listed for the same kind of reason: the release rail reads the built
# rpm's own requirements back, and nFPM writes no dependency it was not told to.
for needed in libgtk-4-dev libadwaita-1-dev gettext appstream desktop-file-utils \
              rpm xvfb xauth librsvg2-bin librsvg2-common webp-pixbuf-loader \
              python3 git curl; do
  grep -q "$needed" "$DOCKERFILE" || fail "the build container does not install $needed"
done
echo "  ok: every gate's dependency is installed"

grep -q 'sha256sum -c -' "$DOCKERFILE" || fail "nfpm is fetched without a digest check"
grep -qE 'ARG NFPM_VERSION=[0-9]+\.[0-9]+\.[0-9]+' "$DOCKERFILE" \
  || fail "nfpm is not pinned to a version"
grep -qE 'ARG RUST_VERSION=[0-9]+\.[0-9]+\.[0-9]+' "$DOCKERFILE" \
  || fail "the toolchain is not pinned to a version"
echo "  ok: the toolchain and nfpm are pinned, and nfpm is checked by digest"

echo "container_build_test: every refusal fired"
