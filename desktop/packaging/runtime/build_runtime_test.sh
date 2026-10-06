#!/usr/bin/env bash
#
# The runtime build's offline tests: its refusals, the lock file's rules, the
# container's claims and the boundary gate, without compiling the runtime.
#
# Docker is never invoked. The build script is driven with an empty PATH or
# against a throwaway copy of the tree, and the boundary gate against small
# objects compiled here with the host's gcc.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNTIME_DIR="$ROOT_DIR/packaging/runtime"
SCRIPT="$RUNTIME_DIR/build_runtime.sh"
GATE="$RUNTIME_DIR/check_boundary.sh"
DROP="$RUNTIME_DIR/drop_unreachable.sh"
SMOKE="$RUNTIME_DIR/smoke_runtime.sh"
SOURCES="$RUNTIME_DIR/package_sources.sh"
LOCK="$RUNTIME_DIR/RUNTIME.lock.json"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.runtime"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/build-runtime-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fail() {
  echo "build_runtime_test: $*" >&2
  exit 1
}

# True when the command fails and says why: a refusal for some other reason,
# a missing tool or a typo, is not the refusal under test.
refuses() {
  local reason="$1" err="$WORK/refusal.err"
  shift
  if "$@" >/dev/null 2>"$err"; then
    return 1
  fi
  grep -q -- "$reason" "$err" || {
    echo "build_runtime_test: refused, but not with '$reason':" >&2
    cat "$err" >&2
    return 1
  }
}

# A PATH holding every tool the scripts use except docker.
make_path_without_docker() {
  local bin="$1" tool
  mkdir -p "$bin"
  for tool in bash dirname basename cat cut find sort sha256sum python3 jq mkdir rm uname id; do
    ln -sf "$(command -v "$tool")" "$bin/$tool"
  done
}

test_syntax() {
  echo "build_runtime_test: the scripts parse"
  local script
  for script in "$SCRIPT" "$GATE" "$DROP" "$SMOKE" "$SOURCES" "$RUNTIME_DIR/fetch_source.sh" \
                "$RUNTIME_DIR/build_runtime_test_boundary.sh" \
                "$RUNTIME_DIR/build_runtime_test_unreachable.sh"; do
    bash -n "$script" || fail "$(basename "$script") does not parse"
  done
  # Compiled in memory, so no __pycache__ lands in the tree.
  python3 -c 'import sys
for name in sys.argv[1:]:
    with open(name, encoding="utf-8") as handle:
        compile(handle.read(), name, "exec")' \
    "$RUNTIME_DIR/write_manifest.py" "$RUNTIME_DIR/compare_manifest.py" \
    "$RUNTIME_DIR/check_options.py" "$RUNTIME_DIR/build_runtime_test_lock.py" \
    "$RUNTIME_DIR/write_crates.py" "$RUNTIME_DIR/build_runtime_test_crates.py" \
    "$RUNTIME_DIR/write_licenses.py" "$RUNTIME_DIR/build_runtime_test_licenses.py" \
    || fail "a python helper does not parse"
  echo "  ok: shell and python syntax"
}

test_build_refusals() {
  echo "build_runtime_test: build_runtime.sh refusals"
  refuses "unknown argument" "$SCRIPT" --unknown-argument \
    || fail "an unknown argument was accepted"
  refuses "no mode given" "$SCRIPT" || fail "a run with no mode was accepted"
  make_path_without_docker "$WORK/no-docker-bin"
  refuses "docker is not installed" env PATH="$WORK/no-docker-bin" bash "$SCRIPT" --container \
    || fail "a host without docker was accepted"
  refuses "no manifest to verify against" env FERMIX_RUNTIME_OUT="$WORK/empty-out" "$SCRIPT" --verify \
    || fail "--verify was accepted with no manifest"

  # A tree whose lock file has been taken away. fetch_source.sh comes too, so
  # the refusal is about the lock file and not about the library.
  mkdir -p "$WORK/no-lock/packaging/runtime/patches" "$WORK/no-lock/packaging/docker"
  cp "$SCRIPT" "$RUNTIME_DIR/fetch_source.sh" "$WORK/no-lock/packaging/runtime/"
  cp "$DOCKERFILE" "$WORK/no-lock/packaging/docker/"
  refuses "no lock file" bash "$WORK/no-lock/packaging/runtime/build_runtime.sh" --print-key \
    || fail "a tree with no lock file was accepted"
  echo "  refused: unknown argument, no mode, no docker, no manifest, no lock file"
}

