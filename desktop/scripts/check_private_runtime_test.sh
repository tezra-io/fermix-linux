#!/usr/bin/env bash
# Offline tests for check_private_runtime.sh. Each case is a small private prefix whose objects are
# compiled here with the host's gcc, so a RUNPATH, an RPATH or a NEEDED entry is exactly the one the
# case names. Nothing here reaches the network or Docker.
#   desktop/scripts/check_private_runtime_test.sh
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
umask 022
PREFIX=usr/lib/fermix-desktop
# shellcheck disable=SC2016 # $ORIGIN is the dynamic linker's token, written literally
ORIGIN='$ORIGIN'

fail() {
  echo "check_private_runtime_test: $*" >&2
  exit 1
}

expect_refusal() {
  local what="$1" reason="$2" err="$work/stderr"
  shift 2
  if "$@" > /dev/null 2> "$err"; then
    fail "$what was accepted"
  fi
  grep -qF -- "$reason" "$err" || fail "$what was refused for another reason: $(cat "$err")"
  echo "  refused: $what"
}

command -v gcc > /dev/null || fail "gcc is needed to compile the fixture objects"
printf 'int private_value(void) { return 1; }\n' > "$work/private.c"
printf 'int main(void) { return 0; }\n' > "$work/main.c"
printf 'int private_value(void);\nint main(void) { return private_value() - 1; }\n' > "$work/window.c"

# A shared object: <out> <soname> <runpath> [linker arguments...]. --no-as-needed keeps a NEEDED
# entry for a library no symbol is taken from, which is all a case needs.
shared() {
  local out="$1" soname="$2" runpath="$3"
  shift 3
  gcc -shared -fPIC -o "$out" "$work/private.c" "-Wl,-soname,$soname" \
    "-Wl,-rpath,$runpath" -Wl,--enable-new-dtags -Wl,--no-as-needed "$@"
}

# An executable: <out> <runpath> [linker arguments...]
executable() {
  local out="$1" runpath="$2"
  shift 2
  gcc -o "$out" "$work/window.c" "-Wl,-rpath,$runpath" -Wl,--enable-new-dtags -Wl,--no-as-needed "$@"
}

# A library outside every prefix, as a host would have it: <soname>
host_library() {
  mkdir -p "$work/host"
  gcc -shared -fPIC -o "$work/host/$1" "$work/private.c" "-Wl,-soname,$1"
}

lock() {
  printf '{"glibc_floor": "2.35", "glibcxx_ceiling": "3.4.30", "cxxabi_ceiling": "1.3.13",
  "host_libraries": ["libc.so.6", "ld-linux-x86-64.so.2"%s]}\n' "${2:-}" > "$1"
}

# A good staged tree: a library, a GIO module three directories down, a libexec program, and the
# window, which links the library.
good_root() {
  local root="$1" prefix="$1/$PREFIX"
  mkdir -p "$prefix/lib/gio/modules" "$prefix/bin" "$prefix/libexec" "$root/usr/bin"
  shared "$prefix/lib/libprivate.so.1.0" libprivate.so.1 "$ORIGIN"
  ln -s libprivate.so.1.0 "$prefix/lib/libprivate.so.1"
  shared "$prefix/lib/gio/modules/libmodule.so" libmodule.so "$ORIGIN/../../../lib"
  gcc -o "$prefix/libexec/helper" "$work/main.c" "-Wl,-rpath,$ORIGIN/../lib" -Wl,--enable-new-dtags
  executable "$prefix/bin/fermix-desktop" "$ORIGIN/../lib" -L"$prefix/lib" -l:libprivate.so.1
  ln -s ../lib/fermix-desktop/bin/fermix-desktop "$root/usr/bin/fermix-desktop"
}

variant() {
  cp -a "$work/good" "$work/$1"
  echo "$work/$1"
}

check() {
  "$here/check_private_runtime.sh" "$1" "${2:-$work/lock.json}"
}

lock "$work/lock.json"
good_root "$work/good"

echo "check_private_runtime_test: a staged tree wired as the design says"
check "$work/good" > "$work/good.out" 2>&1 || fail "the good tree was refused: $(cat "$work/good.out")"
grep -qF "4 private objects" "$work/good.out" || fail "the good tree was not read whole: $(cat "$work/good.out")"
echo "  ok: $(tail -n 1 "$work/good.out")"

