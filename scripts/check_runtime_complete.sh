#!/usr/bin/env bash
#
# Everything the private runtime installs reaches the staged tree, and every
# absolute prefix path compiled into a shipped object exists there.
#
# This gate exists because of a bug it would have caught. The nFPM template
# staged the runtime's `lib/` and `share/` as trees and named `bin/fermix-desktop`
# as a file. The runtime also installs `libexec/`, and nothing carried it, so
# `gio-launch-desktop` and `dconf-service` were absent from every package we
# ever built. GLib spawns that helper for `g_app_info_launch_default_for_uri`,
# so on an installed machine every URL the window opened failed before a browser
# was ever consulted, and the window showed its address fallback instead. The
# owner reported it as "the browser never opens".
#
# Nothing else noticed, and that is the point worth writing down. The staged
# tree was correct, the package was internally consistent, every ELF resolved
# every library it named, and the relations gate held the declared dependencies
# equal to what the binaries needed. A helper that is spawned by path is not a
# `NEEDED` entry, so the one gate that reads what binaries require could not see
# it. A missing file in a directory nobody named is invisible to every check
# that starts from what the package contains.
#
# So this gate starts from the other end -- from what the runtime produced and
# from what the libraries were compiled to look for -- and asks whether the tree
# we are about to package has it.
#
# TWO CLAIMS, because one of them alone would have missed half of this bug:
#
#   1. **Every file the runtime tree installs is in the staged tree.** This is
#      what catches `dconf-service`, which no ELF names: it is started over
#      D-Bus, so a check that reads compiled-in strings never sees it. The
#      runtime manifest is the list of what the runtime build produced, and the
#      package carries the runtime, so the two are the same set by definition.
#   2. **Every absolute prefix path compiled into a shipped object exists.**
#      This is what catches `gio-launch-desktop` the moment a library is built
#      to spawn something at a path, whether or not the runtime tree is the
#      thing that was supposed to provide it.
#
# A path a library looks for is not always a path that must exist: GLib, pango
# and fontconfig all probe optional locations under their prefix and behave
# correctly when they are absent (`etc/pango`, `var/cache/fontconfig`,
# `var/lib/dbus/machine-id` and the rest). Claim 2 would be a wall of false
# refusals if it demanded all of them. So it demands exactly the paths the
# runtime tree actually produced: if the runtime built it and a library was
# compiled to find it there, it must ship. A compiled path that the runtime
# never produced is reported for the record and refuses nothing.
#
# WHERE THIS HAS TO LOOK, which is the second thing the bug taught. The staged
# tree was never wrong: staging copied the runtime whole, `libexec/` included.
# The loss happened at the nFPM step, because the template named the trees to
# carry and `libexec` was not among them. A gate that read the staged tree would
# have passed every build that shipped the defect. So the subject is the built
# package, and the stage is accepted only as a convenience for checking before
# one exists.
#
# Usage:
#   check_runtime_complete.sh --package <deb> --runtime <tarball or listing>
#   check_runtime_complete.sh <stage>        --runtime <tarball or listing>
#   ... --quiet
set -euo pipefail

PREFIX="/usr/lib/fermix-desktop"

STAGE=""
PACKAGE=""
RUNTIME=""
QUIET=0

fail() {
  echo "check_runtime_complete: $*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --runtime)
      [ "$#" -ge 2 ] || fail "--runtime needs the runtime tarball or a listing"
      RUNTIME="$2"
      shift 2
      ;;
    --package)
      [ "$#" -ge 2 ] || fail "--package needs a .deb"
      PACKAGE="$2"
      shift 2
      ;;
    --quiet)
      QUIET=1
      shift
      ;;
    -*)
      fail "unknown argument: $1"
      ;;
    *)
      [ -z "$STAGE" ] || fail "only one stage directory"
      STAGE="$1"
      shift
      ;;
  esac
done

[ -n "$RUNTIME" ] || fail "usage: check_runtime_complete.sh --package <deb> --runtime <tarball>"
[ -e "$RUNTIME" ] || fail "no runtime at $RUNTIME"

if [ -n "$PACKAGE" ]; then
  [ -z "$STAGE" ] || fail "name a package or a stage, not both"
  [ -f "$PACKAGE" ] || fail "no package at $PACKAGE"
else
  [ -n "$STAGE" ] || fail "usage: check_runtime_complete.sh --package <deb> --runtime <tarball>"
  [ -d "$STAGE" ] || fail "no staged tree at $STAGE"
fi

say() {
  [ "$QUIET" = "1" ] || echo "$@"
}

WORK="$(mktemp -d "${TMPDIR:-/tmp}/runtime-complete.XXXXXX")"
trap 'rm -rf -- "$WORK"' EXIT

# ---- the subject ----------------------------------------------------------
#
# A package is unpacked so that both claims read the same bytes a user installs.
if [ -n "$PACKAGE" ]; then
  command -v dpkg-deb >/dev/null 2>&1 ||
    fail "dpkg-deb is needed to read $PACKAGE; run this where it exists"
  mkdir -p "$WORK/subject"
  dpkg-deb -x "$PACKAGE" "$WORK/subject" ||
    fail "could not unpack $PACKAGE"
  STAGE="$WORK/subject"
