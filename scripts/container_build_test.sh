#!/usr/bin/env bash
#
# Exercise container_build.sh's refusals without building anything.
#
# The gate it wraps takes minutes; its argument handling and its preconditions
# take milliseconds, and those are the parts that break silently. Docker itself
# is never invoked here: the script is driven with a stand-in on PATH.
#
# It also holds the build container to what the gates need from it. That list
# changed with the private toolkit: the image no longer installs a host GTK,
# because a host toolkit on this image would be a second answer to the question
# the private prefix already answers, and it now carries the toolkit itself from
# an image named by an argument.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/container_build.sh"
DOCKERFILE="$ROOT_DIR/packaging/docker/Dockerfile.build"
RUNTIME_IMAGE="$ROOT_DIR/scripts/runtime_image.sh"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/container-build-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fail() {
  echo "container_build_test: $*" >&2
  exit 1
}

expect_sentence() {
  local what="$1" needle="$2"
  shift 2
  local output
  output="$("$@" 2>&1 || true)"
  case "$output" in
    *"$needle"*) echo "  refused: $what" ;;
    *) fail "$what was not refused with '$needle'; it said: $output" ;;
  esac
}

expect_success() {
  local what="$1"
  shift
  "$@" >/dev/null 2>&1 || fail "$what was refused"
  echo "  accepted: $what"
}

echo "container_build_test: the scripts parse"
bash -n "$SCRIPT" || fail "the script does not parse"
bash -n "$RUNTIME_IMAGE" || fail "runtime_image.sh does not parse"
echo "  ok: shell syntax"

echo "container_build_test: refusals"

if "$SCRIPT" --unknown-argument >/dev/null 2>&1; then
  fail "an unknown argument was accepted"
fi
echo "  refused: an unknown argument"

if "$SCRIPT" --arch >/dev/null 2>&1; then
  fail "--arch with no architecture was accepted"
fi
echo "  refused: --arch with no architecture named"

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

echo "container_build_test: the toolkit the image is built from"

if "$RUNTIME_IMAGE" >/dev/null 2>&1; then
  fail "runtime_image.sh with no architecture was accepted"
fi
echo "  refused: naming no architecture"

if "$RUNTIME_IMAGE" --arch riscv64 >/dev/null 2>&1; then
  fail "an architecture neither family builds for was accepted"
fi
echo "  refused: an architecture neither family builds for"

# An environment that names a published image imports nothing and answers at
# once, which is the CI path and the one a developer takes after a pull.
NAMED="$(FERMIX_RUNTIME_IMAGE=ghcr.io/tezra-io/fermix-desktop-runtime:key-amd64 \
  "$RUNTIME_IMAGE" --arch amd64)"
[ "$NAMED" = "ghcr.io/tezra-io/fermix-desktop-runtime:key-amd64" ] ||
  fail "a named runtime image was not used as given; it answered '$NAMED'"
echo "  accepted: a published runtime image, named and used as given"

# With no published image and no locally built tree, it refuses rather than
# printing a name that names nothing.
mkdir -p "$WORK/no-runtime/scripts" "$WORK/no-runtime/packaging/out/runtime"
cp "$RUNTIME_IMAGE" "$WORK/no-runtime/scripts/"
if bash "$WORK/no-runtime/scripts/runtime_image.sh" --arch amd64 >/dev/null 2>&1; then
  fail "a repository with no private toolkit was accepted"
fi
echo "  refused: no published image and no locally built toolkit"

# A tree with no cache key beside it names an image whose contents nobody can
# identify, which is the thing the key exists to stop.
printf 'not really a tar\n' > "$WORK/no-runtime/packaging/out/runtime/runtime-dev-amd64.tar"
if bash "$WORK/no-runtime/scripts/runtime_image.sh" --arch amd64 >/dev/null 2>&1; then
  fail "a toolkit tree with no cache key was accepted"
fi
echo "  refused: a toolkit tree that does not say which lock file produced it"

# The tag is not the tree. A cache key covers the lock file, the Dockerfile and
# the patches, not the export, so two exports of one key can differ — and one
# did, by a single file, which is how an image imported before that file was
# added kept a tag saying it had it. So the tree is asked what it is.
grep -q 'image_cache_key' "$RUNTIME_IMAGE" ||
  fail "runtime_image.sh believes the tag instead of reading the tree's own identity"
