#!/usr/bin/env bash
#
# Build the one fermix-desktop deb and the one fermix-desktop rpm from one tree,
# in the build container (amendment sections 3, 4, 6.4, 8).
#
# One run produces: the release binary with its build id compiled in and its
# RUNPATH pointing at the toolkit beside it, one staging tree holding the engine
# from a verified archive, the private toolkit runtime, and the window, one nFPM
# configuration rendered from the checked-in template, and one package per
# family from that single configuration, so the two families cannot drift into
# two file lists.
#
# Five things this script refuses to assume.
#
#   * **A version is a version.** Neither package may carry a Debian revision or
#     an rpm epoch, so a version containing `-` or `:` is refused before
#     anything is built. A desktop-only rebuild is `X.Y.Z+N`, which both dpkg
#     and rpm order after `X.Y.Z`. A prerelease tag therefore produces no
#     packages at all, which is a consequence worth stating rather than
#     discovering.
#   * **A build id is a fact about this build.** It is compiled into the binary
#     and written into the manifest the package installs from one value, so the
#     window's self-skew check compares an id with an id rather than with a
#     guess.
#   * **An engine has no standing until it is verified.** The package carries
#     the engine, so there is no engine without a pin: an unpinned tree is
#     refused rather than producing a package with a hole in it. `--engine`
#     exists for a developer building against a locally built archive and is
#     refused under CI.
#   * **The declaration is only worth what a machine can check it against.**
#     nFPM generates no dependencies, so after the packages exist their declared
#     relations are held equal to the NEEDED entries of every ELF the package
#     carries, in both directions.
#   * **The desktop files are only worth what the desktop can read.**
#     desktop-file-validate and appstreamcli run over the staged copies, not
#     over the sources, so what is checked is what is installed.
#
# Usage:
#   scripts/build_packages.sh <version> <arch> [--container] [--engine <archive>]
#     <version>    X.Y.Z or X.Y.Z+N, no revision and no epoch
#     <arch>       amd64 or arm64, and it must be the machine this runs on:
#                  there is no cross build
#     --container  run the whole thing inside packaging/docker/Dockerfile.build
#     --engine     build against a locally built, unsigned engine archive
#                  instead of the pinned one. Refused when CI is true
#
# Environment (build facts supplied by whatever is driving the build, never
# settings):
#   FERMIX_DESKTOP_BUILD_ID       the build id. Defaults to the source commit,
#                                 with -dirty appended for an uncommitted tree
#   FERMIX_DESKTOP_SOURCE_COMMIT  the commit being built. Defaults to git's answer
#   FERMIX_ENGINE_DOWNLOAD_DIR    where scripts/fetch_engine.sh put the pinned
#                                 archives. Defaults to packaging/out/engine
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_DIR="$ROOT_DIR/App/Fermix"
PACKAGING_DIR="$ROOT_DIR/packaging"
OUT_DIR="$PACKAGING_DIR/out"
# The deliverables, kept apart from the working directories beside them so a
# glob for "the packages" cannot also match a staging tree or a report.
PACKAGES_DIR="$OUT_DIR/packages"
STAGE_DIR="$OUT_DIR/stage"
ENGINE_STAGING="$OUT_DIR/engine-staging"
MAINTAINER_DIR="$OUT_DIR/maintainer"
RUNTIME_DIR="$OUT_DIR/runtime"
TEMPLATE="$PACKAGING_DIR/nfpm-fermix-desktop.yaml.tmpl"
RENDERED="$OUT_DIR/nfpm-fermix-desktop.yaml"
IMAGE="${FERMIX_BUILD_IMAGE:-fermix-desktop-build}"
APP_ID="io.tezra.Fermix"
PREFIX="/usr/lib/fermix-desktop"

# Only the container re-exec below uses these, and they define functions
# rather than doing anything, so the in-container run sources them harmlessly.
# shellcheck source=scripts/container_cache.sh
source "$ROOT_DIR/scripts/container_cache.sh"
# shellcheck source=scripts/crate_gates.sh
source "$ROOT_DIR/scripts/crate_gates.sh"

# The icon sizes the package installs, and the one place they are listed. 512 is
# carried because the artwork master is 1024 px and GNOME draws the application
# icon well above 256 in the overview and in settings on a scaled display.
ICON_SIZES=(16 22 24 32 48 64 128 256 512)

