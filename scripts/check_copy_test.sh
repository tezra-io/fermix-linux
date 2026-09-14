#!/usr/bin/env bash
#
# Exercise check_copy.sh's refusals against a throwaway copy of the crate.
#
# It edits the catalogue in the copy, never in the repository: a forbidden
# substring, a header row ending in a period and a key nothing renders each have
# to be seen failing, because all three are the kind of drift a reviewer reads
# straight past.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE="$ROOT_DIR/App/Fermix"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/check-copy-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

# One build directory for every copy, kept across runs: the dependencies are
# compiled once and each variant recompiles only the crate whose catalogue it
# edited. An outer setting wins, so the container's own cache is used there.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${TMPDIR:-/tmp}/fermix-copy-gate-target}"

fail() {
  echo "check_copy_test: $*" >&2
  exit 1
}

copy_crate() {
  local into="$WORK/$1"
  mkdir -p "$into/App"
  # Everything but the build directory, which is large and rebuilt anyway.
  (cd "$CRATE/.." && tar -cf - --exclude target Fermix) | (cd "$into/App" && tar -xf -)
  mkdir -p "$into/scripts"
  cp "$ROOT_DIR/scripts/check_copy.sh" "$into/scripts/"
  echo "$into"
}

expect_refusal() {
  local what="$1" repo="$2"
  if bash "$repo/scripts/check_copy.sh" >/dev/null 2>&1; then
    fail "$what was accepted"
  fi
  echo "  refused: $what"
}

echo "check_copy_test: the catalogue as it stands"
bash "$ROOT_DIR/scripts/check_copy.sh" >/dev/null || fail "the real catalogue does not pass"
echo "  accepted: the catalogue"

echo "check_copy_test: refusals"

repo="$(copy_crate exclamation)"
python3 - "$repo/App/Fermix/src/copy.rs" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
body = path.read_text()
path.write_text(body.replace('"Nothing needs your attention"', '"Nothing needs your attention!"'))
PY
expect_refusal "an exclamation mark in the catalogue" "$repo"

repo="$(copy_crate header-period)"
python3 - "$repo/App/Fermix/src/copy.rs" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
body = path.read_text()
path.write_text(body.replace('row(Key::MenuQuit, "Quit", Casing::Header)',
                             'row(Key::MenuQuit, "Quit.", Casing::Header)'))
PY
expect_refusal "a header row ending in a period" "$repo"

repo="$(copy_crate title-cased-sentence)"
python3 - "$repo/App/Fermix/src/copy.rs" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
body = path.read_text()
path.write_text(body.replace('"Nothing needs your attention"', '"Nothing Needs Your Attention"'))
PY
expect_refusal "a sentence row written in Header Capitalization" "$repo"

repo="$(copy_crate env-var)"
python3 - "$repo/App/Fermix/src/copy.rs" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
body = path.read_text()
path.write_text(body.replace('"Fermix writes this log to {path}"',
                             '"Fermix writes this log under FERMIX_HOME"'))
PY
expect_refusal "an environment variable name reaching a person" "$repo"

repo="$(copy_crate stale-pending)"
python3 - "$repo/App/Fermix/tests/fixtures/copy/keys_awaiting_a_surface.txt" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
lines = [line for line in path.read_text().splitlines() if line.strip() != "PageDoctor"]
lines.append("PageHome")
path.write_text("\n".join(lines) + "\n")
PY
expect_refusal "a key that renders now still listed as awaiting a surface" "$repo"

repo="$(copy_crate literal-in-a-surface)"
python3 - "$repo/App/Fermix/src/window.rs" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
body = path.read_text()
path.write_text(body.replace('.title(copy::text(destination.title))',
                             '.title("Home")', 1))
PY
expect_refusal "a word a surface shows written as a literal" "$repo"

echo "check_copy_test: every refusal fired"