echo "check_private_runtime_test: a RUNPATH that leads somewhere else"
root="$(variant wrong-library-runpath)"
shared "$root/$PREFIX/lib/libprivate.so.1.0" libprivate.so.1 "$ORIGIN/../lib"
expect_refusal "a library whose RUNPATH is the window's" \
  "$PREFIX/lib/libprivate.so.1.0 has RUNPATH '$ORIGIN/../lib', and where it sits it must be '$ORIGIN'" \
  check "$root"
root="$(variant wrong-module-runpath)"
shared "$root/$PREFIX/lib/gio/modules/libmodule.so" libmodule.so "$ORIGIN/../lib"
expect_refusal "a module one directory short" "must be '$ORIGIN/../../../lib'" check "$root"
root="$(variant wrong-window-runpath)"
executable "$root/$PREFIX/bin/fermix-desktop" /usr/lib/fermix-desktop/lib -L"$root/$PREFIX/lib" -l:libprivate.so.1
expect_refusal "a window with an absolute RUNPATH" \
  "$PREFIX/bin/fermix-desktop has RUNPATH '/usr/lib/fermix-desktop/lib'" check "$root"
root="$(variant no-runpath)"
gcc -o "$root/$PREFIX/libexec/helper" "$work/main.c"
expect_refusal "a program with no RUNPATH" "$PREFIX/libexec/helper has RUNPATH ''" check "$root"

echo "check_private_runtime_test: an RPATH, which LD_LIBRARY_PATH cannot override"
root="$(variant rpath)"
gcc -shared -fPIC -o "$root/$PREFIX/lib/libprivate.so.1.0" "$work/private.c" -Wl,-soname,libprivate.so.1 \
  "-Wl,-rpath,$ORIGIN" -Wl,--disable-new-dtags
expect_refusal "a library with an RPATH" "$PREFIX/lib/libprivate.so.1.0 carries an RPATH" check "$root"

echo "check_private_runtime_test: a private object that names a host GTK, GLib or pango"
host_library libglib-2.0.so.0
root="$(variant host-glib)"
shared "$root/$PREFIX/lib/libprivate.so.1.0" libprivate.so.1 "$ORIGIN" -L"$work/host" -l:libglib-2.0.so.0
expect_refusal "a library that links a host GLib" \
  "$PREFIX/lib/libprivate.so.1.0 needs libglib-2.0.so.0, and the private prefix does not carry it" \
  check "$root"
lock "$work/lock-glib.json" ', "libpango-1.0.so.0"'
expect_refusal "a lock that puts pango on the host" \
  "host_libraries names libpango-1.0.so.0, which must be private" check "$work/good" "$work/lock-glib.json"

echo "check_private_runtime_test: the host boundary, through check_boundary.sh"
host_library libundeclared.so.1
root="$(variant undeclared)"
executable "$root/$PREFIX/bin/fermix-desktop" "$ORIGIN/../lib" -L"$root/$PREFIX/lib" -l:libprivate.so.1 \
  -L"$work/host" -l:libundeclared.so.1
expect_refusal "a window that needs an undeclared host library" \
  "needs libundeclared.so.1, which is neither private nor a declared host library" check "$root"
lock "$work/lock-low.json"
sed -i 's/"glibc_floor": "2.35"/"glibc_floor": "2.2.5"/' "$work/lock-low.json"
expect_refusal "an object above the glibc floor" "above the oldest supported host's GLIBC_2.2.5" \
  check "$work/good" "$work/lock-low.json"

echo "check_private_runtime_test: the window, resolved by ldd with no LD_LIBRARY_PATH"
root="$(variant not-a-library)"
rm "$root/$PREFIX/lib/libprivate.so.1"
printf 'not an ELF\n' > "$root/$PREFIX/lib/libprivate.so.1"
expect_refusal "a window whose private library does not load" "ldd cannot resolve the window" check "$root"

echo "check_private_runtime_test: trees it cannot check"
mkdir -p "$work/empty/$PREFIX/lib"
expect_refusal "a prefix with no window" "no window at /$PREFIX/bin/fermix-desktop" check "$work/empty"
expect_refusal "a tree with no private prefix" "no private prefix" check "$work/nowhere"
expect_refusal "a missing lock" "no lock file" check "$work/good" "$work/absent.json"
expect_refusal "no arguments" "usage" "$here/check_private_runtime.sh"

echo "check_private_runtime_test: ok"