# A runtime output directory package_sources.sh accepts: a manifest of this lock
# file recording the digests of the crate and licence files beside it.
make_runtime_out() {
  local dir="$1"
  mkdir -p "$dir"
  echo "the crates" > "$dir/runtime-crates.tar.gz"
  echo "the licences" > "$dir/runtime-licenses.tar.gz"
  echo '{"schema_version": 1}' > "$dir/runtime-licenses.json"
  jq -n --slurpfile lock "$LOCK" \
    --arg crates "$(sha256sum < "$dir/runtime-crates.tar.gz" | cut -d' ' -f1)" \
    --arg texts "$(sha256sum < "$dir/runtime-licenses.tar.gz" | cut -d' ' -f1)" \
    --arg index "$(sha256sum < "$dir/runtime-licenses.json" | cut -d' ' -f1)" \
    '{lock: $lock[0], archives: {"runtime-crates.tar.gz": $crates,
      "runtime-licenses.tar.gz": $texts, "runtime-licenses.json": $index}}' \
    > "$dir/runtime-manifest.json"
}

test_other_refusals() {
  echo "build_runtime_test: smoke and source archive refusals"
  local runtime="$WORK/runtime-ok"
  make_runtime_out "$runtime"
  refuses "unknown argument" "$SMOKE" --unknown-argument \
    || fail "smoke_runtime.sh accepted an unknown argument"
  refuses "no version given" "$SOURCES" || fail "package_sources.sh accepted a run with no version"
  local bad
  for bad in 1.2.3-1 1.2.3:4 1.2 nightly; do
    refuses "the version '$bad'" "$SOURCES" "$bad" \
      || fail "package_sources.sh accepted the version '$bad'"
  done
  refuses "unknown argument" "$SOURCES" 1.2.3 --unknown-argument \
    || fail "package_sources.sh accepted an unknown argument"
  refuses "no download cache" "$SOURCES" 1.2.3 --no-fetch --sources "$WORK/missing-cache" \
    --runtime "$runtime" --out "$WORK/out" \
    || fail "package_sources.sh --no-fetch accepted a missing cache directory"
  mkdir -p "$WORK/empty-cache"
  refuses "is not cached and --no-fetch" "$SOURCES" 1.2.3 --no-fetch --sources "$WORK/empty-cache" \
    --runtime "$runtime" --out "$WORK/out" \
    || fail "package_sources.sh --no-fetch accepted an empty cache"

  # The crates and licences come from a runtime built from this lock file, as its
  # manifest records them.
  refuses "no runtime manifest" "$SOURCES" 1.2.3 --no-fetch --sources "$WORK/empty-cache" \
    --runtime "$WORK/missing-runtime" --out "$WORK/out" \
    || fail "package_sources.sh accepted a runtime directory with no manifest"
  make_runtime_out "$WORK/runtime-other"
  jq '.lock.source_date_epoch = 0' "$WORK/runtime-other/runtime-manifest.json" > "$WORK/other.json"
  mv "$WORK/other.json" "$WORK/runtime-other/runtime-manifest.json"
  refuses "built from another lock file" "$SOURCES" 1.2.3 --no-fetch --sources "$WORK/empty-cache" \
    --runtime "$WORK/runtime-other" --out "$WORK/out" \
    || fail "package_sources.sh accepted crates of a runtime built from another lock file"
  local name
  for name in runtime-crates.tar.gz runtime-licenses.tar.gz runtime-licenses.json; do
    make_runtime_out "$WORK/runtime-tampered-$name"
    echo "other bytes" > "$WORK/runtime-tampered-$name/$name"
    refuses "$name is not the one the runtime manifest records" "$SOURCES" 1.2.3 --no-fetch \
      --sources "$WORK/empty-cache" --runtime "$WORK/runtime-tampered-$name" --out "$WORK/out" \
      || fail "package_sources.sh accepted a $name the manifest does not record"
  done

  # Files of the right names whose bytes are not the locked ones.
  mkdir -p "$WORK/wrong-cache"
  jq -r '.components[].archive' "$LOCK" | while read -r archive; do
    echo "not the locked tarball" > "$WORK/wrong-cache/$archive"
  done
  refuses "the lock file says" "$SOURCES" 1.2.3 --no-fetch --sources "$WORK/wrong-cache" \
    --runtime "$runtime" --out "$WORK/out" \
    || fail "package_sources.sh accepted tarballs that are not the locked ones"
  echo "  refused: smoke argument, source archive version, cache and digests, runtime crates and licences"
}

