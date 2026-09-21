#!/usr/bin/env bash
#
# Exercise check_copyright.sh and the generator behind it, offline.
#
# The obligation this gate carries is a licensing one, so its failure mode is
# not a broken build: it is a package that ships a library and says nothing
# about its terms. The cases below are the ways that happens, each driven
# against a fixture lock file rather than against the repository's own, so a
# component bump does not rewrite this test.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/check_copyright.sh"
GENERATOR="$ROOT_DIR/scripts/generate_copyright.py"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/check-copyright-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fail() {
  echo "check_copyright_test: $*" >&2
  exit 1
}

expect_sentence() {
  local what="$1" needle="$2"
  shift 2
  local output
  output="$("$@" 2>&1 || true)"
  case "$output" in
    *"$needle"*) echo "  refused: $what" ;;
    *) fail "$what was not refused with '$needle'; it said: $output" ;;
  esac
}

# A fixture world: a lock file naming two components, and a licence text for
# each expression it names.
write_world() {
  local where="$1"
  shift
  rm -rf "$where"
  mkdir -p "$where/licenses"
  python3 - "$where/RUNTIME.lock.json" "$@" <<'PY'
import json
import sys

out, *extra = sys.argv[1:]
components = [
    {
        "name": "libexample",
        "version": "1.2.3",
        "url": "https://example.invalid/libexample-1.2.3.tar.xz",
        "sha256": "0" * 64,
        "license": "MIT",
    },
    {
        "name": "libcopyleft",
        "version": "4.5.6",
        "url": "https://example.invalid/libcopyleft-4.5.6.tar.xz",
        "sha256": "1" * 64,
        "license": "LGPL-2.1-or-later",
    },
]
for name in extra:
    components.append(
        {
            "name": "libextra",
            "version": "0.1.0",
            "url": "https://example.invalid/libextra-0.1.0.tar.gz",
            "sha256": "2" * 64,
            "license": name,
        }
    )
with open(out, "w", encoding="utf-8") as handle:
    json.dump({"schema_version": 1, "components": components}, handle, indent=2)
PY
  # Long enough to be taken for a licence, which the gate checks for.
  local name
  for name in MIT LGPL-2.1-or-later; do
    {
      echo "The $name terms, as they appear in the component's own source."
      printf 'A paragraph of terms, repeated so that this reads as a licence. %.0s' 1 2 3 4
      echo
    } > "$where/licenses/$name.txt"
  done
}

echo "check_copyright_test: the scripts parse"
bash -n "$SCRIPT" || fail "the gate does not parse"
python3 -c "import ast,sys; ast.parse(open(sys.argv[1]).read())" "$GENERATOR" ||
  fail "the generator does not parse"
echo "  ok: shell and python syntax"

echo "check_copyright_test: the arguments"
expect_sentence "an unknown argument" "unknown argument" bash "$SCRIPT" --nonsense
expect_sentence "a lock file that is not there" "no runtime lock file" \
  bash "$SCRIPT" --lock "$WORK/absent.json"

echo "check_copyright_test: the generated file against the lock file"

WORLD="$WORK/world"
write_world "$WORLD"
python3 "$GENERATOR" --lock "$WORLD/RUNTIME.lock.json" --licenses "$WORLD/licenses" \
  --out "$WORLD/copyright" >/dev/null || fail "the generator refused a complete world"
bash "$SCRIPT" --lock "$WORLD/RUNTIME.lock.json" --copyright "$WORLD/copyright" \
  --licenses "$WORLD/licenses" >/dev/null ||
  fail "a generated copyright file was refused against the lock file it came from"
echo "  accepted: a copyright file the lock file renders"

# Every obligation the generated file carries, asserted rather than assumed.
for needle in \
  "libexample 1.2.3" \
  "https://example.invalid/libexample-1.2.3.tar.xz" \
  "sha256 0000" \
  "License: LGPL-2.1-or-later" \
  "replace the file with their own build of the same SONAME"; do
  grep -qF "$needle" "$WORLD/copyright" ||
    fail "the generated copyright file does not carry '$needle'"
done
echo "  accepted: every component's version, source, digest and relinking notice"

printf '\nFiles: /usr/lib/fermix-desktop/*\nCopyright: somebody\nLicense: MIT\n' \
  >> "$WORLD/copyright"
expect_sentence "a copyright file somebody edited by hand" \
  "run scripts/generate_copyright.py" \
  bash "$SCRIPT" --lock "$WORLD/RUNTIME.lock.json" --copyright "$WORLD/copyright" \
  --licenses "$WORLD/licenses"

echo "check_copyright_test: a component with no licence text"

BARE="$WORK/bare"
write_world "$BARE" "Zlib"
expect_sentence "a component whose licence has no text anywhere" \
  "there is no text for it at" \
  python3 "$GENERATOR" --lock "$BARE/RUNTIME.lock.json" --licenses "$BARE/licenses" \
  --out "$BARE/copyright"

echo "check_copyright_test: a licence text nothing declares"

STRAY="$WORK/stray"
write_world "$STRAY"
python3 "$GENERATOR" --lock "$STRAY/RUNTIME.lock.json" --licenses "$STRAY/licenses" \
  --out "$STRAY/copyright" >/dev/null
cp "$STRAY/licenses/MIT.txt" "$STRAY/licenses/Nothing-Declares-This.txt"
expect_sentence "a licence text no component names" \
  "texts no component declares" \
  bash "$SCRIPT" --lock "$STRAY/RUNTIME.lock.json" --copyright "$STRAY/copyright" \
  --licenses "$STRAY/licenses"

# The copyright file is regenerated after the text is shortened, so that the
# file and the lock file still agree and the only thing left to object to is the
# text itself. A stub licence that regenerates cleanly is exactly the shape this
# case exists to catch.
SHORT="$WORK/short"
write_world "$SHORT"
printf 'MIT.\n' > "$SHORT/licenses/MIT.txt"
python3 "$GENERATOR" --lock "$SHORT/RUNTIME.lock.json" --licenses "$SHORT/licenses" \
  --out "$SHORT/copyright" >/dev/null
expect_sentence "a licence text too short to be a licence" \
  "too short to be a licence" \
  bash "$SCRIPT" --lock "$SHORT/RUNTIME.lock.json" --copyright "$SHORT/copyright" \
  --licenses "$SHORT/licenses"

echo "check_copyright_test: the repository's own files"
bash "$SCRIPT" >/dev/null ||
  fail "packaging/copyright does not agree with packaging/runtime/RUNTIME.lock.json"
echo "  accepted: the checked-in copyright file is the one the lock file renders"

echo "check_copyright_test: every refusal fired"
