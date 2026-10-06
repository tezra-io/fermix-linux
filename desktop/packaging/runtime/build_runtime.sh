#!/usr/bin/env bash
#
# Build the private GTK runtime that fermix-desktop carries.
#
# Every component in RUNTIME.lock.json is fetched by URL, refused unless its
# sha256 is the locked one, and configured with --prefix=/usr/lib/fermix-desktop,
# the path it occupies on a user's machine. Nothing is built under a staging
# prefix and relocated: GLib, gdk-pixbuf and GStreamer compile that prefix in as
# where they find their modules, loaders and plugins, so all three are right
# with no environment variable.
#
# Usage:
#   desktop/packaging/runtime/build_runtime.sh --container [--fresh]   build in Docker, export the trees
#   desktop/packaging/runtime/build_runtime.sh --verify      rebuild from empty volumes and compare
#   desktop/packaging/runtime/build_runtime.sh --print-key   print the cache key
#   desktop/packaging/runtime/build_runtime.sh --build       compile here; what runs in the container
#
# The container gets its inputs by `docker cp` and gives its outputs back the
# same way, checked by sha256 on both sides. Nothing is bind-mounted: where the
# daemon runs in a virtual machine, a bind mount can carry another clock and can
# truncate large files written through it.
set -euo pipefail
shopt -s inherit_errexit

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNTIME_DIR="$ROOT_DIR/packaging/runtime"
LOCK_FILE="$RUNTIME_DIR/RUNTIME.lock.json"
PATCH_DIR="$RUNTIME_DIR/patches"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.runtime"
CACHE_HOME="${XDG_CACHE_HOME:-${HOME:-/root}/.cache}/fermix-desktop-runtime"
OUT_DIR="${FERMIX_RUNTIME_OUT:-$CACHE_HOME/out}"
SOURCE_CACHE="${FERMIX_RUNTIME_SOURCES:-$CACHE_HOME/sources}"
IMAGE="${FERMIX_RUNTIME_IMAGE:-fermix-desktop-pkg-runtime-build}"
PREFIX="/usr/lib/fermix-desktop"
JOBS="${FERMIX_RUNTIME_JOBS:-4}"

# Inside the container. The build tree and the prefix are named volumes, so a
# failure in the thirtieth component does not rebuild the first twenty-nine:
# each installed component leaves a stamp, and the volumes are discarded
# whenever the cache key changes, so a stamp always describes this lock file.
BUILD_ROOT="/var/tmp/fermix-runtime-build"
STAMP_DIR="$BUILD_ROOT/stamps"
CONTAINER_OUT="$BUILD_ROOT/out"
CRATES_DIR="$BUILD_ROOT/crates"
LICENSES_DIR="$BUILD_ROOT/licenses"
PREFIX_VOLUME="${FERMIX_RUNTIME_PREFIX_VOLUME:-fermix-desktop-pkg-runtime-prefix}"
BUILD_VOLUME="${FERMIX_RUNTIME_BUILD_VOLUME:-fermix-desktop-pkg-runtime-build}"

# The only state not passed as arguments: an EXIT trap runs after every
# function has returned, and these name what it must remove.
CONTAINER=""
VERIFY_SCRATCH=""
MODE=""
FRESH=0

fail() {
  echo "build_runtime: $*" >&2
  exit 1
}

# Progress goes to standard error. Only --print-key writes to standard output.
log() {
  echo "build_runtime: $*" >&2
}

# shellcheck source=desktop/packaging/runtime/fetch_source.sh
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

require_inputs() {
  [ -f "$LOCK_FILE" ] || fail "no lock file at $LOCK_FILE"
  [ -f "$DOCKERFILE" ] || fail "no runtime container at $DOCKERFILE"
  [ -d "$PATCH_DIR" ] || fail "no patch directory at $PATCH_DIR"
}

require_host_tools() {
  local tool
  for tool in docker sha256sum tar jq curl python3; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed"
  done
  require_inputs
}

lock_get() {
  jq -r "$1" "$LOCK_FILE"
}

component_field() {
  jq -r --arg n "$1" '.components[] | select(.name == $n) | .'"$2" "$LOCK_FILE"
}

component_options() {
  jq -r --arg n "$1" '.components[] | select(.name == $n) | .options[]' "$LOCK_FILE"
}

