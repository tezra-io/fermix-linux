#!/usr/bin/env bash
#
# Exercise render_install_notes.sh, which writes the only page a person reads
# before typing a command as root.
#
# A renderer that left a placeholder behind, or that named a version neither
# package manager can carry, would put a wrong command in front of somebody with
# sudo. The page also names one package and no longer two, which is the change
# this amendment makes to the thing a person actually does.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/render_install_notes.sh"

fail() {
  echo "render_install_notes_test: $*" >&2
  exit 1
}

expect_refusal() {
  local what="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    fail "$what was accepted"
  fi
  echo "  refused: $what"
}

echo "render_install_notes_test: the script parses"
bash -n "$SCRIPT" || fail "the script does not parse"
echo "  ok: shell syntax"

echo "render_install_notes_test: refusals"
expect_refusal "no arguments at all" bash "$SCRIPT"
expect_refusal "a version that is not a version" bash "$SCRIPT" "latest"
expect_refusal "a version carrying a Debian revision" bash "$SCRIPT" "0.11.0-1"
expect_refusal "a version carrying an rpm epoch" bash "$SCRIPT" "1:0.11.0"
expect_refusal "a prerelease version" bash "$SCRIPT" "0.11.0-rc1"
# The two-argument shape belonged to the pinned-or-unpinned world, and a page
# rendered from a stale caller would silently ignore the second argument.
expect_refusal "the old pin state argument" bash "$SCRIPT" 0.11.0 pinned

echo "render_install_notes_test: the page"
page="$(bash "$SCRIPT" 0.11.0)"
for needed in \
  "sudo apt install ./fermix-desktop_0.11.0_" \
  "sudo dnf install ./fermix-desktop-0.11.0-1." \
  "sudo zypper install" \
  "cosign verify-blob" \
  "fermix-desktop-v0.11.0"; do
  case "$page" in
    *"$needed"*) ;;
    *) fail "the page does not carry '$needed'" ;;
  esac
done
case "$page" in
  *"{{"*) fail "the page still carries a placeholder" ;;
esac
echo "  ok: both families, the signing identity, and nothing left unfilled"

echo "render_install_notes_test: it is one package"
# The window carries the engine, so the page must never tell somebody to install
# two things or to match two versions.
for gone in "install ./fermix_" "exact-version" "both packages"; do
  case "$page" in
    *"$gone"*) fail "the page still names two packages: $gone" ;;
  esac
done
case "$page" in
  *"On a machine with no desktop"*) ;;
  *) fail "the page does not say what a headless host installs instead" ;;
esac
echo "  ok: one install command per family, and the headless alternative named"

echo "render_install_notes_test: a desktop-only rebuild"
# `+N` is a version the release rail accepts (amendment section 6.4), so the
# page has to render it rather than refusing at the last step of a release.
rebuild="$(bash "$SCRIPT" 0.11.0+1)"
case "$rebuild" in
  *"fermix-desktop_0.11.0+1_"*) ;;
  *) fail "the page does not carry the deb of a desktop-only rebuild" ;;
esac
case "$rebuild" in
  *"fermix-desktop-v0.11.0+1"*) ;;
  *) fail "the signing identity does not carry the rebuild tag" ;;
esac
echo "  ok: 0.11.0+1 renders, in both families and in the identity"

echo "render_install_notes_test: what the page never does"
# One proper noun carries an exclamation mark, and it is the distribution's own
# name rather than this page's voice, so it is taken out before the check the
# way the copy catalogue keeps its own hand-listed proper nouns.
for body in "$page" "$rebuild"; do
  body="${body//Pop!_OS/Pop_OS}"
  for forbidden in '—' '!'; do
    case "$body" in
      *"$forbidden"*) fail "the page carries $forbidden" ;;
    esac
  done
done
echo "  ok: no em dash, and no exclamation mark outside one distribution's own name"

echo "render_install_notes_test: every refusal fired"
