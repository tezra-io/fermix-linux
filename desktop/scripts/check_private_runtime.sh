#!/usr/bin/env bash
#
# The private GTK runtime in a staged fermix-desktop tree is wired the way A§4.3 says.
#
#   1. Every ELF object under /usr/lib/fermix-desktop finds the private lib/ through its own
#      RUNPATH: the relative path from where it sits, $ORIGIN in lib/ and $ORIGIN/../lib in bin/
#      and libexec/, one more ../ per directory deeper, which is the path the runtime's build
#      writes. Never an RPATH, which LD_LIBRARY_PATH cannot override, so a person replacing an
#      LGPL library could not.
#   2. No private object names a host GTK, GLib or pango: each one an object needs is carried in
#      the prefix, and the lock's host list names none. Two toolkits in one process is the failure
#      the private runtime exists to prevent.
#   3. Every NEEDED is private or a declared host library, and no object needs a symbol version
#      above the oldest host's. That is desktop/packaging/runtime/check_boundary.sh, run here over
#      the staged prefix, the window included.
#   4. The window needs no LD_LIBRARY_PATH: ldd, run with an empty environment, resolves every
#      library it needs, and every private one from the staged prefix.
#
# Usage: check_private_runtime.sh <staged tree> <RUNTIME.lock.json>
set -euo pipefail
shopt -s inherit_errexit

PREFIX=usr/lib/fermix-desktop
WINDOW=bin/fermix-desktop
# The libraries a host GTK, GLib or pango is made of, as SONAME prefixes.
TOOLKIT='^lib(gtk-4|gdk-|adwaita-1|glib-2\.0|gobject-2\.0|gio-2\.0|gmodule-2\.0|gthread-2\.0|girepository-|pango-1\.0|pangocairo-1\.0|pangoft2-1\.0)'
here=$(cd "$(dirname "$0")" && pwd)

fail() {
  echo "check_private_runtime: $*" >&2
  exit 1
}

is_elf() {
  [ "$(od -An -tx1 -N4 -- "$1" | tr -d ' \n')" = "7f454c46" ]
}

# Every regular ELF file under the prefix, NUL-separated, in a stable order.
elf_files() {
  local candidate
  while IFS= read -r -d '' candidate; do
    if is_elf "$candidate"; then
      printf '%s\0' "$candidate"
    fi
  done < <(find "$1" -type f -print0 | sort -z)
}

# What the runtime's build writes for an object at <dir>, relative to the prefix.
runpath_for() {
  local dir="$1" up="" part
  # shellcheck disable=SC2016 # $ORIGIN is the dynamic linker's token
  [ "$dir" != lib ] || { echo '$ORIGIN'; return 0; }
  IFS=/ read -ra parts <<< "$dir"
  for part in "${parts[@]}"; do
    [ -n "$part" ] && up="$up../"
  done
  echo "\$ORIGIN/${up}lib"
}

dynamic_field() {
  sed -n "s/.*($2).*\[\(.*\)\]/\1/p" <<< "$1"
}

check_lock_hosts() {
  local lock="$1" soname
  while IFS= read -r soname; do
    if grep -qE "$TOOLKIT" <<< "$soname"; then
      fail "the lock's host_libraries names $soname, which must be private"
    fi
  done < <(jq -r '.host_libraries[]' "$lock")
}

# Claims 1 and 2 for one object.
check_object() {
  local prefix_dir="$1" object="$2" rel dump want have soname
  rel="${object#"$prefix_dir"/}"
  dump="$(readelf -d -W -- "$object")" || fail "readelf could not read $PREFIX/$rel"
  if [ -n "$(dynamic_field "$dump" RPATH)" ]; then
    fail "$PREFIX/$rel carries an RPATH, which LD_LIBRARY_PATH cannot override"
  fi
  want="$(runpath_for "$(dirname -- "$rel")")"
  have="$(dynamic_field "$dump" RUNPATH)"
  [ "$have" = "$want" ] ||
    fail "$PREFIX/$rel has RUNPATH '$have', and where it sits it must be '$want'"
  while IFS= read -r soname; do
    [ -n "$soname" ] || continue
    grep -qE "$TOOLKIT" <<< "$soname" || continue
    [ -e "$prefix_dir/lib/$soname" ] ||
      fail "$PREFIX/$rel needs $soname, and the private prefix does not carry it: it would load the host's"
  done < <(dynamic_field "$dump" NEEDED)
}

# Claim 4: ldd with nothing in the environment, so no LD_LIBRARY_PATH can help.
check_window() {
  local prefix_dir="$1" window="$1/$WINDOW" report name path
  report="$(env -i "$(command -v ldd)" "$window" 2>&1)" ||
    fail "ldd cannot resolve the window with no LD_LIBRARY_PATH: $report"
  if grep -q 'not found' <<< "$report"; then
    fail "ldd cannot resolve the window with no LD_LIBRARY_PATH: $(grep 'not found' <<< "$report")"
  fi
  while read -r name path; do
    [ -e "$prefix_dir/lib/$name" ] || continue
    [ "$(realpath -- "$path")" = "$(realpath -- "$prefix_dir/lib/$name")" ] ||
      fail "the window loads $name from $path, not from the private prefix"
  done < <(sed -n 's/^[[:space:]]*\([^ ]*\) => \([^ ]*\) (0x.*/\1 \2/p' <<< "$report")
}

main() {
  [ $# -eq 2 ] || fail "usage: check_private_runtime.sh <staged tree> <RUNTIME.lock.json>"
  local root="$1" lock="$2" prefix_dir object checked=0
  prefix_dir="$root/$PREFIX"
  [ -d "$prefix_dir" ] || fail "no private prefix at $prefix_dir"
  [ -f "$lock" ] || fail "no lock file at $lock"
  [ -f "$prefix_dir/$WINDOW" ] || fail "no window at /$PREFIX/$WINDOW in $root"
  for tool in readelf jq od ldd realpath; do
    command -v "$tool" > /dev/null 2>&1 || fail "$tool is not installed"
  done
  check_lock_hosts "$lock"
  while IFS= read -r -d '' object; do
    check_object "$prefix_dir" "$object"
    checked=$((checked + 1))
  done < <(elf_files "$prefix_dir")
  bash "$here/../packaging/runtime/check_boundary.sh" "$prefix_dir" "$lock" ||
    fail "the staged prefix crosses the host boundary"
  check_window "$prefix_dir"
  echo "check_private_runtime: $checked private objects, each RUNPATH relative to where it sits," \
    "no host toolkit, and the window resolved by ldd with no LD_LIBRARY_PATH"
}

main "$@"
