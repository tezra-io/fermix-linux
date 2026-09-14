#!/usr/bin/env bash
#
# Render the release page's install instructions from the checked-in template.
#
# One page has to name the exact commands for three package managers and two
# architectures, and be true for both of the states this release can be in:
# paired with a pinned engine release whose packages are re-attached beside ours,
# or not paired yet, in which case it says so rather than printing a command that
# names a file that is not there.
#
# Usage: render_install_notes.sh <version> <pinned|unpinned> [<engine-version>]
set -euo pipefail

USAGE="usage: render_install_notes.sh <version> <pinned|unpinned> [<engine-version>]"

VERSION="${1:?$USAGE}"
PIN_STATE="${2:?$USAGE}"
ENGINE_VERSION="${3:-}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEMPLATE="$ROOT_DIR/packaging/INSTALL.md.tmpl"
REPOSITORY="${GITHUB_REPOSITORY:-tezra-io/fermix-linux}"
TAG="fermix-desktop-v$VERSION"

fail() {
  echo "render_install_notes: $*" >&2
  exit 1
}

[ -f "$TEMPLATE" ] || fail "no template at $TEMPLATE"

case "$PIN_STATE" in
  pinned)
    [ -n "$ENGINE_VERSION" ] ||
      fail "a pinned release names an engine version, and none was given"
    ENGINE_NOTE="Both packages are on this page. The \`fermix\` packages are the ones engine release v$ENGINE_VERSION published, re-attached here after their digests and signatures were checked against the pin this release was built with."
    ENGINE_SIGNING_NOTE="The \`fermix\` packages carry the engine repository's own release-workflow signing identity rather than this one, because that is the workflow that built and signed them."
    ;;
  unpinned)
    [ -z "$ENGINE_VERSION" ] ||
      fail "an unpinned release names no engine version, and '$ENGINE_VERSION' was given"
    # Nothing on the page is a lie: the engine half is named as missing, and the
    # commands below are written against whatever engine version the person has
    # downloaded from the engine's own release page.
    ENGINE_NOTE="This release carries the window only. The engine release it is paired with is not pinned yet, so download the \`fermix\` packages from the engine's own release page and substitute their version below. The window declares an exact-version relation on the engine, so your package manager says so at once if the two do not match."
    ENGINE_SIGNING_NOTE="The \`fermix\` packages are not on this page, and they carry the engine repository's own signing identity rather than this one."
    ENGINE_VERSION="<engine version>"
    ;;
  *) fail "'$PIN_STATE' is neither pinned nor unpinned" ;;
esac

DESKTOP_IDENTITY="https://github.com/$REPOSITORY/.github/workflows/release-fermix-desktop.yml@refs/tags/$TAG"

python3 - "$TEMPLATE" \
  "VERSION=$VERSION" \
  "ENGINE_VERSION=$ENGINE_VERSION" \
  "ENGINE_NOTE=$ENGINE_NOTE" \
  "ENGINE_SIGNING_NOTE=$ENGINE_SIGNING_NOTE" \
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