grep -q 'identity.json' "$RUNTIME_IMAGE" ||
  fail "runtime_image.sh does not read the toolkit's identity file"
grep -q 'is newer than' "$RUNTIME_IMAGE" ||
  fail "runtime_image.sh reuses an image although the local tar has changed under it"

# The same question again inside the image build, because the toolkit arrives
# there by a second route: FERMIX_RUNTIME_IMAGE names a published image that
# this script never imports and therefore never checks.
grep -q 'ARG RUNTIME_CACHE_KEY' "$DOCKERFILE" ||
  fail "the build container does not check which lock file the toolkit it copied in came from"
grep -q 'identity.json' "$DOCKERFILE" ||
  fail "the build container does not read the toolkit's own identity"
for script in "$SCRIPT" "$ROOT_DIR/scripts/build_packages.sh"; do
  grep -q 'RUNTIME_CACHE_KEY=' "$script" ||
    fail "$(basename "$script") builds the image without telling it which key to expect"
done
echo "  ok: an image is reused only when the tree in it says it is the right tree"

# ---------------------------------------------------------------------------
# The staged toolkit against its manifest
# ---------------------------------------------------------------------------

echo "container_build_test: the staged toolkit is the tree its manifest describes"

TREE_CHECK="$ROOT_DIR/scripts/check_runtime_tree.py"
python3 -c "import ast,sys; ast.parse(open(sys.argv[1]).read())" "$TREE_CHECK" ||
  fail "check_runtime_tree.py does not parse"

TREE="$WORK/tree"
mkdir -p "$TREE/usr/lib/fermix-desktop/lib" "$TREE/usr/lib/fermix-desktop/share"
printf 'payload\n' > "$TREE/usr/lib/fermix-desktop/lib/libpretend.so.1"
ln -sf libpretend.so.1 "$TREE/usr/lib/fermix-desktop/lib/libpretend.so"
python3 - "$TREE" "$WORK/manifest.json" <<'PY'
import hashlib
import json
import os
import sys

tree, out = sys.argv[1:3]
prefix = "/usr/lib/fermix-desktop"
files = []
for directory, _, names in os.walk(os.path.join(tree, prefix.lstrip("/"))):
    for name in sorted(names):
        path = os.path.join(directory, name)
        shown = "/" + os.path.relpath(path, tree)
        if os.path.islink(path):
            files.append({"path": shown, "kind": "link", "target": os.readlink(path)})
        else:
            with open(path, "rb") as handle:
                files.append(
                    {
                        "path": shown,
                        "kind": "file",
                        "sha256": hashlib.sha256(handle.read()).hexdigest(),
                    }
                )
with open(out, "w", encoding="utf-8") as handle:
    json.dump({"schema_version": 1, "prefix": prefix, "files": files}, handle)
PY

run_tree_check() {
  python3 "$TREE_CHECK" --stage "$1" --manifest "$WORK/manifest.json"
}

expect_success "a staged tree that is exactly its manifest" run_tree_check "$TREE"

# A file that goes missing, which is the export that shipped without one.
GONE="$WORK/gone"
cp -a "$TREE" "$GONE"
rm "$GONE/usr/lib/fermix-desktop/lib/libpretend.so.1"
expect_sentence "a tree missing a file its manifest lists" \
  "is in the manifest and not in the staged tree" run_tree_check "$GONE"

# A file that arrives, which no per-entry loop would ever see.
EXTRA="$WORK/extra"
cp -a "$TREE" "$EXTRA"
printf 'uninvited\n' > "$EXTRA/usr/lib/fermix-desktop/lib/libintruder.so.0"
expect_sentence "a tree carrying a file its manifest does not list" \
  "is in the staged tree and not in" run_tree_check "$EXTRA"

# The same path, different bytes: the case a file count cannot catch.
CHANGED="$WORK/changed"
cp -a "$TREE" "$CHANGED"
printf 'different\n' > "$CHANGED/usr/lib/fermix-desktop/lib/libpretend.so.1"
expect_sentence "a file whose bytes are not the bytes the manifest recorded" \
  "and the manifest says" run_tree_check "$CHANGED"

