#!/usr/bin/env bash
#
# Render the release page's install instructions from the checked-in template.
#
# One page has to name the exact commands for three package managers and two
# architectures, and be true for the one state this release can now be in. There
# is no longer an unpinned state to render: the package carries the engine, so a
# release with no pin is not built at all and this page never has to say that
# half of the product is missing.
#
# Usage: render_install_notes.sh <version>
#   <version>  X.Y.Z, or X.Y.Z+N for a desktop-only rebuild
set -euo pipefail

USAGE="usage: render_install_notes.sh <version>"

VERSION="${1:?$USAGE}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEMPLATE="$ROOT_DIR/packaging/INSTALL.md.tmpl"
REPOSITORY="${GITHUB_REPOSITORY:-tezra-io/fermix-linux}"

fail() {
  echo "render_install_notes: $*" >&2
  exit 1
}

[ "$#" -eq 1 ] || fail "$USAGE"
[ -f "$TEMPLATE" ] || fail "no template at $TEMPLATE"

# The same shape the release workflow refuses anything else for. A page naming a
# version neither family can carry would be a page telling somebody to type a
# command that cannot work.
printf '%s' "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(\+[0-9]+)?$' ||
  fail "'$VERSION' is neither X.Y.Z nor X.Y.Z+N"

TAG="fermix-desktop-v$VERSION"
DESKTOP_IDENTITY="https://github.com/$REPOSITORY/.github/workflows/release-fermix-desktop.yml@refs/tags/$TAG"

python3 - "$TEMPLATE" \
  "VERSION=$VERSION" \
  "TAG=$TAG" \
  "DESKTOP_IDENTITY=$DESKTOP_IDENTITY" <<'PY'
import re
import sys

template, *pairs = sys.argv[1:]
values = dict(pair.split("=", 1) for pair in pairs)

with open(template, encoding="utf-8") as handle:
    body = handle.read()

missing = set()


def fill(match):
    name = match.group(1)
    if name not in values:
        missing.add(name)
        return match.group(0)
    return values[name]


body = re.sub(r"\{\{([A-Z_]+)\}\}", fill, body)
if missing:
    sys.exit(
        "render_install_notes: the template carries placeholders nothing fills: "
        + ", ".join(sorted(missing))
    )

sys.stdout.write(body)
PY
