#!/usr/bin/env bash
#
# Exercise build_packages.sh's refusals, and the gates it runs, offline.
#
# The build itself takes minutes and needs the container; its refusals take
# milliseconds and are the half that decides whether a bad release is stopped.
# Every one of them is driven here, on any host, with no docker, no nFPM, no
# private toolkit and no package built:
#
#   * the version rules, including `X.Y.Z+N`, which is how a desktop-only
#     rebuild is published now that there is no exact-version relation to keep
#     equal in two dialects;
#   * the record checks, which run before the machine is touched at all,
#     including the one that makes an unpinned tree unbuildable;
#   * the splice, which is what makes the verified engine archive the single
#     author of the engine's half of the file list, so that an engine release
#     adding a packaged file needs no edit in this repository;
#   * the declared-against-derived check, driven with a fixture tree of real ELF
#     files and fixture relation files that stand in for what dpkg and rpm say
#     about a real build.
#
# The gates this script calls have their own harnesses beside them:
# scripts/check_private_runtime_test.sh, scripts/check_copyright_test.sh and
# scripts/container_build_test.sh. This one drives the build itself.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/build_packages.sh"
CHECK="$ROOT_DIR/scripts/package_dependencies.py"
ENGINE_CHECK="$ROOT_DIR/scripts/check_engine_contents.py"
FIXTURES="$ROOT_DIR/scripts/fixtures/packaging"

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

expect_success() {
  local what="$1"
  shift
  "$@" >/dev/null 2>&1 || fail "$what was refused"
  echo "  accepted: $what"
}

echo "build_packages_test: the scripts parse"
for script in "$SCRIPT" \
  "$ROOT_DIR/scripts/check_private_runtime.sh" \
  "$ROOT_DIR/scripts/check_copyright.sh" \
  "$ROOT_DIR/scripts/assemble_maintainer.sh" \
  "$ROOT_DIR/scripts/runtime_image.sh"; do
  bash -n "$script" || fail "$(basename "$script") does not parse"
done
for module in "$CHECK" "$ENGINE_CHECK" "$ROOT_DIR/scripts/splice_engine_contents.py" \
  "$ROOT_DIR/scripts/elf_facts.py" \
  "$ROOT_DIR/scripts/generate_copyright.py"; do
  python3 -c "import ast,sys; ast.parse(open(sys.argv[1]).read())" "$module" ||
    fail "$(basename "$module") does not parse"
done
echo "  ok: shell and python syntax"

# ---------------------------------------------------------------------------
# The version rules
# ---------------------------------------------------------------------------

echo "build_packages_test: the version rules"

expect_refusal "no arguments at all" bash "$SCRIPT"
expect_refusal "a version with no architecture" bash "$SCRIPT" "$VERSION"
expect_refusal "an unknown flag" bash "$SCRIPT" "$VERSION" arm64 --nonsense
expect_sentence "--engine with no archive named" "needs the path" \
  bash "$SCRIPT" "$VERSION" arm64 --engine

expect_sentence "a version carrying a Debian revision" "Debian revision" \
  bash "$SCRIPT" "$VERSION-2" arm64
expect_sentence "a version carrying an rpm epoch" "rpm epoch" \
  bash "$SCRIPT" "1:$VERSION" arm64
expect_sentence "a prerelease version" "Debian revision" \
  bash "$SCRIPT" "1.2.0-rc1" arm64
expect_sentence "a version that is not X.Y.Z" "neither X.Y.Z nor X.Y.Z+N" \
  bash "$SCRIPT" "1.2" arm64
expect_sentence "a rebuild counter that is not a number" "neither X.Y.Z nor X.Y.Z+N" \
  bash "$SCRIPT" "1.2.0+beta" arm64
expect_sentence "an architecture neither family builds for" "neither amd64 nor arm64" \
  bash "$SCRIPT" "$VERSION" riscv64

# `X.Y.Z+N` is the shape a desktop-only rebuild takes, so it has to get past the
# version rules and be stopped by the next check rather than by this one.
expect_sentence "a desktop-only rebuild reaching the version agreement" "the crate is version" \
  bash "$SCRIPT" "$VERSION+1" arm64

# ---------------------------------------------------------------------------
# The record, before the machine
# ---------------------------------------------------------------------------

echo "build_packages_test: the record"

expect_sentence "a version that is not the crate's" "the crate is version" \
  bash "$SCRIPT" "9.9.9" arm64

