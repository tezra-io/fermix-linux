#!/usr/bin/env bash
#
# Drop from a runtime tree every ELF object nothing in it needs.
#
# The roots are what is loaded from outside the NEEDED graph: the libraries the
# window links, the directories GStreamer, gdk-pixbuf and GIO load plugins from,
# and libexec, whose programs GLib and GStreamer run by path. Every object under
# a root is kept, with every library its NEEDED entries reach in the tree's lib/.
# Every other object is removed, with the links that name it.
#
# A library loaded with dlopen by name appears in no NEEDED entry. A kept file
# that names a library about to be dropped stops the run before anything is
# removed.
#
# Usage: drop_unreachable.sh <prefix directory> <root>...
#   Each root is a file or a directory, relative to the prefix.
set -euo pipefail
shopt -s inherit_errexit

# More expansions than any tree of a few hundred objects needs.
MAX_STEPS=100000

fail() {
  echo "drop_unreachable: $*" >&2
  exit 1
}

log() {
  echo "drop_unreachable: $*" >&2
}

[ $# -ge 2 ] || fail "usage: drop_unreachable.sh <prefix directory> <root>..."
PREFIX_DIR="$1"
shift
[ -d "$PREFIX_DIR" ] || fail "no prefix at $PREFIX_DIR"
for tool in readelf find od sort grep realpath; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed"
done

is_elf() {
  [ "$(od -An -tx1 -N4 -- "$1" | tr -d ' \n')" = "7f454c46" ]
}

# Every ELF object at or under a path, relative to the prefix. Shared objects by
# name and executables by mode, as check_boundary.sh finds them. A root that is
# a link, as libfoo.so.1 is, stands for the file it names.
objects_under() {
  local start candidate
  start="$(realpath -- "$PREFIX_DIR/$1")"
  while IFS= read -r -d '' candidate; do
    if is_elf "$candidate"; then
      realpath --relative-to="$PREFIX_DIR" -- "$candidate"
    fi
  done < <(find "$start" -type f \( -name '*.so' -o -name '*.so.*' -o -perm -u+x \) -print0 | sort -z)
}

# The objects in lib/ an object's NEEDED entries name, as real files. A name
# that is not in lib/ is the host's, which check_boundary.sh answers for.
needed_objects() {
  local dynamic name
  dynamic="$(readelf -d -W -- "$PREFIX_DIR/$1")" || fail "readelf could not read $1"
  while IFS= read -r name; do
    if [ -n "$name" ] && [ -e "$PREFIX_DIR/lib/$name" ]; then
      realpath --relative-to="$PREFIX_DIR" -- "$PREFIX_DIR/lib/$name"
    fi
  done < <(sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' <<< "$dynamic")
}

soname_of() {
  local soname
  soname="$(readelf -d -W -- "$PREFIX_DIR/$1" | sed -n 's/.*(SONAME).*\[\(.*\)\]/\1/p')"
  printf '%s\n' "${soname:-$(basename -- "$1")}"
}

# Fills `kept` (declared by the caller) with the roots and everything they reach.
walk_from_roots() {
  local -a queue=()
  local root found object step=0
  for root in "$@"; do
    [ -e "$PREFIX_DIR/$root" ] || fail "no root at $root"
    found="$(objects_under "$root")"
    [ -n "$found" ] || fail "the root $root holds no ELF object"
    mapfile -t -O "${#queue[@]}" queue <<< "$found"
  done
  while [ "$step" -lt "${#queue[@]}" ]; do
    [ "$step" -lt "$MAX_STEPS" ] || fail "the NEEDED graph did not close within $MAX_STEPS steps"
    object="${queue[$step]}"
    step=$((step + 1))
    [ -z "${kept[$object]:-}" ] || continue
    kept[$object]=1
    mapfile -t -O "${#queue[@]}" queue < <(needed_objects "$object")
  done
}

# Refuses when any file that stays names the soname of an object about to be
# dropped: an object that does may dlopen it, and a data file may tell one to.
refuse_named() {
  local sonames="$1" hits file name named=0 status=0
  hits="$(grep -rlF -f <(printf '%s\n' "$sonames") -- "$PREFIX_DIR")" || status=$?
  [ "$status" -le 1 ] || fail "could not search $PREFIX_DIR"
  while IFS= read -r file; do
    [ -n "$file" ] || continue
    file="$(realpath --relative-to="$PREFIX_DIR" -- "$file")"
    is_dropped "$file" && continue
    while IFS= read -r name; do
      echo "drop_unreachable: $file names $name, which nothing needs; it may dlopen it" >&2
      named=$((named + 1))
    done < <(grep -oF -f <(printf '%s\n' "$sonames") -- "$PREFIX_DIR/$file" | sort -u)
  done <<< "$hits"
  [ "$named" -eq 0 ] || fail "$named name(s) of libraries nothing needs are in files that stay; nothing was dropped"
}

is_dropped() {
  [ -n "${dropped_set[$1]:-}" ]
}

remove_dropped() {
  local object link
  while IFS= read -r -d '' link; do
    if is_dropped "$(realpath -m --relative-to="$PREFIX_DIR" -- "$link")"; then
      rm -f -- "$link"
    fi
  done < <(find "$PREFIX_DIR" -type l -print0)
  for object in "${dropped[@]}"; do
    rm -f -- "$PREFIX_DIR/$object"
    log "dropped $object"
  done
}

main() {
  local -A kept=() dropped_set=()
  local -a all=() dropped=()
  local object sonames bytes=0
  walk_from_roots "$@"
  mapfile -t all < <(objects_under .)
  for object in "${all[@]}"; do
    if [ -z "${kept[$object]:-}" ]; then
      dropped+=("$object")
      dropped_set[$object]=1
      bytes=$((bytes + $(stat -c %s -- "$PREFIX_DIR/$object")))
    fi
  done
  if [ "${#dropped[@]}" -gt 0 ]; then
    sonames="$(for object in "${dropped[@]}"; do soname_of "$object"; done)"
    refuse_named "$sonames"
    remove_dropped
  fi
  log "kept ${#kept[@]} of ${#all[@]} objects; dropped ${#dropped[@]} ($((bytes / 1024)) KiB) that no kept object needs"
}

main "$@"