# The relations the package declares for a reason that is not a NEEDED entry,
# so that the dependency gate does not refuse them as unjustified.
#
# The bundled GSettings backend is a client of the host's dconf service and of
# the host's schemas, and on a session with no Settings portal it is the only
# path the window has to the theme (amendment section 2.1). The private
# fontconfig is built with -Dbaseconfig-dir=/etc/fonts, so it reads the host's
# configuration and the host's fonts, and the package that owns that directory
# is a requirement rather than an assumption.
#
# The font families and the keyring stay on this list although the template
# declares them as recommendations rather than dependencies, so the exemption
# still holds if a later decision makes one of them hard. The gate reads the
# built package's Depends and Requires, where a recommendation does not appear,
# so carrying them here changes nothing today. The keyring is the one that most
# needs the safety net: it is recommended precisely because a KDE or KeePassXC
# machine already owns org.freedesktop.secrets, and if anyone ever promotes it
# to a dependency this gate should not be what stops them finding out.
#
# The OpenGL entry points are the other kind of exemption, and the more
# dangerous one. GTK reaches OpenGL through libepoxy, which does not link
# libGL.so.1, libEGL.so.1 or libGLESv2.so.2 at all: it dlopens whichever it
# needs at the moment it needs it. No ELF header in the package records them, so
# a boundary derived from NEEDED entries alone does not mention them, and a host
# without them gets a window that aborts rather than a package that refuses to
# install. This build found that the hard way, under Xvfb, where GTK took the
# GLES path and died on a missing libGLESv2.so.2.
#
# They are declared for that reason and named here so the gate does not refuse
# them as unjustified. A name on this list is a claim that a person checked, and
# `strings` on the shipped libepoxy is where these three were checked.
#
# libepoxy names six libraries, and the other three are deliberately NOT
# declared. Recording why, because "we did not think of it" and "we decided
# against it" look identical in a list that simply omits them:
#
#   * libGLESv1_CM.so.1 (libgles1 / libGLESv1_CM.so.1()(64bit)) is OpenGL ES 1.
#     epoxy loads it only for a GLES 1 context, and nothing in this application
#     asks for one.
#   * libOpenGL.so.0 (libopengl0) and libGLX are epoxy's glvnd-preferred
#     alternatives. Where they are absent it falls back to libGL.so.1, which is
#     declared, so a machine that satisfies the declaration always has a path.
#     libGLX is the sharper case: epoxy's string says libGLX.so.1, and no
#     distribution ships that soname at all (glvnd's is libGLX.so.0), so that
#     dlopen fails everywhere and always falls back.
#
# libGLdispatch.so.0, libgbm.so.1 and libdrm.so.2 are not declared either, and
# for a different reason: nothing in this package names them. They are
# dependencies of the packages that provide libGL and of the host's Mesa driver,
# so declaring them would be declaring somebody else's dependency, which is the
# over-declaration this gate exists to refuse.
#
# The last entry on each list is the only one that is not a library at all. The
# engine writes a channel bot key through the host's secret service by shelling
# out to `secret-tool` -- System.find_executable, so a PATH lookup -- and a
# desktop running gnome-keyring still refuses to save a key when that program
# is absent. No ELF names it, so this gate's own question cannot reach it and
# the relation has to be justified here by hand.
#
# The two families name it differently and the difference was measured rather
# than assumed: deb splits the executables into libsecret-tools, while both
# almalinux:9 and fedora:44 ship /usr/bin/secret-tool inside libsecret itself,
# with no tools subpackage to depend on. The rpm side therefore declares the
# file capability, which names the file we actually need and cannot be
# satisfied by the i686 build of libsecret that both repositories also carry.
NOT_A_LIBRARY_DEB=(
  dconf-service gsettings-desktop-schemas fontconfig-config fonts-dejavu-core
  libgl1 libegl1 libgles2 libsecret-tools gnome-keyring
)
NOT_A_LIBRARY_RPM=(
  dconf gsettings-desktop-schemas fontconfig dejavu-sans-fonts
  "libGL.so.1()(64bit)" "libEGL.so.1()(64bit)" "libGLESv2.so.2()(64bit)"
  /usr/bin/secret-tool gnome-keyring
)

# The other half of the same problem, and the half that bit us.
#
# scripts/package_dependencies.py reads every private ELF for sonames it names
# as plain strings without linking them, which is what a dlopen looks like from
# the outside, and refuses any that is neither declared nor named here. So a
# component that starts loading a new library by name stops the build with its
# name in the sentence, instead of installing cleanly and failing on the first
# machine that lacks it.
#
# These three are named because somebody looked at them and decided against
# declaring them; the reasons are above. A name here is a decision, not a
# silencer, and the difference is that a decision has a reason written beside it.
DLOPEN_CONSIDERED=(
  libGLESv1_CM.so.1 libGLX.so.1 libOpenGL.so.0
)

CONTAINER=0
VERSION=""
BASE_VERSION=""
ARCH=""
RPM_ARCH=""
LOCAL_ENGINE=""
ENGINE_TARGET=""
SOURCE_COMMIT=""
BUILD_ID=""
PIN_STATE=""
BINARY=""
DEB=""
RPM=""
RELATIONS=""
ENGINE_UNPACKED=""

fail() {
  echo "build_packages: $*" >&2
  exit 1
}

step() {
  echo
  echo "build_packages: == $*"
}

# Everything this run writes under packaging/out is this run's. A previous run's
# stage that survived into this one is how a file nobody built ends up in a
# package, so each is emptied on the way in, and the two that hold nothing worth
# keeping afterwards are emptied on the way out too.
clean_workspace() {
  rm -rf "$STAGE_DIR" "$ENGINE_STAGING" "$MAINTAINER_DIR"
}
trap 'rm -rf "$ENGINE_STAGING"' EXIT

# ---- arguments -------------------------------------------------------------

parse_arguments() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --container)
        CONTAINER=1
        shift
        ;;
      --engine)
        [ "$#" -ge 2 ] || fail "--engine needs the path to an engine archive"
        LOCAL_ENGINE="$2"
        shift 2
        ;;
      -*) fail "unknown argument: $1" ;;
      *)
        if [ -z "$VERSION" ]; then
          VERSION="$1"
        elif [ -z "$ARCH" ]; then
          ARCH="$1"
        else
          fail "unexpected argument: $1"
        fi
        shift
        ;;
    esac
  done

  [ -n "$VERSION" ] ||
    fail "usage: build_packages.sh <version> <arch> [--container] [--engine <archive>]"
  [ -n "$ARCH" ] ||
    fail "usage: build_packages.sh <version> <arch> [--container] [--engine <archive>]"
}

check_version() {
  case "$VERSION" in
    *-*) fail "the version '$VERSION' carries a Debian revision, and neither package may: a desktop-only fix is published as X.Y.Z+N" ;;
    *:*) fail "the version '$VERSION' carries an rpm epoch, and neither package may: a desktop-only fix is published as X.Y.Z+N" ;;
  esac
  [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(\+[0-9]+)?$ ]] ||
    fail "the version '$VERSION' is neither X.Y.Z nor X.Y.Z+N"
  BASE_VERSION="${VERSION%%+*}"
}