# The key of amendment section 4.2: the lock file, the patches and the
# Dockerfile, by content and never by path, so the same inputs give the same key
# in this repository, a CI checkout and an unpacked source archive.
cache_key() {
  local patch
  {
    sha256sum < "$LOCK_FILE"
    sha256sum < "$DOCKERFILE"
    sha256sum < "$RUNTIME_DIR/build_runtime.sh"
    sha256sum < "$RUNTIME_DIR/write_manifest.py"
    sha256sum < "$RUNTIME_DIR/drop_unreachable.sh"
    sha256sum < "$RUNTIME_DIR/write_crates.py"
    sha256sum < "$RUNTIME_DIR/write_licenses.py"
    while IFS= read -r patch; do
      printf '%s ' "$(basename -- "$patch")"
      sha256sum < "$patch"
    done < <(find "$PATCH_DIR" -type f -name '*.patch' | LC_ALL=C sort)
  } | sha256sum | cut -c1-16
}

# Every locked tarball, each held to its digest, and the licence text source,
# which is read and never built. On the host this fills the source cache; in
# the container it re-checks what was copied in.
fetch_all() {
  local name
  mkdir -p "$SOURCE_CACHE"
  for name in $(lock_get '.components[].name'); do
    ensure_source "$name" "$(component_field "$name" url)" \
      "$SOURCE_CACHE/$(component_field "$name" archive)" "$(component_field "$name" sha256)"
  done
  ensure_source "$(lock_get '.license_text_source.name')" "$(lock_get '.license_text_source.url')" \
    "$SOURCE_CACHE/$(lock_get '.license_text_source.archive')" \
    "$(lock_get '.license_text_source.sha256')"
  log "every locked tarball is in $SOURCE_CACHE and matches its digest"
}

# ---------------------------------------------------------------- host side

build_image() {
  log "building $IMAGE"
  docker build -f "$DOCKERFILE" -t "$IMAGE" "$ROOT_DIR/packaging/docker" >&2
}

discard_work_volumes() {
  docker volume rm -f "$BUILD_VOLUME" "$PREFIX_VOLUME" >/dev/null
}

# A resume is only safe while the inputs have not changed, so the key the
# volumes were filled under is their label. Another key, or --fresh, and both go.
prepare_work_volumes() {
  local key="$1" previous=""
  if [ "$FRESH" = "1" ]; then
    log "--fresh: discarding the work volumes"
    discard_work_volumes
    return 0
  fi
  if docker volume inspect "$BUILD_VOLUME" >/dev/null 2>&1; then
    previous="$(docker volume inspect "$BUILD_VOLUME" --format '{{index .Labels "fermix.runtime.key"}}')"
  fi
  if [ "$previous" = "$key" ]; then
    log "resuming in the work volumes built for key $key"
    return 0
  fi
  [ -z "$previous" ] || log "the work volumes were built for key $previous, not $key"
  discard_work_volumes
}

# Created here rather than by `docker create`, which would leave them unlabelled.
label_work_volumes() {
  docker volume create --label "fermix.runtime.key=$1" "$BUILD_VOLUME" >/dev/null
  docker volume create --label "fermix.runtime.key=$1" "$PREFIX_VOLUME" >/dev/null
}

remove_container() {
  [ -n "$CONTAINER" ] || return 0
  docker rm -f "$CONTAINER" >/dev/null
  CONTAINER=""
}

create_build_container() {
  CONTAINER="fermix-desktop-pkg-runtime-$$"
  docker create --name "$CONTAINER" --init \
    -v "$BUILD_VOLUME:$BUILD_ROOT" \
    -v "$PREFIX_VOLUME:$PREFIX" \
    -e FERMIX_RUNTIME_SOURCES=/sources \
    -e FERMIX_RUNTIME_JOBS="$JOBS" \
    -w /workspace \
    "$IMAGE" bash /workspace/packaging/runtime/build_runtime.sh --build >/dev/null
}

# The scripts keep this tree's layout, because build_runtime.sh finds its lock
# file, patches and Dockerfile by paths relative to itself.
copy_inputs() {
  local -a archives
  mapfile -t archives < <(lock_get '.components[].archive, .license_text_source.archive')
  tar -C "$ROOT_DIR" -cf - packaging/runtime packaging/docker/Dockerfile.runtime \
    | docker cp - "$CONTAINER:/workspace"
  tar -C "$SOURCE_CACHE" -cf - "${archives[@]}" | docker cp - "$CONTAINER:/sources"
}

