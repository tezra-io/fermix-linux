#!/usr/bin/env bash
#
# Installs a built fermix-desktop package on a clean distribution image and checks that it works
# there: the package manager installs it with its declared relations, `fermix --version` answers,
# ldd resolves every object under /usr/lib/fermix-desktop with the toolkit from the private prefix,
# and the window draws under Xvfb, in the state it shows when Fermix is not running.
#
#   install_smoke.sh --image <ubuntu:22.04|fedora:44> <package> [--out <dir>]
#   install_smoke.sh --image <ubuntu:22.04|fedora:44> <package> --print-engine-version
#
#   --out <dir>   where the screenshot and the logs go, by default
#                 $FERMIX_PACKAGE_CACHE/out/smoke/<image>, with the image's ':' as '-'
#
# Each image is pinned by digest here. The package goes into the container by docker cp and is
# checked there against its sha256; desktop/packaging/smoke_in_container.sh runs the checks; what
# it writes comes back by docker cp and is checked against the SHA256SUMS it wrote. Its screen dump
# is then decoded and checked by desktop/packaging/convert_screenshot.sh in one converter image,
# pinned here with its packages, the same for every distribution, so no image under test decodes
# its own capture. Both containers are removed on every exit. The images reach their mirrors for
# the package's dependencies, for Xvfb, and for ImageMagick.
set -euo pipefail
shopt -s inherit_errexit

UBUNTU_IMAGE="ubuntu:22.04@sha256:b8b6ee6aa931ecd9d0d952abc34dc0e5f7c6a30c6bb71b079fe399fde0329c02"
FEDORA_IMAGE="fedora:44@sha256:43b29f65a41eb9c35e1cd5323e3bdf3b655c2357a9f4f1ff2f9c2798e5045d80"
# The converter: Alpine 3.22 with ImageMagick 7.1.2-15, which reads xwd's 32-bit pixels right.
CONVERT_IMAGE="alpine:3.22@sha256:5291449c3df73caf6ed85e649dec1b9e818b39a5d8c871e97afc13e9cd5e8fa8"
CONVERT_PACKAGES="imagemagick=7.1.2.15-r0 bash=5.2.37-r0"
SCREEN=1280x800
# A smoke that has not finished by then has failed; the container is removed either way.
SMOKE_TIMEOUT_SECONDS=1800
CONVERT_TIMEOUT_SECONDS=600
ARCH=amd64
RPM_ARCH=x86_64
CACHE_HOME="${XDG_CACHE_HOME:-${HOME:?}/.cache}"
PACKAGE_CACHE="${FERMIX_PACKAGE_CACHE:-$CACHE_HOME/fermix-desktop-package}"
here=$(cd "$(dirname "$0")" && pwd)
desktop=$(dirname "$here")

# What the EXIT trap removes: it runs after every function has returned.
CONTAINER=""
CONVERTER=""
WORK=""

fail() {
  echo "install_smoke: $*" >&2
  exit 1
}

usage() {
  fail "usage: install_smoke.sh --image <ubuntu:22.04|fedora:44> <package>" \
    "[--out <dir> | --print-engine-version]"
}

cleanup() {
  local container
  for container in "$CONTAINER" "$CONVERTER"; do
    [ -z "$container" ] || docker rm -f "$container" > /dev/null 2>&1 ||
      echo "install_smoke: could not remove the container $container" >&2
  done
  [ -z "$WORK" ] || rm -rf -- "$WORK"
}

pinned_image() {
  case "$1" in
    ubuntu:22.04) echo "$UBUNTU_IMAGE" ;;
    fedora:44) echo "$FEDORA_IMAGE" ;;
    *) fail "the smoke runs on ubuntu:22.04 and fedora:44, not $1" ;;
  esac
}

family_of() {
  case "$1" in
    ubuntu:*) echo deb ;;
    fedora:*) echo rpm ;;
  esac
}

# The engine version a package's file name carries: <engine>[+<n> | +0.dev.<...>].
engine_version_of() {
  local package="$1" family="$2" name pattern arch
  name="$(basename -- "$package")"
  case "$family" in
    deb) pattern='^fermix-desktop_([0-9][0-9A-Za-z.+]*)_([a-z0-9_]+)\.deb$' arch="$ARCH" ;;
    rpm) pattern='^fermix-desktop-([0-9][0-9A-Za-z.+]*)-1\.([a-z0-9_]+)\.rpm$' arch="$RPM_ARCH" ;;
  esac
  [[ "$name" =~ $pattern ]] || fail "$name is not a fermix-desktop package as build_packages.sh names them"
  [ "${BASH_REMATCH[2]}" = "$arch" ] || fail "$name is not a fermix-desktop package for $ARCH"
  echo "${BASH_REMATCH[1]%%+*}"
}

check_package() {
  local image="$1" package="$2" family="$3"
  [ -f "$package" ] || fail "no package at $package"
  case "$family" in
    deb) [[ "$package" == *.deb ]] || fail "$image installs a deb, and $package is not one" ;;
    rpm) [[ "$package" == *.rpm ]] || fail "$image installs an rpm, and $package is not one" ;;
  esac
}

