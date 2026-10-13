#!/usr/bin/env bash
# Offline tests for install_smoke.sh: the images and packages it refuses before it creates a
# container, and the plan it prints for each image it runs on. Every case here stops before docker
# is reached, and the test checks that no container of the smoke's name was left. The smoke itself,
# an install on each image, is not run here: it needs the network and a built package.
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

# The smoke runs natively, so this machine's architecture decides which packages it installs.
case "$(uname -m)" in
  x86_64) native=amd64 native_rpm=x86_64 other=arm64 other_rpm=aarch64 ;;
  aarch64) native=arm64 native_rpm=aarch64 other=amd64 other_rpm=x86_64 ;;
  *) fail "the smoke runs on x86_64 and aarch64 machines, not $(uname -m)" ;;
esac

mkdir -p "$work/packages"
for name in "fermix-desktop_0.0.1_$native.deb" "fermix-desktop-0.0.1-1.$native_rpm.rpm" \
  "fermix_0.0.1_$native.deb" "fermix-desktop_0.0.1-1_$native.deb" "fermix-desktop_0.0.1_$other.deb" \
  "fermix-desktop-0.0.1-1.$other_rpm.rpm" "fermix-desktop_0.0.1_i386.deb" \
  "fermix-desktop_0.0.1+dev.20261004000000.0123456789ab.dirty_$native.deb"; do
  printf 'not a package\n' > "$work/packages/$name"
done
deb="$work/packages/fermix-desktop_0.0.1_$native.deb"
rpm="$work/packages/fermix-desktop-0.0.1-1.$native_rpm.rpm"

echo "install_smoke_test: arguments it refuses"
expect_refusal "no arguments" "usage" smoke
expect_refusal "no image" "usage" smoke "$deb"
expect_refusal "no package" "usage" smoke --image ubuntu:22.04
expect_refusal "two packages" "usage" smoke --image ubuntu:22.04 "$deb" "$rpm"
expect_refusal "an unknown flag" "usage" smoke --image ubuntu:22.04 "$deb" --keep
expect_refusal "a display named by the caller: the image decides" "usage" \
  smoke --image ubuntu:26.04 --wayland "$deb"

echo "install_smoke_test: images it does not run on"
images="ubuntu:22.04, ubuntu:24.04, ubuntu:26.04, debian:12, debian:13, fedora:44, almalinux:9 and almalinux:10"
expect_refusal "another distribution" "the smoke runs on $images, not opensuse/leap:15.6" \
  smoke --image opensuse/leap:15.6 "$rpm"
expect_refusal "another release of one it knows" "not fedora:43" smoke --image fedora:43 "$rpm"
expect_refusal "an older Debian" "not debian:11" smoke --image debian:11 "$deb"
expect_refusal "a tag that moves" "not ubuntu:latest" smoke --image ubuntu:latest "$deb"
expect_refusal "an image named by a digest of the caller's" "not ubuntu:22.04@sha256:" \
  smoke --image "ubuntu:22.04@sha256:$(printf '0%.0s' {1..64})" "$deb"

echo "install_smoke_test: packages it does not install"
expect_refusal "a package that is not there" "no package at $work/packages/absent.deb" \
  smoke --image ubuntu:22.04 "$work/packages/absent.deb"
expect_refusal "an rpm on Ubuntu" "ubuntu:22.04 installs a deb" smoke --image ubuntu:22.04 "$rpm"
expect_refusal "an rpm on Debian" "debian:13 installs a deb" smoke --image debian:13 "$rpm"
expect_refusal "a deb on Fedora" "fedora:44 installs an rpm" smoke --image fedora:44 "$deb"
expect_refusal "a deb on AlmaLinux" "almalinux:10 installs an rpm" smoke --image almalinux:10 "$deb"
expect_refusal "the engine's own package" "is not a fermix-desktop package" \
  smoke --image ubuntu:22.04 "$work/packages/fermix_0.0.1_$native.deb"
expect_refusal "a deb with a Debian revision" "is not a fermix-desktop package" \
  smoke --image ubuntu:22.04 "$work/packages/fermix-desktop_0.0.1-1_$native.deb"
expect_refusal "a deb for the other architecture" \
  "fermix-desktop_0.0.1_$other.deb is for $other, and this machine is $native: the smoke runs natively" \
  smoke --image ubuntu:24.04 "$work/packages/fermix-desktop_0.0.1_$other.deb"
expect_refusal "an rpm for the other architecture" \
  "fermix-desktop-0.0.1-1.$other_rpm.rpm is for $other, and this machine is $native: the smoke runs natively" \
  smoke --image almalinux:9 "$work/packages/fermix-desktop-0.0.1-1.$other_rpm.rpm"
expect_refusal "a deb for an architecture the release does not build" \
  "fermix-desktop_0.0.1_i386.deb is for i386, and this machine is $native" \
  smoke --image debian:12 "$work/packages/fermix-desktop_0.0.1_i386.deb"

# <image> <family> <displays>: the plan the smoke prints for <image> names it pinned by a digest,
# then the family, this machine's architecture and the displays it draws on. It names no engine:
# the engine version `fermix --version` must answer with is what the installed package provides,
# read in the container.
expect_plan() {
  local image="$1" family="$2" displays="$3" package plan first
  package="$deb"
  [ "$family" = deb ] || package="$rpm"
  plan="$(smoke --image "$image" "$package" --print-plan)" || fail "$image printed no plan"
  first="${plan%%$'\n'*}"
  [[ "$first" == "image $image@sha256:"* ]] && [[ "${first#*@sha256:}" =~ ^[0-9a-f]{64}$ ]] ||
    fail "$image is not pinned by a digest in its plan: $plan"
  [ "${plan#*$'\n'}" = "$(printf 'family %s\narch %s\ndisplays %s' "$family" "$native" "$displays")" ] ||
    fail "$image's plan is not $family on $native drawing on $displays: $plan"
  echo "  ok: $image: $family on $native, drawn on $displays"
}

echo "install_smoke_test: the plan for each image it runs on"
expect_plan ubuntu:22.04 deb "x11"
expect_plan ubuntu:24.04 deb "x11"
expect_plan ubuntu:26.04 deb "x11 wayland"
expect_plan debian:12 deb "x11"
expect_plan debian:13 deb "x11"
expect_plan fedora:44 rpm "x11 wayland"
expect_plan almalinux:9 rpm "x11"
expect_plan almalinux:10 rpm "wayland"

echo "install_smoke_test: a development package"
dev="$work/packages/fermix-desktop_0.0.1+dev.20261004000000.0123456789ab.dirty_$native.deb"
[ "$(smoke --image ubuntu:22.04 "$dev" --print-plan | tail -n 1)" = "displays x11" ] ||
  fail "a development deb has no plan for ubuntu:22.04"
echo "  ok: planned like a release deb"

[ -z "$(docker ps -aq --filter 'name=^fermix-desktop-pkg-package-smoke-')" ] ||
  fail "a refused smoke left a container"
[ ! -e "$work/cache/out" ] || fail "a refused smoke left an output directory"

echo "install_smoke_test: ok"