check_architecture() {
  case "$ARCH" in
    amd64)
      RPM_ARCH="x86_64"
      ENGINE_TARGET="linux_x86_64"
      ;;
    arm64)
      RPM_ARCH="aarch64"
      ENGINE_TARGET="linux_aarch64"
      ;;
    *) fail "the architecture '$ARCH' is neither amd64 nor arm64" ;;
  esac
}

# ---- the record, before the machine ----------------------------------------
#
# Every check that reads a file rather than the host runs here, before the
# re-exec into the container, so a release that names the wrong version is
# refused in a second rather than after a build.

check_the_version_everything_agrees_on() {
  local crate_version metainfo_versions

  crate_version="$(
    awk -F'"' '/^version = "/ { print $2; exit }' "$CRATE_DIR/Cargo.toml"
  )"
  [ "$crate_version" = "$VERSION" ] ||
    fail "the crate is version $crate_version and $VERSION was asked for; the tag, Cargo.toml and the metainfo name one version"

  metainfo_versions="$(
    grep -c "<release version=\"$VERSION\"" \
      "$PACKAGING_DIR/$APP_ID.metainfo.xml" || true
  )"
  [ "$metainfo_versions" = "1" ] ||
    fail "the metainfo does not carry exactly one release entry for $VERSION; the tag, Cargo.toml and the metainfo name one version"
}

check_the_engine_is_decided() {
  # shellcheck source=scripts/engine_pin.sh
  source "$ROOT_DIR/scripts/engine_pin.sh"
  PIN_STATE="$(engine_pin_state "$ROOT_DIR/engine/PIN.json")" || exit 1

  if [ -n "$LOCAL_ENGINE" ]; then
    [ "${CI:-}" != "true" ] ||
      fail "--engine builds against an unsigned local archive and CI is true; a release rail builds against the pinned engine"
    [ -f "$LOCAL_ENGINE" ] || fail "no engine archive at $LOCAL_ENGINE"
    return 0
  fi

  [ "$PIN_STATE" = "pinned" ] ||
    fail "engine/PIN.json is unpinned, and this package carries the engine; pin an engine release, or pass --engine <archive> to build against a local one"

  local pinned_version
  pinned_version="$(engine_pin_field "$ROOT_DIR/engine/PIN.json" engine_version)"
  [ "$pinned_version" = "$BASE_VERSION" ] ||
    fail "engine/PIN.json pins engine $pinned_version and this is $VERSION; the desktop version is the engine version, with an optional +N for a desktop-only rebuild"
}

# ---- the container ---------------------------------------------------------

reexec_into_container() {
  command -v docker >/dev/null 2>&1 || fail "docker is not installed"
  [ -f "$PACKAGING_DIR/docker/Dockerfile.build" ] ||
    fail "no build container at packaging/docker/Dockerfile.build"

  local runtime_image
  runtime_image="$("$ROOT_DIR/scripts/runtime_image.sh" --arch "$ARCH")" ||
    fail "there is no private runtime to build the image from"

  if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
    echo "build_packages: building $IMAGE from $runtime_image"
    docker build -f "$PACKAGING_DIR/docker/Dockerfile.build" -t "$IMAGE" \
      --build-arg "RUNTIME_IMAGE=$runtime_image" \
      --build-arg "HOST_CAPABILITIES=$(host_capabilities "$ROOT_DIR/scripts/host_relations.map")" \
      --build-arg "RUNTIME_CACHE_KEY=$(tr -d '[:space:]' < "$RUNTIME_DIR/cache-key" 2>/dev/null || true)" \
      "$PACKAGING_DIR/docker"
  fi

  # The tray's D-Bus path, before anything is built. It runs out here rather
  # than inside the build container because it starts a container per arm, and
  # a container starting containers is a complication this build does not need.
  #
  # It is a gate and not a convenience: the tray talks to a panel over D-Bus,
  # and nothing else in this build puts a byte on a bus. The one bug it has
  # already caught -- a reply body that was an array where D-Bus requires a
  # tuple -- passed every pure-value test in the crate and would have shipped
  # as a menu that never drew.
  echo "build_packages: == the tray, against a private session bus"
  "$ROOT_DIR/scripts/tray_smoke.sh" ||
    fail "the tray's D-Bus path did not hold"

  # The cargo cache lives in the same volume the gate run uses, so a release
  # build after a gate run compiles the crate rather than the world. A host
  # short of memory bounds the build's parallelism with CARGO_BUILD_JOBS; it is
  # passed through only when set, because cargo refuses an empty value.
  container_cache_prepare "$IMAGE" "${FERMIX_CARGO_CACHE:-fermix-desktop-cargo}" ||
    fail "could not hand the cargo cache volume to $(id -u):$(id -g)"

  local user_flags=()
  mapfile -t user_flags < <(container_user_flags)

  # The commit is resolved out here rather than in the container. Inside it the
  # repository is a bind mount whose owner git does not recognise as the user
  # running it, so `git rev-parse` refuses with "dubious ownership" and the
  # build stops for a reason that has nothing to do with packaging.
  local source_commit
  source_commit="${FERMIX_DESKTOP_SOURCE_COMMIT:-}"
  [ -n "$source_commit" ] ||
    source_commit="$(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || true)"

  local engine_arguments=()
  [ -z "$LOCAL_ENGINE" ] || engine_arguments=(--engine /engine/"$(basename "$LOCAL_ENGINE")")

  local engine_mount=()
  [ -z "$LOCAL_ENGINE" ] ||
    engine_mount=(-v "$(cd "$(dirname "$LOCAL_ENGINE")" && pwd):/engine:ro")

  exec docker run --rm --init \
    "${user_flags[@]}" \
    -v "$ROOT_DIR:/workspace" \
    -v "${FERMIX_CARGO_CACHE:-fermix-desktop-cargo}:/cache" \
    "${engine_mount[@]}" \
    -e HOME=/tmp \
    -e CARGO_HOME=/cache/cargo \
    -e CARGO_TARGET_DIR=/cache/target \
    -e CARGO_TERM_COLOR=always \
    -e FERMIX_DESKTOP_TEST_ROOT=/tmp/fermix-desktop-tests \
    -e "FERMIX_DESKTOP_BUILD_ID=${FERMIX_DESKTOP_BUILD_ID:-}" \
    -e "FERMIX_DESKTOP_SOURCE_COMMIT=$source_commit" \
    -e "CI=${CI:-}" \
    ${CARGO_BUILD_JOBS:+-e "CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS"} \
    -w /workspace \
    "$IMAGE" scripts/build_packages.sh "$VERSION" "$ARCH" "${engine_arguments[@]}"
}