# The container wrote SHA256SUMS beside its outputs; the copies are held to it.
copy_outputs() {
  local out_dir="$1"
  mkdir -p "$out_dir"
  docker cp "$CONTAINER:$CONTAINER_OUT/." "$out_dir/"
  [ -f "$out_dir/SHA256SUMS" ] || fail "the build container wrote no SHA256SUMS"
  (cd "$out_dir" && sha256sum --quiet -c SHA256SUMS) \
    || fail "the outputs copied to $out_dir do not match the digests the container wrote"
  log "outputs copied to $out_dir, every sha256 matching the container's"
}

run_build_container() {
  local out_dir="$1"
  create_build_container
  copy_inputs
  docker start -a "$CONTAINER" || fail "the build failed; the work volumes keep its state for a resume"
  copy_outputs "$out_dir"
  remove_container
}

run_container_mode() {
  local key arch
  require_host_tools
  key="$(cache_key)"
  arch="$(target_arch)"
  fetch_all
  build_image
  prepare_work_volumes "$key"
  label_work_volumes "$key"
  trap remove_container EXIT
  run_build_container "$OUT_DIR"
  echo "$key" > "$OUT_DIR/cache-key"
  log "runtime built for $arch, cache key $key, in $OUT_DIR"
}

# Needs no daemon: a workflow deciding whether to pull or build the runtime
# image should not need Docker to ask.
run_print_key_mode() {
  command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is not installed"
  require_inputs
  cache_key
}

cleanup_verify() {
  remove_container
  [ -z "$VERIFY_SCRATCH" ] || rm -rf "$VERIFY_SCRATCH"
  discard_work_volumes
}

# A verification that resumed from the first build's volumes would compare a
# tree with itself, so it builds in its own empty pair and removes them after.
run_verify_mode() {
  local reference="$OUT_DIR/runtime-manifest.json"
  [ -f "$reference" ] || fail "no manifest to verify against at $reference"
  require_host_tools
  VERIFY_SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/fermix-runtime-verify.XXXXXX")"
  PREFIX_VOLUME="fermix-desktop-pkg-runtime-verify-prefix"
  BUILD_VOLUME="fermix-desktop-pkg-runtime-verify-build"
  trap cleanup_verify EXIT
  discard_work_volumes
  fetch_all
  build_image
  run_build_container "$VERIFY_SCRATCH"
  python3 "$RUNTIME_DIR/compare_manifest.py" "$reference" "$VERIFY_SCRATCH/runtime-manifest.json" \
    || fail "the rebuild does not reproduce $reference"
  log "the rebuild reproduces $reference"
}

# ----------------------------------------------------------- container side

require_build_tools() {
  local tool
  for tool in jq meson ninja patchelf cmake gcc g++ cargo cargo-cbuild rustc curl tar gzip python3 \
              readelf strip objcopy sassc pkg-config realpath; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed in this container"
  done
  require_inputs
}

apply_patches() {
  local name="$1" src="$2" patch
  for patch in $(component_field "$name" 'patches[]'); do
    [ -f "$PATCH_DIR/$patch" ] || fail "$name: no patch at $PATCH_DIR/$patch"
    log "$name: applying $patch"
    patch -p1 -d "$src" < "$PATCH_DIR/$patch" >&2
  done
}

unpack_component() {
  local name="$1" src="$2" archive
  archive="$(component_field "$name" archive)"
  rm -rf "$src"
  tar -xf "$SOURCE_CACHE/$archive" -C "$BUILD_ROOT"
  [ -d "$src" ] || fail "$name: $archive does not unpack to $src"
  apply_patches "$name" "$src"
}

# libdir is lib on every build system: meson and cmake would choose lib64 on an
# RPM distribution, and the installed layout says lib. Meson never downloads a
# subproject; a dependency the prefix lacks fails the build instead.
build_meson() {
  local name="$1" src="$2"
  local -a options
  mapfile -t options < <(component_options "$name")
  meson setup "$src/_build" "$src" \
    --prefix="$PREFIX" --libdir=lib --buildtype=release --wrap-mode=nodownload \
    --strip -Db_ndebug=true -Ddefault_library=shared \
    "${options[@]}"
  meson compile -C "$src/_build" -j "$JOBS"
  meson install -C "$src/_build" --no-rebuild
}