# A copy of the repository with a filled pin naming another engine version, and
# one with the pin the repository ships, which is unpinned.
make_repository() {
  local where="$1"
  mkdir -p "$where/scripts" "$where/engine" "$where/App/Fermix" "$where/packaging"
  # The siblings the script sources. A fake tree missing one of them would
  # fail for that reason rather than for the reason each case is about.
  cp "$SCRIPT" "$ROOT_DIR/scripts/engine_pin.sh" \
    "$ROOT_DIR/scripts/container_cache.sh" "$ROOT_DIR/scripts/crate_gates.sh" \
    "$where/scripts/"
  cp "$ROOT_DIR/App/Fermix/Cargo.toml" "$where/App/Fermix/"
  cp "$ROOT_DIR/packaging/io.tezra.Fermix.metainfo.xml" "$where/packaging/"
}

PINNED_REPO="$WORK/pinned"
make_repository "$PINNED_REPO"
python3 - "$PINNED_REPO/engine/PIN.json" <<'PY'
import json
import sys

version = "0.9.9"
tag = f"v{version}"
digest = "a" * 64
with open(sys.argv[1], "w", encoding="utf-8") as handle:
    json.dump(
        {
            "schema_version": 2,
            "repository": "tezra-io/fermix",
            "certificate_oidc_issuer": "https://token.actions.githubusercontent.com",
            "tag": tag,
            "engine_version": version,
            "source_commit": "b" * 40,
            "certificate_identity": (
                "https://github.com/tezra-io/fermix/.github/workflows/"
                f"release.yml@refs/tags/{tag}"
            ),
            "artifacts": {
                "linux_x86_64": {
                    "asset": "fermix_app_engine_linux_x86_64.tar.gz",
                    "sha256": digest,
                },
                "linux_aarch64": {
                    "asset": "fermix_app_engine_linux_aarch64.tar.gz",
                    "sha256": digest,
                },
            },
        },
        handle,
        indent=2,
    )
PY
expect_sentence "a pinned engine that is not this version" "pins engine 0.9.9" \
  bash "$PINNED_REPO/scripts/build_packages.sh" "$VERSION" arm64

UNPINNED_REPO="$WORK/unpinned"
make_repository "$UNPINNED_REPO"
cp "$ROOT_DIR/engine/PIN.json" "$UNPINNED_REPO/engine/PIN.json"
expect_sentence "an unpinned tree, which now carries no engine at all" \
  "is unpinned, and this package carries the engine" \
  bash "$UNPINNED_REPO/scripts/build_packages.sh" "$VERSION" arm64

# --engine is the developer's way past that, and it is refused under CI.
touch "$WORK/fermix_app_engine_linux_aarch64.tar.gz"
expect_sentence "--engine under CI" "CI is true" \
  env CI=true bash "$UNPINNED_REPO/scripts/build_packages.sh" \
  "$VERSION" arm64 --engine "$WORK/fermix_app_engine_linux_aarch64.tar.gz"
expect_sentence "--engine naming an archive that is not there" "no engine archive at" \
  env CI=false bash "$UNPINNED_REPO/scripts/build_packages.sh" \
  "$VERSION" arm64 --engine "$WORK/absent.tar.gz"

if [ "$(uname -s)" != "Linux" ]; then
  expect_sentence "a build asked for on a host that is not Linux" "built on Linux" \
    bash "$SCRIPT" "$VERSION" arm64
fi

# ---------------------------------------------------------------------------
# The engine's file list has exactly one author
# ---------------------------------------------------------------------------

echo "build_packages_test: the engine contents, spliced rather than restated"

TEMPLATE="$ROOT_DIR/packaging/nfpm-fermix-desktop.yaml.tmpl"
SPLICE="$ROOT_DIR/scripts/splice_engine_contents.py"
MARKER="# >>> engine contents, spliced from the verified archive"

# The template must carry the marker exactly once and must name none of the
# engine's own paths: two authors and one package is a file carried twice.
[ "$(grep -c -- "$MARKER" "$TEMPLATE")" = "1" ] ||
  fail "the template does not carry the splice marker exactly once"
echo "  ok: the template carries the splice marker exactly once"

# The prefix test needs its separator: every engine-owned root has a
# desktop-owned namesake one word longer, and a grep for /usr/bin/fermix would
# match /usr/bin/fermix-desktop, which is the window's own.
python3 - "$TEMPLATE" "$SPLICE" <<'PY' || fail "the template declares engine-owned paths"
import sys

template, splice = sys.argv[1:3]
sys.path.insert(0, __import__("os").path.dirname(splice))
import splice_engine_contents as engine

destinations = [
    line.strip()[len("dst: ") :].strip('"')
    for line in open(template, encoding="utf-8")
    if line.strip().startswith("dst: ")
]
trespassing = sorted(d for d in destinations if engine.engine_owned(d))
if trespassing:
    print("the template declares engine-owned paths: " + ", ".join(trespassing))
    sys.exit(1)
PY
echo "  ok: the template leaves every engine-owned path to the engine"