# Driven over file:// so the test never touches the network. Each refusal runs
# in its own subshell, because fail() exits.
# The key names what the build produced, so every input that changes the
# produced tree changes it: the lock file, the Dockerfile, each patch, and the
# scripts that compile and describe it.
test_cache_key() {
  echo "build_runtime_test: the cache key"
  local tree="$WORK/key-tree" before input
  mkdir -p "$tree/packaging/docker"
  cp -r "$RUNTIME_DIR" "$tree/packaging/"
  cp "$DOCKERFILE" "$tree/packaging/docker/"
  for input in packaging/runtime/RUNTIME.lock.json packaging/docker/Dockerfile.runtime \
               packaging/runtime/patches/appstream-no-man-pages.patch \
               packaging/runtime/build_runtime.sh packaging/runtime/write_manifest.py \
               packaging/runtime/drop_unreachable.sh packaging/runtime/write_crates.py \
               packaging/runtime/write_licenses.py; do
    before="$(bash "$tree/packaging/runtime/build_runtime.sh" --print-key)"
    echo " " >> "$tree/$input"
    [ "$(bash "$tree/packaging/runtime/build_runtime.sh" --print-key)" != "$before" ] \
      || fail "the cache key does not cover $input"
  done
  echo "  ok: the key covers the lock file, the Dockerfile, the patches and the build scripts"
}

test_fetcher() {
  echo "build_runtime_test: the shared fetcher"
  (
    fail() { echo "fetch: $*" >&2; exit 1; }
    log() { :; }
    FETCH_MAX_ATTEMPTS=2
    FETCH_RETRY_SECONDS=0
    # shellcheck source=desktop/packaging/runtime/fetch_source.sh
    . "$RUNTIME_DIR/fetch_source.sh"

    served="$WORK/served.tar.gz"
    echo "the locked bytes" > "$served"
    digest="$(sha256sum "$served" | cut -d' ' -f1)"
    fetch_source served "file://$served" "$WORK/got.tar.gz" "$digest"
    cmp -s "$served" "$WORK/got.tar.gz" || exit 1

    zeros="0000000000000000000000000000000000000000000000000000000000000000"
    if ( fetch_source wrong "file://$served" "$WORK/wrong.tar.gz" "$zeros" ) 2>/dev/null; then
      exit 1
    fi
    [ ! -e "$WORK/wrong.tar.gz" ] && [ ! -e "$WORK/wrong.tar.gz.partial" ] || exit 1
    if ( fetch_source missing "file://$WORK/not-here.tar.gz" "$WORK/missing.tar.gz" "$digest" ) 2>/dev/null; then
      exit 1
    fi
    if ( fetch_source insecure "http://example.invalid/x.tar.gz" "$WORK/http.tar.gz" "$digest" ) 2>/dev/null; then
      exit 1
    fi
    # A cached file is taken without fetching, and only while its bytes are right.
    ensure_source cached "file://$WORK/not-here.tar.gz" "$WORK/got.tar.gz" "$digest"
    echo "tampered" > "$WORK/got.tar.gz"
    if ( ensure_source cached "file://$WORK/not-here.tar.gz" "$WORK/got.tar.gz" "$digest" ) 2>/dev/null; then
      exit 1
    fi
  ) || fail "the shared fetcher does not hold"
  echo "  ok: fetches, refuses a wrong digest, bounded retries, http and a tampered cache"
}

test_lock() {
  echo "build_runtime_test: the lock file"
  python3 "$RUNTIME_DIR/build_runtime_test_lock.py" "$LOCK" "$DOCKERFILE" "$ROOT_DIR/Cargo.lock" \
    || fail "the lock file does not hold"
}

test_options() {
  echo "build_runtime_test: the meson flags, against upstream"
  python3 "$RUNTIME_DIR/check_options.py" \
    --lock "$LOCK" \
    --sources "${FERMIX_RUNTIME_SOURCES:-${XDG_CACHE_HOME:-$HOME/.cache}/fermix-desktop-runtime/sources}" \
    || fail "a meson flag is not an option upstream declares"
}

