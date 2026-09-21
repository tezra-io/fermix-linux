#!/usr/bin/env bash
#
# Build the private GTK4 runtime the fermix-desktop package carries.
#
# Every component in RUNTIME.lock.json is fetched by URL, refused unless its
# sha256 is the locked one, and configured with --prefix=/usr/lib/fermix-desktop,
# which is the path it will occupy on a user's machine. Nothing is built under a
# staging prefix and relocated: a GLib configured with that prefix compiles it in
# as its GIO module directory, and a gdk-pixbuf configured with it compiles in
# the private loaders.cache path, so both are right with no environment variable
# at all.
#
# Usage:
#   packaging/runtime/build_runtime.sh --container   build in Docker, export the trees
#   packaging/runtime/build_runtime.sh --verify      rebuild and compare against the manifest
#   packaging/runtime/build_runtime.sh --print-key   print the cache key and exit
#   packaging/runtime/build_runtime.sh --fresh       with --container: discard the work volumes
#   packaging/runtime/build_runtime.sh --build       compile here; this is what runs inside
#
# The compile takes tens of minutes. --container mounts a source cache so a
# second run does not re-download, and it is a directory rather than a named
# volume because a directory is one a developer can look inside.
#
# The daemon is whichever one the environment names: this script never passes -H
# and never names a socket, so DOCKER_HOST and `docker context` both work, and
# the same command runs against a desktop VM or a system daemon.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNTIME_DIR="$ROOT_DIR/packaging/runtime"
LOCK_FILE="$RUNTIME_DIR/RUNTIME.lock.json"
PATCH_DIR="$RUNTIME_DIR/patches"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.runtime"
# Both default outside the tracked tree: packaging/out is already ignored, and
# the source cache belongs in the developer's cache directory rather than in a
# repository three other slices are editing at the same time.
OUT_DIR="${FERMIX_RUNTIME_OUT:-$ROOT_DIR/packaging/out/runtime}"
SOURCE_CACHE="${FERMIX_RUNTIME_SOURCES:-${XDG_CACHE_HOME:-$HOME/.cache}/fermix-desktop-runtime/sources}"
IMAGE="${FERMIX_RUNTIME_IMAGE:-fermix-desktop-runtime-build}"
PREFIX="/usr/lib/fermix-desktop"
BUILD_ROOT="/tmp/fermix-runtime-build"
JOBS="${FERMIX_RUNTIME_JOBS:-4}"

# The prefix and the build tree live in named volumes so that a failure in the
# twenty-eighth component does not rebuild the first twenty-seven. build_all
# writes a stamp per installed component into the build volume and skips any
# component whose stamp is already there; the volumes are discarded whole
# whenever the cache key changes, so a stamp can only ever describe the lock
# file that is being built.
PREFIX_VOLUME="${FERMIX_RUNTIME_PREFIX_VOLUME:-fermix-runtime-prefix}"
BUILD_VOLUME="${FERMIX_RUNTIME_BUILD_VOLUME:-fermix-runtime-build}"
STAMP_DIR="$BUILD_ROOT/stamps"

VERIFY_SCRATCH=""
MODE=""
FRESH=0

# fail() and log() are defined below and are what this library calls, so it is
# sourced after them rather than here.

fail() {
  echo "build_runtime: $*" >&2
  exit 1
}

# Progress goes to standard error. No function in this script returns a value
# through standard output, so nothing a build tool prints can become one.
log() {
  echo "build_runtime: $*" >&2
}

# shellcheck source=packaging/runtime/fetch_source.sh
. "$RUNTIME_DIR/fetch_source.sh"

usage() {
  echo "usage: build_runtime.sh --container [--fresh] | --verify | --print-key | --build" >&2
}

parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --container) MODE="container"; shift ;;
      --verify) MODE="verify"; shift ;;
      --print-key) MODE="print-key"; shift ;;
      --build) MODE="build"; shift ;;
      --fresh) FRESH=1; shift ;;
      -h|--help) usage; exit 0 ;;
      *) usage; fail "unknown argument: $1" ;;
    esac
  done
  [ -n "$MODE" ] || { usage; fail "no mode given"; }
}

target_arch() {
  case "$(uname -m)" in
    x86_64) echo "amd64" ;;
    aarch64|arm64) echo "arm64" ;;
    *) fail "unsupported architecture: $(uname -m)" ;;
  esac
}