# ---- the host this must be -------------------------------------------------

check_the_host() {
  [ "$(uname -s)" = "Linux" ] ||
    fail "the packages are built on Linux, in the build container; pass --container"

  local tool
  for tool in cargo nfpm rpm desktop-file-validate appstreamcli python3 xvfb-run tar; do
    command -v "$tool" >/dev/null 2>&1 ||
      fail "$tool is not installed; this runs in the build container, so pass --container"
  done

  local host_arch
  case "$(uname -m)" in
    x86_64) host_arch="amd64" ;;
    aarch64 | arm64) host_arch="arm64" ;;
    *) fail "this build does not run on $(uname -m)" ;;
  esac
  [ "$host_arch" = "$ARCH" ] ||
    fail "this is an $host_arch machine and $ARCH was asked for; there is no cross build, so each architecture is built on its own runner"

  [ -d "$PREFIX/lib/pkgconfig" ] ||
    fail "there is no private toolkit runtime at $PREFIX; this runs in the build container, so pass --container"
}

resolve_build_identity() {
  SOURCE_COMMIT="${FERMIX_DESKTOP_SOURCE_COMMIT:-}"
  if [ -z "$SOURCE_COMMIT" ]; then
    SOURCE_COMMIT="$(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || true)"
  fi
  [ -n "$SOURCE_COMMIT" ] ||
    fail "no source commit: this tree has no commit and FERMIX_DESKTOP_SOURCE_COMMIT names none"

  local dirty=""
  if [ -n "$(git -C "$ROOT_DIR" status --porcelain 2>/dev/null || true)" ]; then
    dirty="-dirty"
  fi

  BUILD_ID="${FERMIX_DESKTOP_BUILD_ID:-}"
  if [ -z "$BUILD_ID" ]; then
    BUILD_ID="${SOURCE_COMMIT:0:12}$dirty"
  fi
}

# ---- the gates that read files ---------------------------------------------

run_record_gates() {
  step "the gates that are shell"
  "$ROOT_DIR/scripts/check_app_identity.sh"
  "$ROOT_DIR/scripts/verify_contract.sh"
  "$ROOT_DIR/scripts/check_vendor_marks.sh"
  "$ROOT_DIR/scripts/check_copyright.sh"
  "$ROOT_DIR/scripts/check_no_network.sh"
}

run_crate_gates() {
  step "format, lint, test"
  (
    cd "$CRATE_DIR"
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings

    # The one case the design allows LD_LIBRARY_PATH, and only here. A test
    # binary sits in target/debug, where $ORIGIN/../lib is not the private
    # prefix, so it cannot find the toolkit the way the installed application
    # does. This is a property of where cargo puts a test binary and not of the
    # product: the shipped ELF is built with its own RUNPATH below, and
    # scripts/check_private_runtime.sh reads that ELF and refuses it if it
    # needs an environment variable to load (amendment section 4.3).
    export LD_LIBRARY_PATH="$PREFIX/lib"

    # Each test binary links the whole toolkit, and linking them all at once is
    # what exhausts a container the linker is killed in.
    local expected log status
    expected="$(crate_test_binary_count "$CRATE_DIR")"
    mkdir -p "$OUT_DIR"
    log="$OUT_DIR/cargo-test.log"
    cargo test --no-fail-fast --jobs 2 2>&1 | tee "$log"
    status="${PIPESTATUS[0]}"
    [ "$status" = "0" ] || fail "the crate's tests failed"
    check_every_test_binary_reported "$log" "$expected" ||
      fail "the crate's test run did not run every test binary it built"

    FERMIX_GTK_TESTS=1 xvfb-run "${XVFB_ARGUMENTS[@]}" \
      cargo test --no-fail-fast --jobs 2 --test widgets
  )
}

# ---- the binary ------------------------------------------------------------

build_the_binary() {
  step "the release binary, against the private toolkit"
  (
    cd "$CRATE_DIR"
    # The application finds its libraries through its own RUNPATH and through
    # no environment variable, so the link line is where that is decided. New
    # dtags make it a RUNPATH rather than an RPATH, which is what lets a person
    # replace a bundled library in place.
    PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig" \
    PATH="$PREFIX/bin:$PATH" \
    RUSTFLAGS="-C link-arg=-Wl,-rpath,\$ORIGIN/../lib -C link-arg=-Wl,--enable-new-dtags" \
    FERMIX_DESKTOP_BUILD_ID="$BUILD_ID" \
      cargo build --release --jobs 2 --bin fermix-desktop
  )

  BINARY="${CARGO_TARGET_DIR:-$CRATE_DIR/target}/release/fermix-desktop"
  [ -x "$BINARY" ] || fail "the release build produced no binary at $BINARY"
}