# The engine stores a channel bot key by shelling out to `secret-tool`
# (System.find_executable, so a PATH lookup rather than a linked library), and a
# host running gnome-keyring still cannot save one when that executable is
# absent. This is the package's first relation declared for a program rather
# than for a library, so both families are asserted by name: on deb the
# executable is split out into libsecret-tools, and on rpm there is no such
# split, so the relation is the file capability rather than a package name.
grep -qF -- "- libsecret-tools" "$TEMPLATE" ||
  fail "the template declares no deb relation for secret-tool, so saving a key fails on a stock desktop"
grep -qF -- "- /usr/bin/secret-tool" "$TEMPLATE" ||
  fail "the template declares no rpm relation for secret-tool, so saving a key fails on a stock desktop"
echo "  ok: the template declares the executable the secret store shells out to"

# The runtime installs three directories under the private prefix -- lib,
# libexec and share -- and the template has to carry all three. libexec held
# gio-launch-desktop, which GLib spawns for every URL the window opens, and the
# template named it nowhere, so every package built before 2026-09-20 shipped
# without it and opening a link silently did nothing on an installed machine.
# A program started by absolute path is not a NEEDED entry, so no relation and
# no link check could see it. This is the cheap half of the guard;
# scripts/check_runtime_complete.sh is the half that reads the built package.
for runtime_tree in lib libexec share; do
  grep -qF -- "dst: /usr/lib/fermix-desktop/$runtime_tree" "$TEMPLATE" ||
    fail "the template does not carry the runtime's $runtime_tree/, so the package ships without it"
done
echo "  ok: the template carries every directory the runtime installs"

# secret-tool is only half of it: it talks to a secret service, and a desktop
# with no keyring at all has nowhere to put a key. A keyring therefore arrives
# as a recommendation in both families, and must never become a dependency -- a
# KDE or KeePassXC user already owns org.freedesktop.secrets, and a hard
# relation would push a second daemon onto a machine whose secret service
# already works. Both halves are asserted: that the name is there, and that it
# is on neither depends list.
python3 - "$TEMPLATE" <<'PY' || fail "the keyring relation is missing, or is not weak in both families"
import re
import sys

import yaml

body = re.sub(r"\{\{[A-Z_]+\}\}", "X", open(sys.argv[1], encoding="utf-8").read())
overrides = yaml.safe_load(body)["overrides"]
for family in ("deb", "rpm"):
    block = overrides[family]
    if "gnome-keyring" not in (block.get("recommends") or []):
        print(f"{family} does not recommend gnome-keyring, so a bare desktop has no secret service")
        sys.exit(1)
    if "gnome-keyring" in (block.get("depends") or []):
        print(f"{family} depends on gnome-keyring, which forces a second daemon onto a KDE or KeePassXC machine")
        sys.exit(1)
PY
echo "  ok: both families recommend a keyring, and neither requires one"

render() {
  python3 - "$TEMPLATE" "$WORK/rendered.yaml" \
    "ARCH=amd64" "VERSION=$VERSION" "STAGE=/stage" \
    "POSTINSTALL=/m/postinstall.sh" "POSTREMOVE=/m/postremove.sh" <<'PY'
import re
import sys

template, out, *pairs = sys.argv[1:]
values = dict(pair.split("=", 1) for pair in pairs)
body = re.sub(r"\{\{([A-Z_]+)\}\}", lambda m: values[m.group(1)], open(template).read())
assert "{{" not in body
open(out, "w").write(body)
PY
}

run_splice() {
  python3 "$SPLICE" --rendered "$WORK/rendered.yaml" --out "$WORK/spliced.yaml" \
    --engine-contents "$1" --staging "$2"
}

render
expect_success "the real engine archive's own contents block" \
  run_splice "$FIXTURES/engine-nfpm-contents.yaml" "$FIXTURES/archive"

# Every destination the archive names is in the spliced configuration, and the
# entries the template authored are still there beside them.
for landed in /usr/bin/fermix /usr/lib/fermix/cosign /usr/share/man/man1/fermix.1.gz \
  /usr/lib/fermix-desktop/bin/fermix-desktop /usr/bin/fermix-desktop; do
  grep -qE "^\s*dst: $landed\$" "$WORK/spliced.yaml" ||
    fail "the spliced configuration installs nothing at $landed"
done
echo "  accepted: both authors' entries, in one configuration"

# An archive that reached outside the paths the engine owns would be overwriting
# the window's own files, and one that named a path the template already claims
# would be the two authors colliding.
python3 - "$FIXTURES/engine-nfpm-contents.yaml" "$WORK/trespass.yaml" <<'PY'
import sys