# ---------------------------------------------------------------- host side

require_host_tools() {
  command -v docker >/dev/null 2>&1 || fail "docker is not installed"
  command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is not installed"
  command -v python3 >/dev/null 2>&1 || fail "python3 is not installed"
  [ -f "$LOCK_FILE" ] || fail "no lock file at $LOCK_FILE"
  [ -f "$DOCKERFILE" ] || fail "no runtime container at $DOCKERFILE"
  [ -d "$PATCH_DIR" ] || fail "no patch directory at $PATCH_DIR"
}

# The cache key of amendment section 4.2: the lock file, the patches and the
# Dockerfile. It names the published image, so the runtime is rebuilt only when
# one of those three moves.
# Contents only, never paths. `sha256sum FILE` prints the file's name beside its
# digest, so a key built that way depends on where the tree happens to sit: this
# repository, a CI checkout under /home/runner, and an unpacked source archive
# would each produce a different key for identical inputs. That was true here
# until unpacking the source archive and running its own build script printed a
# key that disagreed with the repository's. Patches are named by basename,
# because the lock file applies them by name and a rename is a real change.
cache_key() {
  local patch
  {
    sha256sum < "$LOCK_FILE"
    sha256sum < "$DOCKERFILE"
    while IFS= read -r patch; do
      printf '%s ' "$(basename -- "$patch")"
      sha256sum < "$patch"
    done < <(find "$PATCH_DIR" -type f -name '*.patch' | LC_ALL=C sort)
  } | sha256sum | cut -c1-16
}

build_image() {
  log "building $IMAGE"
  docker build -f "$DOCKERFILE" -t "$IMAGE" "$ROOT_DIR/packaging/docker"
}

# The repository is bind-mounted read-only, so a compile cannot edit the tree it
# was told to build. The source cache and the output directory are the only two
# writable bind mounts, and both are written a whole file at a time.
#
# The prefix and the build tree are docker volumes, not bind mounts, for a reason
# that is easy to lose and expensive to rediscover. Where the Docker daemon runs
# in a virtual machine, a bind-mounted host directory carries the host's clock
# while the container carries the machine's, and the two differ by about a
# second. Every file the compile writes therefore has a timestamp in the
# container's own future, and ninja stops on exactly that: "Clock skew detected".
# A volume is written by the daemon itself, so it has one clock. It is also not
# the container's writable layer, which would be discarded with --rm and take
# the half-finished build with it.
#
# Nothing about what is compiled in changes: $PREFIX is still $PREFIX to every
# configure script that sees it, and the tree leaves as the exported tarballs.
run_in_container() {
  local out_dir="$1"
  mkdir -p "$SOURCE_CACHE" "$out_dir"
  docker run --rm --init \
    -v "$ROOT_DIR:/workspace:ro" \
    -v "$SOURCE_CACHE:/sources" \
    -v "$out_dir:/out" \
    -v "$BUILD_VOLUME:$BUILD_ROOT" \
    -v "$PREFIX_VOLUME:$PREFIX" \
    -e FERMIX_RUNTIME_SOURCES=/sources \
    -e FERMIX_RUNTIME_OUT=/out \
    -e FERMIX_RUNTIME_JOBS="$JOBS" \
    -e FERMIX_RUNTIME_OWNER="$(id -u):$(id -g)" \
    -w /workspace \
    "$IMAGE" bash /workspace/packaging/runtime/build_runtime.sh --build
}

# A resumed build is only safe while the thing being built has not changed, so
# the key the volumes were filled under is kept beside them. A different key, or
# --fresh, and both volumes go: a half-built prefix from another lock file is
# worse than an hour of compiling.
discard_work_volumes() {
  docker volume rm -f "$BUILD_VOLUME" "$PREFIX_VOLUME" >/dev/null
}

prepare_work_volumes() {
  local key="$1" previous=""
  if [ "$FRESH" = "1" ]; then
    log "--fresh: discarding the work volumes"
    discard_work_volumes
    return 0
  fi
  previous="$(docker volume inspect "$BUILD_VOLUME" \
    --format '{{index .Labels "fermix.runtime.key"}}' 2>/dev/null || true)"
  if [ -n "$previous" ] && [ "$previous" = "$key" ]; then
    log "resuming in the work volumes built for key $key"
    return 0
  fi
  [ -z "$previous" ] || log "the work volumes were built for key $previous, not $key"
  discard_work_volumes
}

