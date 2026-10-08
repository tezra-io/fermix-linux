#!/usr/bin/env bash
#
# The half of desktop/scripts/build_packages.sh that runs in the runtime's build image, where the
# private toolkit is installed at /usr/lib/fermix-desktop exactly as the package ships it.
#
# build_packages.sh copies in this checkout's files at /workspace/src, the runtime's outputs at
# /workspace/inputs/runtime, the verified engine stage at /workspace/inputs/engine, and nFPM's
# tarball and the digests of everything at /workspace/inputs/tools. This script checks those
# digests; builds the window against the dev tree with RUNPATH $ORIGIN/../lib; stages the package
# tree with its documents, the window's manual page and the deb's lintian overrides; generates the
# copyright file; runs check_private_runtime.sh, check_copyright.sh and
# appstreamcli on the tree; runs nFPM for the deb and the rpm with the engine's maintainer scripts;
# and then reads both packages back: package_dependencies.py over their declared relations, their
# names, versions and relations, their maintainer scripts against the engine's, and their file
# lists against the tree. The packages, the copyright file, the nFPM configuration and the crate
# list land in /workspace/out with a SHA256SUMS.
#
# Usage: build_in_container.sh --version <version> --arch <amd64|arm64> --mtime <YYYY-MM-DDTHH:MM:SSZ>
set -euo pipefail
shopt -s inherit_errexit
umask 022

WORKSPACE=/workspace
DESKTOP="$WORKSPACE/src/desktop"
RUNTIME="$WORKSPACE/inputs/runtime"
ENGINE="$WORKSPACE/inputs/engine"
TOOLS="$WORKSPACE/inputs/tools"
ROOT="$WORKSPACE/stage/root"
BUILD="$WORKSPACE/build"
OUT="$WORKSPACE/out"
PREFIX=/usr/lib/fermix-desktop
LOCK="$DESKTOP/packaging/runtime/RUNTIME.lock.json"
MAP="$DESKTOP/packaging/host_relations.map"
COPYRIGHT="$ROOT/usr/share/doc/fermix-desktop/copyright"
LINTIAN_OVERRIDES=/usr/share/lintian/overrides/fermix-desktop
# The rpm lists the copyright file as its %license here instead (desktop/packaging/nfpm_config.py).
DOC_COPYRIGHT=/usr/share/doc/fermix-desktop/copyright
RPM_LICENSES=/usr/share/licenses/fermix-desktop
# The template's maintainer, which check_identity holds the deb to.
MAINTAINER="Tezra <hello@fermix.ai>"
export CARGO_HOME=/var/cache/fermix-package/cargo
export CARGO_TARGET_DIR=/var/cache/fermix-package/target

fail() {
  echo "build_in_container: $*" >&2
  exit 1
}

log() {
  echo "build_in_container: $*" >&2
}

check_inputs() {
  (cd "$WORKSPACE" && sha256sum --quiet --check "$TOOLS/source.sha256") ||
    fail "the source copied in is not the checkout's"
  (cd "$ENGINE" && sha256sum --quiet --check "$TOOLS/engine.sha256") ||
    fail "the engine stage copied in is not the verified one"
  (cd "$RUNTIME" && sha256sum --quiet --check SHA256SUMS) ||
    fail "the runtime copied in does not match its SHA256SUMS"
  (cd "$TOOLS" && sha256sum --quiet --check nfpm.sha256) || fail "nFPM's tarball is not the pinned one"
  log "the source, the engine stage, the runtime and nFPM match their digests"
}

# The window, built with the private toolkit's pkg-config files and glib-compile-resources. The
# pet's renderer finds EGL and libpng through pkg-config too, so no include or flag variable is set.
# Its symbols are stripped as well as its debug information, as a packaged binary's are.
build_window() {
  local arch="$1"
  tar -xf "$RUNTIME/runtime-dev-$arch.tar" -C /
  [ -f "$PREFIX/lib/pkgconfig/gtk4.pc" ] || fail "the dev runtime has no gtk4.pc"
  export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig"
  export PATH="$PREFIX/bin:$PATH"
  export CARGO_PROFILE_RELEASE_STRIP=symbols
  (cd "$DESKTOP" && cargo fetch --locked)
  # shellcheck disable=SC2016 # $ORIGIN is the dynamic linker's token
  (cd "$DESKTOP" && cargo rustc --locked --offline --release -p fermix-desktop --bin fermix-desktop \
    -- -C 'link-arg=-Wl,-rpath,$ORIGIN/../lib' -C link-arg=-Wl,--enable-new-dtags)
  [ -x "$CARGO_TARGET_DIR/release/fermix-desktop" ] || fail "cargo built no window"
  log "the window is built: $(du -h "$CARGO_TARGET_DIR/release/fermix-desktop" | cut -f1)"
}