# A symlink pointing somewhere else, which a hash of regular files misses.
RELINKED="$WORK/relinked"
cp -a "$TREE" "$RELINKED"
ln -sf /etc/passwd "$RELINKED/usr/lib/fermix-desktop/lib/libpretend.so"
expect_sentence "a symlink pointing somewhere the manifest does not say" \
  "points at" run_tree_check "$RELINKED"

# The tarball's own digest, which is a different question from the tree's. A
# non-reproducible archive around a correct tree passes every per-file check and
# still differs run to run; that happened, and only this catches it.
python3 - "$WORK/manifest.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    manifest = json.load(handle)
manifest["archives"] = {"pretend-runtime.tar": "0" * 64}
with open(sys.argv[1], "w", encoding="utf-8") as handle:
    json.dump(manifest, handle)
PY
printf 'not the archive the manifest names\n' > "$WORK/pretend-runtime.tar"
expect_sentence "an archive whose digest is not the one its manifest records" \
  "is not the one this manifest describes" \
  python3 "$TREE_CHECK" --stage "$TREE" --manifest "$WORK/manifest.json" \
    --archive "$WORK/pretend-runtime.tar"

printf 'nor this one\n' > "$WORK/unlisted.tar"
expect_sentence "an archive the manifest records no digest for" \
  "and not for unlisted.tar" \
  python3 "$TREE_CHECK" --stage "$TREE" --manifest "$WORK/manifest.json" \
    --archive "$WORK/unlisted.tar"

grep -q -- '--archive' "$ROOT_DIR/scripts/build_packages.sh" ||
  fail "build_packages.sh checks the staged tree and not the archive it came out of"
grep -q 'check_runtime_tree.py' "$ROOT_DIR/scripts/build_packages.sh" ||
  fail "build_packages.sh stages the toolkit without checking it against its manifest"
grep -q 'runtime_manifest_sha256' "$ROOT_DIR/scripts/build_packages.sh" ||
  fail "build.json does not record which toolkit tree the package carries"
# A development engine's build id is dev-<commit>-dirty and does not move while
# the code under it does, so two different engines share one id. The engine's
# own tree digest is the part that changes.
grep -q 'engine_tree_sha256' "$ROOT_DIR/scripts/build_packages.sh" ||
  fail "build.json does not record which engine tree the package carries, and a dirty build id cannot tell two of them apart"
echo "  ok: the build checks the staged toolkit and records which tree it was"

echo "container_build_test: the container it declares"
[ -f "$DOCKERFILE" ] || fail "no build container at $DOCKERFILE"

# Every one of these is a gate's dependency rather than a convenience. xauth is
# listed beside Xvfb because xvfb-run refuses without it and the refusal reads
# as a broken test rather than as a missing package. `rpm-build` and `dpkg` are
# listed because the release rail reads the built packages' own relations back,
# and nFPM writes no dependency it was not told to write.
for needed in gcc pkgconf-pkg-config gettext appstream desktop-file-utils \
              rpm-build dpkg xorg-x11-server-Xvfb xorg-x11-xauth \
              dbus-daemon dbus-x11 \
              python3 git tar; do
  grep -q "$needed" "$DOCKERFILE" || fail "the build container does not install $needed"
done
echo "  ok: every gate's dependency is installed"

# curl is needed by two steps and must NOT be installed by name: the base image
# carries curl-minimal, which provides /usr/bin/curl and conflicts with the full
# package, so asking for it fails the image build with a solver error.
grep -qE '^ +curl \\$' "$DOCKERFILE" &&
  fail "the build container asks dnf for curl, which conflicts with the base image's curl-minimal"
grep -qE '^ +libcurl \\$' "$DOCKERFILE" &&
  fail "the build container asks dnf for libcurl, which conflicts with the base image's libcurl-minimal"
echo "  ok: curl and libcurl come from the base image rather than from conflicting packages"