# The binary says what it is, and both halves are checked. The build id in
# particular is the one the manifest below carries, and the window compares the
# two: a package that ships one and not the other is silently unknown rather
# than wrong, which is the failure this assertion exists to stop.
check_the_binary_says_what_it_is() {
  local stamped
  # The installed binary resolves its libraries through $ORIGIN/../lib, and
  # this one is still in the cargo target directory, where that does not apply.
  # It is the one place the design allows LD_LIBRARY_PATH, and it is a property
  # of the build container rather than of the product: check_private_runtime.sh
  # asserts that the staged ELF needs no such variable.
  stamped="$(LD_LIBRARY_PATH="$PREFIX/lib" "$BINARY" --version)" ||
    fail "the built binary cannot say what it is"
  [ "$stamped" = "fermix-desktop $VERSION (build $BUILD_ID)" ] ||
    fail "the binary reports '$stamped', and this build is fermix-desktop $VERSION (build $BUILD_ID)"
  echo "  the binary reports: $stamped"
}

# ---- the staging tree ------------------------------------------------------

stage_the_engine() {
  step "the engine, verified and unpacked"
  local unpacked="$ENGINE_STAGING/$ENGINE_TARGET/fermix_app_engine"

  if [ -n "$LOCAL_ENGINE" ]; then
    "$ROOT_DIR/scripts/verify_engine.sh" \
      --local-archive "$LOCAL_ENGINE" --dev \
      --target "$ENGINE_TARGET" --staging "$ENGINE_STAGING"
  else
    local downloads="${FERMIX_ENGINE_DOWNLOAD_DIR:-$OUT_DIR/engine}"
    [ -d "$downloads" ] ||
      fail "no engine archives at $downloads; run scripts/fetch_engine.sh first"
    "$ROOT_DIR/scripts/verify_engine.sh" \
      "$ROOT_DIR/engine/PIN.json" "$downloads" \
      --target "$ENGINE_TARGET" --staging "$ENGINE_STAGING"
  fi

  [ -d "$unpacked/tree/usr" ] ||
    fail "the verified engine archive carries no tree/usr"
  [ -f "$unpacked/nfpm-contents.yaml" ] ||
    fail "the verified engine archive carries no nfpm-contents.yaml, and that file is the author of the engine's file list"
  ENGINE_UNPACKED="$unpacked"
  echo "  verified and unpacked from $(basename "${LOCAL_ENGINE:-the pinned archive}")"

  # The engine's files are deliberately not copied into the stage. nFPM reads
  # them out of the verified staging directory through the `src:` paths the
  # splice rewrites, so there is one copy of a 150 MB tree on disk rather than
  # two, and the bytes that go into the package are the bytes the verifier
  # checked rather than a copy of them.

  "$ROOT_DIR/scripts/assemble_maintainer.sh" \
    --engine "$unpacked/maintainer" \
    --desktop "$PACKAGING_DIR/scripts" \
    --out "$MAINTAINER_DIR"
  "$ROOT_DIR/scripts/assemble_maintainer.sh" \
    --engine "$unpacked/maintainer" \
    --desktop "$PACKAGING_DIR/scripts" \
    --out "$MAINTAINER_DIR" --check
}

# Every entry in the private runtime archive that is not the toolkit, printed
# one to a line. Empty output is a well-formed archive.
#
# The tarball is rooted at usr/lib/fermix-desktop and nothing else, and one that
# grew a second root would put files in this package that no gate below ever
# looks at. The two parent directory entries are the exception, and only as
# directories: that is how tar records a tree, and an archive without them
# cannot create its own path. Anything else outside the prefix is a refusal, a
# plain file at `usr/lib/` included.
#
# The name is read as the sixth field rather than the last, because tar prints a
# symlink as `name -> target` and the question is where an entry lands, not
# where it points.
runtime_archive_strays() {
  local archive="$1"
  tar -tvf "$archive" | awk '
    $6 ~ /^usr\/lib\/fermix-desktop(\/|$)/ { next }
    $1 ~ /^d/ && ($6 == "usr/" || $6 == "usr/lib/") { next }
    { print $6 }
  '
}

stage_the_runtime() {
  step "the private toolkit runtime"
  local archive="$RUNTIME_DIR/runtime-$ARCH.tar"
  local manifest="$RUNTIME_DIR/runtime-manifest.json"

  [ -f "$archive" ] ||
    fail "no private runtime at $archive; run packaging/runtime/build_runtime.sh --container"
  [ -f "$manifest" ] || fail "no runtime manifest at $manifest"

  # Asked before anything is unpacked, not after: a stray entry found after the
  # extraction has already been written into the staging tree, and the refusal
  # would be a report about a file this build had just created.
  local stray
  stray="$(runtime_archive_strays "$archive")"
  [ -z "$stray" ] ||
    fail "the private runtime archive carries entries outside $PREFIX: $(echo "$stray" | head -3 | tr '\n' ' ')"

  tar -xf "$archive" -C "$STAGE_DIR"

  # The tarball is not taken on trust. A cache key names the lock file and not
  # the export, a tarball's own digest moves with readdir order, and an image
  # tag is a label somebody can point at anything; the manifest is the only one
  # of the four that describes the bytes. So every entry is checked, and so is
  # the absence of anything the manifest does not list.
  python3 "$ROOT_DIR/scripts/check_runtime_tree.py" \
    --stage "$STAGE_DIR" --manifest "$manifest" --archive "$archive" --prefix "$PREFIX" ||
    fail "the staged toolkit is not the tree runtime-manifest.json describes"

  install -d -m 0755 "$STAGE_DIR/usr/share/doc/fermix-desktop"
  install -m 0644 "$manifest" \
    "$STAGE_DIR/usr/share/doc/fermix-desktop/runtime-manifest.json"
  echo "  $(find "$STAGE_DIR$PREFIX" -type f | wc -l) files under $PREFIX"
}