list_window_crates() {
  local triple="$1"
  (cd "$DESKTOP" && cargo metadata --format-version 1 --locked --offline --filter-platform "$triple") \
    > "$BUILD/metadata.json"
  python3 "$DESKTOP/packaging/window_crates.py" --metadata "$BUILD/metadata.json" \
    --root fermix-desktop --out "$BUILD/window-crates.json" >&2
}

# The package's own documents: the runtime's manifest; a changelog of one entry, this build, named
# changelog.gz because the deb is native; the window's manual page; and the deb's lintian overrides.
stage_documents() {
  local version="$1" mtime="$2" doc="$ROOT/usr/share/doc/fermix-desktop" when
  when="$(date -u -R -d "$mtime")"
  install -D -m 0644 "$RUNTIME/runtime-manifest.json" "$doc/runtime-manifest.json"
  printf 'fermix-desktop (%s) unstable; urgency=medium\n\n  * %s\n\n -- %s  %s\n' "$version" \
    "Fermix for Linux $version: the window, its GTK runtime and the engine." "$MAINTAINER" "$when" |
    gzip -9n > "$BUILD/changelog.gz"
  install -m 0644 "$BUILD/changelog.gz" "$doc/changelog.gz"
  gzip -9n -c "$DESKTOP/packaging/fermix-desktop.1" > "$BUILD/fermix-desktop.1.gz"
  install -D -m 0644 "$BUILD/fermix-desktop.1.gz" "$ROOT/usr/share/man/man1/fermix-desktop.1.gz"
  install -D -m 0644 "$DESKTOP/packaging/lintian-overrides" "$ROOT$LINTIAN_OVERRIDES"
}

# The package's tree: the runtime, the engine as is, the window and its link, what the desktop
# reads, with the metainfo given this package's release, and the documents.
stage_tree() {
  local arch="$1" version="$2" mtime="$3" icon
  rm -rf "$ROOT"
  mkdir -p "$ROOT"
  tar -xf "$RUNTIME/runtime-$arch.tar" -C "$ROOT"
  cp -a "$ENGINE/tree/." "$ROOT/"
  install -D -m 0755 "$CARGO_TARGET_DIR/release/fermix-desktop" "$ROOT$PREFIX/bin/fermix-desktop"
  ln -s ../lib/fermix-desktop/bin/fermix-desktop "$ROOT/usr/bin/fermix-desktop"
  install -D -m 0644 "$DESKTOP/data/io.tezra.Fermix.desktop" "$ROOT/usr/share/applications/io.tezra.Fermix.desktop"
  python3 "$DESKTOP/packaging/metainfo_release.py" --metainfo "$DESKTOP/data/io.tezra.Fermix.metainfo.xml" \
    --version "$version" --date "${mtime%%T*}" --out "$BUILD/io.tezra.Fermix.metainfo.xml" >&2
  install -D -m 0644 "$BUILD/io.tezra.Fermix.metainfo.xml" "$ROOT/usr/share/metainfo/io.tezra.Fermix.metainfo.xml"
  install -D -m 0644 "$DESKTOP/data/io.tezra.Fermix.service" "$ROOT/usr/share/dbus-1/services/io.tezra.Fermix.service"
  while IFS= read -r -d '' icon; do
    install -D -m 0644 "$DESKTOP/data/icons/$icon" "$ROOT/usr/share/icons/$icon"
  done < <(cd "$DESKTOP/data/icons" && find hicolor -type f -print0)
  stage_documents "$version" "$mtime"
  log "the tree is staged: $(find "$ROOT" -type f | wc -l) files, $(du -sh "$ROOT" | cut -f1)"
}

# The flags copyright.py and check_copyright.sh both take.
copyright_flags() {
  printf '%s\n' --repo "$WORKSPACE/src" --lock "$LOCK" \
    --runtime-licenses "$RUNTIME/runtime-licenses.json" --runtime-texts "$BUILD/runtime-texts" \
    --window-crates "$BUILD/window-crates.json" \
    --vendored "$DESKTOP/packaging/vendored.json" --standard-texts "$DESKTOP/packaging/licenses" \
    --engine-copyright "$ROOT/usr/share/doc/fermix/copyright"
}

write_copyright() {
  local flags
  mkdir -p "$BUILD/runtime-texts"
  tar -xzf "$RUNTIME/runtime-licenses.tar.gz" -C "$BUILD/runtime-texts"
  mapfile -t flags < <(copyright_flags)
  python3 "$DESKTOP/packaging/copyright.py" "${flags[@]}" --out "$COPYRIGHT" >&2
  chmod 0644 "$COPYRIGHT"
}

