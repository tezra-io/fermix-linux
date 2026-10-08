#!/usr/bin/env bash
#
# Builds fermix-desktop_<version>_<arch>.deb and fermix-desktop-<version>-1.<rpm arch>.rpm: the
# window, the private GTK runtime under /usr/lib/fermix-desktop, and the engine exactly as the fermix
# package installs it.
#
#   build_packages.sh --version <X.Y.Z[+N]> [--pin <PIN.json>] [--engine-download <dir>] [options]
#   build_packages.sh --dev <engine .deb> <engine .rpm> [options]
#
#   --arch <amd64|arm64>   the architecture, amd64 by default
#   --runtime <dir>        the runtime's outputs, by default $FERMIX_RUNTIME_OUT or
#                          ~/.cache/fermix-desktop-runtime/out
#   --out <dir>            where the packages go, by default $FERMIX_PACKAGE_CACHE/out
#   --print-version        print the version the packages would have, and build nothing
#
# A release build carries the engine desktop/engine/PIN.json pins, downloaded by fetch_engine.sh
# unless --engine-download names a directory that already holds it, and verified by
# verify_engine.py either way. Its version is the engine's, or <engine version>+<n> for a
# desktop-only rebuild. A development build carries a local, unsigned engine package, is refused
# under CI, and is versioned <engine version>+0.dev.<UTC time>.<commit>[.dirty]. dpkg and rpm both
# order that after the engine's own version and before its first rebuild, <engine version>+1; with
# +dev and no 0 they would disagree, since dpkg puts a letter after a digit and rpm a digit after
# a letter.
#
# The build runs in the runtime's build image ($FERMIX_PACKAGE_IMAGE), so the window compiles
# against exactly the toolkit the package ships. Everything goes in and out by docker cp, checked
# by sha256 on both sides, and the cargo registry and target directory stay in the named volumes
# fermix-desktop-pkg-package-cargo and fermix-desktop-pkg-package-target between builds. The work
# directory and nFPM's tarball are kept in $FERMIX_PACKAGE_CACHE, by default
# ~/.cache/fermix-desktop-package.
# desktop/packaging/build_in_container.sh is what runs there: it builds the window, stages the
# tree, runs every gate, and runs nFPM.
set -euo pipefail
shopt -s inherit_errexit

NFPM_VERSION=2.47.0
NFPM_SHA256_X86_64=0660ca602b2d2d2ae4781a06c692b3eeb9d437ffea05b831d76e41f4a3188783
NFPM_SHA256_ARM64=1c0f5f2999b9a974bfb04fdb0cc3306096de530ac5dbb25d739cc5f5219c919c
# A download is tried 4 times, each cut off after its timeout; then the build fails.
DOWNLOAD_RETRIES=3
DOWNLOAD_TIMEOUT_SECONDS=300
# A build that has not finished by then has failed; the container is removed either way.
BUILD_TIMEOUT_SECONDS=5400
IMAGE="${FERMIX_PACKAGE_IMAGE:-fermix-desktop-pkg-runtime-build:latest}"
CARGO_VOLUME=fermix-desktop-pkg-package-cargo
TARGET_VOLUME=fermix-desktop-pkg-package-target
CACHE_HOME="${XDG_CACHE_HOME:-${HOME:?}/.cache}"
PACKAGE_CACHE="${FERMIX_PACKAGE_CACHE:-$CACHE_HOME/fermix-desktop-package}"
RUNTIME_FILES=(runtime-manifest.json runtime-licenses.json runtime-licenses.tar.gz)
here=$(cd "$(dirname "$0")" && pwd)
desktop=$(dirname "$here")
repo=$(dirname "$desktop")

# What the EXIT trap removes: it runs after every function has returned.
CONTAINER=""
WORK=""

fail() {
  echo "build_packages: $*" >&2
  exit 1
}

usage() {
  fail "usage: build_packages.sh (--version <X.Y.Z[+N]> [--pin <file>] [--engine-download <dir>] |" \
    "--dev <engine .deb> <engine .rpm>) [--arch <amd64|arm64>] [--runtime <dir>] [--out <dir>]" \
    "[--print-version]"
}