stage_the_window() {
  step "the window, and what the desktop reads"

  install -d -m 0755 "$STAGE_DIR$PREFIX/bin"
  install -m 0755 "$BINARY" "$STAGE_DIR$PREFIX/bin/fermix-desktop"

  install -d -m 0755 \
    "$STAGE_DIR/usr/share/applications" \
    "$STAGE_DIR/usr/share/metainfo" \
    "$STAGE_DIR/usr/share/dbus-1/services" \
    "$STAGE_DIR/usr/lib/systemd/user" \
    "$STAGE_DIR/usr/share/fermix-desktop"

  install -m 0644 "$PACKAGING_DIR/$APP_ID.desktop" \
    "$STAGE_DIR/usr/share/applications/$APP_ID.desktop"
  install -m 0644 "$PACKAGING_DIR/$APP_ID.metainfo.xml" \
    "$STAGE_DIR/usr/share/metainfo/$APP_ID.metainfo.xml"
  install -m 0644 "$PACKAGING_DIR/dbus/$APP_ID.service" \
    "$STAGE_DIR/usr/share/dbus-1/services/$APP_ID.service"
  install -m 0644 "$PACKAGING_DIR/systemd/app-$APP_ID.service" \
    "$STAGE_DIR/usr/lib/systemd/user/app-$APP_ID.service"
  install -m 0644 "$PACKAGING_DIR/copyright" \
    "$STAGE_DIR/usr/share/doc/fermix-desktop/copyright"
}

# The icon set is checked in, regenerated from the vendored artwork masters by
# scripts/render_icons.sh. It is verified here rather than regenerated, so the
# bytes a reviewer looked at are the bytes that ship and a release does not
# depend on whichever image toolkit a build host happens to carry.
stage_the_icons() {
  local size source_png dimensions
  for size in "${ICON_SIZES[@]}"; do
    source_png="$PACKAGING_DIR/icons/hicolor/${size}x${size}/apps/$APP_ID.png"
    [ -f "$source_png" ] ||
      fail "the icon set has no ${size}x${size} raster; run scripts/render_icons.sh"
    dimensions="$(png_dimensions "$source_png")"
    [ "$dimensions" = "$size $size" ] ||
      fail "the ${size}x${size} icon is ${dimensions// /x} pixels; run scripts/render_icons.sh"
    install -d -m 0755 "$STAGE_DIR/usr/share/icons/hicolor/${size}x${size}/apps"
    install -m 0644 "$source_png" \
      "$STAGE_DIR/usr/share/icons/hicolor/${size}x${size}/apps/$APP_ID.png"
  done

  install -d -m 0755 \
    "$STAGE_DIR/usr/share/icons/hicolor/scalable/apps" \
    "$STAGE_DIR/usr/share/icons/hicolor/symbolic/apps"
  install -m 0644 "$PACKAGING_DIR/icons/hicolor/scalable/apps/$APP_ID.svg" \
    "$STAGE_DIR/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
  install -m 0644 "$PACKAGING_DIR/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg" \
    "$STAGE_DIR/usr/share/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"

  # The application compiles its own copy of these two into its gresource and
  # resolves THAT in-process, for its window and its about dialog. If the two
  # copies differ, the launcher draws one icon and the running application draws
  # another, which reads as a stale icon cache for as long as it takes someone
  # to think of comparing them. They are copied by render_icons.sh, so a
  # difference here means one side was edited by hand.
  local packaged resourced
  for packaged in "scalable/apps/$APP_ID.svg" "symbolic/apps/$APP_ID-symbolic.svg"; do
    resourced="$CRATE_DIR/resources/icons/$(basename "$packaged")"
    [ -f "$resourced" ] ||
      fail "the application has no gresource copy of $(basename "$packaged"); run scripts/render_icons.sh"
    cmp -s "$PACKAGING_DIR/icons/hicolor/$packaged" "$resourced" ||
      fail "the packaged $(basename "$packaged") and the application's gresource copy differ, so the launcher and the running application would draw different icons; run scripts/render_icons.sh"
  done

  echo "  ${#ICON_SIZES[@]} rasters, the scalable icon and its symbolic pair"
  echo "  the gresource copies match the packaged SVGs"
}

png_dimensions() {
  python3 - "$1" <<'PY'
import struct
import sys

with open(sys.argv[1], "rb") as handle:
    header = handle.read(24)
if header[:8] != b"\x89PNG\r\n\x1a\n":
    sys.exit("not a PNG")
print(*struct.unpack(">II", header[16:24]))
PY
}

# The manifest the window reads to find out whether it is older than the
# application on disk. Its build id is the one compiled into the binary above.
# The engine artifact's own build id, out of the manifest the verifier unpacked.
engine_manifest_build_id() {
  python3 -c 'import json,sys; print(json.load(open(sys.argv[1],encoding="utf-8")).get("identity",{}).get("build_id","unknown"))' "$1"
}

# The digest of the engine's own tree, which is the only thing about a
# development engine that actually moves.
#
# A build id taken from a dirty working tree is `dev-<commit>-dirty`, and both
# halves stay the same while the code changes underneath, so two different
# engines carry one id. That is the same shape as a runtime cache key naming the
# lock file rather than the export: an identifier that describes an input rather
# than the bytes. This one describes the bytes.
engine_manifest_tree_sha256() {
  python3 -c 'import json,sys; print(json.load(open(sys.argv[1],encoding="utf-8")).get("tree_sha256","unknown"))' "$1"
}