# The host half of the boundary. The image carries the private toolkit and none
# of the host's libraries, so without these a test binary dies at startup with a
# message that reads like a broken build rather than a missing host. The list
# below is the AlmaLinux package for each capability the nFPM template declares
# as an rpm requirement, so the two stay the same answer to the same question.
# The host half is not written into the Dockerfile at all: the scripts pass the
# rpm capability column of scripts/host_relations.map as HOST_CAPABILITIES, and
# the image resolves each capability to whatever provides it on the base. That
# is the property worth holding — one list, shared with the dependency gate —
# so what is checked here is that the seam exists, not that a second list
# matches the first.
grep -q 'ARG HOST_CAPABILITIES' "$DOCKERFILE" ||
  fail "the build container does not take the host boundary as an argument, so its host libraries are a second list that can drift from host_relations.map"
grep -q 'repoquery --whatprovides' "$DOCKERFILE" ||
  fail "the build container does not resolve the declared capabilities to packages on its own base"
grep -q 'rpm -q --whatprovides' "$DOCKERFILE" ||
  fail "the build container does not skip capabilities the base already satisfies, which is what makes libcurl-minimal fail the build"
for script in "$SCRIPT" "$ROOT_DIR/scripts/build_packages.sh"; do
  # shellcheck disable=SC2016  # the needle is the script's literal text
  grep -q 'HOST_CAPABILITIES=$(host_capabilities' "$script" ||
    fail "$(basename "$script") builds the image without passing the host boundary from host_relations.map"
done
# shellcheck source=scripts/container_cache.sh
source "$ROOT_DIR/scripts/container_cache.sh"
[ -n "$(host_capabilities "$ROOT_DIR/scripts/host_relations.map")" ] ||
  fail "host_capabilities reads no capability out of host_relations.map"
echo "  ok: the host half comes from host_relations.map, so it cannot drift"

# The .pc files for the host half. The toolkit's own pkg-config files name the
# host's libraries in their Requires lines, so without these the crate fails to
# configure with a message naming the toolkit rather than the missing header
# package.
for host_headers in zlib-devel libglvnd-devel libX11-devel libxcb-devel \
                    libXext-devel libXrender-devel; do
  grep -qE "^ +$host_headers \\\\$" "$DOCKERFILE" ||
    fail "the build container does not install $host_headers, so a toolkit .pc file cannot resolve"
done
grep -q 'pkg-config --libs --cflags gtk4 libadwaita-1 gio-2.0' "$DOCKERFILE" ||
  fail "the image does not resolve the toolkit's pkg-config files at build time, so a missing host .pc is found by the crate instead"
echo "  ok: the host .pc files are installed, and the image proves they resolve"

# The toolkit is carried, not installed from the host. A host GTK development
# package on this image would compile the crate against a toolkit that is not
# the one it ships with.
for forbidden in libgtk-4-dev libadwaita-1-dev gtk4-devel libadwaita-devel; do
  if grep -q "$forbidden" "$DOCKERFILE"; then
    fail "the build container installs $forbidden, and the package carries its own toolkit"
  fi
done
echo "  ok: no host toolkit, and the crate builds against the private prefix"

grep -q 'COPY --from=runtime /usr/lib/fermix-desktop /usr/lib/fermix-desktop' "$DOCKERFILE" ||
  fail "the build container does not copy the private toolkit in at its final path"
grep -q 'ARG RUNTIME_IMAGE' "$DOCKERFILE" ||
  fail "the runtime image the toolkit comes from is not an argument, so CI cannot name a published one"
# Both pkgconfig directories: wayland-protocols.pc is the one .pc file the
# toolkit installs under share/pkgconfig, and omitting that path fails the
# configure step with a message that reads like a missing library.
grep -q 'PKG_CONFIG_PATH=/usr/lib/fermix-desktop/lib/pkgconfig:/usr/lib/fermix-desktop/share/pkgconfig' "$DOCKERFILE" ||
  fail "the build container does not point pkg-config at both of the private prefix's pkgconfig directories"
echo "  ok: the private toolkit arrives from a named image, at its final path"

grep -qE '^FROM almalinux@sha256:[0-9a-f]{64}$' "$DOCKERFILE" ||
  fail "the base image is not AlmaLinux pinned by digest, and the glibc floor is the base image's"
grep -q 'sha256sum -c -' "$DOCKERFILE" || fail "nfpm is fetched without a digest check"
grep -qE 'ARG NFPM_VERSION=[0-9]+\.[0-9]+\.[0-9]+' "$DOCKERFILE" ||
  fail "nfpm is not pinned to a version"