cleanup() {
  [ -z "$CONTAINER" ] || docker rm -f "$CONTAINER" > /dev/null 2>&1 ||
    echo "build_packages: could not remove the container $CONTAINER" >&2
  [ -z "$WORK" ] || rm -rf -- "$WORK"
}

check_arch() {
  case "$1" in
    amd64 | arm64) ;;
    *) fail "fermix-desktop is built for amd64 and arm64, not $1" ;;
  esac
}

refuse_marks() {
  local version="$1" mark
  for mark in - :; do
    case "$version" in
      *"$mark"*) fail "the version $version has a '$mark': no Debian revision and no rpm epoch" ;;
    esac
  done
}

# A release is the pinned engine's version, or a desktop-only rebuild of it.
release_version() {
  local version="$1" pin="$2" engine
  refuse_marks "$version"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(\+[0-9]+)?$ ]] ||
    fail "a release version is X.Y.Z or X.Y.Z+N, and $version is neither"
  "$here/engine_pin.py" --pin "$pin" --check || fail "the engine pin $pin is not one a build can trust"
  engine="$("$here/engine_pin.py" --pin "$pin" --get engine_version)"
  [ "${version%%+*}" = "$engine" ] ||
    fail "the version $version is not the pinned engine's: the pin's engine is $engine"
  echo "$version"
}

# <engine version>+0.dev.<UTC time>.<commit>[.dirty], from the staged engine and this checkout.
dev_version() {
  local stage="$1" epoch="$2" engine stamp commit status version
  engine="$(jq -er '.engine_version' "$stage/engine.json")" || fail "the dev stage names no engine version"
  stamp="$(date -u -d "@$epoch" +%Y%m%d%H%M%S)"
  commit="$(git -C "$repo" rev-parse --short=12 HEAD)" || fail "$repo is not a git checkout"
  status="$(git -C "$repo" status --porcelain)" || fail "git cannot read the state of $repo"
  version="$engine+0.dev.$stamp.$commit"
  [ -z "$status" ] || version="$version.dirty"
  refuse_marks "$version"
  echo "$version"
}

stage_release_engine() {
  local pin="$1" download="$2" stage="$3" arch="$4"
  if [ -z "$download" ]; then
    download="$WORK/download"
    "$here/fetch_engine.sh" --pin "$pin" "$download" "$arch" >&2
  fi
  "$here/verify_engine.py" --pin "$pin" "$download" "$stage" "$arch" >&2
}

# Every runtime output the build reads, as the runtime's SHA256SUMS records it, built from this
# checkout's lock file for this architecture.
check_runtime() {
  local dir="$1" arch="$2" name listed
  for name in "runtime-$arch.tar" "runtime-dev-$arch.tar" "${RUNTIME_FILES[@]}" SHA256SUMS; do
    [ -f "$dir/$name" ] || fail "no $name in $dir: build the runtime first"
  done
  listed="$(awk '{ print $2 }' "$dir/SHA256SUMS")"
  for name in "runtime-$arch.tar" "runtime-dev-$arch.tar" "${RUNTIME_FILES[@]}"; do
    grep -qxF -- "$name" <<< "$listed" || fail "the runtime's SHA256SUMS does not list $name"
  done
  (cd "$dir" && sha256sum --quiet --check SHA256SUMS) > /dev/null 2>&1 ||
    fail "the files in $dir do not match the runtime's SHA256SUMS"
  [ "$(jq -r '.architecture' "$dir/runtime-manifest.json")" = "$arch" ] ||
    fail "$dir is the $(jq -r '.architecture' "$dir/runtime-manifest.json") runtime, not $arch"
  cmp -s <(jq -S '.lock' "$dir/runtime-manifest.json") <(jq -S . "$desktop/packaging/runtime/RUNTIME.lock.json") ||
    fail "the runtime in $dir was built from another RUNTIME.lock.json than this checkout's"
}