stage_the_build_identity() {
  # What this package was made from, not only what it is. "Which toolkit is
  # in this build" has no answer otherwise: the runtime cache key names the lock
  # file the toolkit was compiled from, and the engine's own build id names the
  # artifact that was verified and unpacked. Both are read from the inputs, so
  # neither can describe a build that did not happen.
  local runtime_key="unknown" engine_build_id="unknown" runtime_manifest="unknown"
  [ ! -f "$RUNTIME_DIR/cache-key" ] ||
    runtime_key="$(tr -d '[:space:]' < "$RUNTIME_DIR/cache-key")"
  # The key says which lock file; this says which tree. Two exports of one lock
  # file can differ, so the digest of the manifest is what names the bytes this
  # package actually carries.
  [ ! -f "$RUNTIME_DIR/runtime-manifest.json" ] ||
    runtime_manifest="$(sha256sum "$RUNTIME_DIR/runtime-manifest.json" | cut -d" " -f1)"
  local engine_tree="unknown"
  if [ -n "$ENGINE_UNPACKED" ] && [ -f "$ENGINE_UNPACKED/engine-manifest.json" ]; then
    engine_build_id="$(engine_manifest_build_id "$ENGINE_UNPACKED/engine-manifest.json")"
    engine_tree="$(engine_manifest_tree_sha256 "$ENGINE_UNPACKED/engine-manifest.json")"
  fi

  python3 - "$OUT_DIR/build.json" "$VERSION" "$BUILD_ID" "$SOURCE_COMMIT" \
    "$runtime_key" "$engine_build_id" "$runtime_manifest" "$engine_tree" <<'PY'
import json
import os
import sys

(
    path,
    version,
    build_id,
    source_commit,
    runtime_key,
    engine_build_id,
    runtime_manifest,
    engine_tree,
) = sys.argv[1:9]
os.makedirs(os.path.dirname(path), exist_ok=True)
with open(path, "w", encoding="utf-8") as handle:
    json.dump(
        {
            "schema_version": 2,
            "product_version": version,
            "build_id": build_id,
            "source_commit": source_commit,
            "runtime_cache_key": runtime_key,
            "runtime_manifest_sha256": runtime_manifest,
            "engine_build_id": engine_build_id,
            "engine_tree_sha256": engine_tree,
        },
        handle,
        indent=2,
    )
    handle.write("\n")
PY
  install -m 0644 "$OUT_DIR/build.json" \
    "$STAGE_DIR/usr/share/fermix-desktop/build.json"
  echo "  build.json"
}

# ---- what the desktop reads ------------------------------------------------

validate_the_desktop_files() {
  step "desktop-file-validate and appstreamcli, over the staged copies"
  desktop-file-validate "$STAGE_DIR/usr/share/applications/$APP_ID.desktop"
  echo "  the desktop entry validates"

  # `--no-net` because a release must not depend on a third party answering: the
  # validator would otherwise fetch the screenshot images and the OARS
  # vocabulary. Straight through, with nothing accepted: an error or a warning
  # fails the build.
  appstreamcli validate --no-net --explain \
    "$STAGE_DIR/usr/share/metainfo/$APP_ID.metainfo.xml" 2>&1 |
    tee "$OUT_DIR/appstreamcli.txt"
  echo "  the metainfo validates"
}

# ---- the gates that read the staged tree -----------------------------------

check_the_staged_tree() {
  step "the private runtime, as staged"
  "$ROOT_DIR/scripts/check_private_runtime.sh" "$STAGE_DIR" \
    --engine-tree "$ENGINE_UNPACKED/tree"
}

# ---- the packages ----------------------------------------------------------

render_the_configuration() {
  step "the nFPM configuration"
  [ -f "$TEMPLATE" ] || fail "no template at $TEMPLATE"
  python3 - "$TEMPLATE" "$RENDERED" \
    "ARCH=$ARCH" \
    "VERSION=$VERSION" \
    "STAGE=$STAGE_DIR" \
    "POSTINSTALL=$MAINTAINER_DIR/postinstall.sh" \
    "POSTREMOVE=$MAINTAINER_DIR/postremove.sh" <<'PY'
import re
import sys

template, out, *pairs = sys.argv[1:]
values = dict(pair.split("=", 1) for pair in pairs)

with open(template, encoding="utf-8") as handle:
    body = handle.read()

missing = set()


def fill(match):
    name = match.group(1)
    if name not in values:
        missing.add(name)
        return match.group(0)
    return values[name]


body = re.sub(r"\{\{([A-Z_]+)\}\}", fill, body)
if missing:
    sys.exit(
        "build_packages: the template carries placeholders nothing fills: "
        + ", ".join(sorted(missing))
    )
if "{{" in body:
    sys.exit("build_packages: the rendered configuration still carries a placeholder")

with open(out, "w", encoding="utf-8") as handle:
    handle.write(body)
PY

  # The engine's half of the contents block is authored by the verified archive
  # and spliced in here, never restated in the template. An engine release that
  # adds a packaged file therefore needs no edit in this repository at all,
  # which is the whole reason one package carries both halves.
  python3 "$ROOT_DIR/scripts/splice_engine_contents.py" \
    --rendered "$RENDERED" \
    --out "$RENDERED" \
    --engine-contents "$ENGINE_UNPACKED/nfpm-contents.yaml" \
    --staging "$ENGINE_UNPACKED"
  echo "  $RENDERED"
}

