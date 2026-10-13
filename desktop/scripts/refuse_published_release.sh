#!/usr/bin/env bash
#
# Refuses a release tag whose GitHub release is already published. A tag is vX.Y.Z, the macOS
# app's form, where X.Y.Z is the app's version; there is no rebuild suffix, since every change to
# what a release ships is a new X.Y.Z.
#
#   refuse_published_release.sh <owner/name> <vX.Y.Z>
#
# The assets a person has downloaded are the release, so a published release is never refilled:
# a fix is the next version. A tag with no release, or with a draft, passes. Any answer that does
# not say which is refused, because taking an error for "not published" would let a release run
# replace published assets. GitHub's release-by-tag lookup serves published releases only; a draft
# it does return passes too. Asks through gh, which reads GH_TOKEN.
set -euo pipefail
shopt -s inherit_errexit

fail() {
  echo "refuse_published_release: $*" >&2
  exit 1
}

# Prints "published" or "unpublished" for the tag, or fails when GitHub's answer says neither.
release_state() {
  local repository="$1" tag="$2" body err draft
  err="$(mktemp)"
  if ! body="$(gh api "repos/$repository/releases/tags/$tag" 2> "$err")"; then
    if grep -qF "HTTP 404" "$err"; then
      rm -f -- "$err"
      echo unpublished
      return 0
    fi
    fail "cannot tell whether $tag is published: $(cat "$err"; rm -f -- "$err")"
  fi
  rm -f -- "$err"
  draft="$(jq -er '.draft | if type == "boolean" then tostring else error("no draft field") end' \
    <<< "$body" 2> /dev/null)" || fail "cannot tell whether $tag is published: GitHub's answer has no draft field"
  if [ "$draft" = true ]; then
    echo unpublished
  else
    echo published
  fi
}

main() {
  [ $# -eq 2 ] || fail "usage: refuse_published_release.sh <owner/name> <vX.Y.Z>"
  local repository="$1" tag="$2" state
  [[ "$repository" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || fail "$repository is not owner/name"
  [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
    fail "$tag is not a release tag, vX.Y.Z"
  # Assigned first, so a state it could not read stops here rather than reading as "published".
  state="$(release_state "$repository" "$tag")"
  [ "$state" = unpublished ] ||
    fail "the $tag release is already published; publish the fix as the next version"
  echo "refuse_published_release: $tag has no published release"
}

main "$@"