# nFPM's release tarball for the build container's architecture, checked against its pinned digest.
fetch_nfpm() {
  local arch="$1" asset digest dir attempt
  case "$arch" in
    amd64) asset="nfpm_${NFPM_VERSION}_Linux_x86_64.tar.gz" digest="$NFPM_SHA256_X86_64" ;;
    arm64) asset="nfpm_${NFPM_VERSION}_Linux_arm64.tar.gz" digest="$NFPM_SHA256_ARM64" ;;
  esac
  dir="$PACKAGE_CACHE/tools"
  mkdir -p "$dir"
  for attempt in $(seq 1 $((DOWNLOAD_RETRIES + 1))); do
    if [ -f "$dir/$asset" ] && echo "$digest  $dir/$asset" | sha256sum --quiet --check - 2> /dev/null; then
      echo "$dir/$asset"
      return 0
    fi
    rm -f -- "$dir/$asset"
    echo "build_packages: downloading $asset, attempt $attempt of $((DOWNLOAD_RETRIES + 1))" >&2
    curl -fsSL --max-time "$DOWNLOAD_TIMEOUT_SECONDS" -o "$dir/$asset" \
      "https://github.com/goreleaser/nfpm/releases/download/v$NFPM_VERSION/$asset" ||
      echo "build_packages: the download of $asset failed" >&2
  done
  fail "nFPM $NFPM_VERSION did not arrive with the pinned sha256 in $((DOWNLOAD_RETRIES + 1)) attempts"
}

# "<sha256>  <path>" for every file under <dir>, relative to it.
manifest_of() {
  (cd "$1" && find . -type f -print0 | sort -z | xargs -0 -r sha256sum)
}

# This checkout's files the build reads: desktop/, committed or not but not ignored, and the
# repository's LICENSE. Written NUL-separated to <list>, and their digests, as src/<path>, to
# <manifest>.
source_files() {
  local list="$1" manifest="$2"
  (cd "$repo" && git ls-files -z --cached --others --exclude-standard -- desktop LICENSE |
    while IFS= read -r -d '' path; do
      [[ "$path" != *$'\n'* ]] || fail "$path has a newline in its name"
      [ ! -e "$path" ] || printf '%s\0' "$path"
    done) > "$list"
  (cd "$repo" && xargs -0 -r sha256sum < "$list") | sed 's|  |  src/|' > "$manifest"
  [ -s "$manifest" ] || fail "git lists no files under $repo/desktop"
}

# The build container, with everything it reads copied in and checked there.
create_container() {
  local version="$1" arch="$2" mtime="$3" runtime="$4" stage="$5" tools="$6" list="$7"
  docker image inspect "$IMAGE" > /dev/null 2>&1 ||
    fail "there is no image $IMAGE: build it with desktop/packaging/runtime/build_runtime.sh --container"
  docker create --name "fermix-desktop-pkg-package-$$" --platform "linux/$arch" \
    --volume "$CARGO_VOLUME:/var/cache/fermix-package/cargo" \
    --volume "$TARGET_VOLUME:/var/cache/fermix-package/target" \
    --workdir /workspace "$IMAGE" \
    bash /workspace/src/desktop/packaging/build_in_container.sh \
    --version "$version" --arch "$arch" --mtime "$mtime" > /dev/null
  CONTAINER="fermix-desktop-pkg-package-$$"
  (cd "$repo" && tar --null -T "$list" --transform 'flags=r;s,^,src/,' -cf -) |
    docker cp - "$CONTAINER:/workspace"
  # The tools' parent first: docker cp creates the last directory of a path, not its parents.
  docker cp "$(dirname "$tools")/." "$CONTAINER:/workspace/inputs"
  docker cp "$runtime/." "$CONTAINER:/workspace/inputs/runtime"
  docker cp "$stage/." "$CONTAINER:/workspace/inputs/engine"
}

