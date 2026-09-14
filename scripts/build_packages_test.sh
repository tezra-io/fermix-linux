#!/usr/bin/env bash
#
# Exercise build_packages.sh's refusals, and the dependency check's, offline.
#
# The build itself takes minutes and needs the container; its refusals take
# milliseconds and are the half that decides whether a bad release is stopped.
# Every one of them is driven here, on any host, with no docker, no nFPM and no
# package built:
#
#   * the version rules, which are what keep `Depends: fermix (= <v>)` and
#     `Requires: fermix = <v>` meaning the same thing in both families;
#   * the record checks, which run before the machine is touched at all;
#   * the declared-against-derived check, driven with fixture files that stand in
#     for what dpkg, rpm and objdump say about a real build.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/build_packages.sh"
CHECK="$ROOT_DIR/scripts/package_dependencies.py"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/build-packages-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

VERSION="$(awk -F'"' '/^version = "/ { print $2; exit }' "$ROOT_DIR/App/Fermix/Cargo.toml")"

fail() {
  echo "build_packages_test: $*" >&2
  exit 1
}

expect_refusal() {
  local what="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    fail "$what was accepted"
  fi
  echo "  refused: $what"
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

echo "build_packages_test: the scripts parse"
bash -n "$SCRIPT" || fail "the script does not parse"
python3 -c "import ast,sys; ast.parse(open(sys.argv[1]).read())" "$CHECK" ||
  fail "the dependency check does not parse"
echo "  ok: shell and python syntax"

# ---------------------------------------------------------------------------
# The version rules
# ---------------------------------------------------------------------------

echo "build_packages_test: the version rules"

expect_refusal "no arguments at all" bash "$SCRIPT"
expect_refusal "a version with no architecture" bash "$SCRIPT" "$VERSION"
expect_refusal "an unknown flag" bash "$SCRIPT" "$VERSION" arm64 --nonsense

expect_sentence "a version carrying a Debian revision" "Debian revision" \
  bash "$SCRIPT" "$VERSION-2" arm64
expect_sentence "a version carrying an rpm epoch" "rpm epoch" \
  bash "$SCRIPT" "1:$VERSION" arm64
expect_sentence "a prerelease version" "Debian revision" \
  bash "$SCRIPT" "1.2.0-rc1" arm64
expect_sentence "a version that is not X.Y.Z" "is not X.Y.Z" \
  bash "$SCRIPT" "1.2" arm64
expect_sentence "an architecture neither family builds for" "neither amd64 nor arm64" \
  bash "$SCRIPT" "$VERSION" riscv64

# ---------------------------------------------------------------------------
# The record, before the machine
# ---------------------------------------------------------------------------

echo "build_packages_test: the record"

expect_sentence "a version that is not the crate's" "the crate is version" \
  bash "$SCRIPT" "9.9.9" arm64

# The engine pin the repository ships is unpinned, so the version agreement
# below cannot fire against it. A filled pin naming another version is written
# into a copy of the repository, and the refusal is driven there.
PINNED_REPO="$WORK/pinned"
mkdir -p "$PINNED_REPO/scripts" "$PINNED_REPO/engine" "$PINNED_REPO/App/Fermix"
cp "$SCRIPT" "$ROOT_DIR/scripts/engine_pin.sh" "$PINNED_REPO/scripts/"
cp "$ROOT_DIR/App/Fermix/Cargo.toml" "$PINNED_REPO/App/Fermix/"
python3 - "$PINNED_REPO/engine/PIN.json" <<'PY'
import json
import sys

version = "0.9.9"
digest = "a" * 64
packages = {
    "linux_x86_64": {
        "deb": {"asset": f"fermix_{version}_amd64.deb", "sha256": digest},
        "rpm": {"asset": f"fermix-{version}-1.x86_64.rpm", "sha256": digest},
    },
    "linux_aarch64": {
        "deb": {"asset": f"fermix_{version}_arm64.deb", "sha256": digest},
        "rpm": {"asset": f"fermix-{version}-1.aarch64.rpm", "sha256": digest},
    },
}
with open(sys.argv[1], "w", encoding="utf-8") as handle:
    json.dump(
        {
            "schema_version": 1,
            "repository": "tezra-io/fermix",
            "certificate_oidc_issuer": "https://token.actions.githubusercontent.com",
            "tag": f"v{version}",
            "source_commit": "b" * 40,
            "certificate_identity": (
                "https://github.com/tezra-io/fermix/.github/workflows/"
                f"release.yml@refs/tags/v{version}"
            ),
            "packages": packages,
            "note": "a fixture pin, written by scripts/build_packages_test.sh",
        },
        handle,
        indent=2,
    )
PY
expect_sentence "a pinned engine that is not this version" "pins engine 0.9.9" \
  bash "$PINNED_REPO/scripts/build_packages.sh" "$VERSION" arm64

if [ "$(uname -s)" != "Linux" ]; then
  expect_sentence "a build asked for on a host that is not Linux" "built on Linux" \
    bash "$SCRIPT" "$VERSION" arm64
fi

# ---------------------------------------------------------------------------
# The declared relations against what the binary needs
# ---------------------------------------------------------------------------

echo "build_packages_test: the declared relations against what the binary needs"

fixture() {
  local name="$1" content="$2"
  printf '%s\n' "$content" > "$WORK/$name"
}

write_fixtures() {
  local declared_deb="$1" declared_rpm="$2" derived_deb="$3"
  fixture declared-deb.txt "$declared_deb"
  fixture declared-rpm.txt "$declared_rpm"
  fixture derived-deb.txt "$derived_deb"
  fixture sonames.txt "libgtk-4.so.1
libadwaita-1.so.0
libc.so.6
libgobject-2.0.so.0"
  fixture closure.txt "libgtk-4-1
libadwaita-1-0
libc6
libglib2.0-0t64
librsvg2-common
webp-pixbuf-loader"
  fixture toolkit.txt "libgtk-4.so.1|libgtk-4-1|gtk4
libadwaita-1.so.0|libadwaita-1-0|libadwaita"
}

GOOD_DEB="fermix (= $VERSION), libgtk-4-1 (>= 4.16), libadwaita-1-0 (>= 1.6), librsvg2-common, webp-pixbuf-loader"
GOOD_RPM="fermix = $VERSION
gtk4 >= 4.16
libadwaita >= 1.6
librsvg2
webp-pixbuf-loader
rpmlib(CompressedFileNames) <= 3.0.4-1"
GOOD_DERIVED="shlibs:Depends=libadwaita-1-0 (>= 1.6), libc6 (>= 2.34), libglib2.0-0t64 (>= 2.80), libgtk-4-1 (>= 4.16)"

run_check() {
  python3 "$CHECK" \
    --version "$VERSION" \
    --declared-deb "$WORK/declared-deb.txt" \
    --declared-rpm "$WORK/declared-rpm.txt" \
    --derived-deb "$WORK/derived-deb.txt" \
    --sonames "$WORK/sonames.txt" \
    --closure "$WORK/closure.txt" \
    --toolkit "$WORK/toolkit.txt"
}

write_fixtures "$GOOD_DEB" "$GOOD_RPM" "$GOOD_DERIVED"
run_check >/dev/null || fail "a declaration that covers everything was refused"
echo "  accepted: a declaration that covers what the binary needs"

write_fixtures \
  "fermix (= $VERSION), libadwaita-1-0 (>= 1.6), librsvg2-common, webp-pixbuf-loader" \
  "$GOOD_RPM" "$GOOD_DERIVED"
expect_sentence "a deb that does not declare the toolkit the binary links" \
  "the deb declares no libgtk-4-1" run_check

write_fixtures "$GOOD_DEB" \
  "fermix = $VERSION
gtk4 >= 4.16
librsvg2
webp-pixbuf-loader" "$GOOD_DERIVED"
expect_sentence "an rpm that does not declare the toolkit the binary links" \
  "the rpm declares no libadwaita" run_check

write_fixtures "$GOOD_DEB" "$GOOD_RPM" \
  "shlibs:Depends=libgtk-4-1 (>= 4.16), libadwaita-1-0 (>= 1.6), libsomething-else1 (>= 3.0)"
expect_sentence "a library neither the declaration nor its closure carries" \
  "libsomething-else1" run_check

write_fixtures \
  "fermix (= $VERSION), libgtk-4-1 (>= 4.16), libadwaita-1-0 (>= 1.6), librsvg2-common, webp-pixbuf-loader" \
  "$GOOD_RPM" \
  "shlibs:Depends=libgtk-4-1 (>= 4.18.2), libadwaita-1-0 (>= 1.6)"
expect_sentence "a floor declared below what the binary was built against" \
  "declared at 4.16 and the binary needs 4.18.2" run_check

write_fixtures \
  "libgtk-4-1 (>= 4.16), libadwaita-1-0 (>= 1.6), librsvg2-common, webp-pixbuf-loader" \
  "$GOOD_RPM" "$GOOD_DERIVED"
expect_sentence "a deb that declares no relation on the engine" \
  "the deb declares no relation on fermix" run_check

write_fixtures "fermix (>= $VERSION), libgtk-4-1 (>= 4.16), libadwaita-1-0 (>= 1.6), librsvg2-common, webp-pixbuf-loader" \
  "$GOOD_RPM" "$GOOD_DERIVED"
expect_sentence "a floor on the engine where the design fixes an exact version" \
  "the deb declares fermix >=" run_check

write_fixtures "$GOOD_DEB" \
  "fermix = 9.9.9
gtk4 >= 4.16
libadwaita >= 1.6
librsvg2
webp-pixbuf-loader" "$GOOD_DERIVED"
expect_sentence "an rpm pinned to another engine version" \
  "the rpm declares fermix = 9.9.9" run_check

write_fixtures "$GOOD_DEB" "$GOOD_RPM" "$GOOD_DERIVED"
: > "$WORK/sonames.txt"
expect_sentence "a binary that links nothing at all" "names no shared library" run_check

write_fixtures "$GOOD_DEB" "$GOOD_RPM" "$GOOD_DERIVED"
: > "$WORK/derived-deb.txt"
expect_sentence "a build that derived nothing" "nothing was derived" run_check

echo "build_packages_test: the version comparison it decides floors with"
python3 - "$CHECK" <<'PY'
import importlib.util
import sys

specification = importlib.util.spec_from_file_location("package_dependencies", sys.argv[1])
module = importlib.util.module_from_spec(specification)
specification.loader.exec_module(module)

cases = [
    ("4.16", "4.16", 0),
    ("4.18", "4.16", 1),
    ("4.16", "4.18", -1),
    ("4.16", "4.16.0", -1),
    ("4.16.1", "4.16", 1),
    ("4.20", "4.9", 1),
    ("1.6", "1.6~beta", 1),
    ("2.80", "2.8", 1),
]
for left, right, expected in cases:
    actual = module.compare_versions(left, right)
    if actual != expected:
        sys.exit(f"build_packages_test: {left} against {right} answered {actual}, not {expected}")
print("  ok: eight orderings, including the one a plain string compare gets wrong")
PY

echo "build_packages_test: every refusal fired"
