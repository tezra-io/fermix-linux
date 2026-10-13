#!/usr/bin/env bash
#
# The private runtime's host boundary, as a gate.
#
# Every NEEDED entry of every object under the prefix must name a library that
# is inside the prefix's lib/ or on the lock file's host_libraries list, and no
# object may need a symbol version newer than the oldest supported host has:
# glibc_floor for GLIBC_, glibcxx_ceiling for GLIBCXX_ and cxxabi_ceiling for
# CXXABI_. Anything else is a library or a symbol the package expects a user's
# machine to have without saying so.
#
# What it cannot see is a library loaded with dlopen, which appears in no NEEDED
# entry. The smoke's GL pass is what covers libepoxy's.
#
# Usage: check_boundary.sh <prefix directory> <RUNTIME.lock.json>
set -euo pipefail
shopt -s inherit_errexit

fail() {
  echo "check_boundary: $*" >&2
  exit 1
}

[ $# -eq 2 ] || fail "usage: check_boundary.sh <prefix directory> <RUNTIME.lock.json>"
PREFIX_DIR="$1"
LOCK_FILE="$2"
[ -d "$PREFIX_DIR" ] || fail "no prefix at $PREFIX_DIR"
[ -f "$LOCK_FILE" ] || fail "no lock file at $LOCK_FILE"
for tool in readelf jq find od sort; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed"
done

is_elf() {
  [ "$(od -An -tx1 -N4 -- "$1" | tr -d ' \n')" = "7f454c46" ]
}

# Shared objects by name and executables by mode: the icon theme alone is
# thousands of files, and none of them is either.
elf_files() {
  local candidate
  while IFS= read -r -d '' candidate; do
    if is_elf "$candidate"; then
      printf '%s\0' "$candidate"
    fi
  done < <(find "$PREFIX_DIR" -type f \( -name '*.so' -o -name '*.so.*' -o -perm -u+x \) -print0 | sort -z)
}

needed_entries() {
  sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' <<< "$1"
}

# Only the version-needs section: a library that defines a version node is not
# needing it.
needed_versions() {
  awk '/^Version needs section/ { needs = 1; next }
       /^Version (definition|symbols) section/ { needs = 0 }
       needs && /Name: / { for (i = 1; i < NF; i++) if ($i == "Name:") print $(i + 1) }' <<< "$1"
}

# True when version $1 sorts above $2.
above() {
  [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n 1)" = "$1" ]
}

# Prints one line per violation in one object, and nothing when it is clean.
object_violations() {
  local object="$1" hosts="$2" glibc="$3" glibcxx="$4" cxxabi="$5"
  local dump name version ceiling rel="${1#"$PREFIX_DIR"/}"
  dump="$(readelf -d -V -W -- "$object")" || fail "readelf could not read $rel"
  while IFS= read -r name; do
    [ -n "$name" ] || continue
    [ -e "$PREFIX_DIR/lib/$name" ] && continue
    case "$hosts" in *" $name "*) continue ;; esac
    echo "$rel needs $name, which is neither private nor a declared host library"
  done < <(needed_entries "$dump")
  while IFS= read -r version; do
    case "$version" in
      GLIBC_[0-9]*) ceiling="GLIBC_$glibc" ;;
      GLIBCXX_[0-9]*) ceiling="GLIBCXX_$glibcxx" ;;
      CXXABI_[0-9]*) ceiling="CXXABI_$cxxabi" ;;
      *) continue ;;
    esac
    if above "${version#*_}" "${ceiling#*_}"; then
      echo "$rel needs $version, above the oldest supported host's $ceiling"
    fi
  done < <(needed_versions "$dump")
}

main() {
  local hosts glibc glibcxx cxxabi object report checked=0 violations=0
  hosts=" $(jq -r '.host_libraries[]' "$LOCK_FILE" | tr '\n' ' ') "
  glibc="$(jq -er '.glibc_floor' "$LOCK_FILE")" || fail "the lock file has no glibc_floor"
  glibcxx="$(jq -er '.glibcxx_ceiling' "$LOCK_FILE")" || fail "the lock file has no glibcxx_ceiling"
  cxxabi="$(jq -er '.cxxabi_ceiling' "$LOCK_FILE")" || fail "the lock file has no cxxabi_ceiling"
  while IFS= read -r -d '' object; do
    checked=$((checked + 1))
    report="$(object_violations "$object" "$hosts" "$glibc" "$glibcxx" "$cxxabi")"
    [ -n "$report" ] || continue
    echo "$report" >&2
    violations=$((violations + $(wc -l <<< "$report")))
  done < <(elf_files)
  # A gate that found nothing to look at has checked nothing.
  [ "$checked" -gt 0 ] || fail "no ELF objects under $PREFIX_DIR"
  [ "$violations" -eq 0 ] || fail "$violations boundary violations in $checked objects"
  echo "check_boundary: $checked objects, every NEEDED private or a declared host library, no symbol version above the floor" >&2
}

main "$@"
