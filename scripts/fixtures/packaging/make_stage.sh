#!/usr/bin/env bash
#
# Build a staged tree that stands in for a real one, from ELF files this host
# already has.
#
# `scripts/package_dependencies.py` and `scripts/check_private_runtime.sh` both
# answer questions about real ELF objects, so a fixture made of text files would
# exercise their argument handling and none of their reasoning. A synthetic ELF
# would exercise the reader rather than the gate. So the fixture is real objects
# in the shape the package puts them: a few host libraries under the private
# prefix, an executable where the application goes, the two generated caches,
# and a static object where the engine's would be.
#
# What it deliberately does not reproduce is the RUNPATH, because a host library
# does not carry the one this design writes. That is the point of the last case
# in `build_packages_test.sh`: the gate refuses this tree, and the sentence it
# refuses it with is the one a real object with the wrong search path would get.
#
# Usage: make_stage.sh <where>
set -euo pipefail

WHERE="${1:?usage: make_stage.sh <where>}"
PREFIX="usr/lib/fermix-desktop"

fail() {
  echo "make_stage: $*" >&2
  exit 1
}

rm -rf "$WHERE"
mkdir -p \
  "$WHERE/$PREFIX/bin" \
  "$WHERE/$PREFIX/lib/gdk-pixbuf-2.0/2.10.0" \
  "$WHERE/$PREFIX/share/glib-2.0/schemas" \
  "$WHERE/usr/bin" \
  "$WHERE/usr/lib/fermix"

# An executable for the application's place. `/bin/sh` rather than something
# chosen for its dependencies: every host has it, and it is dynamically linked
# against libc, which is the one NEEDED entry this fixture relies on.
[ -x /bin/sh ] || fail "this host has no /bin/sh to stand in for the application"
cp /bin/sh "$WHERE/$PREFIX/bin/fermix-desktop"
chmod 0755 "$WHERE/$PREFIX/bin/fermix-desktop"

# A couple of shared objects under the private prefix, so the gates have more
# than one object to read and so at least one NEEDED entry resolves inside the
# tree rather than outside it.
found=0
for candidate in \
  /lib/*/libz.so.1 /usr/lib/*/libz.so.1 /lib64/libz.so.1 /usr/lib64/libz.so.1 \
  /lib/*/libexpat.so.1 /usr/lib/*/libexpat.so.1 /lib64/libexpat.so.1; do
  [ -f "$candidate" ] || continue
  cp "$candidate" "$WHERE/$PREFIX/lib/$(basename "$candidate")"
  found=$((found + 1))
done
[ "$found" -gt 0 ] ||
  fail "this host carries neither libz nor libexpat, and the fixture needs a shared object"

# The two caches the design generates at build time against the final absolute
# prefix, with the shape the gate reads: absolute paths, all of them private.
printf '"/usr/lib/fermix-desktop/lib/gdk-pixbuf-2.0/2.10.0/loaders/libpixbufloader-png.so"\n' \
  > "$WHERE/$PREFIX/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
printf 'GVariant\0/usr/lib/fermix-desktop/share/glib-2.0/schemas\0' \
  > "$WHERE/$PREFIX/share/glib-2.0/schemas/gschemas.compiled"

# The engine's place. A static object, which is what the engine's own
# executables are, so the gates take the branch that exempts them.
printf '\177ELF\002\001\001\000%*s' 60 "" | tr ' ' '\000' > "$WHERE/usr/bin/fermix"
chmod 0755 "$WHERE/usr/bin/fermix"

echo "make_stage: a fixture stage at $WHERE"
