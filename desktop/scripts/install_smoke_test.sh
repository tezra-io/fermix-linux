#!/usr/bin/env bash
# Offline tests for install_smoke.sh: the images and packages it refuses before it creates a
# container. Every case here fails before docker is reached, and the test checks that no container
# of the smoke's name was left. The smoke itself, an install on ubuntu:22.04 and fedora:44, is not
# run here: it needs the network and a built package.
#   desktop/scripts/install_smoke_test.sh
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export FERMIX_PACKAGE_CACHE="$work/cache"

fail() {
  echo "install_smoke_test: $*" >&2
  exit 1
}

expect_refusal() {
  local what="$1" reason="$2" err="$work/stderr"
  shift 2
  if "$@" > /dev/null 2> "$err"; then
    fail "$what was accepted"
  fi
  grep -qF -- "$reason" "$err" || fail "$what was refused for another reason: $(cat "$err")"
  echo "  refused: $what"
}

smoke() {
  "$here/install_smoke.sh" "$@"
}

mkdir -p "$work/packages"
for name in fermix-desktop_0.0.1_amd64.deb fermix-desktop-0.0.1-1.x86_64.rpm \
  fermix_0.0.1_amd64.deb fermix-desktop_0.0.1-1_amd64.deb fermix-desktop_0.0.1_arm64.deb \
  fermix-desktop-0.0.1-1.aarch64.rpm fermix-desktop_0.0.1+0.dev.20261004000000.0123456789ab.dirty_amd64.deb; do
  printf 'not a package\n' > "$work/packages/$name"
done
deb="$work/packages/fermix-desktop_0.0.1_amd64.deb"
rpm="$work/packages/fermix-desktop-0.0.1-1.x86_64.rpm"

echo "install_smoke_test: arguments it refuses"
expect_refusal "no arguments" "usage" smoke
expect_refusal "no image" "usage" smoke "$deb"
expect_refusal "no package" "usage" smoke --image ubuntu:22.04
expect_refusal "two packages" "usage" smoke --image ubuntu:22.04 "$deb" "$rpm"
expect_refusal "an unknown flag" "usage" smoke --image ubuntu:22.04 "$deb" --keep

echo "install_smoke_test: images it does not run on"
expect_refusal "another distribution" "the smoke runs on ubuntu:22.04 and fedora:44, not debian:12" \
  smoke --image debian:12 "$deb"
expect_refusal "another release of one it knows" "not fedora:43" smoke --image fedora:43 "$rpm"
expect_refusal "an image named by a digest of the caller's" "not ubuntu:22.04@sha256:" \
  smoke --image "ubuntu:22.04@sha256:$(printf '0%.0s' {1..64})" "$deb"

echo "install_smoke_test: packages it does not install"
expect_refusal "a package that is not there" "no package at $work/packages/absent.deb" \
  smoke --image ubuntu:22.04 "$work/packages/absent.deb"
expect_refusal "an rpm on Ubuntu" "ubuntu:22.04 installs a deb" smoke --image ubuntu:22.04 "$rpm"
expect_refusal "a deb on Fedora" "fedora:44 installs an rpm" smoke --image fedora:44 "$deb"
expect_refusal "the engine's own package" "is not a fermix-desktop package" \
  smoke --image ubuntu:22.04 "$work/packages/fermix_0.0.1_amd64.deb"
expect_refusal "a deb with a Debian revision" "is not a fermix-desktop package" \
  smoke --image ubuntu:22.04 "$work/packages/fermix-desktop_0.0.1-1_amd64.deb"
expect_refusal "a deb for another architecture" "is not a fermix-desktop package for amd64" \
  smoke --image ubuntu:22.04 "$work/packages/fermix-desktop_0.0.1_arm64.deb"
expect_refusal "an rpm for another architecture" "is not a fermix-desktop package for amd64" \
  smoke --image fedora:44 "$work/packages/fermix-desktop-0.0.1-1.aarch64.rpm"

echo "install_smoke_test: the engine version a package name carries"
[ "$(smoke --image ubuntu:22.04 "$deb" --print-engine-version)" = 0.0.1 ] ||
  fail "the deb's engine version is not 0.0.1"
[ "$(smoke --image fedora:44 "$rpm" --print-engine-version)" = 0.0.1 ] ||
  fail "the rpm's engine version is not 0.0.1"
dev="$work/packages/fermix-desktop_0.0.1+0.dev.20261004000000.0123456789ab.dirty_amd64.deb"
[ "$(smoke --image ubuntu:22.04 "$dev" --print-engine-version)" = 0.0.1 ] ||
  fail "a development deb's engine version is not 0.0.1"
echo "  ok: 0.0.1 from a release deb, a release rpm and a development deb"

[ -z "$(docker ps -aq --filter 'name=^fermix-desktop-pkg-package-smoke-')" ] ||
  fail "a refused smoke left a container"
[ ! -e "$work/cache/out" ] || fail "a refused smoke left an output directory"

echo "install_smoke_test: ok"