# Autotools tarballs ship generated files whose timestamps can arrive out of
# order, and make would then try to regenerate them with the maintainer's
# automake. Every file gets the lock file's epoch, the generated ones a second
# or three later in dependency order.
normalise_autotools_timestamps() {
  local src="$1" epoch="$SOURCE_DATE_EPOCH"
  find "$src" -print0 | xargs -0 -r touch -h -d "@$epoch"
  find "$src" -name 'aclocal.m4' -print0 | xargs -0 -r touch -d "@$((epoch + 1))"
  find "$src" \( -name 'configure' -o -name '*.h.in' -o -name 'config.hin' \) -print0 \
    | xargs -0 -r touch -d "@$((epoch + 2))"
  find "$src" -name 'Makefile.in' -print0 | xargs -0 -r touch -d "@$((epoch + 3))"
}

build_autotools() {
  local name="$1" src="$2"
  local -a options
  mapfile -t options < <(component_options "$name")
  normalise_autotools_timestamps "$src"
  (cd "$src" && ./configure --prefix="$PREFIX" --libdir="$PREFIX/lib" "${options[@]}")
  make -C "$src" -j "$JOBS"
  make -C "$src" install
}

build_cmake() {
  local name="$1" src="$2"
  local -a options
  mapfile -t options < <(component_options "$name")
  # Deterministic archives: GNU ar here stamps members with their mtime otherwise.
  cmake -S "$src" -B "$src/_build" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$PREFIX" \
    -DCMAKE_INSTALL_LIBDIR=lib \
    "-DCMAKE_C_ARCHIVE_CREATE=<CMAKE_AR> qcD <TARGET> <LINK_FLAGS> <OBJECTS>" \
    "-DCMAKE_C_ARCHIVE_APPEND=<CMAKE_AR> qD <TARGET> <LINK_FLAGS> <OBJECTS>" \
    "-DCMAKE_C_ARCHIVE_FINISH=<CMAKE_RANLIB> -D <TARGET>" \
    "-DCMAKE_CXX_ARCHIVE_CREATE=<CMAKE_AR> qcD <TARGET> <LINK_FLAGS> <OBJECTS>" \
    "-DCMAKE_CXX_ARCHIVE_APPEND=<CMAKE_AR> qD <TARGET> <LINK_FLAGS> <OBJECTS>" \
    "-DCMAKE_CXX_ARCHIVE_FINISH=<CMAKE_RANLIB> -D <TARGET>" \
    "${options[@]}"
  cmake --build "$src/_build" -j "$JOBS"
  cmake --install "$src/_build"
}

# A Rust component's crates are the one input the build fetches itself. They are
# pinned by the Cargo.lock its lock entry's cargo object names, inside the
# locked tarball, and cargo checks each one against it. They are vendored into the source tree, and
# the .cargo/config.toml cargo vendor prints points the compile there; cargo
# finds that file from $src/_build, where meson runs it.
vendor_crates() {
  local name="$1" src="$2" cargo_lock="$3"
  [ -f "$src/$cargo_lock" ] || fail "$name: no $cargo_lock in its source tree"
  mkdir -p "$src/.cargo"
  CARGO_NET_OFFLINE=false cargo vendor --locked --versioned-dirs \
    --manifest-path "$src/$(dirname -- "$cargo_lock")/Cargo.toml" "$src/_crates" \
    > "$src/.cargo/config.toml" \
    || fail "$name: its crates could not be vendored at the $cargo_lock checksums"
  log "$name: $(find "$src/_crates" -mindepth 1 -maxdepth 1 -type d | wc -l) crates vendored, by licence:"
  find "$src/_crates" -mindepth 2 -maxdepth 2 -name Cargo.toml -exec sed -n 's/^license = "\(.*\)"$/\1/p' {} + \
    | sort | uniq -c | sort -rn >&2
}

# The crates a Rust component compiled in: their sources into the tree
# runtime-crates.tar.gz is made from, their licence files into the one
# runtime-licenses.tar.gz is made from, and their entries into a fragment of
# runtime-licenses.json. Run while the source tree and its build still exist.
inventory_crates() {
  local name="$1" src="$2"
  python3 "$RUNTIME_DIR/write_crates.py" component --name "$name" --source-dir "$src" \
    --cargo "$(jq -c --arg n "$name" '.components[] | select(.name == $n) | .cargo' "$LOCK_FILE")" \
    --build-dir "$src/_build" --tree "$CRATES_DIR/tree" --licenses-tree "$LICENSES_DIR" \
    --out "$CRATES_DIR/fragments/$name.json" \
    || fail "$name: the crates it compiles in could not be listed"
}