source, out = sys.argv[1:3]
body = open(source, encoding="utf-8").read().replace(
    "dst: /usr/bin/fermix\n", "dst: /usr/lib/fermix-desktop/lib/libgtk-4.so.1\n", 1
)
open(out, "w", encoding="utf-8").write(body)
PY
render
expect_sentence "an engine archive reaching outside the paths the engine owns" \
  "not a path the engine owns" run_splice "$WORK/trespass.yaml" "$FIXTURES/archive"

python3 - "$FIXTURES/engine-nfpm-contents.yaml" "$WORK/collide.yaml" <<'PY'
import sys

source, out = sys.argv[1:3]
body = open(source, encoding="utf-8").read()
body += """
  - src: tree/usr/bin/fermix
    dst: /usr/bin/fermix
    file_info:
      mode: 0755
"""
open(out, "w", encoding="utf-8").write(body)
PY
render
expect_sentence "an engine archive installing one path twice" \
  "this package already claims it" run_splice "$WORK/collide.yaml" "$FIXTURES/archive"

python3 - "$FIXTURES/engine-nfpm-contents.yaml" "$WORK/absent.yaml" <<'PY'
import sys

source, out = sys.argv[1:3]
body = open(source, encoding="utf-8").read().replace(
    "src: tree/usr/bin/fermix\n", "src: tree/usr/bin/fermix-that-is-not-there\n", 1
)
open(out, "w", encoding="utf-8").write(body)
PY
render
expect_sentence "an archive whose contents block names a file the archive does not carry" \
  "the archive and its own contents block disagree" \
  run_splice "$WORK/absent.yaml" "$FIXTURES/archive"

python3 - "$FIXTURES/engine-nfpm-contents.yaml" "$WORK/escape.yaml" <<'PY'
import sys

source, out = sys.argv[1:3]
body = open(source, encoding="utf-8").read().replace(
    "src: tree/usr/bin/fermix\n", "src: ../../../../etc/passwd\n", 1
)
open(out, "w", encoding="utf-8").write(body)
PY
render
expect_sentence "an archive reaching outside its own staging directory" \
  "resolves outside" run_splice "$WORK/escape.yaml" "$FIXTURES/archive"

# The marker is the splice point, and a configuration with none or two of them
# is one nobody can place the engine's entries in.
render
python3 - "$WORK/rendered.yaml" <<'PY'
import sys

path = sys.argv[1]
body = open(path, encoding="utf-8").read()
open(path, "w", encoding="utf-8").write(
    body.replace("# >>> engine contents, spliced from the verified archive", "", 1)
)
PY
expect_sentence "a configuration with no splice marker in it" \
  "carries the splice marker 0 times" \
  run_splice "$FIXTURES/engine-nfpm-contents.yaml" "$FIXTURES/archive"

# ---------------------------------------------------------------------------
# The shape of the private runtime archive
# ---------------------------------------------------------------------------

echo "build_packages_test: the private runtime archive carries the toolkit and nothing else"

# The rule is one function in the build script, so the test asks the build
# script rather than a copy of its awk program that could drift from it.
# shellcheck source=scripts/build_packages.sh
source "$SCRIPT"

TARS="$WORK/tars"
mkdir -p "$TARS/usr/lib/fermix-desktop/lib"
echo payload > "$TARS/usr/lib/fermix-desktop/lib/libgtk-4.so.1"

# A well-formed archive: the toolkit, and the two parent directories tar has to
# record for the tree to be creatable at all.
tar -cf "$WORK/good.tar" -C "$TARS" usr
[ -z "$(runtime_archive_strays "$WORK/good.tar")" ] ||
  fail "a well-formed runtime archive was reported as carrying strays: $(runtime_archive_strays "$WORK/good.tar")"
echo "  accepted: the toolkit under its prefix, with usr/ and usr/lib/ above it"

# A second root is the failure this exists for.
mkdir -p "$TARS/etc"
echo elsewhere > "$TARS/etc/fermix.conf"
tar -cf "$WORK/second-root.tar" -C "$TARS" usr etc
[ "$(runtime_archive_strays "$WORK/second-root.tar")" = "$(printf 'etc/\netc/fermix.conf')" ] ||
  fail "an archive with a second root was not reported: $(runtime_archive_strays "$WORK/second-root.tar")"
echo "  refused: an archive carrying a second root"

# A plain file at one of the two exempt names is not one of the two exempt
# entries. The exemption is for directories, because the reason for it is that
# a tree needs its parents.
rm -rf "${TARS:?}/etc"
FILEROOT="$WORK/fileroot"
mkdir -p "$FILEROOT/usr"
echo not-a-directory > "$FILEROOT/usr/lib"
tar -cf "$WORK/file-at-lib.tar" -C "$FILEROOT" usr
[ "$(runtime_archive_strays "$WORK/file-at-lib.tar")" = "usr/lib" ] ||
  fail "a plain file at usr/lib was not reported: $(runtime_archive_strays "$WORK/file-at-lib.tar")"
