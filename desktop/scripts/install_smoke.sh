#!/usr/bin/env bash
#
# Installs a built fermix-desktop package on a clean distribution image and checks that it works
# there: the package manager installs it with its declared relations, `fermix --version` answers
# with the version the installed package provides fermix at, ldd resolves every object under
# /usr/lib/fermix-desktop with the toolkit from the private prefix, and the window draws, in the
# state it shows when Fermix is not running, on each display the image draws on: under Xvfb, and on
# the newest of each family also on a headless Weston, which is what proves the private
# libwayland-client.
#
#   install_smoke.sh --image <image> <package> [--out <dir>]
#   install_smoke.sh --image <image> <package> --print-plan
#
#   <image>       ubuntu:22.04, ubuntu:24.04, ubuntu:26.04, debian:12, debian:13, fedora:44,
#                 almalinux:9 or almalinux:10
#   --out <dir>   where the screenshots and the logs go, by default
#                 $FERMIX_PACKAGE_CACHE/out/smoke/<image>, with the image's ':' as '-'
#   --print-plan  print the pinned image, the family, the architecture and the displays, and stop
#
# Each image is pinned here by the digest of its multi-arch index, so one pin runs natively on an
# amd64 and an arm64 machine. The smoke runs natively: the package's architecture must be this
# machine's. The package's version is the app's, so its name does not say which engine it carries:
# the container reads that from what the installed package provides. The package goes into the
# container by docker cp and is checked there against its sha256;
# desktop/packaging/smoke_in_container.sh runs the checks; what it writes comes back by docker cp
# and is checked against the SHA256SUMS it wrote. Its captures are then checked by
# desktop/packaging/convert_screenshot.sh in one converter image, pinned here with its packages,
# the same for every distribution, so no image under test decodes its own capture. Both containers
# are removed on every exit. The images reach their mirrors for the package's dependencies, for the
# display tools, and for ImageMagick.
set -euo pipefail
shopt -s inherit_errexit

IMAGES="ubuntu:22.04, ubuntu:24.04, ubuntu:26.04, debian:12, debian:13, fedora:44, almalinux:9 and almalinux:10"
# The converter: Alpine 3.22 with ImageMagick 7.1.2-15, which reads xwd's 32-bit pixels right.
CONVERT_IMAGE="alpine:3.22@sha256:5291449c3df73caf6ed85e649dec1b9e818b39a5d8c871e97afc13e9cd5e8fa8"
CONVERT_PACKAGES="imagemagick=7.1.2.15-r0 bash=5.2.37-r0"
SCREEN=1280x800
# A smoke that has not finished by then has failed; the container is removed either way.
SMOKE_TIMEOUT_SECONDS=1800
CONVERT_TIMEOUT_SECONDS=600
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
  fail "usage: install_smoke.sh --image <image> <package> [--out <dir> | --print-plan]"
}

cleanup() {
  local container
  for container in "$CONTAINER" "$CONVERTER"; do
    [ -z "$container" ] || docker rm -f "$container" > /dev/null 2>&1 ||
      echo "install_smoke: could not remove the container $container" >&2
  done
  [ -z "$WORK" ] || rm -rf -- "$WORK"
}

# Each image's multi-arch index, which names its amd64 and its arm64 image alike.
pinned_image() {
  case "$1" in
    ubuntu:22.04) echo "ubuntu:22.04@sha256:b8b6ee6aa931ecd9d0d952abc34dc0e5f7c6a30c6bb71b079fe399fde0329c02" ;;
    ubuntu:24.04) echo "ubuntu:24.04@sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55" ;;
    ubuntu:26.04) echo "ubuntu:26.04@sha256:f144425ff09be612d6d9ad965196e9cdc23dae1f42110a8a11a3e9a8198759f7" ;;
    debian:12) echo "debian:12@sha256:2c037a04925515fdd6ea85ea14a682d0e79931f5e9f5d07b6dbfc6ba12f9e858" ;;
    debian:13) echo "debian:13@sha256:913f6706df59a68922d1dd08f78c2476560a8d367897200a6005b00e5f67c2d5" ;;
    fedora:44) echo "fedora:44@sha256:43b29f65a41eb9c35e1cd5323e3bdf3b655c2357a9f4f1ff2f9c2798e5045d80" ;;
    almalinux:9) echo "almalinux:9@sha256:9819dc675b67b595c2b59e42be7763fca1a8bb217fa5944e04daa22e9a64db16" ;;
    almalinux:10) echo "almalinux:10@sha256:838c2fafefb1a8a0d7f8cdbc3b0551c2a2b1eb0cd87aad6d62aecadb18311f1b" ;;
    *) fail "the smoke runs on $IMAGES, not $1" ;;
  esac
}

