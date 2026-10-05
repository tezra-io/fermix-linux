#!/usr/bin/env bash
#
# The boundary gate's own tests, run by build_runtime_test.sh.
#
# Each case is a small prefix of objects compiled here with the host's gcc, so
# the gate is shown refusing exactly what it exists to refuse: a NEEDED entry
# that is neither private nor a declared host library, and a symbol version
# above what the oldest supported host provides. Every refusal has a passing
# control beside it, so a gate that refused everything would fail here too.
#
# Usage: build_runtime_test_boundary.sh <check_boundary.sh> <lock file> <work dir>
set -euo pipefail

[ $# -eq 3 ] || { echo "usage: build_runtime_test_boundary.sh <gate> <lock> <work dir>" >&2; exit 1; }
GATE="$1"
LOCK="$2"
WORK="$3"

fail() {
  echo "boundary_test: $*" >&2
  exit 1
}

# A shared object with the given soname. Extra arguments go to gcc, after the source.
build_so() {
  local out="$1" soname="$2" source="$3"
  shift 3
  printf '%s\n' "$source" > "$out.c"
  gcc -shared -fPIC -o "$out" -Wl,-soname,"$soname" "$out.c" "$@"
  rm -f "$out.c"
}

# A library that defines one symbol under the given version node, the way glibc
# and libstdc++ define theirs.
build_versioned() {
  local out="$1" soname="$2" node="$3" symbol="$4"
  printf '%s { global: %s; local: *; };\n' "$node" "$symbol" > "$out.map"
  build_so "$out" "$soname" "int $symbol(void) { return 1; }" -Wl,--version-script="$out.map"
  rm -f "$out.map"
}

gate_passes() {
  local prefix="$1" err="$WORK/gate.err"
  bash "$GATE" "$prefix" "$LOCK" >/dev/null 2>"$err" || {
    cat "$err" >&2
    fail "the gate refused a clean prefix at $prefix"
  }
}

# The gate must refuse, and its message must name the offending object and need.
gate_refuses() {
  local prefix="$1" object="$2" need="$3" err="$WORK/gate.err"
  if bash "$GATE" "$prefix" "$LOCK" >/dev/null 2>"$err"; then
    fail "the gate passed $object, which needs $need"
  fi
  grep -q "$object needs $need" "$err" || {
    cat "$err" >&2
    fail "the gate refused, but did not name $object needing $need"
  }
}

case_arguments() {
  mkdir -p "$WORK/empty"
  if bash "$GATE" "$WORK/missing" "$LOCK" >/dev/null 2>&1; then
    fail "the gate accepted a prefix that does not exist"
  fi
  if bash "$GATE" "$WORK/empty" "$WORK/missing.json" >/dev/null 2>&1; then
    fail "the gate accepted a lock file that does not exist"
  fi
  # A gate that found nothing to check has checked nothing.
  if bash "$GATE" "$WORK/empty" "$LOCK" >/dev/null 2>&1; then
    fail "the gate passed a prefix with no objects in it"
  fi
  echo "  refused: a missing prefix, a missing lock file, a prefix with no objects"
}

case_needed() {
  local clean="$WORK/clean" planted="$WORK/planted"
  mkdir -p "$clean/lib/gdk-pixbuf-2.0/2.10.0/loaders" "$WORK/outside"
  build_so "$clean/lib/libok.so.1" libok.so.1 \
    '#include <string.h>
     unsigned long ok(const char *s) { return strlen(s); }'
  # A module in a subdirectory, needing a private library and the host's libc.
  build_so "$clean/lib/gdk-pixbuf-2.0/2.10.0/loaders/libpixbufloader-test.so" \
    libpixbufloader-test.so 'unsigned long ok(const char *); unsigned long use(void) { return ok("x"); }' \
    -L"$clean/lib" -l:libok.so.1
  echo "not an object" > "$clean/lib/README"
  gate_passes "$clean"

  # The plant: a library linked against one that is neither private nor on the host list.
  cp -a "$clean" "$planted"
  build_so "$WORK/outside/libfakehost.so.1" libfakehost.so.1 'int fake_host(void) { return 1; }'
  build_so "$planted/lib/libplant.so" libplant.so \
    'int fake_host(void); int plant(void) { return fake_host(); }' -L"$WORK/outside" -l:libfakehost.so.1
  gate_refuses "$planted" "lib/libplant.so" "libfakehost.so.1"
  rm "$planted/lib/libplant.so"
  gate_passes "$planted"
  echo "  ok: private and host NEEDED pass; an undeclared one is refused, and passes once removed"
}

# A versioned library inside the prefix keeps NEEDED out of the way, so only the
# symbol version is under test. The library that defines the version is not
# refused for defining it, only the one that needs it.
case_version() {
  local node="$1" verdict="$2" prefix="$WORK/version-$1"
  mkdir -p "$prefix/lib"
  build_versioned "$prefix/lib/libdefines.so.1" libdefines.so.1 "$node" defined
  gate_passes "$prefix"
  build_so "$prefix/lib/libneeds.so" libneeds.so \
    'int defined(void); int needs(void) { return defined(); }' -L"$prefix/lib" -l:libdefines.so.1
  if [ "$verdict" = "accepted" ]; then
    gate_passes "$prefix"
  else
    gate_refuses "$prefix" "lib/libneeds.so" "$node"
  fi
  echo "  ok: an object needing $node is $verdict"
}

main() {
  command -v gcc >/dev/null 2>&1 || fail "gcc is not installed"
  [ -f "$GATE" ] || fail "no gate at $GATE"
  rm -rf "$WORK"
  mkdir -p "$WORK"
  case_arguments
  case_needed
  case_version GLIBC_2.34 accepted
  case_version GLIBC_2.35 refused
  case_version GLIBCXX_3.4.30 accepted
  case_version GLIBCXX_3.4.31 refused
  case_version CXXABI_1.3.13 accepted
  case_version CXXABI_1.3.14 refused
}

main "$@"
