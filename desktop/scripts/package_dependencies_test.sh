#!/usr/bin/env bash
# Offline tests for package_dependencies.py. The ELF objects are compiled here with the host's gcc,
# and the host libraries they need are stubs with the right SONAME, so every NEEDED entry is the one
# a case names. The declared relations are a deb control file and an rpm --requires listing, the
# two things build_packages.sh reads out of the built packages.
#   desktop/scripts/package_dependencies_test.sh
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
  echo "package_dependencies_test: $*" >&2
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
printf 'int private_value(void);\nint main(void) { return private_value() - 1; }\n' > "$work/window.c"
printf 'int main(void) { return 0; }\n' > "$work/main.c"

stub() {
  mkdir -p "$work/host"
  gcc -shared -fPIC -o "$work/host/$1" "$work/private.c" "-Wl,-soname,$1"
}

# The window, needing the private library and whatever else is named: <root> [linker arguments...]
window() {
  local root="$1"
  shift
  gcc -o "$root/$PREFIX/bin/fermix-desktop" "$work/window.c" "-Wl,-rpath,$ORIGIN/../lib" \
    -Wl,--enable-new-dtags -Wl,--no-as-needed -L"$root/$PREFIX/lib" -l:libprivate.so.1 "$@"
}

good_root() {
  local root="$1"
  mkdir -p "$root/$PREFIX/lib" "$root/$PREFIX/bin" "$root/usr/bin" "$root/usr/lib/fermix"
  gcc -shared -fPIC -o "$root/$PREFIX/lib/libprivate.so.1.0" "$work/private.c" \
    -Wl,-soname,libprivate.so.1 "-Wl,-rpath,$ORIGIN" -Wl,--enable-new-dtags
  ln -s libprivate.so.1.0 "$root/$PREFIX/lib/libprivate.so.1"
  window "$root" -L"$work/host" -l:libX11.so.6
  # The engine's two kinds: a static program, and a dynamic one that needs only the C library.
  gcc -static -o "$root/usr/lib/fermix/cosign" "$work/main.c"
  gcc -o "$root/usr/bin/fermix" "$work/main.c"
  ln -s ../lib/fermix-desktop/bin/fermix-desktop "$root/usr/bin/fermix-desktop"
  printf 'not an ELF\n' > "$root/usr/lib/fermix/notes.txt"
}

variant() {
  cp -a "$work/good" "$work/$1"
  echo "$work/$1"
}

control() {
  printf 'Package: fermix-desktop\nVersion: 0.0.1\nArchitecture: amd64\n' > "$1"
  printf 'Depends: %s\nDescription: a fixture\n' "$2" >> "$1"
}

stub libX11.so.6
stub libundeclared.so.1
good_root "$work/good"
printf '{"glibc_floor": "2.34", "host_libraries": ["libc.so.6", "ld-linux-x86-64.so.2", "libX11.so.6"]}\n' \
  > "$work/lock.json"
cat > "$work/map" <<'EOF'
ld-linux-x86-64.so.2 :: libc6 (>= {glibc_floor}) :: libc.so.6(GLIBC_{glibc_floor})(64bit)
libc.so.6 :: libc6 (>= {glibc_floor}) :: libc.so.6(GLIBC_{glibc_floor})(64bit)
libX11.so.6 :: libx11-6 :: libX11.so.6()(64bit)
@dconf :: dconf-service :: dconf
EOF
control "$work/control" 'libc6 (>= 2.34), libx11-6, dconf-service'
printf 'libc.so.6(GLIBC_2.34)(64bit)\nlibX11.so.6()(64bit)\ndconf\n' > "$work/requires"

check() {
  "$here/package_dependencies.py" --root "${ROOT:-$work/good}" --map "${MAP:-$work/map}" \
    --lock "${LOCK:-$work/lock.json}" --deb-control "${CONTROL:-$work/control}" \
    --rpm-requires "${REQUIRES:-$work/requires}"
}

echo "package_dependencies_test: a package whose every NEEDED is private or declared"
check > "$work/good.out" || fail "the good package was refused"
grep -qF "4 ELF objects" "$work/good.out" || fail "the good package was not read whole: $(cat "$work/good.out")"
echo "  ok: $(cat "$work/good.out")"
control "$work/control-folded" 'libc6 (>= 2.34),
 libx11-6 | libx11-6-other,
 dconf-service'
CONTROL="$work/control-folded" check > /dev/null || fail "a folded Depends with an alternative was refused"
echo "  ok: a Depends folded over lines, with an alternative"

echo "package_dependencies_test: an undeclared host NEEDED"
root="$(variant undeclared)"
window "$root" -L"$work/host" -l:libX11.so.6 -l:libundeclared.so.1
ROOT="$root" expect_refusal "a window that needs a library no row names" \
  "/$PREFIX/bin/fermix-desktop needs libundeclared.so.1, which is not in the private prefix and host_relations.map has no row for" \
  check
control "$work/control-no-x11" 'libc6 (>= 2.34), dconf-service'
CONTROL="$work/control-no-x11" expect_refusal "a deb that does not declare libx11-6" \
  "/$PREFIX/bin/fermix-desktop needs libX11.so.6, and the deb declares no 'libx11-6'" check
printf 'libc.so.6(GLIBC_2.34)(64bit)\ndconf\n' > "$work/requires-no-x11"
REQUIRES="$work/requires-no-x11" expect_refusal "an rpm that does not require libX11" \
  "needs libX11.so.6, and the rpm requires no 'libX11.so.6()(64bit)'" check

echo "package_dependencies_test: a program outside the prefix cannot use the private libraries"
root="$(variant engine-private)"
gcc -o "$root/usr/bin/fermix" "$work/window.c" -Wl,--no-as-needed -L"$root/$PREFIX/lib" -l:libprivate.so.1
ROOT="$root" expect_refusal "an engine program needing a private library" \
  "/usr/bin/fermix needs libprivate.so.1, which is not in the private prefix and host_relations.map has no row for" \
  check

echo "package_dependencies_test: the glibc floor"
printf '{"glibc_floor": "2.33", "host_libraries": ["libc.so.6", "ld-linux-x86-64.so.2", "libX11.so.6"]}\n' \
  > "$work/lock-low.json"
control "$work/control-low" 'libc6 (>= 2.33), libx11-6, dconf-service'
printf 'libc.so.6(GLIBC_2.33)(64bit)\nlibX11.so.6()(64bit)\ndconf\n' > "$work/requires-low"
LOCK="$work/lock-low.json" CONTROL="$work/control-low" REQUIRES="$work/requires-low" \
  expect_refusal "an object above the declared floor" "needs GLIBC_2.34, above the declared floor 2.33" check
CONTROL="$work/control-low" expect_refusal "a deb floor that is not the rpm's" \
  "the deb declares glibc 2.33 and the rpm 2.34" check
control "$work/control-no-floor" 'libc6, libx11-6, dconf-service'
CONTROL="$work/control-no-floor" expect_refusal "a deb with no glibc floor" \
  "the deb declares no glibc floor" check

echo "package_dependencies_test: inputs it cannot read"
printf 'Package: fermix-desktop\nVersion: 0.0.1\n' > "$work/control-no-depends"
CONTROL="$work/control-no-depends" expect_refusal "a control file with no Depends" "has no Depends" check
ROOT="$work/nowhere" expect_refusal "no staged tree" "no staged tree" check
expect_refusal "no arguments" "usage" "$here/package_dependencies.py"

echo "package_dependencies_test: ok"
