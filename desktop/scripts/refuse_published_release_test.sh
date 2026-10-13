#!/usr/bin/env bash
# Offline tests for refuse_published_release.sh against a stub gh: a tag with no published release
# passes, a published one is refused, and every answer it cannot read is refused rather than taken
# for "not published". Nothing here reaches the network.
#   desktop/scripts/refuse_published_release_test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
REPO=tezra-io/fermix-linux
TAG=v0.14.0

fail() {
  echo "refuse_published_release_test: $*" >&2
  exit 1
}

# A gh that answers `gh api <path>` from $STUB_STATUS and $STUB_BODY, and logs what it was asked.
mkdir -p "$work/bin"
cat > "$work/bin/gh" << 'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$STUB_LOG"
[ "$1" = api ] || { echo "stub gh: only api is served" >&2; exit 2; }
case "$STUB_STATUS" in
  200) printf '%s\n' "$STUB_BODY" ;;
  404) echo "gh: Not Found (HTTP 404)" >&2; exit 1 ;;
  *) echo "gh: Server Error (HTTP $STUB_STATUS)" >&2; exit 1 ;;
esac
STUB
chmod +x "$work/bin/gh"
export PATH="$work/bin:$PATH" STUB_LOG="$work/gh.log"

run() {
  STUB_STATUS="$1" STUB_BODY="$2" "$here/refuse_published_release.sh" "${@:3}"
}

expect_pass() {
  local what="$1"
  shift
  run "$@" > /dev/null 2> "$work/stderr" || fail "$what was refused: $(cat "$work/stderr")"
  echo "  passed: $what"
}

expect_refusal() {
  local what="$1" reason="$2"
  shift 2
  if run "$@" > /dev/null 2> "$work/stderr"; then
    fail "$what was accepted"
  fi
  grep -qF -- "$reason" "$work/stderr" || fail "$what was refused for another reason: $(cat "$work/stderr")"
  echo "  refused: $what"
}

echo "refuse_published_release_test: what may be released"
expect_pass "a tag with no published release" 404 "" "$REPO" "$TAG"
grep -qxF "api repos/$REPO/releases/tags/$TAG" "$STUB_LOG" || fail "gh was not asked for the tag's release: $(cat "$STUB_LOG")"
expect_pass "a desktop-only rebuild's tag" 404 "" "$REPO" v0.14.0+1
expect_pass "a tag whose release is still a draft" 200 '{"draft": true}' "$REPO" "$TAG"

echo "refuse_published_release_test: what may not"
expect_refusal "a published release" "already published" 200 '{"draft": false}' "$REPO" "$TAG"
expect_refusal "a server error" "cannot tell" 502 "" "$REPO" "$TAG"
! grep -qF "already published" "$work/stderr" || fail "a server error was reported as a published release"
expect_refusal "an answer with no draft field" "cannot tell" 200 '{"name": "x"}' "$REPO" "$TAG"
expect_refusal "an answer that is not JSON" "cannot tell" 200 'not json' "$REPO" "$TAG"
expect_refusal "a tag with another prefix" "not a release tag" 404 "" "$REPO" fermix-desktop-v0.14.0
expect_refusal "a prerelease tag" "not a release tag" 404 "" "$REPO" v0.14.0-rc.1
expect_refusal "a tag with no patch number" "not a release tag" 404 "" "$REPO" v0.14
expect_refusal "a repository that is not owner/name" "owner/name" 404 "" "fermix-linux" "$TAG"
expect_refusal "a missing tag" "usage" 404 "" "$REPO"

echo "refuse_published_release_test: ok"