# The smoke container, with the package and the script that checks it copied in.
create_container() {
  local pinned="$1" family="$2" package="$3" engine="$4" digest name
  digest="$(sha256sum -- "$package" | cut -d' ' -f1)"
  name="$(basename -- "$package")"
  docker create --name "fermix-desktop-pkg-package-smoke-$$" --platform "linux/$ARCH" --pull missing \
    "$pinned" bash /smoke/smoke_in_container.sh --family "$family" --package "/smoke/$name" \
    --sha256 "$digest" --engine-version "$engine" > /dev/null
  CONTAINER="fermix-desktop-pkg-package-smoke-$$"
  tar -cf - --transform 'flags=r;s,^,smoke/,' -C "$(dirname -- "$package")" "$name" \
    -C "$desktop/packaging" smoke_in_container.sh | docker cp - "$CONTAINER:/"
}

# screen.xwd in <out>, decoded to screenshot.png and checked against window-bounds.txt in the
# converter; the PNG comes back checked by the sha256 the converter wrote, and joins SHA256SUMS.
convert_screenshot() {
  local out="$1" received="$WORK/converted"
  # shellcheck disable=SC2016 # expanded in the converter
  docker create --name "fermix-desktop-pkg-package-convert-$$" --platform "linux/$ARCH" --pull missing \
    "$CONVERT_IMAGE" sh -euc 'apk add --no-cache --quiet $1 && bash /convert/convert_screenshot.sh /convert/work "$2"' \
    sh "$CONVERT_PACKAGES" "$SCREEN" > /dev/null
  CONVERTER="fermix-desktop-pkg-package-convert-$$"
  tar -cf - --transform 'flags=r;s,^\(screen\|window\),work/\1,' --transform 'flags=r;s,^,convert/,' \
    -C "$desktop/packaging" convert_screenshot.sh check_screenshot.sh smoke_fixtures/garbled-xwdtopnm.png \
    -C "$out" screen.xwd window-bounds.txt | docker cp - "$CONVERTER:/"
  timeout "$CONVERT_TIMEOUT_SECONDS" docker start --attach "$CONVERTER" ||
    fail "the converter could not decode and check the screenshot; screen.xwd is in $out"
  mkdir -p "$received"
  docker cp "$CONVERTER:/convert/work/screenshot.png" "$received/"
  docker cp "$CONVERTER:/convert/work/screenshot.png.sha256" "$received/"
  (cd "$received" && sha256sum --quiet --check screenshot.png.sha256) ||
    fail "the screenshot copied out of the converter does not match the sha256 it wrote"
  mv -f -- "$received/screenshot.png" "$out/screenshot.png"
  (cd "$out" && sha256sum screenshot.png >> SHA256SUMS)
  echo "install_smoke: $out/screenshot.png, decoded and checked in $CONVERT_IMAGE"
}

# What the container wrote, checked against its SHA256SUMS, moved into <out>.
copy_out() {
  local out="$1" received="$WORK/out" name
  docker cp "$CONTAINER:/smoke/out/." "$received"
  (cd "$received" && sha256sum --quiet --check SHA256SUMS) ||
    fail "the files copied out of the container do not match the SHA256SUMS it wrote"
  mkdir -p "$out"
  while read -r _ name; do
    mv -f -- "$received/$name" "$out/$name"
  done < "$received/SHA256SUMS"
  mv -f -- "$received/SHA256SUMS" "$out/SHA256SUMS"
  echo "install_smoke: $(wc -l < "$out/SHA256SUMS") files in $out"
}

# What a failed smoke wrote, unchecked, for reading: it wrote no SHA256SUMS.
keep_failed() {
  rm -rf -- "$1"
  mkdir -p "$1"
  docker cp "$CONTAINER:/smoke/out/." "$1" || echo "install_smoke: $CONTAINER had nothing to copy out" >&2
}

run_smoke() {
  local image="$1" family="$2" package="$3" engine="$4" out="$5"
  create_container "$(pinned_image "$image")" "$family" "$package" "$engine"
  echo "install_smoke: $(basename -- "$package") on $image" >&2
  if ! timeout "$SMOKE_TIMEOUT_SECONDS" docker start --attach "$CONTAINER"; then
    keep_failed "$out-failed"
    fail "the smoke of $(basename -- "$package") failed on $image; what it wrote is in $out-failed"
  fi
  mkdir -p "$PACKAGE_CACHE"
  WORK="$(mktemp -d "$PACKAGE_CACHE/smoke.XXXXXX")"
  copy_out "$out"
  convert_screenshot "$out"
}

main() {
  local image="" package="" out="" print=0 family engine
  while [ $# -gt 0 ]; do
    case "$1" in
      --image) [ $# -ge 2 ] || usage; image="$2"; shift 2 ;;
      --out) [ $# -ge 2 ] || usage; out="$2"; shift 2 ;;
      --print-engine-version) print=1; shift ;;
      -*) usage ;;
      *) [ -z "$package" ] || usage; package="$1"; shift ;;
    esac
  done
  [ -n "$image" ] && [ -n "$package" ] || usage
  pinned_image "$image" > /dev/null
  family="$(family_of "$image")"
  check_package "$image" "$package" "$family"
  engine="$(engine_version_of "$package" "$family")"
  if [ "$print" = 1 ]; then
    echo "$engine"
    return 0
  fi
  run_smoke "$image" "$family" "$package" "$engine" "${out:-$PACKAGE_CACHE/out/smoke/${image//:/-}}"
}

trap cleanup EXIT
main "$@"