# appstreamcli accepts the installed metainfo, and its one pedantic hint is the app id's capital
# letter, which the Flatpak's id fixes.
check_metainfo() {
  local report hints
  report="$("$PREFIX/bin/appstreamcli" validate --pedantic --no-net \
    "$ROOT/usr/share/metainfo/io.tezra.Fermix.metainfo.xml" 2>&1)" || fail "appstreamcli refuses the metainfo: $report"
  hints="$(grep -E '^[EWIP]: ' <<< "$report" | grep -vF 'cid-contains-uppercase-letter' || [ $? -eq 1 ])"
  [ -z "$hints" ] || fail "appstreamcli has more to say about the metainfo: $hints"
  log "appstreamcli $("$PREFIX/bin/appstreamcli" --version | awk '{ print $NF }') accepts the metainfo:" \
    "$(grep -E '^[EWIP]: ' <<< "$report" | tr '\n' ' ')"
}

run_gates() {
  local flags
  bash "$DESKTOP/scripts/check_private_runtime.sh" "$ROOT" "$LOCK" >&2
  mapfile -t flags < <(copyright_flags)
  bash "$DESKTOP/scripts/check_copyright.sh" "$COPYRIGHT" "${flags[@]}" >&2
  check_metainfo
}

# The maintainer scripts are the engine's alone: desktop-file-utils and hicolor-icon-theme refresh
# the desktop and icon caches from their own triggers on every distribution the package targets.
run_nfpm() {
  local version="$1" arch="$2" mtime="$3" deb="$4" rpm="$5"
  python3 "$DESKTOP/packaging/nfpm_config.py" --template "$DESKTOP/packaging/nfpm-fermix-desktop.yaml.tmpl" \
    --map "$MAP" --lock "$LOCK" --engine-relations "$ENGINE/relations.json" --root "$ROOT" \
    --version "$version" --arch "$arch" --mtime "$mtime" --postinstall "$ENGINE/maintainer/postinstall.sh" \
    --postremove "$ENGINE/maintainer/postremove.sh" --out "$BUILD/nfpm.yaml" >&2
  tar -xzf "$TOOLS"/nfpm_*.tar.gz -C "$BUILD" nfpm
  "$BUILD/nfpm" package --config "$BUILD/nfpm.yaml" --packager deb --target "$OUT/$deb" >&2
  "$BUILD/nfpm" package --config "$BUILD/nfpm.yaml" --packager rpm --target "$OUT/$rpm" >&2
}

# The deb's control members and its file list, as dpkg would read them, with no dpkg here.
read_deb() {
  local deb="$1" dir="$BUILD/deb"
  mkdir -p "$dir/control" "$dir/scripts"
  [ "$(ar t "$deb" | tr '\n' ' ')" = "debian-binary control.tar.gz data.tar.gz " ] ||
    fail "$deb does not hold the members nFPM writes: $(ar t "$deb" | tr '\n' ' ')"
  ar p "$deb" control.tar.gz | tar -xz -C "$dir/control"
  cp "$dir/control/postinst" "$dir/scripts/postinstall.sh"
  cp "$dir/control/postrm" "$dir/scripts/postremove.sh"
  ar p "$deb" data.tar.gz | tar -tvz | awk '$1 !~ /^d/ { print $6 }' |
    sed -e 's|^\./|/|' -e 's|^\([^/]\)|/\1|' | sort > "$dir/files"
}

read_rpm() {
  local rpm="$1" dir="$BUILD/rpm"
  mkdir -p "$dir/scripts"
  rpm -qp --requires "$rpm" > "$dir/requires"
  rpm -qp --queryformat '%{POSTIN}' "$rpm" > "$dir/scripts/postinstall.sh"
  rpm -qp --queryformat '%{POSTUN}' "$rpm" > "$dir/scripts/postremove.sh"
  rpm -qlp "$rpm" | sort > "$dir/files"
}

# What each package should list: every file and link of the tree, and for the rpm also the
# directories the package owns, but not the deb's lintian overrides, and the copyright file in its
# %license directory rather than beside the documents.
expected_files() {
  (cd "$ROOT" && find . \( -type f -o -type l \) -printf '%p\n' | sed 's|^\./|/|' | sort) > "$BUILD/files"
  (cd "$ROOT" && find "./${PREFIX#/}" ./usr/share/doc/fermix-desktop -type d -printf '%p\n' |
    sed 's|^\./|/|' | cat - "$BUILD/files" | grep -vxF -e "$LINTIAN_OVERRIDES" -e "$DOC_COPYRIGHT") \
    > "$BUILD/rpm-files.unsorted"
  printf '%s\n' "$RPM_LICENSES" "$RPM_LICENSES/copyright" >> "$BUILD/rpm-files.unsorted"
  sort "$BUILD/rpm-files.unsorted" > "$BUILD/rpm-files"
}