test_dockerfile() {
  echo "build_runtime_test: the container it declares"
  [ -f "$DOCKERFILE" ] || fail "no runtime container at $DOCKERFILE"
  local base pinned needed forbidden
  base="$(jq -r '.base_image' "$LOCK")"
  grep -qx "FROM $base" "$DOCKERFILE" || fail "the Dockerfile is not FROM the lock file's $base"
  grep -q 'sha256sum -c -' "$DOCKERFILE" || fail "a tool is fetched without a digest check"
  if grep -q 'sh.rustup.rs' "$DOCKERFILE"; then
    fail "rustup is installed by an unpinned script"
  fi
  for pinned in MESON_VERSION NINJA_VERSION PATCHELF_VERSION SASSC_VERSION LIBSASS_VERSION \
                RUSTUP_VERSION RUST_VERSION CARGO_C_VERSION; do
    grep -qE "ARG $pinned=[0-9]+\.[0-9]+\.[0-9]+" "$DOCKERFILE" || fail "$pinned is not pinned"
  done
  # The host half's headers, and the build tools the components stop without.
  for needed in mesa-libGL-devel mesa-libEGL-devel libX11-devel libxkbcommon-devel dbus-devel \
                zlib-devel libcurl-devel libyaml-devel pulseaudio-libs-devel gcc-c++ \
                flex bison gperf itstool shared-mime-info jq sassc patchelf cargo-c; do
    grep -q "$needed" "$DOCKERFILE" || fail "the runtime container does not install $needed"
  done
  for forbidden in gtk4-devel pango-devel cairo-devel harfbuzz-devel gdk-pixbuf2-devel \
                   libadwaita-devel librsvg2-devel gstreamer1-devel glycin; do
    if grep -q "$forbidden" "$DOCKERFILE"; then
      fail "the runtime container installs $forbidden"
    fi
  done
  # Host headers drag in two private ones, which the image removes and asserts gone.
  grep -q 'rpm -e --nodeps glib2-devel libxml2-devel' "$DOCKERFILE" \
    || fail "the container keeps the host glib2-devel and libxml2-devel"
  grep -q 'private-pc-check' "$DOCKERFILE" \
    || fail "the container does not assert that no private .pc file is on the host"
  echo "  ok: pinned base and tools, host headers present, no private toolkit headers"
}

# The build fetches only what the lock file pins: meson may not download a
# subproject, and cargo only vendors what the tarball's Cargo.lock names.
test_fetches() {
  echo "build_runtime_test: what the build may fetch"
  grep -q -- '--wrap-mode=nodownload' "$SCRIPT" || fail "meson may download an unpinned subproject"
  grep -q 'cargo vendor --locked' "$SCRIPT" || fail "cargo may fetch crates Cargo.lock does not pin"
  # shellcheck disable=SC2016 # the script's own text, not an expansion
  grep -qF 'component_field "$name" cargo.lock' "$SCRIPT" \
    || fail "crates are vendored for a component the lock file does not name"
  grep -qx '  export CARGO_NET_OFFLINE=true' "$SCRIPT" || fail "cargo may reach the network while compiling"
  # The standard licence texts come from a pinned tarball, held to its digest.
  grep -q "ensure_source \"\$(lock_get '.license_text_source.name')\"" "$SCRIPT" \
    || fail "the build does not fetch the licence text source by its digest"
  grep -q '(.components\[\], .license_text_source)' "$SOURCES" \
    || fail "the source archive does not carry the licence text source"
  echo "  ok: no meson download, crates vendored at Cargo.lock, cargo offline while compiling,"
  echo "      the licence text source fetched by digest and carried in the source archive"
}

