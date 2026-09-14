#!/usr/bin/env bash
#
# The copy gates, as a shell-visible check.
#
# The gates themselves live in `App/Fermix/tests/copy.rs`, because they read the
# catalogue through the same renderer the interface uses rather than through a
# grep of the source. This script is what CI, a reviewer and a person in a hurry
# run, and what fails loudly when the deck drifts.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE="${FERMIX_CRATE_DIR:-$ROOT_DIR/App/Fermix}"

[ -f "$CRATE/Cargo.toml" ] || {
  echo "check_copy: no crate at $CRATE" >&2
  exit 1
}

echo "check_copy: the catalogue, its casing column and its forbidden substrings"
cargo test --quiet --manifest-path "$CRATE/Cargo.toml" --test copy

echo "check_copy: no word a surface shows is a literal"
cargo test --quiet --manifest-path "$CRATE/Cargo.toml" --test structure

echo "check_copy: the copy gates pass"