family_of() {
  case "$1" in
    ubuntu:* | debian:*) echo deb ;;
    fedora:* | almalinux:*) echo rpm ;;
  esac
}

# The displays <image> draws the window on. The newest of each family also draws on Weston.
# AlmaLinux 10 draws on Weston alone: RHEL 10 ships no Xvfb, xwd or xwininfo, and its desktop
# sessions are Wayland's.
displays_of() {
  case "$1" in
    ubuntu:26.04 | fedora:44) echo "x11 wayland" ;;
    almalinux:10) echo "wayland" ;;
    *) echo "x11" ;;
  esac
}

# This machine's architecture, as docker and the deb name it.
machine_arch() {
  local machine
  machine="$(uname -m)"
  case "$machine" in
    x86_64) echo amd64 ;;
    aarch64) echo arm64 ;;
    *) fail "the smoke runs on x86_64 and aarch64 machines, not $machine" ;;
  esac
}

check_package() {
  local image="$1" package="$2" family="$3"
  [ -f "$package" ] || fail "no package at $package"
  case "$family" in
    deb) [[ "$package" == *.deb ]] || fail "$image installs a deb, and $package is not one" ;;
    rpm) [[ "$package" == *.rpm ]] || fail "$image installs an rpm, and $package is not one" ;;
  esac
}

# The architecture in a package's file name, as build_packages.sh names it:
# fermix-desktop_<version>_<arch>.deb, or fermix-desktop-<version>-1.<arch>.rpm, whose x86_64 and
# aarch64 are amd64 and arm64.
package_arch() {
  local package="$1" family="$2" name pattern arch
  name="$(basename -- "$package")"
  case "$family" in
    deb) pattern='^fermix-desktop_([0-9][0-9A-Za-z.+]*)_([a-z0-9_]+)\.deb$' ;;
    rpm) pattern='^fermix-desktop-([0-9][0-9A-Za-z.+]*)-1\.([a-z0-9_]+)\.rpm$' ;;
  esac
  [[ "$name" =~ $pattern ]] || fail "$name is not a fermix-desktop package as build_packages.sh names them"
  arch="${BASH_REMATCH[2]}"
  if [ "$family" = rpm ]; then
    case "$arch" in x86_64) arch=amd64 ;; aarch64) arch=arm64 ;; esac
  fi
  echo "$arch"
}

# The smoke container, with the package and the script that checks it copied in.
create_container() {
  local pinned="$1" family="$2" package="$3" arch="$4" displays="$5" digest name
  digest="$(sha256sum -- "$package" | cut -d' ' -f1)"
  name="$(basename -- "$package")"
  docker create --name "fermix-desktop-pkg-package-smoke-$$" --platform "linux/$arch" --pull missing \
    "$pinned" bash /smoke/smoke_in_container.sh --family "$family" --package "/smoke/$name" \
    --sha256 "$digest" --displays "$displays" > /dev/null
  CONTAINER="fermix-desktop-pkg-package-smoke-$$"
  tar -cf - --transform 'flags=r;s,^,smoke/,' -C "$(dirname -- "$package")" "$name" \
    -C "$desktop/packaging" smoke_in_container.sh | docker cp - "$CONTAINER:/"
}

