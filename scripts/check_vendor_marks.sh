#!/usr/bin/env bash
#
# The vendor mark gate (M38 section 12.2).
#
# Marks ship byte for byte from the vendors' own brand kits with a provenance
# record beside them, and nothing is ever redrawn, recoloured or invented. This
# runs the offline check: every recorded hash against the bytes on disk, every
# file's format against its name, the roster against the record, the record
# against the section inventory the vendored fixtures publish, and every mark
# against the resource bundle the application draws it from.
#
# The plugin roster's upstream half needs a fermix checkout, which a runner
# does not have, so it is opt-in and names itself when it is skipped:
#
#   scripts/check_vendor_marks.sh --fermix-repo ../fermix
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v python3 >/dev/null 2>&1; then
  echo "check_vendor_marks: python3 is required and is not on PATH" >&2
  exit 1
fi

exec python3 "$ROOT_DIR/scripts/vendor_marks.py" --root "$ROOT_DIR" "$@"
