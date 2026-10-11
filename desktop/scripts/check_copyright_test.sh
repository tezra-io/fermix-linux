#!/usr/bin/env bash
# Offline tests for desktop/packaging/copyright.py and check_copyright.sh, over small inputs made
# here in the shapes the build hands them: the runtime lock, the runtime's runtime-licenses.json
# with the archive its paths point into, the window's crate list, the vendored code, the standard
# licence texts and the engine's own copyright file.
#   desktop/scripts/check_copyright_test.sh
set -euo pipefail
shopt -s inherit_errexit

here=$(cd "$(dirname "$0")" && pwd)
packaging="$here/../packaging"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() {
  echo "check_copyright_test: $*" >&2
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

expect_text() {
  grep -qF -- "$2" "$1" || fail "$(basename "$1") does not say: $2"
}

inputs="$work/inputs"
mkdir -p "$inputs/texts/components/glib-2.88.3" "$inputs/texts/components/expat-2.8.4" \
  "$inputs/texts/crates/cssparser-0.35.0" "$inputs/texts/crates/tendril-0.4.3" "$inputs/texts/rust-std" \
  "$inputs/repo/desktop/vendor/thing" "$inputs/licenses"

printf 'GNU LESSER GENERAL PUBLIC LICENSE\nVersion 2.1, February 1999\n\nThe fixture text.\n' \
  > "$inputs/texts/components/glib-2.88.3/COPYING"
mkdir -p "$inputs/texts/components/glib-2.88.3/glib"
printf 'GBSearchArray fixture notice: permission to use, copy, modify.\n' \
  > "$inputs/texts/components/glib-2.88.3/glib/gbsearcharray.h.notice"
printf 'Copyright (c) 1998-2000 Thai Open Source Software Center Ltd\n\nPermission is granted.\n' \
  > "$inputs/texts/components/expat-2.8.4/COPYING"
printf 'Mozilla Public License Version 2.0\n\nThe fixture text.\n' \
  > "$inputs/texts/crates/cssparser-0.35.0/LICENSE"
printf 'The standard library notices.\n' > "$inputs/texts/rust-std/COPYRIGHT-library.html"
printf 'tendril MIT.\n' > "$inputs/texts/crates/tendril-0.4.3/LICENSE-MIT"
cp "$packaging/licenses/MIT.txt" "$packaging/licenses/Apache-2.0.txt" "$inputs/licenses/"
printf 'MIT License\n\nCopyright (c) 2026 Tezra\n\nPermission is hereby granted.\n' > "$inputs/repo/LICENSE"
printf 'Copyright (c) 2020 Thing\n\nThe thing licence.\n' > "$inputs/repo/desktop/vendor/thing/LICENSE"
printf 'Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\n' \
  > "$inputs/engine-copyright"

# wayland-protocols is build-only: the runtime is built with it and ships none of it. glib's
# expression names a LicenseRef, whose text is the notice license_refs maps it to.
cat > "$inputs/lock.json" <<'EOF'
{"schema_version": 1, "components": [
  {"name": "glib", "version": "2.88.3", "source_dir": "glib-2.88.3",
   "license": "LGPL-2.1-or-later AND LicenseRef-glib-gbsearcharray",
   "url": "https://download.gnome.org/sources/glib/2.88/glib-2.88.3.tar.xz", "sha256": "aa"},
  {"name": "wayland-protocols", "version": "1.49", "source_dir": "wayland-protocols-1.49",
   "license": "MIT", "url": "https://example.org/wayland-protocols-1.49.tar.xz", "sha256": "cc"},
  {"name": "expat", "version": "2.8.4", "source_dir": "expat-2.8.4", "license": "MIT",
   "url": "https://example.org/expat-2.8.4.tar.xz", "sha256": "bb"}]}
EOF
cat > "$inputs/runtime-licenses.json" <<'EOF'
{"schema_version": 1, "archive": "runtime-licenses.tar.gz",
 "components": [
  {"name": "glib", "version": "2.88.3", "license": "LGPL-2.1-or-later AND LicenseRef-glib-gbsearcharray",
   "build_only": false, "license_files": ["components/glib-2.88.3/COPYING",
                                          "components/glib-2.88.3/glib/gbsearcharray.h.notice"]},
  {"name": "wayland-protocols", "version": "1.49", "license": "MIT", "build_only": true,
   "license_files": []},
  {"name": "expat", "version": "2.8.4", "license": "MIT", "build_only": false,
   "license_files": ["components/expat-2.8.4/COPYING"]}],
 "crates": [
  {"component": "librsvg", "name": "cssparser", "version": "0.35.0", "license": "MPL-2.0",
   "source": "crates.io", "checksum": "ee", "license_files": ["crates/cssparser-0.35.0/LICENSE"]},
  {"component": "librsvg", "name": "tendril", "version": "0.4.3", "license": "MIT / Apache-2.0",
   "source": "crates.io", "checksum": "ff", "license_files": ["crates/tendril-0.4.3/LICENSE-MIT"]}],
 "rust_std": {"version": "1.97.1", "license": "MIT OR Apache-2.0",
              "license_files": ["rust-std/COPYRIGHT-library.html"]},
 "license_refs": {"LicenseRef-glib-gbsearcharray": "components/glib-2.88.3/glib/gbsearcharray.h.notice"}}
EOF
# cc only runs while the window is built; the others are linked into it.
cat > "$inputs/window-crates.json" <<'EOF'
{"schema_version": 1, "root": "fermix-desktop", "crates": [
  {"name": "cc", "version": "1.0.0", "linked": false, "license": "MIT OR Apache-2.0", "authors": [],
   "repository": null, "files": [{"name": "LICENSE-MIT", "sha256": "w", "text": "cc MIT.\n"}]},
  {"name": "gio", "version": "0.22.0", "linked": true, "license": "MIT",
   "authors": ["The gtk-rs Project Developers"], "repository": null,
   "files": [{"name": "LICENSE", "sha256": "x", "text": "The gtk-rs licence.\n"}]},
  {"name": "glib", "version": "0.22.0", "linked": true, "license": "MIT",
   "authors": ["The gtk-rs Project Developers"], "repository": null,
   "files": [{"name": "LICENSE", "sha256": "x", "text": "The gtk-rs licence.\n"}]},
  {"name": "serde", "version": "1.0.0", "linked": true, "license": "MIT OR Apache-2.0", "authors": [],
   "repository": null, "files": [{"name": "LICENSE-MIT", "sha256": "y", "text": "serde MIT.\n"},
                                  {"name": "LICENSE-APACHE", "sha256": "z", "text": "serde Apache.\n"}]},
  {"name": "earshot", "version": "1.2.2", "linked": true, "license": "MIT OR Apache-2.0",
   "authors": ["A Person"], "repository": null, "files": []}]}
EOF
cat > "$inputs/vendored.json" <<'EOF'
{"schema_version": 1, "vendored": [
  {"files": ["desktop/vendor/thing/*"], "name": "the thing", "license": "MIT",
   "copyright": ["2020 Thing"], "texts": [{"file": "desktop/vendor/thing/LICENSE"}],
   "comment": "A vendored thing."},
  {"files": ["desktop/vendor/thing/part/*"], "name": "a part", "license": "Apache-2.0",
   "copyright": ["Someone"], "texts": [], "comment": "A part of the thing under another licence."}]}
EOF

# The flags both programs take, for the inputs in <dir>.
flags() {
  local dir="$1"
  printf '%s\n' --repo "$dir/repo" --lock "$dir/lock.json" --runtime-licenses "$dir/runtime-licenses.json" \
    --runtime-texts "$dir/texts" \
    --window-crates "$dir/window-crates.json" --vendored "$dir/vendored.json" \
    --standard-texts "$dir/licenses" --engine-copyright "$dir/engine-copyright"
}

generate() {
  local dir="$1" out="$2" args
  mapfile -t args < <(flags "$dir")
  "$packaging/copyright.py" "${args[@]}" --out "$out"
}

check() {
  local copyright="$1" dir="$2" args
  mapfile -t args < <(flags "$dir")
  "$here/check_copyright.sh" "$copyright" "${args[@]}"
}

# A copy of the inputs with one edit: <name> <shell statement run in the copy>
inputs_variant() {
  cp -a "$inputs" "$work/$1"
  (cd "$work/$1" && eval "$2")
  echo "$work/$1"
}

echo "check_copyright_test: the copyright file of a package, generated and checked"
generate "$inputs" "$work/copyright" > /dev/null
check "$work/copyright" "$inputs" > "$work/check.out" || fail "the generated file was refused"
expect_text "$work/check.out" "build-only, with no paragraph: wayland-protocols 1.49"
out="$work/copyright"
[ "$(head -n 1 "$out")" = "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/" ] ||
  fail "the file does not open with the Format line"
expect_text "$out" "Files: *"
expect_text "$out" "Files: desktop/vendor/thing/*"
expect_text "$out" "Files: runtime/glib-2.88.3/*"
expect_text "$out" " glib 2.88.3, built from https://download.gnome.org/sources/glib/2.88/glib-2.88.3.tar.xz"
expect_text "$out" " replace the file with their own build of the same SONAME"
expect_text "$out" "Files: runtime/crates/cssparser-0.35.0/*"
expect_text "$out" "Files: runtime/rust-std-1.97.1/*"
grep -qxF "Files: runtime/crates/tendril-0.4.3/*" "$out" || fail "tendril has no paragraph of its own"
grep -A2 -xF "Files: runtime/crates/tendril-0.4.3/*" "$out" | grep -qxF "License: MIT OR Apache-2.0" ||
  fail "tendril's 'MIT / Apache-2.0' is not read as MIT OR Apache-2.0"
expect_text "$out" " The Rust standard library 1.97.1"
expect_text "$out" "Files: crates/gio-0.22.0/* crates/glib-0.22.0/*"
expect_text "$out" " cc 1.0.0 runs while the window is built"
expect_text "$out" "License: MIT OR Apache-2.0"
expect_text "$out" " serde Apache."
expect_text "$out" " earshot 1.2.2 ships no licence file; its text is the licence's standard one."
expect_text "$out" "Files: engine/*"
expect_text "$out" " /usr/share/doc/fermix/copyright"
expect_text "$out" " Permission is hereby granted."
grep -qxF "License: MIT" "$out" || fail "there is no stand-alone MIT paragraph"
grep -A2 -xF "Files: runtime/glib-2.88.3/*" "$out" |
  grep -qxF "License: LGPL-2.1-or-later AND LicenseRef-glib-gbsearcharray" ||
  fail "glib's paragraph does not state its expression with the LicenseRef"
grep -A1 -xF "License: LicenseRef-glib-gbsearcharray" "$out" |
  grep -qxF " GBSearchArray fixture notice: permission to use, copy, modify." ||
  fail "the LicenseRef has no stand-alone paragraph holding the text license_refs maps it to"
[ "$(grep -cF "GBSearchArray fixture notice" "$out")" = 1 ] ||
  fail "the LicenseRef's text is not in the file exactly once"
expect_text "$out" " The notice in glib-2.88.3/glib/gbsearcharray.h of the private runtime's sources."
! grep -qF wayland-protocols "$out" || fail "the build-only component has a paragraph"
echo "  ok: every shipped component, crate and vendored part, the engine's pointer, and the texts"
echo "  ok: the LicenseRef has its own License paragraph, with the mapped text, given once"
echo "  ok: the build-only component is accounted for in the check and has no paragraph"
echo "  ok: crates with the same texts share a paragraph: $(grep -F 'crates/gio-0.22.0' "$out")"
order="$(grep -n '^Files: ' "$out" | head -n 3 | cut -d: -f3 | tr '\n' '|')"
[ "$order" = " *| desktop/vendor/thing/*| desktop/vendor/thing/part/*|" ] ||
  fail "the overlapping patterns are not in the order the last match needs: $order"
echo "  ok: * first, then the vendored parts in their order, so the last match is the right one"

echo "check_copyright_test: a copyright file that disagrees with its inputs"
# A JSON edit of one input file in the copy: <file> <python statement over d>
edit_json() {
  python3 -c "import json; d = json.load(open('$1')); $2; json.dump(d, open('$1', 'w'))"
}
ZLIB='{"name": "zlib", "version": "1.3", "source_dir": "zlib-1.3", "license": "Zlib", "url": "u", "sha256": "dd"}'
ZLIB_LICENSES='{"name": "zlib", "version": "1.3", "license": "Zlib", "build_only": False, "license_files": ["components/expat-2.8.4/COPYING"]}'
dir="$(inputs_variant new-component "edit_json lock.json 'd[\"components\"].append($ZLIB)' &&
  edit_json runtime-licenses.json 'd[\"components\"].append($ZLIB_LICENSES)'")"
expect_refusal "a component the file does not name" "no paragraph for runtime/zlib-1.3/* with License: Zlib" \
  check "$work/copyright" "$dir"
dir="$(inputs_variant unaccounted "edit_json lock.json 'd[\"components\"].append($ZLIB)'")"
expect_refusal "a lock component runtime-licenses.json does not account for" \
  "the lock's component zlib 1.3 is not in runtime-licenses.json" check "$work/copyright" "$dir"
dir="$(inputs_variant unknown "edit_json runtime-licenses.json 'd[\"components\"].append($ZLIB_LICENSES)'")"
expect_refusal "a runtime-licenses.json component the lock does not have" \
  "runtime-licenses.json has the component zlib 1.3, which the lock does not" check "$work/copyright" "$dir"
GLIB_REF="AND LicenseRef-glib-gbsearcharray"
dir="$(inputs_variant relicensed-lock "sed -i 's/LGPL-2.1-or-later/LGPL-2.0-or-later/' lock.json")"
expect_refusal "a lock licence runtime-licenses.json does not repeat" \
  "the lock gives glib 2.88.3 the licence LGPL-2.0-or-later $GLIB_REF, and runtime-licenses.json LGPL-2.1-or-later $GLIB_REF" \
  check "$work/copyright" "$dir"
dir="$(inputs_variant relicensed "sed -i 's/LGPL-2.1-or-later/LGPL-2.0-or-later/' lock.json runtime-licenses.json")"
expect_refusal "a component whose licence changed" \
  "no paragraph for runtime/glib-2.88.3/* with License: LGPL-2.0-or-later $GLIB_REF" check "$work/copyright" "$dir"
dir="$(inputs_variant shipped "edit_json runtime-licenses.json 'd[\"components\"][1].update(build_only=False, license_files=[\"components/expat-2.8.4/COPYING\"])'")"
expect_refusal "a build-only component that turns out to ship" \
  "no paragraph for runtime/wayland-protocols-1.49/* with License: MIT" check "$work/copyright" "$dir"
dir="$(inputs_variant dropped-crate "sed -i 's/\"name\": \"serde\"/\"name\": \"serde2\"/' window-crates.json")"
expect_refusal "a crate the inputs no longer have" "names crates/serde-1.0.0/*, which no input has" \
  check "$work/copyright" "$dir"
expect_refusal "a crate the file does not name" "no paragraph for crates/serde2-1.0.0/*" \
  check "$work/copyright" "$dir"
dir="$(inputs_variant new-runtime-crate "sed -i 's/\"name\": \"cssparser\"/\"name\": \"selectors\"/' runtime-licenses.json")"
expect_refusal "a runtime crate the file does not name" \
  "no paragraph for runtime/crates/selectors-0.35.0/* with License: MPL-2.0" check "$work/copyright" "$dir"
dir="$(inputs_variant new-rust "sed -i 's/\"1.97.1\"/\"1.98.0\"/' runtime-licenses.json")"
expect_refusal "another Rust standard library" "no paragraph for runtime/rust-std-1.98.0/*" \
  check "$work/copyright" "$dir"

UNMAPPED="glib 2.88.3's licence names LicenseRef-glib-gbsearcharray, which runtime-licenses.json's license_refs does not map"
UNUSED="runtime-licenses.json's license_refs maps LicenseRef-unused, which no licence names"
CRATE_REF="serde 1.0.0's licence names LicenseRef-serde-terms, which runtime-licenses.json's license_refs does not map"
NO_REFS="runtime-licenses.json has no license_refs object"
dir="$(inputs_variant ref-unmapped "edit_json runtime-licenses.json 'd[\"license_refs\"] = {}'")"
expect_refusal "a LicenseRef license_refs does not map" "check_copyright: $UNMAPPED" check "$work/copyright" "$dir"
dir="$(inputs_variant ref-unused "edit_json runtime-licenses.json 'd[\"license_refs\"][\"LicenseRef-unused\"] = \"components/expat-2.8.4/COPYING\"'")"
expect_refusal "a license_refs mapping no licence names" "check_copyright: $UNUSED" check "$work/copyright" "$dir"
dir="$(inputs_variant ref-crate "sed -i '/\"name\": \"serde\"/s/\"MIT OR Apache-2.0\"/\"MIT AND LicenseRef-serde-terms\"/' window-crates.json")"
grep -qF LicenseRef-serde-terms "$dir/window-crates.json" || fail "the window crate variant was not made"
expect_refusal "a window crate's LicenseRef, which nothing maps" "check_copyright: $CRATE_REF" check "$work/copyright" "$dir"
dir="$(inputs_variant ref-none "edit_json runtime-licenses.json 'del(d[\"license_refs\"])'")"
expect_refusal "a runtime-licenses.json with no license_refs" "check_copyright: $NO_REFS" check "$work/copyright" "$dir"
awk 'BEGIN { RS = ""; ORS = "\n\n" } !/^License: LicenseRef-glib-gbsearcharray\n/' "$out" > "$work/edit-no-ref"
expect_refusal "a file whose LicenseRef has no text" "License: LicenseRef-glib-gbsearcharray has no text" \
  check "$work/edit-no-ref" "$inputs"
{ cat "$out"; printf '\nLicense: LicenseRef-stray\n A text nothing names.\n'; } > "$work/edit-stray"
expect_refusal "a stand-alone License paragraph no Files paragraph names" \
  "License: LicenseRef-stray stands alone, and no Files paragraph names it" check "$work/edit-stray" "$inputs"

echo "check_copyright_test: a LicenseRef only a build-only component names has no paragraph"
dir="$(inputs_variant ref-build-only "sed -i 's/\"license\": \"MIT\", \"build_only\": true/\"license\": \"MIT AND LicenseRef-wp\", \"build_only\": true/' runtime-licenses.json &&
  sed -i 's/\"license\": \"MIT\", \"url\": \"https:\/\/example.org\/wayland/\"license\": \"MIT AND LicenseRef-wp\", \"url\": \"https:\/\/example.org\/wayland/' lock.json &&
  edit_json runtime-licenses.json 'd[\"license_refs\"][\"LicenseRef-wp\"] = \"components/expat-2.8.4/COPYING\"'")"
grep -qF '"MIT AND LicenseRef-wp", "url"' "$dir/lock.json" || fail "the build-only variant's lock was not made"
generate "$dir" "$work/copyright-build-only" > /dev/null
check "$work/copyright-build-only" "$dir" > /dev/null || fail "a build-only component's mapped LicenseRef was refused"
! grep -qF LicenseRef-wp "$work/copyright-build-only" || fail "a build-only component's LicenseRef has a paragraph"
echo "  ok: mapped, accounted for, and not in the file"

echo "check_copyright_test: a hand-edited file"
sed 's/^ serde Apache\.$/ serde Apache, edited by hand./' "$out" > "$work/edited-text"
expect_refusal "a licence text edited by hand" "is not what copyright.py generates from its inputs" \
  check "$work/edited-text" "$inputs"
awk 'BEGIN { RS = ""; ORS = "\n\n" } !/^Files: engine/' "$out" > "$work/edit-no-engine"
expect_refusal "a file with no pointer to the engine's copyright" "no paragraph for engine/*" \
  check "$work/edit-no-engine" "$inputs"
awk 'BEGIN { RS = ""; ORS = "\n\n" } !/^License: MIT\n/' "$out" > "$work/edit-no-mit"
expect_refusal "a short name with no text" "License: MIT has no text" check "$work/edit-no-mit" "$inputs"

echo "check_copyright_test: inputs the generator refuses"
dir="$(inputs_variant no-standard "rm licenses/MIT.txt")"
expect_refusal "a crate with no licence file and no standard text" \
  "earshot 1.2.2 ships no licence file, and there is no standard text $dir/licenses/MIT.txt" \
  generate "$dir" "$work/x"
dir="$(inputs_variant no-text "rm texts/components/expat-2.8.4/COPYING")"
expect_refusal "a licence file the archive does not hold" \
  "the runtime's licence archive has no components/expat-2.8.4/COPYING, which expat 2.8.4 names" \
  generate "$dir" "$work/x"
dir="$(inputs_variant no-engine "rm engine-copyright")"
expect_refusal "no engine copyright file" "engine-copyright" generate "$dir" "$work/x"
dir="$(inputs_variant unlisted "edit_json runtime-licenses.json 'd[\"components\"][2][\"license_files\"] = []'")"
expect_refusal "a shipped component with no licence files listed" \
  "runtime-licenses.json lists no licence files for expat 2.8.4" generate "$dir" "$work/x"
dir="$(inputs_variant gen-ref-unmapped "edit_json runtime-licenses.json 'd[\"license_refs\"] = {}'")"
expect_refusal "a LicenseRef license_refs does not map" "copyright: $UNMAPPED" generate "$dir" "$work/x"
dir="$(inputs_variant gen-ref-unused "edit_json runtime-licenses.json 'd[\"license_refs\"][\"LicenseRef-unused\"] = \"components/expat-2.8.4/COPYING\"'")"
expect_refusal "a license_refs mapping no licence names" "$UNUSED" generate "$dir" "$work/x"
expect_refusal "a window crate's LicenseRef, which nothing maps" "$CRATE_REF" generate "$work/ref-crate" "$work/x"
expect_refusal "a runtime-licenses.json with no license_refs" "$NO_REFS" generate "$work/ref-none" "$work/x"
dir="$(inputs_variant gen-ref-no-text "rm texts/components/glib-2.88.3/glib/gbsearcharray.h.notice")"
expect_refusal "a LicenseRef text the archive does not hold" \
  "the runtime's licence archive has no components/glib-2.88.3/glib/gbsearcharray.h.notice, which LicenseRef-glib-gbsearcharray names" \
  generate "$dir" "$work/x"
dir="$(inputs_variant gen-unaccounted "edit_json lock.json 'd[\"components\"].append($ZLIB)'")"
expect_refusal "a lock component the generator cannot account for" \
  "the lock's component zlib 1.3 is not in runtime-licenses.json" generate "$dir" "$work/x"
dir="$(inputs_variant gen-relicensed "sed -i 's/LGPL-2.1-or-later/LGPL-2.0-or-later/' lock.json")"
expect_refusal "a licence the two runtime inputs disagree on" \
  "the lock gives glib 2.88.3 the licence LGPL-2.0-or-later $GLIB_REF, and runtime-licenses.json LGPL-2.1-or-later $GLIB_REF" \
  generate "$dir" "$work/x"
[ ! -e "$work/x" ] || fail "a refused generation left a file behind"
expect_refusal "no arguments" "usage" "$packaging/copyright.py"
expect_refusal "the checker with no arguments" "usage" "$here/check_copyright.sh"

echo "check_copyright_test: ok"
