#!/usr/bin/env bash
#
# Exercise check_private_runtime.sh's refusals, offline, with no package built
# and no private toolkit on this host.
#
# The gate reads real ELF objects, so the fixtures are real ELF objects: a
# staged tree built from what this host already carries, by
# scripts/fixtures/packaging/make_stage.sh. A host library carries no RUNPATH of
# its own, which is exactly the shape the first refusal is about, and the
# remaining cases edit that tree into each of the other shapes.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/check_private_runtime.sh"
MAKE_STAGE="$ROOT_DIR/scripts/fixtures/packaging/make_stage.sh"
PREFIX="usr/lib/fermix-desktop"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/check-private-runtime-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fail() {
  echo "check_private_runtime_test: $*" >&2
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

stage() {
  local where="$WORK/$1"
  "$MAKE_STAGE" "$where" >/dev/null || fail "the fixture stage could not be built"
  printf '%s\n' "$where"
}

echo "check_private_runtime_test: the script parses"
bash -n "$SCRIPT" || fail "the script does not parse"
python3 -c "import ast,sys; ast.parse(open(sys.argv[1]).read())" \
  "$ROOT_DIR/scripts/elf_facts.py" || fail "the ELF reader does not parse"
echo "  ok: shell and python syntax"

echo "check_private_runtime_test: the arguments"
expect_sentence "an unknown argument" "unknown argument" bash "$SCRIPT" "$WORK" --nonsense
expect_sentence "a stage that is not there" "no staged tree" bash "$SCRIPT" "$WORK/absent"
expect_sentence "a stage with no private prefix in it" "no private prefix" \
  bash "$SCRIPT" "$WORK"
expect_sentence "a lock file that is not there" "no runtime lock file" \
  bash "$SCRIPT" "$WORK" --lock "$WORK/absent.json"

echo "check_private_runtime_test: the search path, per object"

# Host objects carry no RUNPATH, which is the shape this refuses: an object that
# does not find its libraries through its own RUNPATH is an object that finds
# them through LD_LIBRARY_PATH or not at all.
BARE="$(stage bare)"
expect_sentence "a private object with no RUNPATH of its own" \
  "does not lead to the private lib directory" bash "$SCRIPT" "$BARE"

# A RUNPATH that leads to the private lib directory by a longer road is the same
# RUNPATH. The toolkit's own build system writes `$ORIGIN/../../lib` for a module
# one directory down, where `$ORIGIN/..` would do, and a gate that compared the
# text refused a correct tree; this is that case, as a real object rather than as
# an argument. A host with no compiler says so rather than passing quietly.
if command -v gcc >/dev/null 2>&1; then
  SPELLING="$(stage spelling)"
  MODULE_DIR="$SPELLING/$PREFIX/lib/cairo"
  mkdir -p "$MODULE_DIR"
  echo 'int fermix_fixture(void) { return 0; }' > "$WORK/fixture.c"
  # shellcheck disable=SC2016  # $ORIGIN is the loader's variable, not the shell's
  gcc -shared -fPIC -o "$MODULE_DIR/libfixture.so" "$WORK/fixture.c" \
    -Wl,-rpath,'$ORIGIN/../../lib' -Wl,--enable-new-dtags ||
    fail "the fixture module could not be built"
  output="$(bash "$SCRIPT" "$SPELLING" 2>&1 || true)"
  case "$output" in
    *libfixture.so*)
      fail "a RUNPATH that resolves to the private lib directory was refused for its spelling: $output"
      ;;
    *) echo "  accepted: a RUNPATH spelled the long way round, which is the same directory" ;;
  esac
else
  echo "  skipped: no compiler on this host, so the RUNPATH spelling case did not run"
fi

echo "check_private_runtime_test: the generated caches"

CACHES="$(stage caches)"
printf '"/usr/lib/x86_64-linux-gnu/gdk-pixbuf-2.0/2.10.0/loaders/libpixbufloader-png.so"\n' \
  > "$CACHES/$PREFIX/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
expect_sentence "a pixbuf cache naming a host path" \
  "which is outside the private prefix" bash "$SCRIPT" "$CACHES"

CACHES="$(stage caches-missing)"
rm "$CACHES/$PREFIX/share/glib-2.0/schemas/gschemas.compiled"
expect_sentence "a prefix with no compiled schemas in it" \
  "there is no the compiled GSettings schemas at" bash "$SCRIPT" "$CACHES"

echo "check_private_runtime_test: the application"

NO_APPLICATION="$(stage no-application)"
rm "$NO_APPLICATION/$PREFIX/bin/fermix-desktop"
expect_sentence "a prefix with no application ELF in it" \
  "there is no application ELF at" bash "$SCRIPT" "$NO_APPLICATION"

echo "check_private_runtime_test: the host boundary"

# A lock file whose host list is empty leaves every NEEDED entry outside the
# boundary, which is the refusal a library nobody decided to take would get.
NARROW="$(stage narrow)"
python3 - "$WORK/narrow-lock.json" <<'PY'
import json
import sys

with open(sys.argv[1], "w", encoding="utf-8") as handle:
    json.dump({"host_libraries": ["libnothing.so.0"], "components": []}, handle)
PY
expect_sentence "a NEEDED entry that is neither carried nor on the host list" \
  "nor on the host list" bash "$SCRIPT" "$NARROW" --lock "$WORK/narrow-lock.json"

echo "check_private_runtime_test: the engine's own objects"

# A static object outside the prefix is the engine's and is exempt; a
# dynamically linked one outside the prefix is not, and is a refusal.
STRAY="$(stage stray)"
cp /bin/sh "$STRAY/usr/bin/something-dynamic"
expect_sentence "a dynamically linked ELF outside the private prefix" \
  "every ELF in this package is either the private runtime or the engine's own" \
  bash "$SCRIPT" "$STRAY"

echo "check_private_runtime_test: every refusal fired"