# librsvg's meson asks rustc which system libraries its static half needs and
# passes them to the link through a Python set, so their order, and with it the
# order of DT_NEEDED in librsvg-2.so, followed the hash seed. The patch keeps
# rustc's order; the seeds here would scatter it again.
test_patches() {
  echo "build_runtime_test: the patches"
  local cache archive source_dir dir="$WORK/patched" patch seed order
  cache="${FERMIX_RUNTIME_SOURCES:-${XDG_CACHE_HOME:-$HOME/.cache}/fermix-desktop-runtime/sources}"
  archive="$(jq -r '.components[] | select(.name == "librsvg") | .archive' "$LOCK")"
  source_dir="$(jq -r '.components[] | select(.name == "librsvg") | .source_dir' "$LOCK")"
  if [ ! -f "$cache/$archive" ]; then
    echo "  skipped: $archive is not in $cache"
    return 0
  fi
  mkdir -p "$dir"
  tar -xf "$cache/$archive" -C "$dir" "$source_dir/meson/query-rustc.py"
  for patch in $(jq -r '.components[] | select(.name == "librsvg") | .patches[]' "$LOCK"); do
    patch -s -p1 -d "$dir/$source_dir" < "$RUNTIME_DIR/patches/$patch" || fail "$patch does not apply"
  done
  for seed in 1 2 3 4 5 6 7 8; do
    # shellcheck disable=SC2016 # Python, not shell
    order="$(PYTHONHASHSEED="$seed" PYTHONDONTWRITEBYTECODE=1 python3 -c '
import importlib.util, sys
spec = importlib.util.spec_from_file_location("query_rustc", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
module.retrieve_native_static_libs_from_output(
    "note: native-static-libs: -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc")
' "$dir/$source_dir/meson/query-rustc.py")"
    [ "$order" = "gcc_s util rt pthread m dl c" ] \
      || fail "librsvg links rustc's native libraries in hash seed order: $order"
  done
  echo "  ok: librsvg links rustc's native libraries in rustc's order under any hash seed"
}

# GNU ar on the base image stamps each member with its build time unless told
# D, and CMake calls it without; abseil's archives differed between two builds.
test_archives() {
  echo "build_runtime_test: static archives"
  local lang
  for lang in C CXX; do
    grep -qF -- "-DCMAKE_${lang}_ARCHIVE_CREATE=<CMAKE_AR> qcD <TARGET>" "$SCRIPT" \
      || fail "CMake writes $lang archives with member timestamps"
    grep -qF -- "-DCMAKE_${lang}_ARCHIVE_APPEND=<CMAKE_AR> qD <TARGET>" "$SCRIPT" \
      || fail "CMake appends to $lang archives with member timestamps"
    grep -qF -- "-DCMAKE_${lang}_ARCHIVE_FINISH=<CMAKE_RANLIB> -D <TARGET>" "$SCRIPT" \
      || fail "CMake indexes $lang archives with a timestamp"
  done
  echo "  ok: CMake writes and indexes static archives in deterministic mode"
}

test_boundary() {
  echo "build_runtime_test: the boundary gate"
  command -v gcc >/dev/null 2>&1 || fail "gcc is needed to build the gate's test objects"
  command -v readelf >/dev/null 2>&1 || fail "readelf is needed by the boundary gate"
  bash "$RUNTIME_DIR/build_runtime_test_boundary.sh" "$GATE" "$LOCK" "$WORK/boundary" \
    || fail "the boundary gate does not hold"
}

test_crates() {
  echo "build_runtime_test: the list of Rust crates compiled in"
  PYTHONDONTWRITEBYTECODE=1 python3 "$RUNTIME_DIR/build_runtime_test_crates.py" \
    "$RUNTIME_DIR/write_crates.py" "$WORK/crates" \
    || fail "write_crates.py does not hold"
}

test_licenses() {
  echo "build_runtime_test: the licence files of everything compiled in"
  PYTHONDONTWRITEBYTECODE=1 python3 "$RUNTIME_DIR/build_runtime_test_licenses.py" \
    "$RUNTIME_DIR/write_licenses.py" "$WORK/licenses" \
    || fail "write_licenses.py does not hold"
}

test_unreachable() {
  echo "build_runtime_test: dropping what nothing needs"
  command -v gcc >/dev/null 2>&1 || fail "gcc is needed to build the test objects"
  bash "$RUNTIME_DIR/build_runtime_test_unreachable.sh" "$DROP" "$WORK/unreachable" \
    || fail "drop_unreachable.sh does not hold"
}

main() {
  test_syntax
  test_build_refusals
  test_cache_key
  test_other_refusals
  test_fetcher
  test_lock
  test_options
  test_dockerfile
  test_fetches
  test_patches
  test_archives
  test_boundary
  test_unreachable
  test_crates
  test_licenses
  echo "build_runtime_test: every check passed"
}

main "$@"
