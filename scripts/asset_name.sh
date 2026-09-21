#!/usr/bin/env bash
#
# shellcheck disable=SC2034
# The constant in this file is read by the scripts that source it, which is the
# whole reason it exists; shellcheck cannot see across that boundary.
#
# The one place a built file name becomes a published asset name, and back.
#
# A desktop-only rebuild is versioned `<engine version>+<n>` (amendment section
# 6.4), because with no exact-version relation between two packages left to keep
# equal, `+` is what both `dpkg --compare-versions` and `rpmdev-vercmp` order
# correctly and `release` can stay empty in both families. The character is
# correct in a Debian version, in an rpm version and in a git ref. Where it is
# not yet proven is a GitHub release asset name: GitHub rewrites some characters
# in an uploaded asset's name, and if `+` is one of them then the name the
# release page carries and the name a downloader asks for diverge, silently, in
# the one direction nobody tests.
#
# So every place that writes or asks for an asset name goes through here, and
# the answer to that question is one constant rather than a decision taken under
# release pressure. Until the dry run of docs/RELEASING.md answers it,
# `ASSET_PLUS` is `+`: the name is the file name. If the dry run shows GitHub
# rewriting it, `ASSET_PLUS` becomes `~plus~` and every caller changes with it,
# in one edit, with no change to the version a user sees or to the installed
# package.
#
# Usage:  source "$(dirname "$0")/asset_name.sh"
#         asset_name <file name>      -> the name to publish it under
#         asset_pattern <file name>   -> the `gh release download --pattern`
#                                        that asks for it back

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  echo "asset_name.sh: must be sourced from bash" >&2
  exit 1
fi

# What `+` becomes in a published asset name. See the note above: this is the
# one line that changes if the dry run finds GitHub rewriting the character.
ASSET_PLUS="+"

asset_name() {
  local file="${1:?asset_name: <file name> is required}"
  case "$file" in
    */*) file="${file##*/}" ;;
  esac
  case "$file" in
    "") echo "asset_name: an empty file name has no asset name" >&2; return 1 ;;
  esac
  printf '%s\n' "${file//+/$ASSET_PLUS}"
}

# The pattern is matched against the published name, so it is the published name
# with nothing else done to it. It is a separate function rather than the same
# one so that a caller asking for a file back reads as asking for a file back,
# and so that a future encoding rule has one place to live.
asset_pattern() {
  asset_name "${1:?asset_pattern: <file name> is required}"
}