build_component() {
  local name="$1" system src cargo_lock stamp="$STAMP_DIR/$1"
  if [ -f "$stamp" ]; then
    log "--- $name $(component_field "$name" version) is already installed"
    return 0
  fi
  system="$(component_field "$name" build_system)"
  log "=== $name $(component_field "$name" version) ($system)"
  src="$BUILD_ROOT/$(component_field "$name" source_dir)"
  unpack_component "$name" "$src"
  cargo_lock="$(component_field "$name" cargo.lock)"
  [ "$cargo_lock" = "null" ] || vendor_crates "$name" "$src" "$cargo_lock"
  case "$system" in
    meson) build_meson "$name" "$src" ;;
    autotools) build_autotools "$name" "$src" ;;
    cmake) build_cmake "$name" "$src" ;;
    *) fail "$name: unknown build system $system" ;;
  esac
  [ "$cargo_lock" = "null" ] || inventory_crates "$name" "$src"
  # At once: the next component's configure may run a tool this one installed,
  # and with no LD_LIBRARY_PATH that tool finds its libraries by this RUNPATH.
  fix_runpaths quiet
  # And the boundary, while the component that crossed it is the one just built.
  bash "$RUNTIME_DIR/check_boundary.sh" "$PREFIX" "$LOCK_FILE" \
    || fail "$name crossed the host boundary"
  rm -rf "$src"
  # Last, so a component that died half way through install is built again.
  mkdir -p "$STAMP_DIR"
  date -u +%s > "$stamp"
}

# There is no LD_LIBRARY_PATH. It would reach the host's own tools too, and the
# host's itstool and xsltproc would load the private libxml2 instead of the one
# they were built against. Private tools find their libraries by RUNPATH, the
# same way they will on a user's machine.
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
  # Cargo compiles only what vendor_crates checked.
  export CARGO_NET_OFFLINE=true
  export PYTHONDONTWRITEBYTECODE=1
}

# Emptied rather than removed: both are mount points.
clean_dir() {
  mkdir -p "$1"
  find "${1:?}" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
}

build_all() {
  local name
  if [ -d "$STAMP_DIR" ] && [ -n "$(ls -A "$STAMP_DIR")" ]; then
    log "resuming: $(find "$STAMP_DIR" -type f | wc -l) components are already installed"
  else
    clean_dir "$PREFIX"
    clean_dir "$BUILD_ROOT"
  fi
  mkdir -p "$STAMP_DIR"
  for name in $(lock_get '.components[].name'); do
    build_component "$name"
  done
  log "every component is installed under $PREFIX"
}

# ------------------------------------------------------- caches and RUNPATH

# The caches GLib and GTK read at their compiled-in paths, generated against the
# final absolute prefix, so the application sets no GDK_PIXBUF_MODULE_FILE and
# no GIO_MODULE_DIR.
generate_caches() {
  local loaders_dir="$PREFIX/lib/gdk-pixbuf-2.0/2.10.0"
  [ -d "$loaders_dir/loaders" ] || fail "no pixbuf loaders were installed"
  "$PREFIX/bin/gdk-pixbuf-query-loaders" > "$loaders_dir/loaders.cache"
  grep -q "^\"$loaders_dir/loaders/" "$loaders_dir/loaders.cache" \
    || fail "loaders.cache does not name the private loader directory"
  "$PREFIX/bin/glib-compile-schemas" "$PREFIX/share/glib-2.0/schemas"
  [ -f "$PREFIX/share/glib-2.0/schemas/gschemas.compiled" ] || fail "the private schemas were not compiled"
  "$PREFIX/bin/gio-querymodules" "$PREFIX/lib/gio/modules"
  [ -f "$PREFIX/lib/gio/modules/giomodule.cache" ] || fail "the GIO module cache was not written"
  "$PREFIX/bin/gtk4-update-icon-cache" -q -t -f "$PREFIX/share/icons/Adwaita"
  [ -f "$PREFIX/share/icons/Adwaita/icon-theme.cache" ] || fail "the bundled icon theme has no cache"
  log "loaders.cache, gschemas.compiled, giomodule.cache and the icon cache are generated"
}

is_elf() {
  [ "$(od -An -tx1 -N4 -- "$1" | tr -d ' \n')" = "7f454c46" ]
}

private_elf_files() {
  local candidate
  while IFS= read -r -d '' candidate; do
    if is_elf "$candidate"; then
      printf '%s\0' "$candidate"
    fi
  done < <(find "$PREFIX" -type f \( -name '*.so' -o -name '*.so.*' -o -perm -u+x \) -print0)
}