grep -qE 'ARG RUST_VERSION=[0-9]+\.[0-9]+\.[0-9]+' "$DOCKERFILE" ||
  fail "the toolchain is not pinned to a version"
echo "  ok: the base, the toolchain and nfpm are pinned, and nfpm is checked by digest"

echo "container_build_test: the run it makes"

# Without --no-fail-fast cargo stops at the first test binary that fails and
# never runs the rest, so one broken test hides every other failure in the run.
grep -q 'cargo test --no-fail-fast' "$SCRIPT" ||
  fail "the gate run stops at the first failing test binary and hides the rest"
echo "  ok: every test binary runs, so one failure does not hide the others"

# ---------------------------------------------------------------------------
# The guard that proves the suite ran
# ---------------------------------------------------------------------------

echo "container_build_test: a green run that did not run"

# shellcheck source=scripts/crate_gates.sh
source "$ROOT_DIR/scripts/crate_gates.sh"

# This is the shape of the failure the guard exists for: --no-fail-fast carried
# on past three test binaries that died at load, so the log is all "ok" lines
# and nothing says that a hundred tests never ran.
cat > "$WORK/skipped.log" <<'LOG'
     Running tests/contract.rs (target/debug/deps/contract-1)
test result: ok. 18 passed; 0 failed; 0 ignored
     Running tests/models.rs (target/debug/deps/models-2)
target/debug/deps/models-2: error while loading shared libraries: libgtk-4.so.1: cannot open shared object file: No such file or directory
error: test failed, to rerun pass `--test models`
LOG
if check_every_test_binary_reported "$WORK/skipped.log" 2 >/dev/null 2>&1; then
  fail "a run where a test binary never started was reported as having run"
fi
echo "  refused: a run where a test binary died at load and printed no result"

# A doc-test result line is not one of the binaries cargo built, so it is
# subtracted rather than counted; this log is two binaries and a doc-test run.
cat > "$WORK/whole.log" <<'LOG'
test result: ok. 18 passed; 0 failed; 0 ignored
test result: ok. 91 passed; 0 failed; 0 ignored
   Doc-tests fermix_desktop
test result: ok. 0 passed; 0 failed; 0 ignored
LOG
check_every_test_binary_reported "$WORK/whole.log" 2 >/dev/null ||
  fail "a run where every test binary reported was refused"
echo "  accepted: a run where every binary cargo built reported a result"

check_every_test_binary_reported "$WORK/whole.log" "" >/dev/null 2>&1 &&
  fail "a run was accepted although cargo never said how many binaries it built"
echo "  refused: a count cargo could not supply"

# Both scripts run the tests, so both carry the guard and the same display.
for script in "$SCRIPT" "$ROOT_DIR/scripts/build_packages.sh"; do
  grep -q 'check_every_test_binary_reported' "$script" ||
    fail "$(basename "$script") does not prove the test suite actually ran"
  grep -q 'XVFB_ARGUMENTS' "$script" ||
    fail "$(basename "$script") does not take the display size from the one place both scripts read"
done
grep -q 'screen 0 1280x1024x24' "$ROOT_DIR/scripts/crate_gates.sh" ||
  fail "the shared display is Xvfb's 640x480 default, which GTK clamps a window to"
echo "  ok: both scripts prove the run happened, on a display big enough for the window"

# ---------------------------------------------------------------------------
# The image's own closing checks
# ---------------------------------------------------------------------------

echo "container_build_test: the image proves itself at build time"

# --modversion reads one .pc file and stops; it answers correctly on an image
# where nothing can be compiled. --libs --cflags resolves Requires, which is
# what a build does.
grep -q 'pkg-config --libs --cflags gtk4 libadwaita-1 gio-2.0' "$DOCKERFILE" ||
  fail "the image does not resolve the toolkit's pkg-config files transitively, so a missing host .pc is found by the crate instead"
# shellcheck disable=SC2016  # the needle is the Dockerfile's literal text
grep -q 'ldd "$library"' "$DOCKERFILE" ||
  fail "the image does not check that every private library's NEEDED entries resolve, so a missing host library is found by a test binary exiting 127"
echo "  ok: a missing host .pc and a missing host library both fail the image build"

echo "container_build_test: every refusal fired"
