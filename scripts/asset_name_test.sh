#!/usr/bin/env bash
#
# Exercise asset_name.sh, which is the one place a built file name becomes a
# published asset name.
#
# The whole reason it exists is the `+` of a desktop-only rebuild (amendment
# section 6.4). Two things have to hold whatever that constant is set to: the
# publish side and the download side agree, and an ordinary `X.Y.Z` name is
# untouched. Both are checked here for both settings of the constant, so that
# the day the dry run answers the question the change is one line and this
# harness already covers it.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/asset_name.sh"

fail() {
  echo "asset_name_test: $*" >&2
  exit 1
}

# Each case runs in its own bash so that a changed constant never leaks into the
# next one.
name_with() {
  local plus="$1" file="$2"
  bash -c 'source "$1"; ASSET_PLUS="$2"; asset_name "$3"' asset_name_test \
    "$SCRIPT" "$plus" "$file"
}

pattern_with() {
  local plus="$1" file="$2"
  bash -c 'source "$1"; ASSET_PLUS="$2"; asset_pattern "$3"' asset_name_test \
    "$SCRIPT" "$plus" "$file"
}

echo "asset_name_test: the script parses"
bash -n "$SCRIPT" || fail "the script does not parse"
echo "  ok: shell syntax"

echo "asset_name_test: it refuses to be run rather than sourced"
if bash "$SCRIPT" >/dev/null 2>&1; then
  fail "running it instead of sourcing it was accepted"
fi
echo "  refused: running it instead of sourcing it"

echo "asset_name_test: refusals"
if bash -c 'source "$1"; asset_name' asset_name_test "$SCRIPT" >/dev/null 2>&1; then
  fail "a call with no file name was accepted"
fi
echo "  refused: a call with no file name"
if bash -c 'source "$1"; asset_pattern' asset_name_test "$SCRIPT" >/dev/null 2>&1; then
  fail "a pattern with no file name was accepted"
fi
echo "  refused: a pattern with no file name"

echo "asset_name_test: an ordinary version is untouched"
for plus in "+" "~plus~"; do
  for file in \
    "fermix-desktop_0.11.0_amd64.deb" \
    "fermix-desktop-0.11.0-1.x86_64.rpm" \
    "fermix-desktop_0.11.0_arm64.deb.sha256"; do
    [ "$(name_with "$plus" "$file")" = "$file" ] ||
      fail "$file changed under ASSET_PLUS=$plus"
  done
done
echo "  ok: X.Y.Z names are the same under either setting"

echo "asset_name_test: a directory is stripped, because a name is not a path"
[ "$(name_with "+" "packaging/out/fermix-desktop_0.11.0_amd64.deb")" \
  = "fermix-desktop_0.11.0_amd64.deb" ] ||
  fail "the directory was published as part of the name"
echo "  ok: packaging/out/ is not part of the asset name"

echo "asset_name_test: the plus, under the default"
for file in \
  "fermix-desktop_0.11.0+1_amd64.deb" \
  "fermix-desktop-0.11.0+1-1.x86_64.rpm"; do
  [ "$(name_with "+" "$file")" = "$file" ] ||
    fail "$file was rewritten while the default is to keep the plus"
done
echo "  ok: the published name is the built name"

echo "asset_name_test: the plus, under the fallback"
[ "$(name_with "~plus~" "fermix-desktop_0.11.0+1_amd64.deb")" \
  = "fermix-desktop_0.11.0~plus~1_amd64.deb" ] ||
  fail "the fallback does not substitute the plus"
[ "$(name_with "~plus~" "fermix-desktop-0.11.0+2-1.x86_64.rpm.sig")" \
  = "fermix-desktop-0.11.0~plus~2-1.x86_64.rpm.sig" ] ||
  fail "the fallback does not substitute the plus in a sidecar"
echo "  ok: the fallback rewrites the name and nothing else"

echo "asset_name_test: the two sides agree, whatever the constant is"
# This is the property the whole file exists for. A publish that wrote one name
# and a download that asked for another would fail at the only moment nobody is
# watching, which is a person following the release page.
for plus in "+" "~plus~"; do
  for file in \
    "fermix-desktop_0.11.0_amd64.deb" \
    "fermix-desktop_0.11.0+1_amd64.deb" \
    "fermix-desktop-0.11.0+1-1.aarch64.rpm.pem"; do
    [ "$(name_with "$plus" "$file")" = "$(pattern_with "$plus" "$file")" ] ||
      fail "the name and the pattern disagree for $file under ASSET_PLUS=$plus"
  done
done
echo "  ok: the name published and the pattern asked for are one string"

echo "asset_name_test: the checked-in default is the unproven one, and says so"
grep -q '^ASSET_PLUS="+"$' "$SCRIPT" ||
  fail "the default is no longer the plus, and docs/RELEASING.md says it is"
grep -q 'dry run' "$SCRIPT" ||
  fail "the file no longer says what would change the constant"
echo "  ok: the default keeps the plus, until the dry run says otherwise"

echo "asset_name_test: every refusal fired"
