#!/usr/bin/env bash
#
# The copyright file agrees with the runtime lock file (amendment section 4.6).
#
# The package carries twenty-nine third-party libraries. Every one of them has a
# licence, most of them are LGPL, and the obligation is not satisfied by intent:
# a component added to `RUNTIME.lock.json` without its licence would ship with
# nothing anywhere saying what its terms are, and nothing at build time would
# notice.
#
# So `packaging/copyright` is generated rather than maintained, and this gate is
# what makes "generated" true: it regenerates the file from the lock file and the
# texts in `packaging/licenses/` and refuses any difference. A component with no
# licence text refuses here too, in the generator, by name.
#
# It reads three things and writes nothing, so it runs on any host with python3,
# in or out of the build container, before anything is built.
#
# Usage:
#   check_copyright.sh                       the repository's own files
#   check_copyright.sh --lock <path> --copyright <path> --licenses <dir>
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

LOCK="$ROOT_DIR/packaging/runtime/RUNTIME.lock.json"
COPYRIGHT="$ROOT_DIR/packaging/copyright"
LICENSES="$ROOT_DIR/packaging/licenses"
GENERATOR="$ROOT_DIR/scripts/generate_copyright.py"

fail() {
  echo "check_copyright: $*" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --lock)
      [ "$#" -ge 2 ] || fail "--lock needs a path"
      LOCK="$2"
      shift 2
      ;;
    --copyright)
      [ "$#" -ge 2 ] || fail "--copyright needs a path"
      COPYRIGHT="$2"
      shift 2
      ;;
    --licenses)
      [ "$#" -ge 2 ] || fail "--licenses needs a directory"
      LICENSES="$2"
      shift 2
      ;;
    *) fail "unknown argument: $1" ;;
  esac
done

[ -f "$LOCK" ] || fail "no runtime lock file at $LOCK"
[ -f "$COPYRIGHT" ] || fail "no copyright file at $COPYRIGHT"
[ -d "$LICENSES" ] || fail "no licence texts at $LICENSES"
[ -f "$GENERATOR" ] || fail "no generator at $GENERATOR"
command -v python3 >/dev/null 2>&1 || fail "python3 is not installed"

echo "check_copyright: the copyright file against the runtime lock file"
python3 "$GENERATOR" \
  --lock "$LOCK" \
  --licenses "$LICENSES" \
  --out "$COPYRIGHT" \
  --check ||
  fail "the copyright file and the lock file disagree"

# A licence text nothing names is a text that was added for a component that was
# then removed or renamed, and it is worth saying so: the next person reading
# packaging/licenses/ would otherwise take it for a component that ships.
python3 - "$LOCK" "$LICENSES" <<'PY' || exit 1
import json
import os
import sys

lock_path, licenses = sys.argv[1], sys.argv[2]

with open(lock_path, encoding="utf-8") as handle:
    lock = json.load(handle)

wanted = {"MIT.txt"}
wanted |= {
    component["license"].replace(" ", "_") + ".txt"
    for component in lock["components"]
}
present = {name for name in os.listdir(licenses) if name.endswith(".txt")}

stray = sorted(present - wanted)
if stray:
    print(
        "check_copyright: packaging/licenses carries texts no component declares: "
        + ", ".join(stray),
        file=sys.stderr,
    )
    sys.exit(1)

empty = sorted(
    name for name in present if os.path.getsize(os.path.join(licenses, name)) < 200
)
if empty:
    print(
        "check_copyright: these licence texts are too short to be a licence: "
        + ", ".join(empty),
        file=sys.stderr,
    )
    sys.exit(1)

print(f"  {len(present)} licence texts, every one of them named by a component")
PY

echo "check_copyright: every bundled component is declared with its licence"