# Annobin notes go with the symbols: their size varies with how a compile was
# parallelised, so they would make one tree compare as two.
strip_objects() {
  local target
  while IFS= read -r -d '' target; do
    strip --strip-unneeded "$target"
    objcopy -w --remove-section='.gnu.build.attributes*' "$target"
  done < <(private_elf_files)
  log "every private object is stripped"
}

# $ORIGIN for a library in lib/, and one step up per directory for anything
# deeper or in bin/ or libexec/, computed from where the object sits.
runpath_for() {
  local dir rel depth up="" i
  dir="$(dirname -- "$1")"
  if [ "$dir" = "$PREFIX/lib" ]; then
    # shellcheck disable=SC2016 # $ORIGIN is the dynamic linker's token
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

# patchelf writes DT_RUNPATH, which is what --enable-new-dtags asks for. One
# place decides it, rather than each build system's idea of an install rpath.
fix_runpaths() {
  local target want
  while IFS= read -r -d '' target; do
    want="$(runpath_for "$target")"
    [ "$(patchelf --print-rpath "$target")" = "$want" ] || patchelf --set-rpath "$want" "$target"
  done < <(private_elf_files)
  [ "${1:-}" = "quiet" ] || log "every private object carries its RUNPATH"
}

check_runpaths() {
  local target want have dynamic
  while IFS= read -r -d '' target; do
    want="$(runpath_for "$target")"
    have="$(patchelf --print-rpath "$target")"
    [ "$have" = "$want" ] || fail "$target has RUNPATH '$have', expected '$want'"
    dynamic="$(readelf -d "$target")"
    if grep -q '(RPATH)' <<< "$dynamic"; then
      fail "$target carries RPATH rather than RUNPATH"
    fi
  done < <(private_elf_files)
  log "every RUNPATH is the relative path to the private lib directory"
}

check_no_build_paths() {
  local hits status=0
  hits="$(grep -rl -- "$BUILD_ROOT" "$PREFIX")" || status=$?
  [ "$status" -le 1 ] || fail "could not search $PREFIX for the build path"
  [ -z "$hits" ] || fail "the build path leaks into: $hits"
  log "no build path leaks into the installed tree"
}

# --modversion reads one .pc file; resolving the flags walks every Requires, as
# the application's build will.
report_versions() {
  local gtk adw glib gst
  gtk="$(pkg-config --modversion gtk4)"
  adw="$(pkg-config --modversion libadwaita-1)"
  glib="$(pkg-config --modversion glib-2.0)"
  gst="$(pkg-config --modversion gstreamer-1.0)"
  case "$gtk" in 4.22.*) ;; *) fail "gtk4 is $gtk, the window needs 4.22.x" ;; esac
  case "$adw" in 1.9.*) ;; *) fail "libadwaita is $adw, the window needs 1.9.x" ;; esac
  case "$glib" in 2.88.*) ;; *) fail "glib is $glib, the window needs 2.88.x" ;; esac
  case "$gst" in 1.26.*) ;; *) fail "gstreamer is $gst, the lock pins 1.26.x" ;; esac
  pkg-config --libs --cflags gtk4 libadwaita-1 gstreamer-1.0 gstreamer-app-1.0 gstreamer-audio-1.0 \
    >/dev/null || fail "the toolkit reports versions but cannot resolve its flags"
  log "gtk4 $gtk, libadwaita $adw, glib $glib, gstreamer $gst, all resolving their flags"
}

# The tree says which tree it is, in one file both variants and the manifest
# carry: the cache key, the lock file's digest and the versions.
write_identity() {
  local dir="$PREFIX/share/fermix-desktop-runtime"
  mkdir -p "$dir"
  jq -n --arg key "$(cache_key)" --arg lock "$(sha256sum < "$LOCK_FILE" | cut -d' ' -f1)" \
    --arg prefix "$PREFIX" --arg glibc "$(lock_get '.glibc_floor')" \
    --arg gtk "$(pkg-config --modversion gtk4)" --arg adw "$(pkg-config --modversion libadwaita-1)" \
    --arg glib "$(pkg-config --modversion glib-2.0)" --arg gst "$(pkg-config --modversion gstreamer-1.0)" \
    -S '{cache_key: $key, glibc_floor: $glibc, lock_sha256: $lock, prefix: $prefix,
         versions: {glib: $glib, gstreamer: $gst, gtk4: $gtk, "libadwaita-1": $adw}}' \
    > "$dir/identity.json"
  [ -s "$dir/identity.json" ] || fail "the identity file was not written"
  log "identity.json names key $(cache_key)"
}

# ----------------------------------------------------------------- exporting