label_work_volumes() {
  local key="$1"
  # Created here rather than left to `docker run`, because a volume docker
  # creates implicitly carries no label, and the label is what makes a resume
  # safe. Creating one that exists is a no-op that keeps its contents.
  docker volume create --label "fermix.runtime.key=$key" "$BUILD_VOLUME" >/dev/null
  docker volume create --label "fermix.runtime.key=$key" "$PREFIX_VOLUME" >/dev/null
}

run_container_mode() {
  local key arch
  require_host_tools
  key="$(cache_key)"
  arch="$(target_arch)"
  build_image
  prepare_work_volumes "$key"
  label_work_volumes "$key"
  run_in_container "$OUT_DIR"
  echo "$key" > "$OUT_DIR/cache-key"
  log "runtime built for $arch, cache key $key"
  log "  $OUT_DIR/runtime-$arch.tar"
  log "  $OUT_DIR/runtime-dev-$arch.tar"
  log "  $OUT_DIR/runtime-manifest.json"
}

# The one mode that prints to standard output, because its whole output is the
# key. It needs no daemon: a release workflow deciding whether to pull
# ghcr.io/tezra-io/fermix-desktop-runtime:<key>-<arch> or build it should not
# have to have Docker running to ask.
run_print_key_mode() {
  command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is not installed"
  [ -f "$LOCK_FILE" ] || fail "no lock file at $LOCK_FILE"
  [ -f "$DOCKERFILE" ] || fail "no runtime container at $DOCKERFILE"
  [ -d "$PATCH_DIR" ] || fail "no patch directory at $PATCH_DIR"
  cache_key
}

run_verify_mode() {
  local reference
  require_host_tools
  reference="$OUT_DIR/runtime-manifest.json"
  [ -f "$reference" ] || fail "no manifest to verify against at $reference"

  # Not `local`. An EXIT trap runs after the function's scope is gone, so a trap
  # naming a local variable dies on `set -u` at the moment it was supposed to
  # clean up — which is exactly what happened here, leaving the scratch
  # directory and both verify volumes behind after a successful run.
  VERIFY_SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/fermix-runtime-verify.XXXXXX")"
  # A verification that resumed from the first build's volumes would be checking
  # a tar of the tree against itself. It gets its own empty pair, and takes them
  # with it on every path out.
  PREFIX_VOLUME="fermix-runtime-verify-prefix"
  BUILD_VOLUME="fermix-runtime-verify-build"
  trap 'rm -rf "$VERIFY_SCRATCH"; discard_work_volumes' EXIT
  discard_work_volumes

  build_image
  run_in_container "$VERIFY_SCRATCH"
  python3 "$RUNTIME_DIR/compare_manifest.py" "$reference" "$VERIFY_SCRATCH/runtime-manifest.json" \
    || fail "the rebuild does not reproduce $reference"
  log "the rebuild reproduces $reference"
}

# ----------------------------------------------------------- container side

require_build_tools() {
  local tool
  for tool in jq meson ninja patchelf cmake gcc curl tar python3 cargo readelf strip objcopy; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed in this container"
  done
  [ -f "$LOCK_FILE" ] || fail "no lock file at $LOCK_FILE"
}

lock_get() {
  jq -r "$1" "$LOCK_FILE"
}

component_names() {
  lock_get '.components[].name'
}

component_field() {
  local name="$1" field="$2"
  jq -r --arg n "$name" '.components[] | select(.name == $n) | .'"$field" "$LOCK_FILE"
}

component_options() {
  jq -r --arg n "$1" '.components[] | select(.name == $n) | .options[]' "$LOCK_FILE"
}

# The fetch and the digest check live in fetch_source.sh, which
# package_sources.sh sources too. One rule for what a locked tarball is, because
# a source archive assembled by a slightly different rule than the build used is
# a source archive that does not correspond to the binaries.
fetch_component() {
  local name="$1" archive url want
  archive="$(component_field "$name" archive)"
  url="$(component_field "$name" url)"
  want="$(component_field "$name" sha256)"
  ensure_source "$name" "$url" "$SOURCE_CACHE/$archive" "$want"
}

fetch_all() {
  local name
  mkdir -p "$SOURCE_CACHE"
  for name in $(component_names); do
    fetch_component "$name"
  done
  log "every locked tarball is present and matches its digest"
}