# The captures in <out> checked in the converter, each against its control: the X11 dump decoded
# into screenshot.png, which comes back checked by the sha256 the converter wrote and joins
# SHA256SUMS; the Wayland captures, which the smoke container wrote as PNG, as they are.
convert_screenshots() {
  local out="$1" arch="$2" displays="$3" received="$WORK/converted" display
  local -a shown captures=()
  read -r -a shown <<< "$displays"
  for display in "${shown[@]}"; do
    case "$display" in
      x11) captures+=(screen.xwd window-bounds.txt) ;;
      wayland) captures+=(wayland-screenshot.png wayland-empty.png) ;;
    esac
  done
  # shellcheck disable=SC2016 # expanded in the converter
  docker create --name "fermix-desktop-pkg-package-convert-$$" --platform "linux/$arch" --pull missing \
    "$CONVERT_IMAGE" sh -euc 'apk add --no-cache --quiet $1 && shift && bash /convert/convert_screenshot.sh /convert/work "$@"' \
    sh "$CONVERT_PACKAGES" "$SCREEN" "${shown[@]}" > /dev/null
  CONVERTER="fermix-desktop-pkg-package-convert-$$"
  tar -cf - --transform 'flags=r;s,^\(screen\|window\|wayland\),work/\1,' --transform 'flags=r;s,^,convert/,' \
    -C "$desktop/packaging" convert_screenshot.sh check_screenshot.sh smoke_fixtures/garbled-xwdtopnm.png \
    -C "$out" "${captures[@]}" | docker cp - "$CONVERTER:/"
  timeout "$CONVERT_TIMEOUT_SECONDS" docker start --attach "$CONVERTER" ||
    fail "the converter refused the captures in $out"
  [[ " $displays " == *" x11 "* ]] || return 0
  mkdir -p "$received"
  docker cp "$CONVERTER:/convert/work/screenshot.png" "$received/"
  docker cp "$CONVERTER:/convert/work/screenshot.png.sha256" "$received/"
  (cd "$received" && sha256sum --quiet --check screenshot.png.sha256) ||
    fail "the screenshot copied out of the converter does not match the sha256 it wrote"
  mv -f -- "$received/screenshot.png" "$out/screenshot.png"
  (cd "$out" && sha256sum screenshot.png >> SHA256SUMS)
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
  local image="$1" family="$2" package="$3" arch="$4" out="$5" displays
  displays="$(displays_of "$image")"
  create_container "$(pinned_image "$image")" "$family" "$package" "$arch" "$displays"
  echo "install_smoke: $(basename -- "$package") on $image, $arch, drawn on $displays" >&2
  if ! timeout "$SMOKE_TIMEOUT_SECONDS" docker start --attach "$CONTAINER"; then
    keep_failed "$out-failed"
    fail "the smoke of $(basename -- "$package") failed on $image; what it wrote is in $out-failed"
  fi
  mkdir -p "$PACKAGE_CACHE"
  WORK="$(mktemp -d "$PACKAGE_CACHE/smoke.XXXXXX")"
  copy_out "$out"
  convert_screenshots "$out" "$arch" "$displays"
  echo "install_smoke: the captures in $out are checked in $CONVERT_IMAGE"
}

main() {
  local image="" package="" out="" print=0 family arch machine
  while [ $# -gt 0 ]; do
    case "$1" in
      --image) [ $# -ge 2 ] || usage; image="$2"; shift 2 ;;
      --out) [ $# -ge 2 ] || usage; out="$2"; shift 2 ;;
      --print-plan) print=1; shift ;;
      -*) usage ;;
      *) [ -z "$package" ] || usage; package="$1"; shift ;;
    esac
  done
  [ -n "$image" ] && [ -n "$package" ] || usage
  pinned_image "$image" > /dev/null
  family="$(family_of "$image")"
  check_package "$image" "$package" "$family"
  arch="$(package_arch "$package" "$family")"
  machine="$(machine_arch)"
  [ "$arch" = "$machine" ] ||
    fail "$(basename -- "$package") is for $arch, and this machine is $machine: the smoke runs natively"
  if [ "$print" = 1 ]; then
    printf 'image %s\nfamily %s\narch %s\ndisplays %s\n' \
      "$(pinned_image "$image")" "$family" "$arch" "$(displays_of "$image")"
    return 0
  fi
  run_smoke "$image" "$family" "$package" "$arch" "${out:-$PACKAGE_CACHE/out/smoke/${image//:/-}}"
}

trap cleanup EXIT
main "$@"