# The shipped variant carries no headers, .pc or CMake files, static libraries,
# development symlinks, documentation, debugger or valgrind helpers, build-time
# binaries or Python bytecode. From dconf only the GSettings module and its
# library ship: the host runs dconf-service. The MIME database is the host's,
# read through XDG_DATA_DIRS.
prune_shipped_tree() {
  local root="$1$PREFIX" doomed
  for doomed in include lib/pkgconfig share/pkgconfig lib/cmake share/cmake lib/glib-2.0 \
                share/gir-1.0 lib/girepository-1.0 share/gtk-doc share/doc share/man share/aclocal \
                share/vala share/bash-completion share/installed-tests share/wayland-protocols \
                share/wayland share/glib-2.0/codegen share/glib-2.0/gettext share/glib-2.0/gdb \
                share/glib-2.0/valgrind share/glib-2.0/dtds share/gtk-4.0/valgrind \
                share/gtk-4.0/gtk4builder.rng share/gstreamer-1.0/gdb share/gdb share/gettext \
                share/dbus-1 share/mime share/thumbnailers share/metainfo share/xml \
                libexec/dconf-service libexec/gstreamer-1.0/gst-completion-helper \
                libexec/gstreamer-1.0/gst-hotdoc-plugins-scanner \
                libexec/gstreamer-1.0/gst-plugins-doc-cache-generator etc var bin; do
    rm -rf "${root:?}/$doomed"
  done
  find "$root" \( -name '*.a' -o -name '*.la' -o -name '*.h' \) -delete
  # libfoo.so is the link-time name only; what loads is the SONAME beside it.
  find "$root/lib" -maxdepth 1 -type l -name '*.so' -delete
  find "$root" -type d -name '__pycache__' -prune -exec rm -rf {} +
  drop_unreachable_objects "$root"
  find "$root" -type d -empty -delete
}

# The libraries the window links: every -l of the lock file's application
# packages that the prefix holds, as the file it resolves to. The rest, -lm and
# the like, are the host's.
application_roots() {
  local -a packages
  local flags flag
  mapfile -t packages < <(lock_get '.application_packages[]')
  [ "${#packages[@]}" -gt 0 ] || fail "the lock file names no application package"
  flags="$(pkg-config --libs-only-l "${packages[@]}")" \
    || fail "pkg-config does not find every application package: ${packages[*]}"
  for flag in $flags; do
    if [ -e "$PREFIX/lib/lib${flag#-l}.so" ]; then
      realpath --relative-to="$PREFIX" -- "$PREFIX/lib/lib${flag#-l}.so"
    fi
  done
}

