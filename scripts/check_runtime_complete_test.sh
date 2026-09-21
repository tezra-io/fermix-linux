#!/usr/bin/env bash
#
# Exercise check_runtime_complete.sh's refusals, offline, against trees built
# here rather than against a real package.
#
# Every case is built from a fixture so that each refusal is provable without a
# build, and so that the gate is shown to REFUSE as well as to pass. A gate only
# ever observed passing is indistinguishable from a gate that cannot fail, which
# is the mistake this whole area of the tree keeps producing: the first draft of
# the gate under test passed a tree it had never read, because its own file
# search resolved no paths and an empty result reads like a clean one.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$ROOT_DIR/scripts/check_runtime_complete.sh"
PREFIX_REL="usr/lib/fermix-desktop"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/runtime-complete-test.XXXXXX")"
trap 'rm -rf -- "$WORK"' EXIT

fail() {
  echo "check_runtime_complete_test: $*" >&2
  exit 1
}

# A tree with one library that mentions the prefix, plus whatever extra files
# the caller names. The library's text is what makes the gate's own positive
# control pass, so a case that wants the control to fire simply omits it.
make_tree() {
  local name="$1"
  shift
  local tree="$WORK/$name"

  mkdir -p "$tree/$PREFIX_REL/lib" "$tree/$PREFIX_REL/libexec"
  printf 'nothing to see' > "$tree/$PREFIX_REL/lib/libplain.so"

  for extra in "$@"; do
    mkdir -p "$(dirname "$tree/$PREFIX_REL/$extra")"
    printf 'x' > "$tree/$PREFIX_REL/$extra"
  done

  printf '%s\n' "$tree"
}

# A library carrying a compiled-in absolute path, as a real one does.
plant_compiled_path() {
  local tree="$1"
  local path="$2"
  printf 'prologue\0/usr/lib/fermix-desktop/%s\0epilogue' "$path" \
    > "$tree/$PREFIX_REL/lib/libwithpath.so"
}

# A runtime listing naming exactly the given prefix-relative entries.
make_listing() {
  local name="$1"
  shift
  local listing="$WORK/$name.txt"

  : > "$listing"
  for entry in "$@"; do
    printf '%s/%s\n' "$PREFIX_REL" "$entry" >> "$listing"
  done

  printf '%s\n' "$listing"
}

run_gate() {
  "$GATE" "$@" > "$WORK/out.txt" 2> "$WORK/err.txt"
}

# ---- a complete tree passes ------------------------------------------------
tree="$(make_tree complete libexec/gio-launch-desktop)"
plant_compiled_path "$tree" "libexec/gio-launch-desktop"
listing="$(make_listing complete lib/libplain.so lib/libwithpath.so libexec/gio-launch-desktop)"

run_gate "$tree" --runtime "$listing" ||
  fail "a tree carrying every runtime file was refused: $(cat "$WORK/err.txt")"
echo "  ok: a complete tree passes"

# ---- the real bug: a runtime file the tree does not have -------------------
tree="$(make_tree dropped)"
plant_compiled_path "$tree" "libexec/gio-launch-desktop"
listing="$(make_listing dropped lib/libplain.so lib/libwithpath.so libexec/gio-launch-desktop)"

if run_gate "$tree" --runtime "$listing"; then
  fail "a tree missing a file the runtime installed was accepted"
fi
grep -q "libexec/gio-launch-desktop" "$WORK/err.txt" ||
  fail "the refusal did not name the missing file"
echo "  refused: a file the runtime installed and the package does not carry"

# ---- a compiled-in path the runtime produced and the tree lacks ------------
#
# The same file, reached by the other claim: here the runtime listing carries it
# and so does the compiled path, and the refusal must name it as compiled in.
grep -q "compiled into shipped objects" "$WORK/err.txt" ||
  fail "the refusal did not report the missing path as one compiled into an object"
echo "  refused: a path compiled into a shipped object that is missing"

# ---- the invented-negative control -----------------------------------------
#
# A path that is compiled in but that the runtime never produced is optional --
# GLib and fontconfig probe several such. It must NOT be demanded, or the gate
# refuses every correct package.
tree="$(make_tree optional)"
plant_compiled_path "$tree" "var/cache/fontconfig"
listing="$(make_listing optional lib/libplain.so lib/libwithpath.so)"

run_gate "$tree" --runtime "$listing" ||
  fail "a compiled-in path the runtime never produced was demanded: $(cat "$WORK/err.txt")"
grep -q "optional and not required" "$WORK/out.txt" ||
  fail "the optional path was not reported for the record"
echo "  ok: a compiled-in path the runtime never produced is reported, not demanded"

# ---- an invented path is never found ---------------------------------------
#
# The control that proves the search is searching: a path no object contains
# must not turn up in the report at all.
grep -q "libexec/gio-launch-dezktop" "$WORK/out.txt" "$WORK/err.txt" 2>/dev/null &&
  fail "the gate reported a path that is in no object"
echo "  ok: a path no object carries is not reported"

# ---- the gate's own positive control ---------------------------------------
#
# A tree where nothing mentions the prefix means the reading failed, not that
# the tree is clean. The gate must refuse rather than pass, which is exactly the
# bug its first draft had.
tree="$WORK/unread"
mkdir -p "$tree/$PREFIX_REL/lib"
printf 'no absolute paths here' > "$tree/$PREFIX_REL/lib/libquiet.so"
listing="$(make_listing unread lib/libquiet.so)"

if run_gate "$tree" --runtime "$listing"; then
  fail "a tree in which nothing mentions the prefix was accepted as clean"
fi
grep -q "did not read the tree" "$WORK/err.txt" ||
  fail "the refusal did not say that the gate failed to read the tree"
echo "  refused: a tree the gate could not read is not a tree that is clean"

# ---- arguments -------------------------------------------------------------
if run_gate "$WORK/does-not-exist" --runtime "$listing"; then
  fail "a stage directory that does not exist was accepted"
fi
echo "  refused: a stage directory that does not exist"

if run_gate "$tree" --runtime "$WORK/no-such-listing"; then
  fail "a runtime listing that does not exist was accepted"
fi
echo "  refused: a runtime listing that does not exist"

if run_gate "$tree"; then
  fail "a run with no runtime named was accepted"
fi
echo "  refused: no runtime named"

empty="$WORK/empty.txt"
: > "$empty"
if run_gate "$tree" --runtime "$empty"; then
  fail "an empty runtime listing was accepted"
fi
echo "  refused: a runtime listing naming nothing under the prefix"

echo "check_runtime_complete_test: every case holds"