apply_patches() {
  local name="$1" src="$2" patch
  for patch in $(component_field "$name" 'patches[]'); do
    [ -f "$PATCH_DIR/$patch" ] || fail "$name: no patch at $PATCH_DIR/$patch"
    log "$name: applying $patch"
    patch -p1 -d "$src" < "$PATCH_DIR/$patch" >&2
  done
}

# Returns nothing: the caller already knows where the source will land, because
# the lock file says so. An earlier version returned the path on standard
# output, and `patch` printing "patching file meson.build" to standard output
# then became part of it.
unpack_component() {
  local name="$1" src="$2" archive
  archive="$(component_field "$name" archive)"

  rm -rf "$src"
  tar -xf "$SOURCE_CACHE/$archive" -C "$BUILD_ROOT"
  [ -d "$src" ] || fail "$name: $archive does not unpack to $src"
  apply_patches "$name" "$src"
}

component_source_dir() {
  echo "$BUILD_ROOT/$(component_field "$1" source_dir)"
}

# libdir is forced to lib on every build system. AlmaLinux is an RPM
# distribution, so meson and cmake would both choose lib64, and the installed
# layout of amendment section 3.2 says lib. One spelling, decided here.
build_meson() {
  local name="$1" src="$2"
  local -a options
  mapfile -t options < <(component_options "$name")
  meson setup "$src/_build" "$src" \
    --prefix="$PREFIX" --libdir=lib --buildtype=release \
    --strip -Db_ndebug=true -Ddefault_library=shared \
    "${options[@]}"
  meson compile -C "$src/_build" -j "$JOBS"
  meson install -C "$src/_build" --no-rebuild
}

# Autotools tarballs ship generated files whose recorded timestamps can arrive
# out of order, and make then tries to regenerate them with the exact automake
# the maintainer had, which this image does not have and must not need. Every
# file is stamped from the lock file's epoch and the generated ones are bumped in
# dependency order after it, which settles that and removes a source of
# build-to-build variation at the same time.
normalise_autotools_timestamps() {
  local src="$1" epoch="$SOURCE_DATE_EPOCH"
  find "$src" -print0 | xargs -0 -r touch -h -d "@$epoch"
  find "$src" \( -name 'aclocal.m4' \) -print0 |
    xargs -0 -r touch -d "@$((epoch + 1))"
  find "$src" \( -name 'configure' -o -name '*.h.in' -o -name 'config.hin' \) -print0 |
    xargs -0 -r touch -d "@$((epoch + 2))"
  find "$src" \( -name 'Makefile.in' \) -print0 |
    xargs -0 -r touch -d "@$((epoch + 3))"
}

build_autotools() {
  local name="$1" src="$2"
  local -a options
  mapfile -t options < <(component_options "$name")
  normalise_autotools_timestamps "$src"
  ( cd "$src" && ./configure --prefix="$PREFIX" --libdir="$PREFIX/lib" "${options[@]}" )
  make -C "$src" -j "$JOBS"
  make -C "$src" install
}

build_cmake() {
  local name="$1" src="$2"
  local -a options
  mapfile -t options < <(component_options "$name")
  cmake -S "$src" -B "$src/_build" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$PREFIX" \
    -DCMAKE_INSTALL_LIBDIR=lib \
    "${options[@]}"
  cmake --build "$src/_build" -j "$JOBS"
  cmake --install "$src/_build"
}

build_component() {
  local name="$1" system src stamp
  system="$(component_field "$name" build_system)"
  stamp="$STAMP_DIR/$name"
  if [ -f "$stamp" ]; then
    log "--- $name $(component_field "$name" version) is already installed"
    return 0
  fi
  log "=== $name $(component_field "$name" version) ($system)"
  src="$(component_source_dir "$name")"
  unpack_component "$name" "$src"
  case "$system" in
    meson) build_meson "$name" "$src" ;;
    autotools) build_autotools "$name" "$src" ;;
    cmake) build_cmake "$name" "$src" ;;
    *) fail "$name: unknown build system $system" ;;
  esac
  # Immediately, not at the end: the next component's configure may run a tool
  # this one just installed, and with no LD_LIBRARY_PATH that tool finds its
  # libraries only through the RUNPATH written here.
  fix_runpaths quiet
  rm -rf "$src"
  # Written last, so a component that died half way through `install` has no
  # stamp and is built again from the top on the next run.
  mkdir -p "$STAMP_DIR"
  date -u +%s > "$stamp"
}