# What loads without a NEEDED entry: the plugin directories compiled into
# GStreamer, gdk-pixbuf and GIO, and libexec, whose programs run by path.
plugin_roots() {
  local dir
  for dir in "$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
             "$(pkg-config --variable=gdk_pixbuf_moduledir gdk-pixbuf-2.0)" \
             "$(pkg-config --variable=giomoduledir gio-2.0)"; do
    case "$dir" in
      "$PREFIX"/*) printf '%s\n' "${dir#"$PREFIX"/}" ;;
      *) fail "a plugin directory is not in the prefix: '$dir'" ;;
    esac
  done
  echo libexec
}

# Every object no root reaches through NEEDED goes: gst-plugins-bad's and
# gst-plugins-base's unused libraries, among others.
drop_unreachable_objects() {
  local root="$1" found
  local -a roots
  found="$(application_roots)"
  mapfile -t roots <<< "$found"
  found="$(plugin_roots)"
  mapfile -t -O "${#roots[@]}" roots <<< "$found"
  bash "$RUNTIME_DIR/drop_unreachable.sh" "$root" "${roots[@]}" \
    || fail "the shipped tree could not be pruned to what is reached"
}

stage_tree() {
  local dest="$1" parent
  parent="$(dirname -- "$PREFIX")"
  rm -rf "${dest:?}"
  mkdir -p "$dest$parent"
  cp -a "$PREFIX" "$dest$parent/"
  # A .pyc records the interpreter that wrote it, so neither tree keeps one.
  find "$dest$PREFIX" -type d -name '__pycache__' -prune -exec rm -rf {} +
}

# --sort=name makes the entry order a property of the tree, not of readdir.
write_tar() {
  LC_ALL=C tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" \
    --owner=0 --group=0 --numeric-owner -C "$1" -cf "$2" usr
}

archive_digest() {
  sha256sum "$1" | cut -d' ' -f1
}

# A tree's top entries in name order, gzipped with no name or time in the header.
write_tar_gz() {
  local tree="$1" out="$2"
  local -a tops
  mapfile -t tops < <(find "$tree" -mindepth 1 -maxdepth 1 -printf '%f\n' | LC_ALL=C sort)
  [ "${#tops[@]}" -gt 0 ] || fail "nothing to archive in $tree"
  LC_ALL=C tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner \
    -C "$tree" -cf - "${tops[@]}" | gzip -n > "$out"
}

# runtime-crates.tar.gz: the compiled-in crates' sources, for the source archive.
# runtime-licenses.tar.gz: the licence files of every component, crate and std,
# and runtime-licenses.json, its index.
export_crates_and_licences() {
  local index="$CONTAINER_OUT/runtime-licenses.json"
  mkdir -p "$CRATES_DIR/fragments" "$CRATES_DIR/tree" "$LICENSES_DIR"
  python3 "$RUNTIME_DIR/write_licenses.py" --lock "$LOCK_FILE" --sources "$SOURCE_CACHE" \
    --crates "$CRATES_DIR/fragments" --tree "$LICENSES_DIR" --out "$index" \
    || fail "runtime-licenses.json could not be written"
  write_tar_gz "$CRATES_DIR/tree" "$CONTAINER_OUT/runtime-crates.tar.gz"
  write_tar_gz "$LICENSES_DIR" "$CONTAINER_OUT/runtime-licenses.tar.gz"
  log "runtime-licenses.json: $(jq -r '"\(.components | length) components, \(.crates | length) crates"' "$index")"
}

export_trees() {
  local arch="$1" stage_dev="$BUILD_ROOT/stage-dev" stage_ship="$BUILD_ROOT/stage-ship"
  local shipped="runtime-$arch.tar" dev="runtime-dev-$arch.tar"
  clean_dir "$CONTAINER_OUT"
  stage_tree "$stage_dev"
  stage_tree "$stage_ship"
  prune_shipped_tree "$stage_ship"
  bash "$RUNTIME_DIR/check_boundary.sh" "$stage_ship$PREFIX" "$LOCK_FILE" \
    || fail "the pruned shipped tree crossed the host boundary"
  export_crates_and_licences
  write_tar "$stage_ship" "$CONTAINER_OUT/$shipped"
  write_tar "$stage_dev" "$CONTAINER_OUT/$dev"
  log "shipped tree $(du -sh "$stage_ship$PREFIX" | cut -f1), dev tree $(du -sh "$stage_dev$PREFIX" | cut -f1)"
  python3 "$RUNTIME_DIR/write_manifest.py" \
    --lock "$LOCK_FILE" --tree "$stage_ship$PREFIX" --prefix "$PREFIX" --arch "$arch" \
    --archive "$shipped=$(archive_digest "$CONTAINER_OUT/$shipped")" \
    --archive "$dev=$(archive_digest "$CONTAINER_OUT/$dev")" \
    --archive "runtime-crates.tar.gz=$(archive_digest "$CONTAINER_OUT/runtime-crates.tar.gz")" \
    --archive "runtime-licenses.tar.gz=$(archive_digest "$CONTAINER_OUT/runtime-licenses.tar.gz")" \
    --archive "runtime-licenses.json=$(archive_digest "$CONTAINER_OUT/runtime-licenses.json")" \
    --out "$CONTAINER_OUT/runtime-manifest.json" >&2
  rm -rf "$stage_dev" "$stage_ship"
  (cd "$CONTAINER_OUT" && sha256sum "$shipped" "$dev" runtime-crates.tar.gz runtime-licenses.tar.gz \
     runtime-licenses.json runtime-manifest.json > SHA256SUMS)
  log "exported: $(tr '\n' ' ' < "$CONTAINER_OUT/SHA256SUMS")"
}

run_build_mode() {
  local arch
  require_build_tools
  arch="$(target_arch)"
  export_build_environment
  fetch_all
  build_all
  generate_caches
  strip_objects
  fix_runpaths
  check_runpaths
  bash "$RUNTIME_DIR/check_boundary.sh" "$PREFIX" "$LOCK_FILE"
  check_no_build_paths
  report_versions
  write_identity
  export_trees "$arch"
  log "done"
}

main() {
  parse_args "$@"
  case "$MODE" in
    container) run_container_mode ;;
    verify) run_verify_mode ;;
    print-key) run_print_key_mode ;;
    build) run_build_mode ;;
    *) fail "unreachable mode $MODE" ;;
  esac
}

main "$@"
