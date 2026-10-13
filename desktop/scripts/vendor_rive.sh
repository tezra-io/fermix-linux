#!/usr/bin/env bash
# Vendors the Rive C++ runtime into rive/vendor/, the part of it the mascot build
# compiles, at the commit the macOS app's RiveRuntime 6.27.0 embeds (rive-ios's
# submodules/rive-runtime at its 6.27.0 tag), so both apps play the file with the
# same runtime. The build never fetches anything; this script is how the pin moves.
#   scripts/vendor_rive.sh
#
# The GL renderer embeds its shaders as generated headers. They are generated here
# with the runtime's own minify.py in its --human-readable mode (the runtime's
# raw_shaders option, built with RIVE_RAW_SHADERS): the minified shaders fail to
# compile on Mesa 25.1 (radeonsi, "syntax error, unexpected NEW_IDENTIFIER").
set -euo pipefail
RIVE_COMMIT=1af8ccbefdf906ea5c33e80845360c62b43bafb3
# The PLY version the runtime's own premake pins for minify.py.
PLY_TAG=3.11

here=$(cd "$(dirname "$0")/.." && pwd)
dest="$here/rive/vendor"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

runtime="$work/rive-runtime"
git init -q "$runtime"
git -C "$runtime" fetch -q --depth 1 https://github.com/rive-app/rive-runtime "$RIVE_COMMIT"
git -C "$runtime" checkout -q FETCH_HEAD
git -c advice.detachedHead=false clone -q --depth 1 --branch "$PLY_TAG" https://github.com/dabeaz/ply "$work/ply"
make -s -C "$runtime/renderer/src/shaders" OUT="$work/generated/shaders" \
  FLAGS="-p $work/ply --human-readable" minify > /dev/null

rm -rf "${dest:?}/rive-runtime" "${dest:?}/generated"
mkdir -p "$dest/rive-runtime/renderer/src/ore" "$dest/rive-runtime/decoders/src" "$dest/generated"
cp -r "$runtime/LICENSE" "$runtime/include" "$runtime/src" "$dest/rive-runtime/"
cp -r "$runtime/renderer/LICENSE" "$runtime/renderer/include" "$runtime/renderer/glad" \
  "$dest/rive-runtime/renderer/"
cp "$runtime"/renderer/src/*.cpp "$runtime"/renderer/src/*.hpp "$dest/rive-runtime/renderer/src/"
cp -r "$runtime/renderer/src/gl" "$dest/rive-runtime/renderer/src/"
# The shader sources the generated headers come from; the C++ also includes constants.glsl.
mkdir -p "$dest/rive-runtime/renderer/src/shaders"
cp "$runtime"/renderer/src/shaders/*.glsl "$runtime"/renderer/src/shaders/*.vert \
  "$runtime"/renderer/src/shaders/*.frag "$dest/rive-runtime/renderer/src/shaders/"
cp "$runtime/renderer/src/ore/ore_binding_map.cpp" "$runtime/renderer/src/ore/ore_bind_group_layout.cpp" \
  "$dest/rive-runtime/renderer/src/ore/"
cp -r "$runtime/decoders/include" "$dest/rive-runtime/decoders/"
cp "$runtime/decoders/src/bitmap_decoder.cpp" "$runtime/decoders/src/bitmap_decoder_thirdparty.cpp" \
  "$runtime/decoders/src/decode_png.cpp" "$dest/rive-runtime/decoders/src/"
# Only the headers the sources include, and the exports each one includes; the
# minifier's other intermediates stay behind.
mkdir -p "$dest/generated/shaders"
cp "$work"/generated/shaders/*.hpp "$work"/generated/shaders/*.exports.h "$dest/generated/shaders/"
echo "$RIVE_COMMIT" > "$dest/COMMIT"
echo "vendored rive-runtime $RIVE_COMMIT into $dest"