# The build environment.
#
# There is deliberately no LD_LIBRARY_PATH. The obvious way to let each
# component's build-time tools find the libraries just installed is to point it
# at the private lib directory, and it is wrong, because that variable is not
# scoped to the tools it was meant for. The host's itstool and xsltproc both
# link libxml2.so.2, so they would load the private libxml2 2.13 instead of the
# system 2.9 they were built against and die on a symbol that no longer exists.
# The private tools find their own libraries through their own RUNPATH instead,
# which build_component writes as soon as each component is installed: the same
# mechanism that will resolve them on a user's machine.
export_build_environment() {
  SOURCE_DATE_EPOCH="$(lock_get '.source_date_epoch')"
  export SOURCE_DATE_EPOCH
  export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig"
  export PATH="$PREFIX/bin:$PATH"
  export XDG_DATA_DIRS="$PREFIX/share:/usr/local/share:/usr/share"
  export CFLAGS="-O2 -g0 -fno-semantic-interposition -ffile-prefix-map=$BUILD_ROOT=/build"
  export CXXFLAGS="$CFLAGS"
  export LDFLAGS="-Wl,-O1 -Wl,--as-needed -Wl,--enable-new-dtags"
  export RUSTFLAGS="--remap-path-prefix=$BUILD_ROOT=/build"
}

# Emptied rather than removed: both are mount points inside the container, and a
# mount point cannot be unlinked.
clean_dir() {
  local dir="$1"
  mkdir -p "$dir"
  find "${dir:?}" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
}

# A run either starts from an empty prefix or resumes one this same lock file
# filled. It cannot be asked to do anything else: the host side discards both
# volumes whenever the cache key changes, so a stamp present here was written by
# a build of exactly these components with exactly these options.
build_all() {
  local name
  if [ -d "$STAMP_DIR" ] && [ -n "$(ls -A "$STAMP_DIR")" ]; then
    log "resuming: $(find "$STAMP_DIR" -type f | wc -l) components are already installed"
  else
    clean_dir "$PREFIX"
    clean_dir "$BUILD_ROOT"
  fi
  mkdir -p "$STAMP_DIR"
  for name in $(component_names); do
    build_component "$name"
  done
  log "every component is installed under $PREFIX"
}

# ------------------------------------------------------- caches and RUNPATH

# The three caches that must exist at their compiled-in paths, because the
# application sets no GDK_PIXBUF_MODULE_FILE, no GIO_MODULE_DIR and no
# XDG_DATA_DIRS. Each is generated here against the final absolute prefix.
generate_caches() {
  local loaders_dir="$PREFIX/lib/gdk-pixbuf-2.0/2.10.0"

  [ -d "$loaders_dir/loaders" ] || fail "no pixbuf loaders were installed"
  "$PREFIX/bin/gdk-pixbuf-query-loaders" > "$loaders_dir/loaders.cache"
  grep -q "^\"$loaders_dir/loaders/" "$loaders_dir/loaders.cache" \
    || fail "loaders.cache does not name the private loader directory"

  "$PREFIX/bin/glib-compile-schemas" "$PREFIX/share/glib-2.0/schemas"
  [ -f "$PREFIX/share/glib-2.0/schemas/gschemas.compiled" ] \
    || fail "the private schemas were not compiled"

  "$PREFIX/bin/gtk4-update-icon-cache" -q -t -f "$PREFIX/share/icons/Adwaita"
  [ -f "$PREFIX/share/icons/Adwaita/icon-theme.cache" ] \
    || fail "the bundled icon theme has no cache"

  log "loaders.cache, gschemas.compiled and the icon cache are generated"
}

is_elf() {
  [ "$(head -c 4 -- "$1" | od -An -tx1 | tr -d ' \n')" = "7f454c46" ]
}

private_elf_files() {
  local candidate
  while IFS= read -r -d '' candidate; do
    if is_elf "$candidate"; then
      printf '%s\0' "$candidate"
    fi
  done < <(find "$PREFIX" -type f \( -name '*.so' -o -name '*.so.*' -o -perm -u+x \) -print0)
}