build_the_packages() {
  step "nfpm, one configuration, two families"
  mkdir -p "$PACKAGES_DIR"
  rm -f "$PACKAGES_DIR"/*.deb "$PACKAGES_DIR"/*.rpm
  nfpm package --config "$RENDERED" --packager deb --target "$PACKAGES_DIR"
  nfpm package --config "$RENDERED" --packager rpm --target "$PACKAGES_DIR"
}

# The names nFPM chose, read from the run rather than assumed.
#
# A `+` in the version is legal in both families and orders correctly in both,
# and whether nFPM keeps it in the file name it chooses is a fact about nFPM.
# The expected name is asserted so that a future version quietly reshaping it is
# a failed build rather than an asset nothing can download by name.
find_the_packages() {
  DEB="$PACKAGES_DIR/fermix-desktop_${VERSION}_${ARCH}.deb"
  RPM="$PACKAGES_DIR/fermix-desktop-${VERSION}-1.${RPM_ARCH}.rpm"

  [ -f "$DEB" ] ||
    fail "nfpm produced no $(basename "$DEB"); it wrote $(cd "$PACKAGES_DIR" && echo ./*.deb)"
  [ -f "$RPM" ] ||
    fail "nfpm produced no $(basename "$RPM"); it wrote $(cd "$PACKAGES_DIR" && echo ./*.rpm)"
  echo "  $(basename "$DEB")"
  echo "  $(basename "$RPM")"
}

# ---- declared against derived ----------------------------------------------

read_back_the_relations() {
  RELATIONS="$OUT_DIR/relations.json"

  python3 - "$RELATIONS" "$DEB" "$RPM" <<'PY'
"""What the two built packages actually declare, from the packages themselves.

Read back rather than taken from the configuration: the configuration is what
was asked for and the package is what was produced, and the whole point of the
check below is to compare a declaration with a machine's own answer.
"""
import json
import subprocess
import sys

out_path, deb, rpm = sys.argv[1:4]


def deb_field(name):
    result = subprocess.run(
        ["dpkg-deb", "--field", deb, name],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        sys.exit(f"build_packages: dpkg-deb could not read {name}: {result.stderr.strip()}")
    return [piece.strip() for piece in result.stdout.split(",") if piece.strip()]


def rpm_query(flag):
    result = subprocess.run(
        ["rpm", "-qp", flag, rpm],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        sys.exit(f"build_packages: rpm could not read {flag}: {result.stderr.strip()}")
    return [line.strip() for line in result.stdout.splitlines() if line.strip()]


relations = {
    "deb": {
        "depends": deb_field("Depends"),
        "provides": deb_field("Provides"),
        "conflicts": deb_field("Conflicts"),
        "replaces": deb_field("Replaces"),
        "obsoletes": [],
    },
    "rpm": {
        "depends": rpm_query("--requires"),
        "provides": rpm_query("--provides"),
        "conflicts": rpm_query("--conflicts"),
        "replaces": [],
        "obsoletes": rpm_query("--obsoletes"),
    },
}

with open(out_path, "w", encoding="utf-8") as handle:
    json.dump(relations, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
}

# Every file the engine archive authors is in both packages, at its mode. The
# splice refuses an entry it cannot place; this is the other end of the pipe,
# where an entry the packager dropped or a mode the umask reshaped would show.
check_the_engine_landed() {
  step "the engine's own files, in both packages"
  python3 "$ROOT_DIR/scripts/check_engine_contents.py" \
    --engine-contents "$ENGINE_UNPACKED/nfpm-contents.yaml" \
    --rendered "$RENDERED" \
    --deb "$DEB" \
    --rpm "$RPM"
}

# The runtime's own files are all in the package, and every absolute prefix path
# compiled into a shipped object exists.
#
# This reads the built package rather than the staged tree on purpose. The
# staged tree was never wrong: staging copies the runtime whole. libexec/ was
# lost at the nFPM step, because the template named the trees to carry and that
# one was not among them, so a gate reading the stage would have passed every
# build that shipped the defect.
check_the_runtime_landed() {
  step "the runtime's own files, in the package"
  "$ROOT_DIR/scripts/check_runtime_complete.sh" \
    --package "$DEB" \
    --runtime "$RUNTIME_DIR/runtime-$ARCH.tar"
}

check_the_relations() {
  local argument
  local arguments=()
  for argument in "${NOT_A_LIBRARY_DEB[@]}"; do
    arguments+=(--not-a-library-deb "$argument")
  done
  for argument in "${NOT_A_LIBRARY_RPM[@]}"; do
    arguments+=(--not-a-library-rpm "$argument")
  done
  for argument in "${DLOPEN_CONSIDERED[@]}"; do
    arguments+=(--dlopen-considered "$argument")
  done

  step "the declared relations against what the package's ELFs need"
  python3 "$ROOT_DIR/scripts/package_dependencies.py" \
    --version "$VERSION" \
    --stage "$STAGE_DIR" \
    --engine-tree "$ENGINE_UNPACKED/tree" \
    --relations "$RELATIONS" \
    --host-map "$ROOT_DIR/scripts/host_relations.map" \
    --lock "$PACKAGING_DIR/runtime/RUNTIME.lock.json" \
    --prefix "$PREFIX" \
    "${arguments[@]}"
}

# ---- what came out ---------------------------------------------------------

report() {
  step "the packages"
  ls -l "$DEB" "$RPM"
  echo "  installed size: $(du -sh "$STAGE_DIR" | cut -f1)"
  echo
  echo "build_packages: fermix-desktop $VERSION for $ARCH, both families, from one configuration"
}

# ---- the run ---------------------------------------------------------------

main() {
  parse_arguments "$@"
  check_version
  check_architecture
  check_the_version_everything_agrees_on
  check_the_engine_is_decided

  [ "$CONTAINER" = "0" ] || reexec_into_container

  check_the_host
  resolve_build_identity

  echo "build_packages: fermix-desktop $VERSION ($ARCH)"
  echo "  build id      $BUILD_ID"
  echo "  source commit $SOURCE_COMMIT"
  echo "  engine        ${LOCAL_ENGINE:-the pinned release} (pin: $PIN_STATE)"

  run_record_gates
  run_crate_gates

  build_the_binary
  check_the_binary_says_what_it_is

  step "the staging tree"
  clean_workspace
  install -d -m 0755 "$STAGE_DIR/usr"
  stage_the_engine
  stage_the_runtime
  stage_the_window
  stage_the_icons
  stage_the_build_identity

  validate_the_desktop_files
  check_the_staged_tree

  render_the_configuration
  build_the_packages

  find_the_packages
  check_the_engine_landed
  check_the_runtime_landed
  read_back_the_relations
  check_the_relations

  report
}

# Sourced rather than run means a test wants one function out of this file; run
# means build the packages.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  main "$@"
fi