echo "  refused: a plain file standing where a parent directory belongs"

# A near-namesake outside the prefix. The prefix test is written with its
# separator for the same reason the engine-owned test is.
NEAR="$WORK/near"
mkdir -p "$NEAR/usr/lib/fermix-desktop-extra"
echo elsewhere > "$NEAR/usr/lib/fermix-desktop-extra/thing"
tar -cf "$WORK/near.tar" -C "$NEAR" usr
case "$(runtime_archive_strays "$WORK/near.tar")" in
  *fermix-desktop-extra*) echo "  refused: a directory one word longer than the prefix" ;;
  *) fail "usr/lib/fermix-desktop-extra was taken for the private prefix" ;;
esac

# A symlink that lands outside the prefix is judged by where it lands. Reading
# the last field instead of the sixth would judge it by its target and let a
# link pointing into the prefix through.
LINK="$WORK/link"
mkdir -p "$LINK/usr/lib/fermix-desktop"
echo payload > "$LINK/usr/lib/fermix-desktop/real"
ln -s /usr/lib/fermix-desktop/real "$LINK/usr/lib/sneak"
tar -cf "$WORK/link.tar" -C "$LINK" usr
[ "$(runtime_archive_strays "$WORK/link.tar")" = "usr/lib/sneak" ] ||
  fail "a symlink landing outside the prefix was not reported: $(runtime_archive_strays "$WORK/link.tar")"
echo "  refused: a symlink landing outside the prefix, whatever it points at"

# ---------------------------------------------------------------------------
# The maintainer scripts, assembled from two fragments
# ---------------------------------------------------------------------------

echo "build_packages_test: the maintainer scripts"

ASSEMBLE="$ROOT_DIR/scripts/assemble_maintainer.sh"
MAINTAINER="$WORK/maintainer"
expect_success "an assembly from the fixture engine fragments" \
  "$ASSEMBLE" --engine "$FIXTURES/maintainer" --desktop "$ROOT_DIR/packaging/scripts" \
  --out "$MAINTAINER"
expect_success "the assembled scripts read back as their fragments" \
  "$ASSEMBLE" --engine "$FIXTURES/maintainer" --desktop "$ROOT_DIR/packaging/scripts" \
  --out "$MAINTAINER" --check

printf '# a line nobody assembled\n' >> "$MAINTAINER/postinstall.sh"
expect_sentence "an assembled script somebody edited afterwards" \
  "is not what this assembly writes" \
  "$ASSEMBLE" --engine "$FIXTURES/maintainer" --desktop "$ROOT_DIR/packaging/scripts" \
  --out "$MAINTAINER" --check

# The engine half is the half that has to be byte-identical, so the refusal for
# it is its own sentence rather than the general one above.
"$ASSEMBLE" --engine "$FIXTURES/maintainer" --desktop "$ROOT_DIR/packaging/scripts" \
  --out "$MAINTAINER" >/dev/null
CHANGED_ENGINE="$WORK/changed-engine"
mkdir -p "$CHANGED_ENGINE"
cp "$FIXTURES/maintainer/"*.sh "$CHANGED_ENGINE/"
printf 'echo "a step the engine package does not take"\n' >> "$CHANGED_ENGINE/postinstall.sh"
expect_sentence "an engine fragment that is not the one the package was built from" \
  "byte for byte" \
  "$ASSEMBLE" --engine "$CHANGED_ENGINE" --desktop "$ROOT_DIR/packaging/scripts" \
  --out "$MAINTAINER" --check

# The whole reason for the subshell wrapping: the engine fragment ends in
# `exit 0`, and a plain concatenation would stop the desktop half from running.
FRAGMENTS="$WORK/fragments"
mkdir -p "$FRAGMENTS/engine" "$FRAGMENTS/desktop" "$FRAGMENTS/out"
printf '#!/bin/sh\nset -eu\necho engine-ran\nexit 0\n' > "$FRAGMENTS/engine/postinstall.sh"
printf '#!/bin/sh\nset -eu\necho engine-ran\nexit 0\n' > "$FRAGMENTS/engine/postremove.sh"
printf '#!/bin/sh\nset -eu\necho desktop-ran\nexit 0\n' > "$FRAGMENTS/desktop/postinstall.sh"
printf '#!/bin/sh\nset -eu\necho desktop-ran\nexit 0\n' > "$FRAGMENTS/desktop/postremove.sh"
"$ASSEMBLE" --engine "$FRAGMENTS/engine" --desktop "$FRAGMENTS/desktop" \
  --out "$FRAGMENTS/out" >/dev/null