strip_objects() {
  local target
  while IFS= read -r -d '' target; do
    # No `|| true`. Every file reaching here passed is_elf, so a strip that
    # fails means an object this runtime ships is not what it looks like, and
    # that is worth an hour rather than a silent pass.
    strip --strip-unneeded "$target"
    # Annobin's notes go too, and they are the reason this line exists. They
    # occupy a non-loadable section — nothing maps it, nothing reads it at
    # runtime — but their size varies with how the compile was parallelised, so
    # they make a tree that is otherwise bit-identical compare as different. A
    # rebuild of librsvg from the published source archive at 8 jobs matched the
    # shipped tree in every allocated byte and in its GNU build ID, and differed
    # in 36 bytes of this. Reproducibility that depends on a machine's core
    # count is not reproducibility.
    objcopy -w --remove-section='.gnu.build.attributes*' "$target"
  done < <(private_elf_files)
  log "every private object is stripped"
}

# A library in lib/ points at its own directory; a binary in bin/ and a module in
# a subdirectory of lib/ point back up to lib/. The relative path is computed
# from where the object sits, so a loader four directories down gets four steps
# up and not a guess.
runpath_for() {
  local target="$1" dir rel depth up="" i
  dir="$(dirname -- "$target")"
  if [ "$dir" = "$PREFIX/lib" ]; then
    # shellcheck disable=SC2016  # $ORIGIN is the dynamic linker's token, not a variable
    echo '$ORIGIN'
    return 0
  fi
  rel="${dir#"$PREFIX"/}"
  depth="$(awk -F/ '{print NF}' <<< "$rel")"
  for ((i = 0; i < depth; i++)); do
    up="$up../"
  done
  echo "\$ORIGIN/${up}lib"
}

# patchelf writes the final answer, so there is one place this is decided rather
# than one per build system's idea of what to strip at install time. patchelf
# writes DT_RUNPATH, which is what --enable-new-dtags asks for.
fix_runpaths() {
  local target want
  while IFS= read -r -d '' target; do
    want="$(runpath_for "$target")"
    patchelf --set-rpath "$want" "$target"
  done < <(private_elf_files)
  [ "${1:-}" = "quiet" ] || log "every private object carries its RUNPATH"
}

check_runpaths() {
  local target want have
  while IFS= read -r -d '' target; do
    want="$(runpath_for "$target")"
    have="$(patchelf --print-rpath "$target")"
    [ "$have" = "$want" ] || fail "$target has RUNPATH '$have', expected '$want'"
    # Not a pipe: under `set -o pipefail`, grep -q exits at the first match and
    # SIGPIPEs readelf, which would fail this check on the objects that are fine.
    if grep -q '(RPATH)' <<< "$(readelf -d "$target")"; then
      fail "$target carries RPATH rather than RUNPATH"
    fi
  done < <(private_elf_files)
  log "every RUNPATH is exactly the relative path to the private lib directory"
}

# ------------------------------------------------------------ host boundary

# What makes amendment section 4.1's host column worth anything: every NEEDED
# entry of every private object either names a file inside the prefix or is on
# the host list in the lock file. Anything else is a library the package expects
# a user's machine to have and has not declared, which is the failure this whole
# design exists to remove.
check_needed_entries() {
  local list target needed violations=0
  list=" $(lock_get '.host_libraries[]' | tr '\n' ' ') "
  while IFS= read -r -d '' target; do
    while read -r needed; do
      [ -n "$needed" ] || continue
      if [ -e "$PREFIX/lib/$needed" ]; then
        continue
      fi
      case "$list" in
        *" $needed "*) continue ;;
      esac
      echo "build_runtime: $target needs $needed, which is neither private nor a declared host library" >&2
      violations=$((violations + 1))
    done < <(readelf -d "$target" | sed -n 's/.*NEEDED.*\[\(.*\)\]/\1/p')
  done < <(private_elf_files)
  [ "$violations" -eq 0 ] || fail "$violations undeclared library dependencies"
  log "every NEEDED entry is private or a declared host library"
}