fi

# ---- what the runtime installed ------------------------------------------
#
# A tarball is listed; a plain file is taken as a listing already. Both are
# reduced to prefix-relative paths with no trailing slashes, so a directory and
# a file compare the same way.
case "$RUNTIME" in
  *.tar|*.tar.gz|*.tgz)
    tar -tf "$RUNTIME" ;;
  *)
    cat "$RUNTIME" ;;
esac | sed 's|/$||' | grep "^usr/lib/fermix-desktop/" | sort -u > "$WORK/runtime.txt" || true

[ -s "$WORK/runtime.txt" ] ||
  fail "the runtime listing named no files under the prefix; it is the wrong file, or empty"

# ---- what the staged tree has --------------------------------------------
( cd "$STAGE" && find "usr/lib/fermix-desktop" -mindepth 1 \( -type f -o -type l -o -type d \) 2>/dev/null ) \
  | sed 's|/$||' | sort -u > "$WORK/staged.txt" || true

[ -s "$WORK/staged.txt" ] || fail "the staged tree has nothing under $PREFIX"

# ---- claim 1: nothing the runtime installed is missing --------------------
comm -23 "$WORK/runtime.txt" "$WORK/staged.txt" > "$WORK/dropped.txt"

# ---- claim 2: compiled-in prefix paths that the runtime produced ----------
#
# Read from the staged tree's own objects, because those are the bytes that
# ship. Paths are collected from every regular file: a compiled-in path is a
# string in the object whatever the file's mode happens to be.
# The whole pipeline runs inside the stage directory. Running only `find` there
# and letting `xargs grep` run elsewhere hands grep relative paths that do not
# resolve from the caller's directory: every open fails, stderr is discarded,
# the result is empty, and an empty result reads exactly like "no compiled-in
# paths anywhere", which is a pass. This gate was written with that bug in it
# and passed a tree it had not read.
( cd "$STAGE" && find "usr/lib/fermix-desktop" -type f -print0 2>/dev/null \
  | xargs -0 -r grep -aoh "$PREFIX/[A-Za-z0-9._/+-]*" 2>/dev/null ) \
  | sort -u > "$WORK/compiled.txt" || true

# The gate's own positive control. A private runtime is hundreds of shared
# objects built against this absolute prefix; if not one of them mentions it,
# the reading failed rather than the tree being clean, and a gate that cannot
# tell those apart is worse than no gate.
compiled_total="$(wc -l < "$WORK/compiled.txt" | tr -d ' ')"
[ "$compiled_total" -gt 0 ] ||
  fail "no object under $PREFIX mentions the prefix at all; this gate did not read the tree"

: > "$WORK/missing_compiled.txt"
: > "$WORK/optional.txt"

while read -r path; do
  [ -n "$path" ] || continue
  relative="${path#/}"

  if [ -e "$STAGE/$relative" ]; then
    continue
  fi

  # Produced by the runtime and absent here: a drop, and a refusal.
  if grep -qxF "$relative" "$WORK/runtime.txt"; then
    printf '%s\n' "$path" >> "$WORK/missing_compiled.txt"
  else
    printf '%s\n' "$path" >> "$WORK/optional.txt"
  fi
done < "$WORK/compiled.txt"

# ---- the verdict ----------------------------------------------------------
dropped_count="$(wc -l < "$WORK/dropped.txt" | tr -d ' ')"
missing_count="$(wc -l < "$WORK/missing_compiled.txt" | tr -d ' ')"
optional_count="$(wc -l < "$WORK/optional.txt" | tr -d ' ')"

say "check_runtime_complete: $(wc -l < "$WORK/runtime.txt" | tr -d ' ') runtime files, $(wc -l < "$WORK/compiled.txt" | tr -d ' ') compiled-in prefix paths"

if [ "$optional_count" -gt 0 ]; then
  say "  $optional_count compiled-in path(s) the runtime never produced, so optional and not required:"
  while read -r path; do
    say "    $path"
  done < "$WORK/optional.txt"
fi

if [ "$dropped_count" = "0" ] && [ "$missing_count" = "0" ]; then
  say "  every runtime file is staged, and every compiled-in path the runtime produced exists"
  exit 0
fi

if [ "$dropped_count" != "0" ]; then
  echo "check_runtime_complete: the runtime installed $dropped_count file(s) the staged tree does not have:" >&2
  while read -r path; do
    echo "    /$path" >&2
  done < "$WORK/dropped.txt"
fi

if [ "$missing_count" != "0" ]; then
  echo "check_runtime_complete: $missing_count path(s) are compiled into shipped objects and are missing:" >&2
  while read -r path; do
    echo "    $path" >&2
  done < "$WORK/missing_compiled.txt"
  echo "  A library spawns or opens these by absolute path. Nothing links them, so no" >&2
  echo "  relation and no NEEDED entry names them, and their absence surfaces only as" >&2
  echo "  a feature that quietly does nothing on an installed machine." >&2
fi

exit 1