check_identity() {
  local version="$1" arch="$2" rpm_arch="$3" deb_dir="$BUILD/deb/control" rpm="$4" field
  for field in "Package: fermix-desktop" "Version: $version" "Architecture: $arch" \
    "Maintainer: $MAINTAINER" "Provides: fermix (= $version)" "Conflicts: fermix" "Replaces: fermix"; do
    grep -qxF -- "$field" "$deb_dir/control" || fail "the deb's control has no '$field'"
  done
  [ "$(rpm -qp --queryformat '%{NAME} %{VERSION} %{RELEASE} %{ARCH}' "$rpm")" = \
    "fermix-desktop $version 1 $rpm_arch" ] || fail "the rpm is not fermix-desktop $version-1.$rpm_arch"
  rpm -qp --provides "$rpm" | grep -qxF "fermix = $version" || fail "the rpm does not provide fermix = $version"
  [ "$(rpm -qp --conflicts "$rpm")" = fermix ] || fail "the rpm does not conflict with fermix alone"
  [ -z "$(rpm -qp --obsoletes "$rpm")" ] || fail "the rpm obsoletes something: $(rpm -qp --obsoletes "$rpm")"
}

check_packages() {
  local version="$1" arch="$2" rpm_arch="$3" deb="$OUT/$4" rpm="$OUT/$5" family
  read_deb "$deb"
  read_rpm "$rpm"
  check_identity "$version" "$arch" "$rpm_arch" "$rpm"
  python3 "$DESKTOP/scripts/package_dependencies.py" --root "$ROOT" --map "$MAP" --lock "$LOCK" \
    --deb-control "$BUILD/deb/control/control" --rpm-requires "$BUILD/rpm/requires" >&2
  for family in deb rpm; do
    for script in postinstall.sh postremove.sh; do
      cmp "$ENGINE/maintainer/$script" "$BUILD/$family/scripts/$script" >&2 ||
        fail "the $family's $script is not the engine's, byte for byte"
    done
  done
  log "both packages' postinstall and postremove are the engine's, byte for byte"
  expected_files
  diff -u "$BUILD/files" "$BUILD/deb/files" >&2 || fail "the deb's files are not the staged tree's"
  diff -u "$BUILD/rpm-files" "$BUILD/rpm/files" >&2 || fail "the rpm's files are not the staged tree's"
  log "both packages are fermix-desktop $version, relations, scripts and files as staged"
}

write_outputs() {
  local deb="$1" rpm="$2"
  cp "$COPYRIGHT" "$OUT/copyright"
  cp "$BUILD/nfpm.yaml" "$BUILD/window-crates.json" "$OUT/"
  (cd "$OUT" && sha256sum "$deb" "$rpm" copyright nfpm.yaml window-crates.json > SHA256SUMS)
  log "$(cd "$OUT" && du -h "$deb" "$rpm" | tr '\t\n' ' ')"
}

main() {
  local version="" arch="" mtime="" triple rpm_arch deb rpm
  while [ $# -gt 0 ]; do
    case "$1" in
      --version | --arch | --mtime)
        [ $# -ge 2 ] || fail "usage: build_in_container.sh --version <v> --arch <arch> --mtime <time>"
        case "$1" in --version) version="$2" ;; --arch) arch="$2" ;; --mtime) mtime="$2" ;; esac
        shift 2 ;;
      *) fail "usage: build_in_container.sh --version <v> --arch <arch> --mtime <time>" ;;
    esac
  done
  case "$arch" in
    amd64) triple=x86_64-unknown-linux-gnu rpm_arch=x86_64 ;;
    arm64) triple=aarch64-unknown-linux-gnu rpm_arch=aarch64 ;;
    *) fail "fermix-desktop is built for amd64 and arm64, not '$arch'" ;;
  esac
  [ -n "$version" ] && [ -n "$mtime" ] || fail "usage: build_in_container.sh --version <v> --arch <arch> --mtime <time>"
  deb="fermix-desktop_${version}_${arch}.deb"
  rpm="fermix-desktop-${version}-1.${rpm_arch}.rpm"
  mkdir -p "$BUILD" "$OUT"
  check_inputs
  build_window "$arch"
  list_window_crates "$triple"
  stage_tree "$arch" "$version" "$mtime"
  write_copyright
  run_gates
  run_nfpm "$version" "$arch" "$mtime" "$deb" "$rpm"
  check_packages "$version" "$arch" "$rpm_arch" "$deb" "$rpm"
  write_outputs "$deb" "$rpm"
}

main "$@"