# The tree says which tree it is.
#
# Three runtimes built from three different lock files all answer "gtk 4.16.7,
# libadwaita 1.6.9" to pkg-config, so a version check cannot tell them apart and
# a build image carrying a stale prefix is undetectable without hashing three
# thousand files. Slice 5 found exactly that, by hand. This file is the answer a
# consumer can read in one open: the cache key the tree was built under, the
# digest of the lock file that produced it, and the versions it carries.
#
# It is written into $PREFIX before the trees are staged, so it is in both the
# shipped and the dev variant, it is in runtime-manifest.json, and --verify
# covers it. Content is deterministic: fixed key order, no timestamps, no
# hostname, nothing that varies between two builds of the same lock file.
write_identity() {
  local dir="$PREFIX/share/fermix-desktop-runtime"
  mkdir -p "$dir"
  python3 - "$dir/identity.json" "$(cache_key)" "$(sha256sum < "$LOCK_FILE" | cut -d' ' -f1)" \
      "$(pkg-config --modversion gtk4)" \
      "$(pkg-config --modversion libadwaita-1)" \
      "$(pkg-config --modversion glib-2.0)" \
      "$(lock_get '.prefix')" <<'PY'
import json
import sys

out, key, lock_sha256, gtk, libadwaita, glib, prefix = sys.argv[1:8]
identity = {
    "cache_key": key,
    "glibc_floor": "2.34",
    "lock_sha256": lock_sha256,
    "prefix": prefix,
    "versions": {"glib": glib, "gtk4": gtk, "libadwaita-1": libadwaita},
}
with open(out, "w", encoding="utf-8") as handle:
    json.dump(identity, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
  [ -s "$dir/identity.json" ] || fail "the identity file was not written"
  log "identity.json names key $(cache_key)"
}

check_no_build_paths() {
  local hits
  hits="$(grep -rl "$BUILD_ROOT" "$PREFIX" 2>/dev/null || true)"
  [ -z "$hits" ] || fail "the build path leaks into: $hits"
  log "no build path leaks into the installed tree"
}

report_versions() {
  local gtk adw
  gtk="$(pkg-config --modversion gtk4)"
  adw="$(pkg-config --modversion libadwaita-1)"
  case "$gtk" in 4.16.*) ;; *) fail "gtk4 is $gtk, the floor says 4.16.x" ;; esac
  case "$adw" in 1.6.*) ;; *) fail "libadwaita is $adw, the floor says 1.6.x" ;; esac
  # --modversion reads one .pc file and stops, so it answers "4.16.7" for a
  # toolkit that cannot be compiled against. Resolving the flags walks every
  # Requires transitively, which is what a real build does, and fails loudly
  # when a .pc names a dependency that is not installed. Slice 5 found this the
  # hard way: the version check passed in a build image where gtk4's x11, xcb
  # and zlib requirements were absent, and the crate would not configure.
  pkg-config --libs --cflags gtk4 libadwaita-1 >/dev/null \
    || fail "gtk4 and libadwaita report a version but cannot resolve their flags"
  log "gtk4 $gtk, libadwaita $adw, both resolving their flags transitively"
}

# ----------------------------------------------------------------- exporting

# The dev variant carries the headers and the .pc files the application build
# needs; the shipped variant carries neither, nor static libraries, nor
# documentation, nor introspection data, nor the build-time binaries. The split
# is here rather than in the package, so what slice 4 copies is already exactly
# what installs.
prune_shipped_tree() {
  local root="$1" doomed
  for doomed in include lib/pkgconfig share/pkgconfig share/gir-1.0 \
                lib/girepository-1.0 share/gtk-doc share/doc share/man \
                share/aclocal share/vala share/bash-completion \
                share/installed-tests share/wayland-protocols share/wayland \
                share/glib-2.0/codegen \
                etc var bin; do
    rm -rf "${root:?}$PREFIX/$doomed"
  done
  # gdbus-codegen's Python: a build tool, with no business in the shipped tree.
  # Its __pycache__ was also the single file out of 1512 that a rebuild did not
  # reproduce, because a .pyc embeds the interpreter and source metadata that
  # wrote it. --verify found it, which is exactly what --verify is for.
  find "${root:?}$PREFIX" -type d -name '__pycache__' -exec rm -rf {} +
  find "${root:?}$PREFIX" -name '*.a' -delete
  find "${root:?}$PREFIX" -name '*.la' -delete
  find "${root:?}$PREFIX" -type d -empty -delete
}

shipped_stage_dir() {
  echo "$BUILD_ROOT/stage-ship"
}

stage_tree() {
  local dest="$1" parent
  parent="$(dirname -- "$PREFIX")"
  rm -rf "${dest:?}"
  mkdir -p "$dest$parent"
  cp -a "$PREFIX" "$dest$parent/"
  drop_bytecode_caches "$dest"
}

