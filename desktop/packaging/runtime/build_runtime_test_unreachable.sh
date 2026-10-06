#!/usr/bin/env bash
#
# drop_unreachable.sh's own tests, run by build_runtime_test.sh.
#
# Each case is a small tree of objects compiled here with the host's gcc: a
# library the window links, the libraries it needs, a plugin directory, and
# libraries nothing needs. The script must keep the first three, drop the last
# with every link naming them, and refuse, removing nothing, when a kept file
# names a library it would drop, which is what a dlopen by name looks like.
#
# Usage: build_runtime_test_unreachable.sh <drop_unreachable.sh> <work dir>
set -euo pipefail

[ $# -eq 2 ] || { echo "usage: build_runtime_test_unreachable.sh <script> <work dir>" >&2; exit 1; }
DROP="$1"
WORK="$2"

fail() {
  echo "unreachable_test: $*" >&2
  exit 1
}

# A shared object with the given soname. Extra arguments go to gcc, after the source.
build_so() {
  local out="$1" soname="$2"
  shift 2
  printf 'int %s(void) { return 1; }\n' "f_$(basename -- "$out" | tr -c 'a-zA-Z0-9\n' _)" > "$out.c"
  gcc -shared -fPIC -o "$out" -Wl,-soname,"$soname" "$out.c" -Wl,--no-as-needed "$@"
  rm -f "$out.c"
}

# lib/libapp.so.1 is what the window links, a link to the versioned file the
# way a real library is, and needs libneed. The plugin in lib/plugins needs
# libplugdep. libunused, its link and lib/extra/libtool.so are needed by nothing.
make_tree() {
  local tree="$1"
  mkdir -p "$tree/lib/plugins" "$tree/lib/extra" "$tree/share"
  build_so "$tree/lib/libneed.so.1" libneed.so.1
  build_so "$tree/lib/libplugdep.so.1" libplugdep.so.1
  build_so "$tree/lib/libunused.so.1.0.0" libunused.so.1
  ln -s libunused.so.1.0.0 "$tree/lib/libunused.so.1"
  build_so "$tree/lib/libapp.so.1.0.0" libapp.so.1 -L"$tree/lib" -l:libneed.so.1
  ln -s libapp.so.1.0.0 "$tree/lib/libapp.so.1"
  build_so "$tree/lib/plugins/libplug.so" libplug.so -L"$tree/lib" -l:libplugdep.so.1
  build_so "$tree/lib/extra/libtool.so" libtool.so
  echo "not an object" > "$tree/share/data.txt"
}

case_arguments() {
  if bash "$DROP" "$WORK/missing" lib/libapp.so.1 >/dev/null 2>&1; then
    fail "it accepted a prefix that does not exist"
  fi
  make_tree "$WORK/args"
  if bash "$DROP" "$WORK/args" >/dev/null 2>&1; then
    fail "it accepted a run with no root"
  fi
  if bash "$DROP" "$WORK/args" lib/libmissing.so.1 >/dev/null 2>&1; then
    fail "it accepted a root that does not exist"
  fi
  if bash "$DROP" "$WORK/args" share >/dev/null 2>&1; then
    fail "it accepted a root with no object in it"
  fi
  [ -f "$WORK/args/lib/libunused.so.1.0.0" ] || fail "a refused run removed a file"
  echo "  refused: a missing prefix, no root, a missing root, a root with no object"
}

case_drop() {
  local tree="$WORK/drop" out="$WORK/drop.out" kept
  make_tree "$tree"
  bash "$DROP" "$tree" lib/libapp.so.1 lib/plugins > "$out" 2>&1 || {
    cat "$out" >&2
    fail "it refused a tree it should have pruned"
  }
  for kept in lib/libapp.so.1 lib/libapp.so.1.0.0 lib/libneed.so.1 lib/plugins/libplug.so \
              lib/libplugdep.so.1 share/data.txt; do
    [ -e "$tree/$kept" ] || fail "it dropped $kept, which is kept or needed"
  done
  for dropped in lib/libunused.so.1.0.0 lib/libunused.so.1 lib/extra/libtool.so; do
    if [ -e "$tree/$dropped" ] || [ -L "$tree/$dropped" ]; then
      fail "it kept $dropped, which nothing needs"
    fi
  done
  grep -q "dropped lib/libunused.so.1.0.0" "$out" || fail "it did not report the library it dropped"
  echo "  ok: kept the roots and what they need, dropped what nothing needs with its link"
}

# share/data.txt names libunused.so.1: the drop has to stop before removing anything.
case_named() {
  local tree="$WORK/named" err="$WORK/named.err"
  make_tree "$tree"
  echo "libunused.so.1" > "$tree/share/data.txt"
  if bash "$DROP" "$tree" lib/libapp.so.1 lib/plugins >/dev/null 2>"$err"; then
    fail "it dropped a library a kept file names"
  fi
  grep -q "share/data.txt names libunused.so.1" "$err" || {
    cat "$err" >&2
    fail "it refused, but did not name the file that names the library"
  }
  [ -f "$tree/lib/libunused.so.1.0.0" ] && [ -f "$tree/lib/extra/libtool.so" ] \
    || fail "a refused run removed a file"
  echo "  refused: a library a kept file names, as a dlopen would, with nothing removed"
}

main() {
  command -v gcc >/dev/null 2>&1 || fail "gcc is needed to build the test objects"
  mkdir -p "$WORK"
  case_arguments
  case_drop
  case_named
}

main "$@"