RAN="$(sh "$FRAGMENTS/out/postinstall.sh")"
[ "$RAN" = "engine-ran
desktop-ran" ] ||
  fail "the assembled postinstall ran '$RAN', and both halves have to run in order"
echo "  accepted: both halves run, in order, past the engine half's own exit"

printf '#!/bin/sh\nset -eu\necho engine-ran\nexit 3\n' > "$FRAGMENTS/engine/postinstall.sh"
"$ASSEMBLE" --engine "$FRAGMENTS/engine" --desktop "$FRAGMENTS/desktop" \
  --out "$FRAGMENTS/out" >/dev/null
if sh "$FRAGMENTS/out/postinstall.sh" >/dev/null 2>&1; then
  fail "an engine half that failed did not fail the assembled script"
fi
echo "  refused: an engine half that fails, which fails the whole script"

# ---------------------------------------------------------------------------
# The declared relations against what the package's ELFs need
# ---------------------------------------------------------------------------

echo "build_packages_test: the declared relations against what the ELFs need"

STAGE="$WORK/stage"
"$ROOT_DIR/scripts/fixtures/packaging/make_stage.sh" "$STAGE" ||
  fail "the fixture stage could not be built on this host"

run_check() {
  python3 "$CHECK" \
    --version "$VERSION" \
    --stage "$STAGE" \
    --relations "$WORK/relations.json" \
    --host-map "$ROOT_DIR/scripts/host_relations.map" \
    --not-a-library-deb dconf-service \
    --not-a-library-deb gsettings-desktop-schemas \
    --not-a-library-rpm dconf \
    --not-a-library-rpm gsettings-desktop-schemas
}

# The relations a correct build produces, derived from the fixture stage itself
# so that this test says nothing about which libraries the host happens to have.
write_relations() {
  python3 - "$STAGE" "$ROOT_DIR/scripts/host_relations.map" "$VERSION" \
    "$WORK/relations.json" "$ROOT_DIR/scripts" "$@" <<'PY'
import json
import os
import sys

stage, host_map, version, out, scripts_dir, *edits = sys.argv[1:]
sys.path.insert(0, scripts_dir)
import elf_facts

relations = {}
for line in open(host_map, encoding="utf-8"):
    row = line.strip()
    if row and not row.startswith("#"):
        soname, deb, rpm = (piece.strip() for piece in row.split(" :: "))
        relations[soname] = (deb, rpm)

prefix = os.path.join(stage, "usr/lib/fermix-desktop")
carried = set()
for directory, _, names in os.walk(prefix):
    carried.update(names)

deb, rpm = set(), set()
for path, facts in elf_facts.walk(stage):
    if not os.path.abspath(path).startswith(os.path.abspath(prefix) + os.sep):
        continue
    for soname in facts["needed"]:
        if soname in carried or soname not in relations:
            continue
        deb.add(relations[soname][0])
        rpm.add(relations[soname][1])

# The glibc floor is declared as its own versioned relation, so the plain libc
# rows the map carries are dropped in favour of it.
deb -= {"libc6"}
rpm -= {"libc.so.6()(64bit)"}
deb |= {"libc6 (>= 2.34)", "dconf-service", "gsettings-desktop-schemas"}
rpm |= {"libc.so.6(GLIBC_2.34)(64bit)", "dconf", "gsettings-desktop-schemas"}

built = {
    "deb": {
        "depends": sorted(deb),
        "provides": [f"fermix (= {version})"],
        "conflicts": ["fermix"],
        "replaces": ["fermix"],
        "obsoletes": [],
    },
    "rpm": {
        "depends": sorted(rpm) + ["rpmlib(CompressedFileNames) <= 3.0.4-1"],
        "provides": [f"fermix = {version}"],
        "conflicts": ["fermix"],
        "replaces": [],
        "obsoletes": [],
    },
}

for edit in edits:
    family, field, action, value = edit.split(":", 3)
    if action == "add":
        built[family][field].append(value)
    elif action == "drop":
        built[family][field] = [
            item for item in built[family][field] if not item.startswith(value)
        ]
    elif action == "replace":
        old, new = value.split("=>", 1)
        built[family][field] = [
            new if item == old else item for item in built[family][field]
        ]
    else:
        sys.exit(f"build_packages_test: '{action}' is not an edit")

with open(out, "w", encoding="utf-8") as handle:
    json.dump(built, handle, indent=2, sort_keys=True)
PY
}

write_relations
expect_success "a declaration the package's own ELFs justify" run_check

write_relations "deb:depends:add:libwidget42"
expect_sentence "a deb relation no ELF in the package needs" \
  "the deb declares libwidget42 and no ELF in the package needs it" run_check

write_relations "rpm:depends:add:libwidget42.so.1()(64bit)"
expect_sentence "an rpm relation no ELF in the package needs" \
  "no ELF in the package needs it" run_check

write_relations "deb:depends:drop:libc6 (>=" "deb:depends:add:libc6 (>= 2.17)"
expect_sentence "a glibc floor below what the ELFs require" \
  "and the deb declares a floor of 2.17" run_check

write_relations "deb:depends:drop:libc6"
expect_sentence "a deb that declares no glibc floor at all" \
  "the deb declares no glibc floor" run_check

write_relations "deb:provides:drop:fermix"
expect_sentence "a deb that does not provide the engine" \
  "does not provide 'fermix (= $VERSION)'" run_check

write_relations "rpm:conflicts:drop:fermix"
expect_sentence "an rpm that does not conflict with the engine-only package" \
  "not mutually exclusive" run_check

write_relations "deb:replaces:drop:fermix"
expect_sentence "a deb that cannot hand /usr/bin/fermix over" \
  "does not replace fermix" run_check

write_relations "rpm:obsoletes:add:fermix"
expect_sentence "an rpm that would convert a headless install on the next upgrade" \
  "which would convert a headless install" run_check

write_relations "deb:provides:replace:fermix (= $VERSION)=>fermix (= 9.9.9)"
expect_sentence "a deb that provides another engine version" \
  "does not provide 'fermix (= $VERSION)'" run_check

# ---------------------------------------------------------------------------
# A library the package can load by name
# ---------------------------------------------------------------------------

echo "build_packages_test: nothing the package can dlopen is left to luck"

# The failure this is for: libepoxy links none of the GL libraries and loads
# whichever it needs by name, so a boundary read from NEEDED entries listed none
# of them, every gate passed, and the window aborted on the first machine that
# lacked one. The fixture is the same shape: a private object carrying a soname
# as a plain string that it does not link.
DLOPEN_STAGE="$WORK/dlopen"
"$FIXTURES/make_stage.sh" "$DLOPEN_STAGE" >/dev/null || fail "the fixture stage could not be built"
LOADER="$DLOPEN_STAGE/usr/lib/fermix-desktop/lib/libpretend-loader.so.0"
cp "$DLOPEN_STAGE/usr/lib/fermix-desktop/bin/fermix-desktop" "$LOADER"
printf 'libmadeup.so.7\0' >> "$LOADER"

run_dlopen_check() {
  python3 "$CHECK" \
    --version "$VERSION" \
    --stage "$DLOPEN_STAGE" \
    --relations "$WORK/relations.json" \
    --host-map "$ROOT_DIR/scripts/host_relations.map" \
    --not-a-library-deb dconf-service \
    --not-a-library-deb gsettings-desktop-schemas \
    --not-a-library-rpm dconf \
    --not-a-library-rpm gsettings-desktop-schemas \
    "$@"
}

write_relations
expect_sentence "a soname the package names but nobody decided about" \
  "is a dependency no ELF header records" run_dlopen_check

# The same soname, once somebody has looked at it and written it down.
expect_success "a soname somebody considered and deliberately did not declare" \
  run_dlopen_check --dlopen-considered libmadeup.so.7

# ---------------------------------------------------------------------------
# The host boundary, spelled in two files
# ---------------------------------------------------------------------------

echo "build_packages_test: the host map and the runtime lock are one boundary"

# The boundary is one list in two files: scripts/host_relations.map, which is
# the authority and says what each soname is called in each family, and the
# runtime lock's host_libraries, which the toolkit's own build enforces. Neither
# can prove the other complete, so what a machine can hold is that they are the
# same set. This is the check that would have caught libGLESv2.so.2 being in
# neither, months before a window aborted on a user's machine.
run_check_with_lock() {
  python3 "$CHECK" \
    --version "$VERSION" \
    --stage "$STAGE" \
    --relations "$WORK/relations.json" \
    --host-map "$ROOT_DIR/scripts/host_relations.map" \
    --lock "$1" \
    --not-a-library-deb dconf-service \
    --not-a-library-deb gsettings-desktop-schemas \
    --not-a-library-rpm dconf \
    --not-a-library-rpm gsettings-desktop-schemas
}

write_relations
python3 - "$ROOT_DIR/scripts/host_relations.map" "$WORK/lock-agrees.json" \
  "$WORK/lock-short.json" "$WORK/lock-extra.json" <<'PY'
import json
import sys

map_path, agrees, short, extra = sys.argv[1:5]

sonames = []
with open(map_path, encoding="utf-8") as handle:
    for line in handle:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        sonames.append(line.split(" :: ")[0])


def write(path, names):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump({"host_libraries": sorted(names), "components": []}, handle)


write(agrees, sonames)
write(short, sonames[1:])
write(extra, sonames + ["libmadeup.so.9"])
PY

expect_success "a lock whose host boundary is the map's" \
  run_check_with_lock "$WORK/lock-agrees.json"
expect_sentence "a lock naming a library the map cannot spell in either family" \
  "has no row in host_relations.map" run_check_with_lock "$WORK/lock-extra.json"
expect_sentence "a map row the runtime lock's boundary does not carry" \
  "one of the two files is behind the other" run_check_with_lock "$WORK/lock-short.json"

echo "build_packages_test: the icon set is the real artwork, at every size"
# The packages shipped a placeholder for the whole of this product's life: a
# brand-blue rounded square with a wordmark "F". No gate could see it, because
# every gate asked whether an icon was PRESENT and one always was. So what is
# asserted here is identity, not presence.
ICON_DIR="$ROOT_DIR/packaging/icons"
PLACEHOLDER_256="83ba704eed3db1c163b16d7d341d0dc0dd5b023a3315666ab111d31b1630396c"

for size in 16 22 24 32 48 64 128 256 512; do
  raster="$ICON_DIR/hicolor/${size}x${size}/apps/io.tezra.Fermix.png"
  [ -f "$raster" ] ||
    fail "the icon set has no ${size}x${size} raster; run scripts/render_icons.sh"
  # The directory a raster sits in is a claim about its pixels, and a 64 px image
  # filed under 512x512 resolves at the wrong size everywhere without erroring.
  dims="$(python3 -c "
import struct,sys
d=open(sys.argv[1],'rb').read()
print('%d %d' % struct.unpack('>II', d[16:24]))
" "$raster")"
  [ "$dims" = "$size $size" ] ||
    fail "the ${size}x${size} raster is ${dims// /x} pixels"
done
echo "  ok: nine rasters, each the size its directory claims"

# The negative control, and the only assertion here that would have caught the
# original defect: the 256 px icon must not be the placeholder that shipped.
# Named by digest rather than described, so it cannot return by another route.
current_256="$(sha256sum "$ICON_DIR/hicolor/256x256/apps/io.tezra.Fermix.png" | cut -d' ' -f1)"
[ "$current_256" != "$PLACEHOLDER_256" ] ||
  fail "the 256px icon is still the blue-square placeholder; run scripts/render_icons.sh"
echo "  ok: the 256px icon is not the placeholder that shipped"

# ...and the positive control for it, because "the digest differs" is equally
# what a truncated or empty file says.
for master in FermixAppIconMaster.png FermixMarkMaster.png; do
  [ -f "$ICON_DIR/masters/$master" ] ||
    fail "the artwork master $master is missing, so the icon set cannot be regenerated"
done
[ "$(stat -c %s "$ICON_DIR/hicolor/256x256/apps/io.tezra.Fermix.png")" -gt 4000 ] ||
  fail "the 256px icon is implausibly small; a truncated file also differs from the placeholder"
echo "  ok: the artwork masters are vendored and the icon has real content"

# The two copies that must not drift. build_packages.sh refuses a mismatch; this
# says so without a build, because the failure it prevents -- launcher and
# application drawing different icons -- is invisible in either surface alone.
for pair in "scalable/apps/io.tezra.Fermix.svg" "symbolic/apps/io.tezra.Fermix-symbolic.svg"; do
  resourced="$ROOT_DIR/App/Fermix/resources/icons/$(basename "$pair")"
  [ -f "$resourced" ] ||
    fail "the application has no gresource copy of $(basename "$pair")"
  cmp -s "$ICON_DIR/hicolor/$pair" "$resourced" ||
    fail "the packaged and gresource copies of $(basename "$pair") differ"
done
grep -q 'the launcher and the running application would draw different icons' "$SCRIPT" ||
  fail "build_packages.sh no longer refuses a drifted gresource icon"
echo "  ok: the packaged SVGs and the application's gresource copies are identical"

# The symbolic icon is recoloured by the theme. A hard-coded fill draws a black
# mark on a dark header, which still looks like an icon in a screenshot.
SYMBOLIC="$ICON_DIR/hicolor/symbolic/apps/io.tezra.Fermix-symbolic.svg"
grep -q 'currentColor' "$SYMBOLIC" ||
  fail "the symbolic icon does not use currentColor, so the theme cannot recolour it"
if grep -qE 'fill="#[0-9a-fA-F]{3,6}"' "$SYMBOLIC"; then
  fail "the symbolic icon carries a literal colour, which the theme will not override"
fi
echo "  ok: the symbolic icon is recolourable"

echo "build_packages_test: every refusal fired"