# Python bytecode caches go from BOTH trees, not only the shipped one.
#
# A .pyc records metadata about the source and the interpreter that wrote it, so
# it does not reproduce across two builds of the same tree. The shipped variant
# lost gdbus-codegen's codegen/ directory outright when --verify caught this the
# first time; the dev variant keeps codegen/, because gdbus-codegen needs it, and
# kept its __pycache__ along with it. So the next --verify failed on the dev
# tarball alone while the shipped one reproduced byte for byte — a real defect
# that only the archives check in the manifest could see, since the tree inside
# was fine. The .py files stay and Python rewrites the cache on first use.
drop_bytecode_caches() {
  local root="$1"
  find "${root:?}$PREFIX" -type d -name '__pycache__' -prune -exec rm -rf {} +
}

# `--sort=name` makes the entry order a property of the tree rather than of the
# filesystem's readdir. Note that the order it produces is a directory walk with
# each directory's entries sorted, which is NOT the same sequence as piping every
# path through `LC_ALL=C sort` — `/` sorts before most characters, so a flat sort
# interleaves a directory's files with its subdirectories' and tar's does not.
# Both are deterministic; only one of them is what tar writes. LC_ALL=C is set
# so the sort cannot follow a locale either.
write_tar() {
  local stage="$1" out="$2"
  LC_ALL=C tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" \
    --owner=0 --group=0 --numeric-owner \
    -C "$stage" -cf "$out" usr
}

archive_digest() {
  sha256sum "$1" | cut -d' ' -f1
}

export_trees() {
  local arch="$1" stage_dev stage_ship
  stage_dev="$BUILD_ROOT/stage-dev"
  stage_ship="$(shipped_stage_dir)"

  stage_tree "$stage_dev"
  stage_tree "$stage_ship"
  prune_shipped_tree "$stage_ship"

  write_tar "$stage_ship" "$OUT_DIR/runtime-$arch.tar"
  write_tar "$stage_dev" "$OUT_DIR/runtime-dev-$arch.tar"

  log "shipped tree $(du -sh "$stage_ship" | cut -f1), dev tree $(du -sh "$stage_dev" | cut -f1)"
}

write_manifest() {
  local stage_ship="$1" arch="$2" shipped dev
  shipped="runtime-$arch.tar"
  dev="runtime-dev-$arch.tar"
  python3 "$RUNTIME_DIR/write_manifest.py" \
    --lock "$LOCK_FILE" \
    --tree "$stage_ship$PREFIX" \
    --prefix "$PREFIX" \
    --arch "$arch" \
    --archive "$shipped=$(archive_digest "$OUT_DIR/$shipped")" \
    --archive "$dev=$(archive_digest "$OUT_DIR/$dev")" \
    --out "$OUT_DIR/runtime-manifest.json"
  log "runtime-manifest.json written"
  log "  $shipped sha256 $(archive_digest "$OUT_DIR/$shipped")"
  log "  $dev sha256 $(archive_digest "$OUT_DIR/$dev")"
}

# ---------------------------------------------------------- the build itself

run_build_mode() {
  local arch
  require_build_tools
  arch="$(target_arch)"
  mkdir -p "$OUT_DIR"
  export_build_environment
  fetch_all
  build_all
  generate_caches
  strip_objects
  fix_runpaths
  check_runpaths
  check_needed_entries
  check_no_build_paths
  report_versions
  # After the checks and before the trees are staged, so both variants carry it
  # and the manifest records it.
  write_identity
  export_trees "$arch"
  write_manifest "$(shipped_stage_dir)" "$arch"
  hand_output_back
  log "done"
}

# This container runs as root, because installing into /usr/lib does. With a
# native daemon the output directory is a host path, so everything written there
# would be root-owned in the invoking user's own working tree. The caller says
# who it is and gets its files back.
hand_output_back() {
  [ -n "${FERMIX_RUNTIME_OWNER:-}" ] || return 0
  chown -R "$FERMIX_RUNTIME_OWNER" "$OUT_DIR"
}

main() {
  parse_args "$@"
  case "$MODE" in
    container) run_container_mode ;;
    verify) run_verify_mode ;;
    print-key) run_print_key_mode ;;
    # No cleanup trap: the build tree is a volume the host side owns and
    # discards when the cache key moves, and wiping it here would throw away
    # both the stamps a resumed run needs and the evidence a failed one left.
    build) run_build_mode ;;
    *) fail "unreachable mode $MODE" ;;
  esac
}

main "$@"