# The container's outputs, checked against the SHA256SUMS it wrote, moved into <out>.
copy_out() {
  local out="$1" received="$WORK/out" name
  docker cp "$CONTAINER:/workspace/out/." "$received"
  (cd "$received" && sha256sum --quiet --check SHA256SUMS) ||
    fail "the packages copied out of the container do not match the SHA256SUMS it wrote"
  mkdir -p "$out"
  while read -r _ name; do
    mv -f -- "$received/$name" "$out/$name"
  done < "$received/SHA256SUMS"
  mv -f -- "$received/SHA256SUMS" "$out/SHA256SUMS"
  sed 's/^/build_packages: /' "$out/SHA256SUMS"
}

build() {
  local version="$1" arch="$2" mtime="$3" runtime="$4" stage="$5" out="$6" nfpm tools="$WORK/inputs/tools"
  nfpm="$(fetch_nfpm "$arch")"
  mkdir -p "$tools"
  cp -- "$nfpm" "$tools/"
  (cd "$tools" && sha256sum -- "$(basename "$nfpm")" > nfpm.sha256)
  manifest_of "$stage" > "$tools/engine.sha256"
  source_files "$WORK/source.list" "$tools/source.sha256"
  create_container "$version" "$arch" "$mtime" "$runtime" "$stage" "$tools" "$WORK/source.list"
  timeout "$BUILD_TIMEOUT_SECONDS" docker start --attach "$CONTAINER" ||
    fail "the build of fermix-desktop $version failed in $CONTAINER"
  copy_out "$out"
}

main() {
  local mode="" version="" deb="" rpm="" pin="$desktop/engine/PIN.json" download="" arch=amd64
  local runtime="${FERMIX_RUNTIME_OUT:-$CACHE_HOME/fermix-desktop-runtime/out}"
  local out="$PACKAGE_CACHE/out" print=0 stage epoch mtime
  while [ $# -gt 0 ]; do
    case "$1" in
      --version) [ $# -ge 2 ] && [ -z "$mode" ] || usage; mode=release version="$2"; shift 2 ;;
      --dev) [ $# -ge 3 ] && [ -z "$mode" ] || usage; mode=dev deb="$2" rpm="$3"; shift 3 ;;
      --pin | --engine-download | --arch | --runtime | --out)
        [ $# -ge 2 ] || usage
        case "$1" in
          --pin) pin="$2" ;; --engine-download) download="$2" ;; --arch) arch="$2" ;;
          --runtime) runtime="$2" ;; --out) out="$2" ;;
        esac
        shift 2 ;;
      --print-version) print=1; shift ;;
      *) usage ;;
    esac
  done
  [ -n "$mode" ] || usage
  check_arch "$arch"
  if [ "$mode" = dev ] && [ -n "${CI+set}" ]; then
    fail "a development build carries an unsigned engine and is refused under CI"
  fi
  if [ "$mode" = release ] && [ "$print" = 1 ]; then
    release_version "$version" "$pin"
    return 0
  fi
  [ "$print" = 1 ] || check_runtime "$runtime" "$arch"
  mkdir -p "$PACKAGE_CACHE"
  WORK="$(mktemp -d "$PACKAGE_CACHE/work.XXXXXX")"
  stage="$WORK/engine"
  if [ "$mode" = release ]; then
    version="$(release_version "$version" "$pin")"
    stage_release_engine "$pin" "$download" "$stage" "$arch"
    epoch="${SOURCE_DATE_EPOCH:-$(git -C "$repo" log -1 --format=%ct)}"
  else
    "$here/verify_engine.py" --dev "$stage" "$deb" "$rpm" >&2
    epoch="${SOURCE_DATE_EPOCH:-$(date -u +%s)}"
    version="$(dev_version "$stage" "$epoch")"
  fi
  if [ "$print" = 1 ]; then
    echo "$version"
    return 0
  fi
  mtime="$(date -u -d "@$epoch" +%Y-%m-%dT%H:%M:%SZ)"
  echo "build_packages: fermix-desktop $version for $arch" >&2
  build "$version" "$arch" "$mtime" "$runtime" "$stage" "$out"
}

trap cleanup EXIT
main "$@"
